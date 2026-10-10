//! Writer for CalculiX input files, following the layout of PrePoMax's `CalculixFileWriter`.
//!
//! The mesh is written together with the analysis of a [`FeModel`]. Regions the user picked in
//! the GUI become node sets, element sets and surfaces named like PrePoMax's internal
//! selections (`Internal_Selection-1_Fixed-1`), so the user never defines sets by hand. The
//! file is built as a [`Keyword`] tree first, which is where the keyword editor inserts the
//! user's own keywords.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use plx_mesh::{ElementFamily, ElementId, FeMesh, NodeId, SurfaceDefinition};
use plx_model::{
    Amplitude, AmplitudeTime, BoundaryKind, BuckleStep, Constraint, ContactMethod, ContactPair,
    DynamicStep, FeModel, FieldOutput, FrequencyStep, GapConductance, HeatTransferStep,
    HistoryKind, HistoryOutput, Incrementation, InitialConditionKind, InteractionProperty,
    LoadKind, ModalDamping, ModalDynamicsStep, ModelSpace, NodeTie, OutputKind, Region, Section,
    SectionKind, StaticStep, SteadyStateDynamicsStep, Step, StepKind, SurfaceBehavior,
    SurfaceInteraction, Totals, UserKeyword, line_tangent,
};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WriteError {
    #[error("{item}: Bereich enthält keine {what}")]
    EmptyRegion { item: String, what: &'static str },
    #[error("{item}: Material {material} existiert nicht")]
    UnknownMaterial { item: String, material: String },
    #[error("{item}: Surface Interaction {interaction} existiert nicht")]
    UnknownInteraction { item: String, interaction: String },
    #[error("{item}: Amplitude {amplitude} existiert nicht")]
    UnknownAmplitude { item: String, amplitude: String },
    #[error("{item}: {reason}")]
    InvalidAmplitude { item: String, reason: String },
    #[error("{item}: Surface {surface} existiert nicht")]
    UnknownSurface { item: String, surface: String },
    /// Contact forces need the surfaces of an active contact pair.
    #[error("{item}: Contact Pair {pair} existiert nicht oder ist deaktiviert")]
    UnknownContactPair { item: String, pair: String },
    /// A section that does not fit its elements, see [`Section::kind_problem`].
    #[error("{item}: {reason}")]
    InvalidSection { item: String, reason: String },
    /// A load without a direction, such as a gravity of zero.
    #[error("{item}: {reason}")]
    InvalidLoad { item: String, reason: String },
    /// A submodel boundary condition in a model that names no global results file.
    #[error(
        "{item}: the submodel has no global results file (Model > Model Properties: model \
         type Submodel)"
    )]
    NoGlobalResults { item: String },
}

/// One entry of the keyword tree of an input file, the structure PrePoMax's keyword editor
/// shows: section titles hold the keywords written under them, a step holds its loads and
/// outputs. The text of the file is the tree written depth first.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyword {
    pub kind: KeywordKind,
    /// The lines of a generated keyword, each ending in a newline, or the user's text.
    pub text: String,
    pub children: Vec<Keyword>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeywordKind {
    /// Section banner such as `** Materials ++++`; the text is empty.
    Title(String),
    /// Written from the mesh and the FE model.
    Generated,
    /// Added by the user; written commented out when inactive.
    User { active: bool },
}

impl Keyword {
    fn title(name: &str, children: Vec<Keyword>) -> Self {
        Self {
            kind: KeywordKind::Title(name.to_owned()),
            text: String::new(),
            children,
        }
    }

    fn generated(text: String) -> Self {
        Self::parent(text, Vec::new())
    }

    fn parent(text: String, children: Vec<Keyword>) -> Self {
        Self {
            kind: KeywordKind::Generated,
            text,
            children,
        }
    }

    pub fn user(keyword: &UserKeyword) -> Self {
        Self {
            kind: KeywordKind::User {
                active: keyword.active,
            },
            text: keyword.text.clone(),
            children: Vec::new(),
        }
    }

    /// What this keyword adds to the file, without its children.
    pub fn output(&self) -> String {
        match &self.kind {
            KeywordKind::Title(name) => format!("**\n{:+<60}\n**\n", format!("** {name} ")),
            KeywordKind::Generated => self.text.clone(),
            KeywordKind::User { active } => {
                let mut out = String::new();
                for line in self.text.lines() {
                    if !active {
                        out.push_str("** ");
                    }
                    out.push_str(line);
                    out.push('\n');
                }
                out
            }
        }
    }
}

/// Writes the mesh and the analysis as the text of an input file, with the user's keywords.
pub fn write_inp(mesh: &FeMesh, model: &FeModel, heading: &str) -> Result<String, WriteError> {
    let mut tree = model_keywords(mesh, model, heading)?;
    insert_user_keywords(&mut tree, &model.user_keywords);
    Ok(write_keywords(&tree))
}

/// An input file with only the mesh, its nodes moved by `scale` times `displacements` (one per
/// node, in the order of [`FeMesh::coords`]), as PrePoMax's "Export deformed mesh": the
/// deformed shape, e.g. a buckling mode as imperfection, can be imported into another model.
pub fn write_deformed_mesh_inp(
    mesh: &FeMesh,
    displacements: &[[f32; 3]],
    scale: f64,
    heading: &str,
) -> Result<String, WriteError> {
    let mut deformed = mesh.clone();
    for ((&id, coords), u) in mesh.node_ids().iter().zip(mesh.coords()).zip(displacements) {
        let moved = std::array::from_fn(|k| coords[k] + scale * f64::from(u[k]));
        deformed.set_node(id, moved);
    }
    write_inp(&deformed, &FeModel::default(), heading)
}

/// The input file for PrePoMax's "Check Model": every step's procedure is replaced by
/// `*No analysis`, so that CalculiX only reads and checks the model. A model without active
/// steps gets PrePoMax's `CheckModel` step.
pub fn write_check_inp(
    mesh: &FeMesh,
    model: &FeModel,
    heading: &str,
) -> Result<String, WriteError> {
    let mut tree = model_keywords(mesh, model, heading)?;
    let no_analysis = || Keyword::generated("*No analysis\n".into());
    if let Some(steps) = tree
        .iter_mut()
        .find(|k| matches!(&k.kind, KeywordKind::Title(name) if name == "Steps"))
    {
        // A deactivated step stays a comment.
        for (step, _) in (steps.children.iter_mut().zip(&model.steps)).filter(|(_, s)| s.active) {
            if let Some(procedure) =
                (step.children.first_mut()).and_then(|header| header.children.first_mut())
            {
                *procedure = no_analysis();
            }
        }
        if !model.steps.iter().any(|s| s.active) {
            let end = Keyword::title("End step", vec![Keyword::generated("*End step\n".into())]);
            let header = Keyword::parent("*Step\n".into(), vec![no_analysis(), end]);
            steps
                .children
                .push(Keyword::title("CheckModel", vec![header]));
        }
    }
    insert_user_keywords(&mut tree, &model.user_keywords);
    Ok(write_keywords(&tree))
}

/// The text of a keyword tree.
pub fn write_keywords(tree: &[Keyword]) -> String {
    fn write(out: &mut String, keyword: &Keyword) {
        out.push_str(&keyword.output());
        for child in &keyword.children {
            write(out, child);
        }
    }
    let mut out = String::new();
    for keyword in tree {
        write(&mut out, keyword);
    }
    out
}

/// Inserts the user keywords at their positions, the way PrePoMax's
/// `AddUserKeywordByIndices` does. Returns whether each one found its place; a keyword whose
/// place no longer exists because the model changed is left out.
pub fn insert_user_keywords(tree: &mut Vec<Keyword>, keywords: &[UserKeyword]) -> Vec<bool> {
    keywords
        .iter()
        .map(|keyword| {
            let Some((&index, parents)) = keyword.position.split_last() else {
                return false;
            };
            let mut siblings = &mut *tree;
            for &parent in parents {
                match siblings.get_mut(parent) {
                    Some(p) if !matches!(p.kind, KeywordKind::User { .. }) => {
                        siblings = &mut p.children
                    }
                    _ => return false,
                }
            }
            if index > siblings.len() {
                return false;
            }
            siblings.insert(index, Keyword::user(keyword));
            true
        })
        .collect()
}

/// The user keywords of a tree with their positions, in the order they are inserted again.
pub fn user_keywords(tree: &[Keyword]) -> Vec<UserKeyword> {
    fn collect(siblings: &[Keyword], path: &mut Vec<usize>, out: &mut Vec<UserKeyword>) {
        for (index, keyword) in siblings.iter().enumerate() {
            path.push(index);
            if let KeywordKind::User { active } = keyword.kind {
                out.push(UserKeyword {
                    position: path.clone(),
                    text: keyword.text.clone(),
                    active,
                });
            }
            collect(&keyword.children, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    collect(tree, &mut Vec::new(), &mut out);
    out
}

/// The keyword tree written from mesh and model, without user keywords. Like PrePoMax it
/// always has every section title, so the user can add keywords under any of them.
pub fn model_keywords(
    mesh: &FeMesh,
    model: &FeModel,
    heading: &str,
) -> Result<Vec<Keyword>, WriteError> {
    // The faces of 2D elements are their edges; a mesh whose surface elements are still
    // typed as shells is written as the model space types them.
    let space = model.properties.space;
    let retyped;
    let mesh = if (mesh.elements().iter())
        .any(|e| space.element_type(&e.type_name, e.shape) != e.type_name)
    {
        let mut copy = mesh.clone();
        space.convert_mesh(&mut copy);
        retyped = copy;
        &retyped
    } else {
        mesh
    };
    let lines = LineElements::new(mesh, model)?;
    // Node ties of beams, and of anything without rotations, are written as one shared node.
    let merged_nodes = node_merges(mesh, model, &lines);
    let (merged_mesh, merged_model);
    let (mesh, model) = if merged_nodes.is_empty() {
        (mesh, model)
    } else {
        let mut copy = mesh.clone();
        copy.merge_nodes(&merged_nodes);
        merged_mesh = copy;
        let mut copy = model.clone();
        copy.merge_nodes(&merged_nodes);
        merged_model = copy;
        (&merged_mesh, &merged_model)
    };
    let mut sets = Sets::new(mesh);
    let lines = LineElements::new(mesh, model)?;
    // Regions are resolved first: their sets must precede the materials and steps.
    let materials = materials(model);
    let mut sections = sections(&mut sets, model)?;
    let generated = constraints::springs(&mut sets, model)?;
    let mut constraints = constraints(&mut sets, model)?;
    constraints.extend(generated.equations);
    sections.extend(generated.sections);
    let mut materials = materials;
    materials.extend(generated.material);
    let interactions = model.surface_interactions.iter().map(interaction).collect();
    let (mut contact_pairs, pair_surfaces) = contact_pairs(&mut sets, model)?;
    contact_pairs.extend(node_ties(&mut sets, model, &lines)?);
    let initial_conditions = initial_conditions(&mut sets, model)?;
    let amplitudes = amplitudes(model)?;
    let flux_kinds = FluxKinds::of(model);
    let steps = model
        .steps
        .iter()
        .map(|step| {
            let context = StepContext {
                extra_boundary: generated.boundary.as_ref(),
                space,
                lines: &lines,
                flux_kinds,
                amplitudes: &model.amplitudes,
                pair_surfaces: &pair_surfaces,
            };
            write_step(&mut sets, step, &context)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut nodes = String::from("*Node\n");
    for (id, &[x, y, z]) in mesh.node_ids().iter().zip(mesh.coords()) {
        let _ = writeln!(nodes, "{id}, {:.8E}, {:.8E}, {:.8E}", x, y, z);
    }
    for (id, [x, y, z]) in &generated.nodes {
        let _ = writeln!(nodes, "{id}, {:.8E}, {:.8E}, {:.8E}", x, y, z);
    }
    let mut element_blocks = elements(mesh, space, &lines);
    element_blocks.extend(generated.elements);
    let node_sets = sets
        .node_sets
        .iter()
        .map(|(name, ids)| Keyword::generated(id_list(&format!("*Nset, Nset={name}"), ids)))
        .collect();
    let element_sets = sets
        .element_sets
        .iter()
        .map(|(name, members)| {
            let header = format!("*Elset, Elset={name}");
            Keyword::generated(match members {
                Members::Ids(ids) => id_list(&header, ids),
                Members::Parts(parts) => format!("{header}\n{}\n", parts.join(",\n")),
            })
        })
        .collect();
    let surfaces = sets
        .surfaces
        .iter()
        .map(|(name, surface)| {
            Keyword::generated(match surface {
                Surface::Faces(faces) => {
                    let mut out = format!("*Surface, Name={name}, Type=Element\n");
                    for (set, face) in faces {
                        let _ = writeln!(out, "{set}, S{face}");
                    }
                    out
                }
                Surface::Nodes(set) => format!("*Surface, Name={name}, Type=Node\n{set}\n"),
            })
        })
        .collect();
    let heading = format!("*Heading\n{}\n", heading.lines().next().unwrap_or_default());
    let empty = |name| Keyword::title(name, Vec::new());
    let mut tree = vec![Keyword::title("Heading", vec![Keyword::generated(heading)])];
    if let Some(submodel) = submodel(model, &sets.submodel_sets)? {
        tree.push(Keyword::title("Submodel", vec![submodel]));
    }
    tree.extend([
        Keyword::title("Nodes", vec![Keyword::generated(nodes)]),
        Keyword::title("Elements", element_blocks),
        Keyword::title("Node sets", node_sets),
        Keyword::title("Element sets", element_sets),
        Keyword::title("Surfaces", surfaces),
        Keyword::title("Physical constants", physical_constants(model)),
        empty("Coordinate systems"),
        Keyword::title("Materials", materials),
        Keyword::title("Sections", sections),
        empty("Pre-tension sections"),
        Keyword::title("Constraints", constraints),
        Keyword::title("Surface interactions", interactions),
        Keyword::title("Contact pairs", contact_pairs),
        Keyword::title("Amplitudes", amplitudes),
        Keyword::title("Initial conditions", initial_conditions),
        Keyword::title("Steps", steps),
    ]);
    Ok(tree)
}

/// `*SUBMODEL` with the node sets of the submodel boundary conditions, as PrePoMax writes it
/// after the heading. The global results file is named without its directory; it has to be
/// next to the input file, where the analysis and the export put it.
fn submodel(
    model: &FeModel,
    node_sets: &[(String, String)],
) -> Result<Option<Keyword>, WriteError> {
    let Some((_, first)) = node_sets.first() else {
        return Ok(None);
    };
    let input = (model.properties.submodel_input())
        .and_then(|path| path.file_name())
        .ok_or_else(|| WriteError::NoGlobalResults {
            item: first.clone(),
        })?;
    let mut out = format!(
        "*Submodel, Type=Node, Input=\"{}\"\n",
        input.to_string_lossy()
    );
    for (set, _) in node_sets {
        let _ = writeln!(out, "{set}");
    }
    Ok(Some(Keyword::generated(out)))
}

/// The line elements of beam and truss sections: the CalculiX type each one is written
/// with, and the nodes that have no rotations because they belong to trusses only.
struct LineElements {
    types: BTreeMap<ElementId, &'static str>,
    truss_nodes: BTreeSet<NodeId>,
    /// Nodes of beams, which have rotations.
    beam_nodes: BTreeSet<NodeId>,
}

impl LineElements {
    fn new(mesh: &FeMesh, model: &FeModel) -> Result<Self, WriteError> {
        let mut types = BTreeMap::new();
        for section in &model.sections {
            if let Some(reason) = section.kind_problem(mesh) {
                return Err(WriteError::InvalidSection {
                    item: section.name.clone(),
                    reason,
                });
            }
            if !section.kind.is_line() {
                continue;
            }
            for element in
                (section.region.elements(mesh).into_iter()).filter_map(|id| mesh.element(id))
            {
                if let Some(type_name) = section.kind.element_type(element.shape) {
                    types.insert(element.id, type_name);
                }
            }
        }
        // A node of a truss has translations only; a beam or solid at the same node adds
        // its rotations back.
        let mut truss_nodes = BTreeSet::new();
        let mut other_nodes = BTreeSet::new();
        let mut beam_nodes = BTreeSet::new();
        for element in mesh.elements() {
            let type_name = types.get(&element.id).copied();
            let nodes = if type_name == Some("T3D2") {
                &mut truss_nodes
            } else {
                &mut other_nodes
            };
            nodes.extend(element.nodes.iter().copied());
            if type_name.is_some_and(|t| t.starts_with('B')) {
                beam_nodes.extend(element.nodes.iter().copied());
            }
        }
        truss_nodes.retain(|n| !other_nodes.contains(n));
        Ok(Self {
            types,
            truss_nodes,
            beam_nodes,
        })
    }

    /// Type and nodes an element is written with. A truss is always `T3D2`, so a 3-node line
    /// keeps its end nodes only; see [`SectionKind::element_type`].
    fn written(&self, element: &plx_mesh::Element, space: ModelSpace) -> (String, Vec<NodeId>) {
        match self.types.get(&element.id) {
            Some(&"T3D2") if element.nodes.len() == 3 => {
                ("T3D2".into(), vec![element.nodes[0], element.nodes[2]])
            }
            Some(&type_name) => (type_name.into(), element.nodes.clone()),
            None => (
                space.element_type(&element.type_name, element.shape),
                element.nodes.clone(),
            ),
        }
    }

    /// Whether every node of the list belongs to trusses only.
    fn all_truss(&self, nodes: &[NodeId]) -> bool {
        !nodes.is_empty() && nodes.iter().all(|n| self.truss_nodes.contains(n))
    }
}

/// One `*Element` block per part and element type; the part name is the element set.
/// Surface elements get the type of the model space, e.g. `CAX6` in an axisymmetric model,
/// line elements the type of their beam or truss section.
fn elements(mesh: &FeMesh, space: ModelSpace, lines: &LineElements) -> Vec<Keyword> {
    let mut written = vec![false; mesh.element_count()];
    let mut groups: Vec<(Option<&str>, String, Vec<ElementId>)> = Vec::new();
    let type_name = |element: &plx_mesh::Element| lines.written(element, space).0;
    for part in &mesh.parts {
        for &id in &part.elements {
            let Some(index) = mesh.element_index(id) else {
                continue;
            };
            if !std::mem::replace(&mut written[index], true) {
                let type_name = type_name(&mesh.elements()[index]);
                group(&mut groups, Some(&part.name), type_name).push(id);
            }
        }
    }
    for (element, done) in mesh.elements().iter().zip(&written) {
        if !done {
            group(&mut groups, None, type_name(element)).push(element.id);
        }
    }
    let mut blocks = Vec::new();
    for (set, type_name, mut ids) in groups {
        let mut out = match set {
            Some(set) => format!("*Element, Type={type_name}, Elset={set}\n"),
            None => format!("*Element, Type={type_name}\n"),
        };
        ids.sort_unstable();
        for element in ids.iter().filter_map(|&id| mesh.element(id)) {
            let _ = write!(out, "{}", element.id);
            let (_, nodes) = lines.written(element, space);
            // A line holds 16 entries; the rest continues on the next line.
            for (entry, node) in (2..).zip(&nodes) {
                let separator = if entry == 17 { ",\n" } else { ", " };
                let _ = write!(out, "{separator}{node}");
            }
            out.push('\n');
        }
        blocks.push(Keyword::generated(out));
    }
    blocks
}

fn group<'a, 'b>(
    groups: &'b mut Vec<(Option<&'a str>, String, Vec<ElementId>)>,
    set: Option<&'a str>,
    type_name: String,
) -> &'b mut Vec<ElementId> {
    let index = match groups.iter().position(|g| g.0 == set && g.1 == type_name) {
        Some(index) => index,
        None => {
            groups.push((set, type_name, Vec::new()));
            groups.len() - 1
        }
    };
    &mut groups[index].2
}

enum Members {
    Ids(Vec<ElementId>),
    /// Element sets of whole parts, written by part name.
    Parts(Vec<String>),
}

enum Surface {
    /// Element set and face number per face side.
    Faces(Vec<(String, u8)>),
    /// A node set.
    Nodes(String),
}

/// All sets of the file: those read with the mesh and those derived from regions.
struct Sets<'a> {
    mesh: &'a FeMesh,
    node_sets: Vec<(String, Vec<NodeId>)>,
    element_sets: Vec<(String, Members)>,
    surfaces: Vec<(String, Surface)>,
    /// Upper-case names in use; CalculiX names are case-insensitive.
    used: BTreeSet<String>,
    /// Face element sets and node set of each element surface.
    surface_sets: BTreeMap<String, (Vec<(String, u8)>, String)>,
    /// Node sets of submodel boundary conditions, with the first boundary condition naming
    /// each; `*SUBMODEL` lists them.
    submodel_sets: Vec<(String, String)>,
}

impl<'a> Sets<'a> {
    fn new(mesh: &'a FeMesh) -> Self {
        let mut sets = Self {
            mesh,
            node_sets: Vec::new(),
            element_sets: Vec::new(),
            surfaces: Vec::new(),
            used: mesh
                .parts
                .iter()
                .map(|p| p.name.to_ascii_uppercase())
                .chain(mesh.node_sets.keys().map(|n| n.to_ascii_uppercase()))
                .chain(mesh.element_sets.keys().map(|n| n.to_ascii_uppercase()))
                .chain(mesh.surfaces.keys().map(|n| n.to_ascii_uppercase()))
                .collect(),
            surface_sets: BTreeMap::new(),
            submodel_sets: Vec::new(),
        };
        for (name, ids) in &mesh.node_sets {
            sets.node_sets.push((name.clone(), ids.clone()));
        }
        for (name, ids) in &mesh.element_sets {
            // Part sets come from the element blocks; only extra members are written.
            let part = mesh
                .parts
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(name));
            let extra: Vec<ElementId> = ids
                .iter()
                .copied()
                .filter(|id| part.is_none_or(|p| !p.elements.contains(id)))
                .collect();
            if part.is_none() || !extra.is_empty() {
                sets.element_sets.push((name.clone(), Members::Ids(extra)));
            }
        }
        for (name, surface) in &mesh.surfaces {
            match surface {
                SurfaceDefinition::ElementFaces(faces) => sets.add_face_surface(name, faces),
                SurfaceDefinition::Nodes(nodes) => {
                    let set = sets.free_name("Internal", name);
                    sets.node_sets.push((set.clone(), nodes.clone()));
                    sets.surfaces.push((name.clone(), Surface::Nodes(set)));
                }
            }
        }
        sets
    }

    /// `name` itself if it is free, otherwise the next free `<name>-<n>`.
    fn unique(&mut self, name: &str) -> String {
        if self.used.insert(name.to_ascii_uppercase()) {
            return name.to_owned();
        }
        (2..)
            .map(|n| format!("{name}-{n}"))
            .find(|name| self.used.insert(name.to_ascii_uppercase()))
            .expect("unbounded range")
    }

    /// Next free name `<prefix>-<n>`.
    fn unique_numbered(&mut self, prefix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}-{n}"))
            .find(|name| self.used.insert(name.to_ascii_uppercase()))
            .expect("unbounded range")
    }

    /// Next free name `<prefix>-<n>_<postfix>`, like PrePoMax's `GetNextNumberedKey`.
    fn free_name(&mut self, prefix: &str, postfix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}-{n}_{postfix}"))
            .find(|name| self.used.insert(name.to_ascii_uppercase()))
            .expect("unbounded range")
    }

    /// An element surface as PrePoMax builds it: one element set per face number and a node
    /// set with all face nodes for boundary conditions.
    fn add_face_surface(&mut self, name: &str, faces: &[(ElementId, u8)]) {
        let mut by_face: BTreeMap<u8, BTreeSet<ElementId>> = BTreeMap::new();
        for &(element, face) in faces {
            by_face.entry(face).or_default().insert(element);
        }
        let base = self.free_name("Internal", name);
        let mut sides = Vec::new();
        for (face, ids) in by_face {
            let set = format!("{base}_S{face}");
            self.used.insert(set.to_ascii_uppercase());
            self.element_sets
                .push((set.clone(), Members::Ids(ids.into_iter().collect())));
            sides.push((set, face));
        }
        let nodes = Region::Faces(faces.to_vec()).nodes(self.mesh);
        self.node_sets.push((base.clone(), nodes));
        self.surfaces
            .push((name.to_owned(), Surface::Faces(sides.clone())));
        self.surface_sets.insert(name.to_owned(), (sides, base));
    }

    /// Element set holding the elements of a region.
    fn element_set(&mut self, item: &str, region: &Region) -> Result<String, WriteError> {
        let members = match region {
            Region::ElementSet(set) => return Ok(set.clone()),
            Region::Parts(parts) if !parts.is_empty() => Members::Parts(parts.clone()),
            _ => {
                let ids = region.elements(self.mesh);
                if ids.is_empty() {
                    return Err(empty(item, "Elemente"));
                }
                Members::Ids(ids)
            }
        };
        let set = self.free_name("Internal_Selection", &name(item));
        self.element_sets.push((set.clone(), members));
        Ok(set)
    }

    /// Node set holding the nodes of a region.
    fn node_set(&mut self, item: &str, region: &Region) -> Result<String, WriteError> {
        match region {
            Region::NodeSet(set) => return Ok(set.clone()),
            Region::Surface(surface) => {
                if let Some((_, nodes)) = self.surface_sets.get(surface) {
                    return Ok(nodes.clone());
                }
            }
            _ => {}
        }
        let ids = region.nodes(self.mesh);
        if ids.is_empty() {
            return Err(empty(item, "Knoten"));
        }
        let set = self.free_name("Internal_Selection", &name(item));
        self.node_sets.push((set.clone(), ids));
        Ok(set)
    }

    /// Surface of a contact or tie side: a surface of the input file, or one built from
    /// picked faces and named like PrePoMax's `Internal_Selection-1_Tie-1_Master`.
    fn surface(&mut self, item: &str, side: &str, region: &Region) -> Result<String, WriteError> {
        match region {
            Region::Surface(surface) if self.mesh.surfaces.contains_key(surface) => {
                Ok(surface.clone())
            }
            Region::Surface(surface) => Err(WriteError::UnknownSurface {
                item: item.to_owned(),
                surface: surface.clone(),
            }),
            Region::Faces(_) | Region::Geometry(_) => {
                let faces = region.faces(self.mesh);
                if faces.is_empty() {
                    return Err(empty(item, "Elementflächen"));
                }
                let postfix = format!("{}_{side}", name(item));
                let surface = self.free_name("Internal_Selection", &postfix);
                self.add_face_surface(&surface, &faces);
                Ok(surface)
            }
            _ => Err(empty(item, "Elementflächen")),
        }
    }

    /// Element sets and face numbers of a face region, for pressure loads.
    fn face_sets(&mut self, item: &str, region: &Region) -> Result<Vec<(String, u8)>, WriteError> {
        match region {
            Region::Surface(surface) => {
                if let Some((sides, _)) = self.surface_sets.get(surface) {
                    return Ok(sides.clone());
                }
            }
            Region::Faces(_) | Region::Geometry(_) => {
                let faces = region.faces(self.mesh);
                if !faces.is_empty() {
                    let surface = self.free_name("Internal_Selection", &name(item));
                    self.add_face_surface(&surface, &faces);
                    return Ok(self.surface_sets[&surface].0.clone());
                }
            }
            _ => {}
        }
        Err(empty(item, "Elementflächen"))
    }
}

fn materials(model: &FeModel) -> Vec<Keyword> {
    model
        .materials
        .iter()
        .map(|material| {
            let mut properties = Vec::new();
            if let Some(density) = material.density {
                properties.push(Keyword::generated(format!(
                    "*Density\n{}\n",
                    number(density)
                )));
            }
            if let Some(elastic) = material.elastic {
                let (young, poisson) = (number(elastic.young), number(elastic.poisson));
                properties.push(Keyword::generated(format!(
                    "*Elastic\n{young}, {poisson}\n"
                )));
            }
            // Thermal properties in PrePoMax's order.
            if let Some(expansion) = material.expansion {
                let mut out = String::from("*Expansion");
                if expansion.zero_temperature != 0.0 {
                    let _ = write!(out, ", Zero={}", number(expansion.zero_temperature));
                }
                let _ = writeln!(out, "\n{}", number(expansion.coefficient));
                properties.push(Keyword::generated(out));
            }
            if let Some(conductivity) = material.conductivity {
                properties.push(Keyword::generated(format!(
                    "*Conductivity\n{}\n",
                    number(conductivity)
                )));
            }
            if let Some(specific_heat) = material.specific_heat {
                properties.push(Keyword::generated(format!(
                    "*Specific heat\n{}\n",
                    number(specific_heat)
                )));
            }
            let header = format!("*Material, Name={}\n", name(&material.name));
            Keyword::parent(header, properties)
        })
        .collect()
}

/// `*Physical constants` as PrePoMax's `CalPhysicalConstants` writes it, if any is defined.
fn physical_constants(model: &FeModel) -> Vec<Keyword> {
    let properties = &model.properties;
    let mut out = String::from("*Physical constants");
    if let Some(zero) = properties.absolute_zero {
        let _ = write!(out, ", Absolute zero={}", number(zero));
    }
    if let Some(sigma) = properties.stefan_boltzmann {
        let _ = write!(out, ", Stefan Boltzmann={}", number(sigma));
    }
    if properties.absolute_zero.is_none() && properties.stefan_boltzmann.is_none() {
        return Vec::new();
    }
    out.push('\n');
    vec![Keyword::generated(out)]
}

/// `*Amplitude` as PrePoMax's `CalAmplitude` writes it: four points per line, the most
/// CalculiX reads on one.
fn amplitudes(model: &FeModel) -> Result<Vec<Keyword>, WriteError> {
    let mut keywords = Vec::new();
    for amplitude in &model.amplitudes {
        if let Some(reason) = amplitude.points_problem() {
            return Err(WriteError::InvalidAmplitude {
                item: amplitude.name.clone(),
                reason,
            });
        }
        let mut out = format!("*Amplitude, Name={}", name(&amplitude.name));
        if amplitude.time_span == AmplitudeTime::Total {
            out.push_str(", Time=Total time");
        }
        if amplitude.shift_time != 0.0 {
            let _ = write!(out, ", Shiftx={}", number(amplitude.shift_time));
        }
        if amplitude.shift_amplitude != 0.0 {
            let _ = write!(out, ", Shifty={}", number(amplitude.shift_amplitude));
        }
        out.push('\n');
        for line in amplitude.points.chunks(4) {
            let pairs: Vec<String> = line
                .iter()
                .map(|[t, a]| format!("{}, {}", number(*t), number(*a)))
                .collect();
            let _ = writeln!(out, "{}", pairs.join(", "));
        }
        keywords.push(Keyword::generated(out));
    }
    Ok(keywords)
}

/// The amplitude parameter of a boundary condition or load, such as `, Amplitude=Ramp`;
/// empty without one. `parameter` is the name CalculiX gives it on the keyword.
fn amplitude_parameter(
    amplitudes: &[Amplitude],
    item: &str,
    parameter: &str,
    reference: &Option<String>,
) -> Result<String, WriteError> {
    let Some(reference) = reference else {
        return Ok(String::new());
    };
    if !amplitudes.iter().any(|a| &a.name == reference) {
        return Err(WriteError::UnknownAmplitude {
            item: item.to_owned(),
            amplitude: reference.clone(),
        });
    }
    Ok(format!(", {parameter}={}", name(reference)))
}

/// Initial temperatures and velocities as PrePoMax's `CalInitialTemperature`,
/// `CalInitialTranslationalVelocity` and `CalInitialAngularVelocity` write them: one line
/// per set or node and non-zero component, 2D models without the third.
fn initial_conditions(sets: &mut Sets, model: &FeModel) -> Result<Vec<Keyword>, WriteError> {
    let components = if model.properties.space.is_2d() { 2 } else { 3 };
    let mut keywords = Vec::new();
    for condition in &model.initial_conditions {
        if !condition.active {
            keywords.push(deactivated(&condition.name));
            continue;
        }
        let mut out = format!("** Name: {}\n", condition.name);
        let velocity_lines = |out: &mut String, target: &str, v: [f64; 3]| {
            for (dof, value) in v.iter().enumerate().take(components) {
                if *value != 0.0 {
                    out.push_str(&format!("{target}, {}, {}\n", dof + 1, number(*value)));
                }
            }
        };
        match condition.kind {
            InitialConditionKind::Temperature(t) => {
                let set = sets.node_set(&condition.name, &condition.region)?;
                out.push_str(&format!(
                    "*Initial conditions, Type=Temperature\n{set}, {}\n",
                    number(t)
                ));
            }
            InitialConditionKind::Velocity(v) => {
                let set = sets.node_set(&condition.name, &condition.region)?;
                out.push_str("*Initial conditions, Type=Velocity\n");
                velocity_lines(&mut out, &set, v);
            }
            InitialConditionKind::AngularVelocity { .. } => {
                let nodes = condition.region.nodes(sets.mesh);
                if nodes.is_empty() {
                    return Err(empty(&condition.name, "Knoten"));
                }
                out.push_str("*Initial conditions, Type=Velocity\n");
                for id in nodes {
                    let Some(position) = sets.mesh.node(id) else {
                        continue;
                    };
                    let v = condition.kind.velocity_at(position).unwrap_or_default();
                    velocity_lines(&mut out, &id.to_string(), v);
                }
            }
        }
        keywords.push(Keyword::generated(out));
    }
    Ok(keywords)
}

fn sections(sets: &mut Sets, model: &FeModel) -> Result<Vec<Keyword>, WriteError> {
    let mut keywords = Vec::new();
    for section in &model.sections {
        if !model.materials.iter().any(|m| m.name == section.material) {
            return Err(WriteError::UnknownMaterial {
                item: section.name.clone(),
                material: section.material.clone(),
            });
        }
        let material = name(&section.material);
        let mut out = format!("** Name: {}\n", section.name);
        match &section.kind {
            SectionKind::Solid => {
                let set = sets.element_set(&section.name, &section.region)?;
                let _ = writeln!(out, "*Solid section, Elset={set}, Material={material}");
                // Plane stress and plane strain sections have a thickness, as in PrePoMax.
                if model.properties.space.has_thickness() {
                    let _ = writeln!(out, "{}", number(section.thickness));
                }
            }
            SectionKind::Truss { area } => {
                let set = sets.element_set(&section.name, &section.region)?;
                let _ = writeln!(out, "*Solid section, Elset={set}, Material={material}");
                let _ = writeln!(out, "{}", number(*area));
            }
            SectionKind::Beam(beam) => {
                let mut options = format!("Section={}", beam.profile.keyword());
                for (k, offset) in (1..).zip(beam.offset) {
                    if offset != 0.0 {
                        let _ = write!(options, ", Offset{k}={}", number(offset));
                    }
                }
                let dimensions: Vec<String> = beam
                    .profile
                    .data_line()
                    .iter()
                    .map(|&v| number(v))
                    .collect();
                // One keyword per normal: CalculiX takes one normal per section, and an
                // automatic orientation may differ between the beams of the region.
                for (normal, set) in beam_groups(sets, section)? {
                    let _ = writeln!(
                        out,
                        "*Beam section, Elset={set}, Material={material}, {options}"
                    );
                    let _ = writeln!(out, "{}", dimensions.join(", "));
                    let _ = writeln!(
                        out,
                        "{}, {}, {}",
                        number(normal[0]),
                        number(normal[1]),
                        number(normal[2])
                    );
                }
            }
        }
        keywords.push(Keyword::generated(out));
    }
    Ok(keywords)
}

/// The beams of a section grouped by the normal they are written with, each group as an
/// element set. A region whose beams all share one normal keeps its own set.
fn beam_groups(sets: &mut Sets, section: &Section) -> Result<Vec<([f64; 3], String)>, WriteError> {
    let SectionKind::Beam(beam) = &section.kind else {
        return Ok(Vec::new());
    };
    let mesh = sets.mesh;
    let mut groups: Vec<([f64; 3], Vec<ElementId>)> = Vec::new();
    for element in (section.region.elements(mesh).into_iter()).filter_map(|id| mesh.element(id)) {
        if element.shape.family() != ElementFamily::Line {
            continue;
        }
        let normal =
            (beam.orientation.normal_for(line_tangent(mesh, element))).ok_or_else(|| {
                WriteError::InvalidSection {
                    item: section.name.clone(),
                    reason: format!(
                        "Die Normale ist parallel zur Achse von Element {}",
                        element.id
                    ),
                }
            })?;
        match groups.iter_mut().find(|(n, _)| *n == normal) {
            Some((_, ids)) => ids.push(element.id),
            None => groups.push((normal, vec![element.id])),
        }
    }
    if groups.is_empty() {
        return Err(empty(&section.name, "Linienelemente"));
    }
    // CalculiX 2.21 reads the element set of a pipe or box section 20 characters wide and
    // fails on longer names, so those get short sets of their own.
    let short = beam.profile.needs_reduced_integration();
    if groups.len() == 1 && !short {
        let set = sets.element_set(&section.name, &section.region)?;
        return Ok(vec![(groups[0].0, set)]);
    }
    let mut named = Vec::new();
    for (normal, ids) in groups {
        let set = if short {
            sets.unique_numbered("Beam")
        } else {
            sets.free_name("Internal_Selection", &name(&section.name))
        };
        sets.element_sets.push((set.clone(), Members::Ids(ids)));
        named.push((normal, set));
    }
    Ok(named)
}

/// Tie constraints as PrePoMax's `CalTie` writes them: slave surface first. Springs and
/// supports are written as elements, see [`constraints::springs`].
fn constraints(sets: &mut Sets, model: &FeModel) -> Result<Vec<Keyword>, WriteError> {
    let mut keywords = Vec::new();
    for constraint in &model.constraints {
        if !constraint.active() {
            keywords.push(deactivated(constraint.name()));
            continue;
        }
        match constraint {
            Constraint::PointSpring(_)
            | Constraint::SurfaceSpring(_)
            | Constraint::CompressionOnly(_)
            | Constraint::SurfaceToSurfaceSpring(_) => {}
            // Moved to the model's node ties when the project was read.
            Constraint::NodeTie(_) => {}
            Constraint::Tie(tie) => {
                let master = sets.surface(&tie.name, "Master", &tie.master)?;
                let slave = sets.surface(&tie.name, "Slave", &tie.slave)?;
                let mut out = format!("*Tie, Name={}", name(&tie.name));
                if let Some(tolerance) = tie.position_tolerance {
                    let _ = write!(out, ", Position tolerance={}", number(tolerance));
                }
                if !tie.adjust {
                    out.push_str(", Adjust=No");
                }
                let _ = writeln!(out, "\n{slave}, {master}");
                keywords.push(Keyword::generated(out));
            }
        }
    }
    Ok(keywords)
}

/// The node ties, listed with the contact pairs.
fn node_ties(
    sets: &mut Sets,
    model: &FeModel,
    lines: &LineElements,
) -> Result<Vec<Keyword>, WriteError> {
    model
        .node_ties
        .iter()
        .map(|tie| {
            if tie.active {
                node_tie(sets, tie, lines)
            } else {
                Ok(deactivated(&tie.name))
            }
        })
        .collect()
}

/// Whether a node tie is a hinge between beams, written as equations of the translations.
/// Every other tie is written as one shared node (see [`node_merges`]): CalculiX 2.21 does
/// not couple the rotations of beam nodes through equations, and nodes without rotations
/// (trusses, solids) are joined completely by their translations anyway.
fn hinge_between_beams(tie: &NodeTie, nodes: &[NodeId], lines: &LineElements) -> bool {
    !tie.rotations && nodes.iter().all(|n| lines.beam_nodes.contains(n))
}

/// The nodes every active node tie merges into its first node, as nodes to replace by the
/// node that stays.
fn node_merges(mesh: &FeMesh, model: &FeModel, lines: &LineElements) -> BTreeMap<NodeId, NodeId> {
    let mut replaced = BTreeMap::new();
    for tie in model.node_ties.iter().filter(|t| t.active) {
        let nodes = tie.region.nodes(mesh);
        if hinge_between_beams(tie, &nodes, lines) {
            continue;
        }
        if let Some((&first, rest)) = nodes.split_first() {
            let first = replaced.get(&first).copied().unwrap_or(first);
            for &node in rest {
                if node != first {
                    replaced.insert(node, first);
                }
            }
        }
    }
    replaced
}

/// A node tie: a hinge between beams as equations of the translations, every other tie as
/// one shared node, which [`node_merges`] put into the mesh already; the comment names it.
fn node_tie(sets: &mut Sets, tie: &NodeTie, lines: &LineElements) -> Result<Keyword, WriteError> {
    let nodes = tie.region.nodes(sets.mesh);
    let Some((&first, rest)) = nodes.split_first() else {
        return Err(WriteError::EmptyRegion {
            item: tie.name.clone(),
            what: "Knoten",
        });
    };
    let mut out = format!("** Name: {}\n", tie.name);
    if !hinge_between_beams(tie, &nodes, lines) {
        let _ = writeln!(out, "** Knoten {first} (zusammengelegt)");
        return Ok(Keyword::generated(out));
    }
    for &node in rest {
        for dof in 1..=3 {
            let _ = writeln!(out, "*Equation\n2\n{node}, {dof}, 1, {first}, {dof}, -1");
        }
    }
    Ok(Keyword::generated(out))
}

/// A surface interaction with its models as children, like PrePoMax's
/// `CalSurfaceInteraction`.
fn interaction(interaction: &SurfaceInteraction) -> Keyword {
    let properties = (interaction.properties.iter())
        .map(|property| {
            let mut out = String::new();
            match property {
                InteractionProperty::SurfaceBehavior(behavior) => {
                    let kind = behavior.keyword();
                    let _ = writeln!(out, "*Surface behavior, Pressure-overclosure={kind}");
                    match behavior {
                        SurfaceBehavior::Hard => {}
                        SurfaceBehavior::Linear { k, sigma_inf, c0 } => {
                            let _ = write!(out, "{}, {}", number(*k), number(*sigma_inf));
                            if let Some(c0) = c0 {
                                let _ = write!(out, ", {}", number(*c0));
                            }
                            out.push('\n');
                        }
                        SurfaceBehavior::Exponential { c0, p0 } => {
                            let _ = writeln!(out, "{}, {}", number(*c0), number(*p0));
                        }
                        SurfaceBehavior::Tabular(rows) => {
                            for [pressure, overclosure] in rows {
                                let _ = writeln!(
                                    out,
                                    "{}, {}",
                                    number(*pressure),
                                    number(*overclosure)
                                );
                            }
                        }
                        SurfaceBehavior::Tied { k } => {
                            let _ = writeln!(out, "{}", number(*k));
                        }
                    }
                }
                InteractionProperty::Friction(friction) => {
                    let _ = write!(out, "*Friction\n{}", number(friction.coefficient));
                    if let Some(slope) = friction.stick_slope {
                        let _ = write!(out, ", {}", number(slope));
                    }
                    out.push('\n');
                }
                InteractionProperty::GapConductance(conductance) => {
                    out.push_str("*Gap conductance\n");
                    match conductance {
                        GapConductance::Constant(value) => {
                            let _ = writeln!(out, "{}", number(*value));
                        }
                        GapConductance::Tabular(rows) => {
                            for row in rows {
                                let row: Vec<String> = row.iter().map(|&v| number(v)).collect();
                                let _ = writeln!(out, "{}", row.join(", "));
                            }
                        }
                    }
                }
            }
            Keyword::generated(out)
        })
        .collect();
    let header = format!("*Surface interaction, Name={}\n", name(&interaction.name));
    Keyword::parent(header, properties)
}

/// Contact pairs as PrePoMax's `CalContactPair` writes them: slave surface first.
/// Master and slave surface of each active contact pair, by the pair's name.
type PairSurfaces = BTreeMap<String, (String, String)>;

fn contact_pairs(
    sets: &mut Sets,
    model: &FeModel,
) -> Result<(Vec<Keyword>, PairSurfaces), WriteError> {
    let mut keywords = Vec::new();
    let mut surfaces = PairSurfaces::new();
    for pair in &model.contact_pairs {
        if !pair.active {
            keywords.push(deactivated(&pair.name));
            continue;
        }
        if !(model.surface_interactions.iter()).any(|s| s.name == pair.interaction) {
            return Err(WriteError::UnknownInteraction {
                item: pair.name.clone(),
                interaction: pair.interaction.clone(),
            });
        }
        let master = sets.surface(&pair.name, "Master", &pair.master)?;
        let slave = sets.surface(&pair.name, "Slave", &pair.slave)?;
        keywords.push(Keyword::generated(contact_pair(pair, &master, &slave)));
        surfaces.insert(pair.name.clone(), (master, slave));
    }
    Ok((keywords, surfaces))
}

fn contact_pair(pair: &ContactPair, master: &str, slave: &str) -> String {
    let mut out = format!(
        "** Name: {}\n*Contact pair, Interaction={}, Type={}",
        pair.name,
        name(&pair.interaction),
        pair.method.name()
    );
    if pair.method == ContactMethod::NodeToSurface && pair.small_sliding {
        out.push_str(", Small sliding");
    }
    if pair.adjust {
        let size = pair.adjustment_size.unwrap_or(0.0);
        let _ = write!(out, ", Adjust={}", number(size));
    }
    let _ = writeln!(out, "\n{slave}, {master}");
    out
}

/// What the steps share: the model's settings and the items written before them.
struct StepContext<'a> {
    extra_boundary: Option<&'a Keyword>,
    space: ModelSpace,
    lines: &'a LineElements,
    flux_kinds: FluxKinds,
    amplitudes: &'a [Amplitude],
    pair_surfaces: &'a PairSurfaces,
}

/// A step as PrePoMax structures it: the step title holds `*Step`, which holds the procedure
/// and a title for each kind of item, down to the one holding `*End step`.
///
/// Like PrePoMax, a deactivated step or item keeps its place in the file as a comment
/// (`** Name: Fixed-1: Deactivated`), and nothing of it is written, not even its sets.
fn write_step(sets: &mut Sets, step: &Step, context: &StepContext) -> Result<Keyword, WriteError> {
    let &StepContext {
        extra_boundary,
        space,
        lines,
        flux_kinds,
        amplitudes,
        pair_surfaces,
    } = context;
    // Nodes of 2D models move in the x-y plane only; CalculiX fails on rotations there.
    let dofs = if space.is_2d() { 2 } else { 6 };
    if !step.active {
        return Ok(deactivated_step(step));
    }
    let (header, procedure) = match &step.kind {
        StepKind::Static(settings) => static_step(settings),
        StepKind::Frequency(settings) => frequency_step(settings),
        StepKind::Buckle(settings) => buckle_step(settings),
        StepKind::HeatTransfer(settings) => heat_transfer_step(settings, "*Heat transfer", false),
        StepKind::CoupledTempDisp(settings) => {
            heat_transfer_step(settings, "*Coupled temperature-displacement", true)
        }
        StepKind::Dynamic(settings) => dynamic_step(settings),
        StepKind::ModalDynamics(settings) => modal_dynamics_step(settings),
        StepKind::SteadyStateDynamics(settings) => steady_state_dynamics_step(settings),
    };
    // A modal step keeps the supports of the frequency step that stored the modes;
    // CalculiX refuses new ones ("in a modal dynamic step new SPCs are not allowed"), and
    // the reset would make the same supports new. Writing them again is accepted.
    let mut boundaries = if step.kind.is_modal() {
        Vec::new()
    } else {
        vec![Keyword::generated("*Boundary, op=New\n".into())]
    };
    for bc in &step.boundary_conditions {
        // A step leaves out what it cannot take, like a deactivated item: displacements in
        // a heat transfer step, temperatures in a static one.
        if !bc.active || !step.kind.supports_boundary(&bc.kind) {
            boundaries.push(deactivated(&bc.name));
            continue;
        }
        // Truss nodes have no rotations either; CalculiX fails when they are fixed.
        let dofs = if dofs == 6 && lines.all_truss(&bc.region.nodes(sets.mesh)) {
            3
        } else {
            dofs
        };
        let set = sets.node_set(&bc.name, &bc.region)?;
        // Fixed supports stay zero; an amplitude would not change them.
        let reference = bc.amplitude.as_ref().filter(|_| bc.kind.takes_amplitude());
        let amplitude =
            amplitude_parameter(amplitudes, &bc.name, "Amplitude", &reference.cloned())?;
        let options = match bc.kind {
            BoundaryKind::Submodel { step, .. } => format!(", Submodel, Step={}", step.max(1)),
            _ => amplitude,
        };
        let mut out = format!("** Name: {}\n*Boundary{options}\n", bc.name);
        match bc.kind {
            BoundaryKind::Fixed => {
                let _ = writeln!(out, "{set}, 1, {dofs}, 0");
            }
            BoundaryKind::Displacement(values) => {
                for (dof, value) in (1..).zip(values).take(dofs) {
                    if let Some(value) = value {
                        let _ = writeln!(out, "{set}, {dof}, {dof}, {}", number(value));
                    }
                }
            }
            BoundaryKind::Temperature(t) => {
                let _ = writeln!(out, "{set}, 11, 11, {}", number(t));
            }
            BoundaryKind::Submodel { dofs: held, .. } => {
                for dof in (1..=dofs).filter(|&d| held[d - 1]) {
                    let _ = writeln!(out, "{set}, {dof}, {dof}");
                }
                if !sets.submodel_sets.iter().any(|(s, _)| *s == set) {
                    sets.submodel_sets.push((set, bc.name.clone()));
                }
            }
        }
        boundaries.push(Keyword::generated(out));
    }
    // Boundary conditions that constraints need, PrePoMax's additional boundary conditions.
    if step.kind.is_mechanical() {
        boundaries.extend(extra_boundary.cloned());
    }
    let mut loads = Vec::new();
    // Like PrePoMax, a step resets the loads it takes; one that takes no loads gets none
    // written, not even the reset. Distributed fluxes, films and radiation are reset only
    // when the model has them.
    let kind = &step.kind;
    let resets = [
        ("Cload", kind.supports_load(&LoadKind::Pressure(0.0))),
        ("Dload", kind.supports_load(&LoadKind::Pressure(0.0))),
        (
            "Cflux",
            kind.supports_load(&LoadKind::ConcentratedFlux(0.0)),
        ),
        ("Dflux", kind.is_thermal() && flux_kinds.distributed),
        ("Film", kind.is_thermal() && flux_kinds.film),
        ("Radiate", kind.is_thermal() && flux_kinds.radiation),
    ];
    for (keyword, _) in resets.into_iter().filter(|(_, reset)| *reset) {
        loads.push(Keyword::generated(format!("*{keyword}, op=New\n")));
    }
    let step_loads: &[_] = if kind.supports_loads() {
        &step.loads
    } else {
        &[]
    };
    for load in step_loads {
        if !load.active || !kind.supports_load(&load.kind) {
            loads.push(deactivated(&load.name));
            continue;
        }
        let mut out = format!("** Name: {}\n", load.name);
        let amplitude = amplitude_parameter(amplitudes, &load.name, "Amplitude", &load.amplitude)?;
        // The second amplitude of a film scales its coefficient, of radiation the emissivity.
        let factor = match load.kind {
            LoadKind::Film { .. } => Some("Film amplitude"),
            LoadKind::Radiation { .. } => Some("Radiation amplitude"),
            _ => None,
        };
        let factor_amplitude = match factor {
            Some(parameter) => {
                amplitude_parameter(amplitudes, &load.name, parameter, &load.factor_amplitude)?
            }
            None => String::new(),
        };
        match load.kind {
            LoadKind::ConcentratedForce(force) => {
                let set = sets.node_set(&load.name, &load.region)?;
                let _ = writeln!(out, "*Cload{amplitude}");
                for (dof, value) in (1..).zip(force).take(dofs) {
                    if value != 0.0 {
                        let _ = writeln!(out, "{set}, {dof}, {}", number(value));
                    }
                }
            }
            LoadKind::Pressure(pressure) => {
                let _ = writeln!(out, "*Dload{amplitude}");
                for (set, face) in sets.face_sets(&load.name, &load.region)? {
                    let _ = writeln!(out, "{set}, P{face}, {}", number(pressure));
                }
            }
            LoadKind::SurfaceTraction(force) => {
                let faces = load.region.faces(sets.mesh);
                let axisymmetric = space == ModelSpace::Axisymmetric;
                let nodal = traction_forces(sets.mesh, &faces, force, axisymmetric);
                if nodal.is_empty() {
                    return Err(empty(&load.name, "Elementflächen"));
                }
                let _ = writeln!(out, "*Cload{amplitude}");
                for (node, values) in nodal {
                    for (dof, value) in (1..).zip(values).take(dofs) {
                        if value != 0.0 {
                            let _ = writeln!(out, "{node}, {dof}, {}", number(value));
                        }
                    }
                }
            }
            LoadKind::ConcentratedFlux(flux) => {
                let set = sets.node_set(&load.name, &load.region)?;
                let _ = writeln!(out, "*Cflux{amplitude}\n{set}, 11, {}", number(flux));
            }
            LoadKind::SurfaceFlux(flux) => {
                let _ = writeln!(out, "*Dflux{amplitude}");
                for (set, face) in sets.face_sets(&load.name, &load.region)? {
                    let _ = writeln!(out, "{set}, S{face}, {}", number(flux));
                }
            }
            LoadKind::BodyFlux(flux) => {
                let set = sets.element_set(&load.name, &load.region)?;
                let _ = writeln!(out, "*Dflux{amplitude}\n{set}, BF, {}", number(flux));
            }
            LoadKind::Film { sink, coefficient } => {
                let _ = writeln!(out, "*Film{amplitude}{factor_amplitude}");
                for (set, face) in sets.face_sets(&load.name, &load.region)? {
                    let (sink, h) = (number(sink), number(coefficient));
                    let _ = writeln!(out, "{set}, F{face}, {sink}, {h}");
                }
            }
            LoadKind::Radiation { sink, emissivity } => {
                let _ = writeln!(out, "*Radiate{amplitude}{factor_amplitude}");
                for (set, face) in sets.face_sets(&load.name, &load.region)? {
                    let (sink, e) = (number(sink), number(emissivity));
                    let _ = writeln!(out, "{set}, R{face}, {sink}, {e}");
                }
            }
            // As PrePoMax's CalGravityLoad: the size of the acceleration and its direction.
            LoadKind::Gravity(acceleration) => {
                let set = sets.element_set(&load.name, &load.region)?;
                let Some((size, direction)) = unit_vector(acceleration) else {
                    return Err(WriteError::InvalidLoad {
                        item: load.name.clone(),
                        reason: "die Erdbeschleunigung ist null".into(),
                    });
                };
                let _ = writeln!(out, "*Dload{amplitude}");
                let [x, y, z] = direction.map(number);
                let _ = writeln!(out, "{set}, Grav, {}, {x}, {y}, {z}", number(size));
            }
            // As PrePoMax's CalCentrifLoad: the square of the rotational speed, a point on
            // the axis and the axis direction.
            LoadKind::Centrifugal { point, axis, speed } => {
                let set = sets.element_set(&load.name, &load.region)?;
                let Some((_, direction)) = unit_vector(axis) else {
                    return Err(WriteError::InvalidLoad {
                        item: load.name.clone(),
                        reason: "die Drehachse hat keine Richtung".into(),
                    });
                };
                let _ = writeln!(out, "*Dload{amplitude}");
                let [px, py, pz] = point.map(number);
                let [x, y, z] = direction.map(number);
                let _ = writeln!(
                    out,
                    "{set}, Centrif, {}, {px}, {py}, {pz}, {x}, {y}, {z}",
                    number(speed * speed)
                );
            }
        }
        loads.push(Keyword::generated(out));
    }
    let mut history_outputs = Vec::new();
    for output in &step.history_outputs {
        if !output.active {
            history_outputs.push(deactivated(&output.name));
        } else if let Some(keyword) = history_output(sets, output, pair_surfaces)? {
            history_outputs.push(keyword);
        }
    }
    let field_outputs = step.field_outputs.iter().filter_map(field_output).collect();
    let end = Keyword::generated("*End step\n".into());
    let contents = vec![
        Keyword::generated(procedure),
        Keyword::title("Controls", Vec::new()),
        Keyword::title("Output frequency", Vec::new()),
        Keyword::title("Boundary conditions", boundaries),
        Keyword::title("Loads", loads),
        Keyword::title("Defined fields", Vec::new()),
        Keyword::title("History outputs", history_outputs),
        Keyword::title("Field outputs", field_outputs),
        Keyword::title("End step", vec![end]),
    ];
    Ok(Keyword::title(
        &step.name,
        vec![Keyword::parent(header, contents)],
    ))
}

/// Comment that stands for a deactivated item, PrePoMax's `CalDeactivated`.
fn deactivated(name: &str) -> Keyword {
    Keyword::generated(format!("** Name: {name}: Deactivated\n"))
}

/// A deactivated step as PrePoMax writes it: the titles of the step with a comment for the
/// step, its procedure and each of its items, but no keyword CalculiX would read.
fn deactivated_step(step: &Step) -> Keyword {
    let procedure = match step.kind {
        StepKind::Static(_) => "StaticStep",
        StepKind::Frequency(_) => "FrequencyStep",
        StepKind::Buckle(_) => "BuckleStep",
        StepKind::HeatTransfer(_) => "HeatTransferStep",
        StepKind::CoupledTempDisp(_) => "CoupledTempDispStep",
        StepKind::Dynamic(_) => "DynamicStep",
        StepKind::ModalDynamics(_) => "ModalDynamicsStep",
        StepKind::SteadyStateDynamics(_) => "SteadyStateDynamicsStep",
    };
    fn all<'a>(names: impl Iterator<Item = &'a str>) -> Vec<Keyword> {
        names.map(deactivated).collect()
    }
    let loads = if step.kind.supports_loads() {
        all(step.loads.iter().map(|l| l.name.as_str()))
    } else {
        Vec::new()
    };
    let contents = vec![
        deactivated(procedure),
        Keyword::title(
            "Boundary conditions",
            all(step.boundary_conditions.iter().map(|b| b.name.as_str())),
        ),
        Keyword::title("Loads", loads),
        Keyword::title("Defined fields", Vec::new()),
        Keyword::title(
            "History outputs",
            all(step.history_outputs.iter().map(|h| h.name.as_str())),
        ),
        Keyword::title(
            "Field outputs",
            all(step.field_outputs.iter().map(|f| f.name.as_str())),
        ),
        Keyword::title("End step", vec![deactivated(&step.name)]),
    ];
    Keyword::title(
        &step.name,
        vec![Keyword::parent(
            format!("** Name: {}: Deactivated\n", step.name),
            contents,
        )],
    )
}

/// Equivalent nodal forces of a total force spread evenly over element faces, the way
/// PrePoMax turns a surface traction into concentrated loads. Each face takes its share by
/// area; within a face the share follows the shape functions: equal parts on linear faces,
/// only midside nodes on quadratic triangles, and -1/12 per corner and 1/3 per midside
/// node on quadratic quadrilaterals.
///
/// The faces of 2D elements are their edges, which share the force by length. In an
/// axisymmetric model the force acts on the whole revolution, as CalculiX's concentrated
/// loads there do, so an edge's share grows with its distance from the axis.
fn traction_forces(
    mesh: &FeMesh,
    faces: &[(ElementId, u8)],
    force: [f64; 3],
    axisymmetric: bool,
) -> BTreeMap<NodeId, [f64; 3]> {
    let (weights, total_area) = constraints::node_areas(mesh, faces, axisymmetric);
    if total_area <= 0.0 {
        return BTreeMap::new();
    }
    (weights.into_iter())
        .map(|(node, weight)| (node, force.map(|f| f * weight / total_area)))
        .collect()
}

/// Integrals of the shape functions along an edge of two end points, or of two end points
/// and a midside point: the nodes' shares of the edge's length, or with `axisymmetric` of
/// the length times the radius x (the area the edge sweeps, divided by 2 pi).
fn edge_weights(points: &[[f64; 3]], axisymmetric: bool) -> Vec<f64> {
    // Three-point Gauss rule, exact for the quadratic edge with a linear radius.
    let gauss = [
        (-(0.6f64.sqrt()), 5.0 / 9.0),
        (0.0, 8.0 / 9.0),
        (0.6f64.sqrt(), 5.0 / 9.0),
    ];
    let mut weights = vec![0.0; points.len()];
    for (t, w) in gauss {
        let (shape, slope): (Vec<f64>, Vec<f64>) = if points.len() == 3 {
            (
                vec![t * (t - 1.0) / 2.0, t * (t + 1.0) / 2.0, 1.0 - t * t],
                vec![t - 0.5, t + 0.5, -2.0 * t],
            )
        } else {
            (vec![(1.0 - t) / 2.0, (1.0 + t) / 2.0], vec![-0.5, 0.5])
        };
        let combine =
            |f: &[f64], k: usize| -> f64 { f.iter().zip(points).map(|(n, p)| n * p[k]).sum() };
        let jacobian = (0..3)
            .map(|k| combine(&slope, k).powi(2))
            .sum::<f64>()
            .sqrt();
        let radius = if axisymmetric {
            combine(&shape, 0)
        } else {
            1.0
        };
        for (weight, n) in weights.iter_mut().zip(&shape) {
            *weight += w * n * jacobian * radius;
        }
    }
    weights
}

/// Area of a (possibly slightly warped) polygon from its vector area.
fn polygon_area(corners: &[[f64; 3]]) -> f64 {
    let mut sum = [0.0; 3];
    for (i, a) in corners.iter().enumerate() {
        let b = corners[(i + 1) % corners.len()];
        sum[0] += a[1] * b[2] - a[2] * b[1];
        sum[1] += a[2] * b[0] - a[0] * b[2];
        sum[2] += a[0] * b[1] - a[1] * b[0];
    }
    0.5 * (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt()
}

/// Which kinds of heat loads the model has in any step: PrePoMax resets distributed fluxes,
/// films and radiation only then.
#[derive(Clone, Copy, Debug, Default)]
struct FluxKinds {
    distributed: bool,
    film: bool,
    radiation: bool,
}

impl FluxKinds {
    fn of(model: &FeModel) -> Self {
        let mut kinds = Self::default();
        for load in model.steps.iter().flat_map(|s| &s.loads) {
            match load.kind {
                LoadKind::SurfaceFlux(_) | LoadKind::BodyFlux(_) => kinds.distributed = true,
                LoadKind::Film { .. } => kinds.film = true,
                LoadKind::Radiation { .. } => kinds.radiation = true,
                _ => {}
            }
        }
        kinds
    }
}

/// The `*Step` line and the procedure keyword of a static step.
fn static_step(settings: &StaticStep) -> (String, String) {
    let mut keyword = String::from("*Static");
    if let Some(solver) = settings.solver.keyword() {
        let _ = write!(keyword, ", Solver={solver}");
    }
    incremented_step(settings, keyword, settings.nlgeom)
}

/// The `*Step` line and the procedure keyword of a heat transfer or coupled step, as
/// PrePoMax's `CalHeatTransferStep` and `CalCoupledTempDispStep` write them.
fn heat_transfer_step(
    settings: &HeatTransferStep,
    keyword: &str,
    coupled: bool,
) -> (String, String) {
    let increments = &settings.increments;
    let mut keyword = keyword.to_string();
    if let Some(solver) = increments.solver.keyword() {
        let _ = write!(keyword, ", Solver={solver}");
    }
    if settings.steady_state {
        keyword.push_str(", Steady state");
    }
    if let Some(deltmx) = settings.deltmx.filter(|_| !settings.steady_state) {
        let _ = write!(keyword, ", Deltmx={}", number(deltmx));
    }
    incremented_step(increments, keyword, coupled && increments.nlgeom)
}

/// The `*Step` line and the procedure keyword of a dynamic step, as PrePoMax's
/// `CalDynamicStep` and `CalDamping` write them: the Rayleigh damping follows the
/// procedure in the step, where CalculiX applies it to the whole model.
fn dynamic_step(settings: &DynamicStep) -> (String, String) {
    let mut keyword = String::from("*Dynamic");
    if let Some(solver) = settings.increments.solver.keyword() {
        let _ = write!(keyword, ", Solver={solver}");
    }
    if settings.alpha != -0.05 {
        let _ = write!(keyword, ", Alpha={}", number(settings.alpha));
    }
    if let Some(explicit) = settings.procedure.keyword() {
        let _ = write!(keyword, ", Explicit={explicit}");
    }
    let (header, mut procedure) =
        incremented_step(&settings.increments, keyword, settings.increments.nlgeom);
    if let Some(damping) = &settings.damping {
        let _ = writeln!(
            procedure,
            "*Damping, Alpha={}, Beta={}",
            number(damping.alpha),
            number(damping.beta)
        );
    }
    (header, procedure)
}

/// The `*Step` line and the procedure keyword of a modal dynamics step, as PrePoMax's
/// `CalModalDynamicsStep` writes them, with the modal damping after the procedure.
fn modal_dynamics_step(settings: &ModalDynamicsStep) -> (String, String) {
    let header = format!("*Step, Inc={}\n", settings.increments());
    let mut procedure = String::from("*Modal dynamics");
    if let Some(solver) = settings.solver.keyword() {
        let _ = write!(procedure, ", Solver={solver}");
    }
    if settings.steady_state {
        procedure.push_str(", Steady state");
    }
    let second = if settings.steady_state {
        settings.relative_error
    } else {
        settings.time_period
    };
    let _ = writeln!(
        procedure,
        "\n{}, {}",
        number(settings.increment),
        number(second)
    );
    modal_damping(&mut procedure, settings.damping.as_ref());
    (header, procedure)
}

/// The `*Step` line and the procedure keyword of a steady state dynamics step, as
/// PrePoMax's `CalSteadyStateDynamicsStep` writes them.
fn steady_state_dynamics_step(settings: &SteadyStateDynamicsStep) -> (String, String) {
    let mut procedure = String::from("*Steady state dynamics");
    if !settings.harmonic {
        procedure.push_str(", Harmonic=No");
    }
    if let Some(solver) = settings.solver.keyword() {
        let _ = write!(procedure, ", Solver={solver}");
    }
    let _ = write!(
        procedure,
        "\n{}, {}, {}, {}",
        number(settings.lower_frequency),
        number(settings.upper_frequency),
        settings.data_points,
        number(settings.bias)
    );
    if !settings.harmonic {
        let _ = write!(
            procedure,
            ", {}, {}, {}",
            settings.fourier_terms,
            number(settings.time_lower),
            number(settings.time_upper)
        );
    }
    procedure.push('\n');
    modal_damping(&mut procedure, settings.damping.as_ref());
    ("*Step\n".into(), procedure)
}

/// `*Modal damping` as PrePoMax's `CalModalDamping` writes it: a constant ratio covers
/// modes 1 to 1000000, Rayleigh damping leaves the mode range empty.
fn modal_damping(out: &mut String, damping: Option<&ModalDamping>) {
    match damping {
        None => {}
        Some(ModalDamping::Constant(ratio)) => {
            let _ = writeln!(out, "*Modal damping\n1, 1000000, {}", number(*ratio));
        }
        Some(ModalDamping::Direct(ranges)) => {
            out.push_str("*Modal damping\n");
            for range in ranges {
                let _ = writeln!(
                    out,
                    "{}, {}, {}",
                    range.lowest,
                    range.highest,
                    number(range.ratio)
                );
            }
        }
        Some(ModalDamping::Rayleigh(r)) => {
            let _ = writeln!(
                out,
                "*Modal damping, Rayleigh\n , , {}, {}",
                number(r.alpha),
                number(r.beta)
            );
        }
    }
}

/// The `*Step` line and the procedure with its increments, for steps with a time period.
fn incremented_step(settings: &StaticStep, keyword: String, nlgeom: bool) -> (String, String) {
    let default = settings.incrementation == Incrementation::Default;
    let mut header = String::from("*Step");
    if nlgeom {
        header.push_str(", Nlgeom");
    }
    if !default {
        let _ = write!(header, ", Inc={}", settings.max_increments);
    }
    header.push('\n');
    let procedure = match settings.incrementation {
        Incrementation::Default => format!("{keyword}\n"),
        Incrementation::Automatic => format!(
            "{keyword}\n{}, {}, {}, {}\n",
            number(settings.initial_increment),
            number(settings.time_period),
            number(settings.min_increment),
            number(settings.max_increment)
        ),
        Incrementation::Direct => format!(
            "{keyword}, Direct\n{}, {}\n",
            number(settings.initial_increment),
            number(settings.time_period)
        ),
    };
    (header, procedure)
}

/// The `*Step` line and the procedure keyword of a frequency step, as PrePoMax writes them:
/// the lower bound is written as 0 when only an upper bound is given.
fn frequency_step(settings: &FrequencyStep) -> (String, String) {
    let mut header = String::from("*Step");
    if settings.perturbation {
        header.push_str(", Perturbation");
    }
    header.push('\n');
    let mut procedure = String::from("*Frequency");
    if let Some(solver) = settings.solver.keyword() {
        let _ = write!(procedure, ", Solver={solver}");
    }
    if settings.storage {
        procedure.push_str(", Storage=Yes");
    }
    let _ = write!(procedure, "\n{}", settings.num_frequencies);
    match (settings.lower_frequency, settings.upper_frequency) {
        (None, None) => {}
        (lower, None) => {
            let _ = write!(procedure, ", {}", number(lower.unwrap_or(0.0)));
        }
        (lower, Some(upper)) => {
            let lower = number(lower.unwrap_or(0.0));
            let _ = write!(procedure, ", {lower}, {}", number(upper));
        }
    }
    procedure.push('\n');
    (header, procedure)
}

/// The `*Step` line and the procedure keyword of a buckle step, as PrePoMax's
/// `CalBuckleStep` writes them.
fn buckle_step(settings: &BuckleStep) -> (String, String) {
    let mut header = String::from("*Step");
    if settings.perturbation {
        header.push_str(", Perturbation");
    }
    header.push('\n');
    let mut procedure = String::from("*Buckle");
    if let Some(solver) = settings.solver.keyword() {
        let _ = write!(procedure, ", Solver={solver}");
    }
    let _ = writeln!(
        procedure,
        "\n{}, {}",
        settings.num_factors,
        number(settings.accuracy)
    );
    (header, procedure)
}

fn field_output(output: &FieldOutput) -> Option<Keyword> {
    if output.variables.is_empty() {
        return None;
    }
    let mut variables = output.variables.join(", ");
    let keyword = match output.kind {
        OutputKind::Node => "*Node file",
        OutputKind::Element => {
            // PrePoMax adds nodal extrapolation of stresses unless error estimators are requested.
            let has = |v: &str| output.variables.iter().any(|x| x == v);
            if has("S") && !has("ERR") && !has("HER") && !has("ZZS") {
                variables.push_str(", NOE");
            }
            "*El file"
        }
    };
    Some(Keyword::generated(format!(
        "** Name: {}\n{keyword}\n{variables}\n",
        output.name
    )))
}

/// `*Node print`, `*El print` or `*Contact print` of a history output, as PrePoMax's
/// `CalNodePrint`, `CalElPrint` and `CalContactPrint` write them; none without variables.
fn history_output(
    sets: &mut Sets,
    output: &HistoryOutput,
    pair_surfaces: &PairSurfaces,
) -> Result<Option<Keyword>, WriteError> {
    if output.variables.is_empty() {
        return Ok(None);
    }
    let totals = match output.totals {
        Totals::No => "",
        Totals::Yes => ", Totals=Yes",
        Totals::Only => ", Totals=Only",
    };
    let header = match &output.kind {
        HistoryKind::Node { region } => {
            let set = sets.node_set(&output.name, region)?;
            format!("*Node print, Nset={set}{totals}")
        }
        HistoryKind::Element { region } => {
            let set = sets.element_set(&output.name, region)?;
            format!("*El print, Elset={set}{totals}")
        }
        HistoryKind::Contact { pair } => {
            // Contact forces are summed over the slave surface of the pair.
            let mut header = format!("*Contact print{totals}");
            if output.variables.iter().any(|v| v == "CF") {
                let (master, slave) =
                    pair_surfaces
                        .get(pair)
                        .ok_or_else(|| WriteError::UnknownContactPair {
                            item: output.name.clone(),
                            pair: pair.clone(),
                        })?;
                let _ = write!(header, ", Master={master}, Slave={slave}");
            }
            header
        }
    };
    Ok(Some(Keyword::generated(format!(
        "** Name: {}\n{header}\n{}\n",
        output.name,
        output.variables.join(", ")
    ))))
}

/// A keyword line followed by ids, 16 per line, the most CalculiX reads on one line.
fn id_list(header: &str, ids: &[u32]) -> String {
    let lines: Vec<String> = ids
        .chunks(16)
        .map(|chunk| {
            let ids: Vec<String> = chunk.iter().map(u32::to_string).collect();
            ids.join(", ")
        })
        .collect();
    format!("{header}\n{}\n", lines.join(",\n"))
}

/// A real number in at most 16 characters, CalculiX's field width.
/// Length and direction of a vector; `None` for the zero vector.
fn unit_vector(v: [f64; 3]) -> Option<(f64, [f64; 3])> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 0.0).then(|| (length, v.map(|c| c / length)))
}

fn number(value: f64) -> String {
    let short = value.to_string();
    if short.len() <= 16 {
        short
    } else {
        format!("{value:.8E}")
    }
}

/// Item names as CalculiX accepts them: no spaces, commas or other special characters.
fn name(item: &str) -> String {
    item.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn empty(item: &str, what: &'static str) -> WriteError {
    WriteError::EmptyRegion {
        item: item.to_owned(),
        what,
    }
}

mod constraints;
#[cfg(test)]
mod tests;
