//! Writer for CalculiX input files, following the layout of PrePoMax's `CalculixFileWriter`.
//!
//! The mesh is written together with the analysis of a [`FeModel`]. Regions the user picked in
//! the GUI become node sets, element sets and surfaces named like PrePoMax's internal
//! selections (`Internal_Selection-1_Fixed-1`), so the user never defines sets by hand. The
//! file is built as a [`Keyword`] tree first, which is where the keyword editor inserts the
//! user's own keywords.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use plx_mesh::{ElementId, FeMesh, NodeId, SurfaceDefinition};
use plx_model::{
    BoundaryKind, FeModel, FieldOutput, FrequencyStep, Incrementation, LoadKind, OutputKind,
    Region, StaticStep, Step, StepKind, UserKeyword,
};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WriteError {
    #[error("{item}: Bereich enthält keine {what}")]
    EmptyRegion { item: String, what: &'static str },
    #[error("{item}: Material {material} existiert nicht")]
    UnknownMaterial { item: String, material: String },
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
    let mut sets = Sets::new(mesh);
    // Regions are resolved first: their sets must precede the materials and steps.
    let materials = materials(model);
    let sections = sections(&mut sets, model)?;
    let steps = model
        .steps
        .iter()
        .map(|step| write_step(&mut sets, step))
        .collect::<Result<Vec<_>, _>>()?;

    let mut nodes = String::from("*Node\n");
    for (id, &[x, y, z]) in mesh.node_ids().iter().zip(mesh.coords()) {
        let _ = writeln!(nodes, "{id}, {:.8E}, {:.8E}, {:.8E}", x, y, z);
    }
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
    Ok(vec![
        Keyword::title("Heading", vec![Keyword::generated(heading)]),
        Keyword::title("Nodes", vec![Keyword::generated(nodes)]),
        Keyword::title("Elements", elements(mesh)),
        Keyword::title("Node sets", node_sets),
        Keyword::title("Element sets", element_sets),
        Keyword::title("Surfaces", surfaces),
        empty("Physical constants"),
        empty("Coordinate systems"),
        Keyword::title("Materials", materials),
        Keyword::title("Sections", sections),
        empty("Pre-tension sections"),
        empty("Constraints"),
        empty("Surface interactions"),
        empty("Contact pairs"),
        empty("Amplitudes"),
        empty("Initial conditions"),
        Keyword::title("Steps", steps),
    ])
}

/// One `*Element` block per part and element type; the part name is the element set.
fn elements(mesh: &FeMesh) -> Vec<Keyword> {
    let mut written = vec![false; mesh.element_count()];
    let mut groups: Vec<(Option<&str>, &str, Vec<ElementId>)> = Vec::new();
    for part in &mesh.parts {
        for &id in &part.elements {
            let Some(index) = mesh.element_index(id) else {
                continue;
            };
            if !std::mem::replace(&mut written[index], true) {
                let type_name = mesh.elements()[index].type_name.as_str();
                group(&mut groups, Some(&part.name), type_name).push(id);
            }
        }
    }
    for (element, done) in mesh.elements().iter().zip(&written) {
        if !done {
            group(&mut groups, None, &element.type_name).push(element.id);
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
            // A line holds 16 entries; the rest continues on the next line.
            for (entry, node) in (2..).zip(&element.nodes) {
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
    groups: &'b mut Vec<(Option<&'a str>, &'a str, Vec<ElementId>)>,
    set: Option<&'a str>,
    type_name: &'a str,
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

    /// Element sets and face numbers of a face region, for pressure loads.
    fn face_sets(&mut self, item: &str, region: &Region) -> Result<Vec<(String, u8)>, WriteError> {
        match region {
            Region::Surface(surface) => {
                if let Some((sides, _)) = self.surface_sets.get(surface) {
                    return Ok(sides.clone());
                }
            }
            Region::Faces(faces) if !faces.is_empty() => {
                let surface = self.free_name("Internal_Selection", &name(item));
                self.add_face_surface(&surface, faces);
                return Ok(self.surface_sets[&surface].0.clone());
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
            let header = format!("*Material, Name={}\n", name(&material.name));
            Keyword::parent(header, properties)
        })
        .collect()
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
        let set = sets.element_set(&section.name, &section.region)?;
        let material = name(&section.material);
        keywords.push(Keyword::generated(format!(
            "** Name: {}\n*Solid section, Elset={set}, Material={material}\n",
            section.name
        )));
    }
    Ok(keywords)
}

/// A step as PrePoMax structures it: the step title holds `*Step`, which holds the procedure
/// and a title for each kind of item, down to the one holding `*End step`.
///
/// Like PrePoMax, a deactivated step or item keeps its place in the file as a comment
/// (`** Name: Fixed-1: Deactivated`), and nothing of it is written, not even its sets.
fn write_step(sets: &mut Sets, step: &Step) -> Result<Keyword, WriteError> {
    if !step.active {
        return Ok(deactivated_step(step));
    }
    let (header, procedure) = match &step.kind {
        StepKind::Static(settings) => static_step(settings),
        StepKind::Frequency(settings) => frequency_step(settings),
    };
    let mut boundaries = vec![Keyword::generated("*Boundary, op=New\n".into())];
    for bc in &step.boundary_conditions {
        if !bc.active {
            boundaries.push(deactivated(&bc.name));
            continue;
        }
        let set = sets.node_set(&bc.name, &bc.region)?;
        let mut out = format!("** Name: {}\n*Boundary\n", bc.name);
        match bc.kind {
            BoundaryKind::Fixed => {
                let _ = writeln!(out, "{set}, 1, 6, 0");
            }
            BoundaryKind::Displacement(values) => {
                for (dof, value) in (1..).zip(values) {
                    if let Some(value) = value {
                        let _ = writeln!(out, "{set}, {dof}, {dof}, {}", number(value));
                    }
                }
            }
        }
        boundaries.push(Keyword::generated(out));
    }
    let mut loads = Vec::new();
    // Like PrePoMax, a step that takes no loads gets none written, not even the reset.
    let step_loads: &[_] = if step.kind.supports_loads() {
        loads.push(Keyword::generated("*Cload, op=New\n".into()));
        loads.push(Keyword::generated("*Dload, op=New\n".into()));
        &step.loads
    } else {
        &[]
    };
    for load in step_loads {
        if !load.active {
            loads.push(deactivated(&load.name));
            continue;
        }
        let mut out = format!("** Name: {}\n", load.name);
        match load.kind {
            LoadKind::ConcentratedForce(force) => {
                let set = sets.node_set(&load.name, &load.region)?;
                out.push_str("*Cload\n");
                for (dof, value) in (1..).zip(force) {
                    if value != 0.0 {
                        let _ = writeln!(out, "{set}, {dof}, {}", number(value));
                    }
                }
            }
            LoadKind::Pressure(pressure) => {
                out.push_str("*Dload\n");
                for (set, face) in sets.face_sets(&load.name, &load.region)? {
                    let _ = writeln!(out, "{set}, P{face}, {}", number(pressure));
                }
            }
            LoadKind::SurfaceTraction(force) => {
                let faces = load.region.faces(sets.mesh);
                let nodal = traction_forces(sets.mesh, &faces, force);
                if nodal.is_empty() {
                    return Err(empty(&load.name, "Elementflächen"));
                }
                out.push_str("*Cload\n");
                for (node, values) in nodal {
                    for (dof, value) in (1..).zip(values) {
                        if value != 0.0 {
                            let _ = writeln!(out, "{node}, {dof}, {}", number(value));
                        }
                    }
                }
            }
        }
        loads.push(Keyword::generated(out));
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
        Keyword::title("History outputs", Vec::new()),
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
        Keyword::title("History outputs", Vec::new()),
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
fn traction_forces(
    mesh: &FeMesh,
    faces: &[(ElementId, u8)],
    force: [f64; 3],
) -> BTreeMap<NodeId, [f64; 3]> {
    let mut weighted: Vec<(NodeId, f64)> = Vec::new();
    let mut total_area = 0.0;
    for &(element, face) in faces {
        let Some(element) = mesh.element(element) else {
            continue;
        };
        let Some(topology) = (face as usize)
            .checked_sub(1)
            .and_then(|f| element.shape.faces().get(f))
        else {
            continue;
        };
        let node = |local: usize| element.nodes.get(local).copied();
        let point = |local: usize| node(local).and_then(|id| mesh.node(id));
        let corners: Vec<[f64; 3]> = topology.corners.iter().filter_map(|&l| point(l)).collect();
        if corners.len() != topology.corners.len() {
            continue;
        }
        let area = polygon_area(&corners);
        total_area += area;
        let quadratic = element.shape.is_quadratic() && !topology.mids.is_empty();
        let (corner_weight, mid_weight) = match (corners.len(), quadratic) {
            (3, true) => (0.0, 1.0 / 3.0),
            (4, true) => (-1.0 / 12.0, 1.0 / 3.0),
            (n, _) => (1.0 / n as f64, 0.0),
        };
        for &local in topology.corners {
            weighted.extend(node(local).map(|id| (id, area * corner_weight)));
        }
        if quadratic {
            for &local in topology.mids {
                weighted.extend(node(local).map(|id| (id, area * mid_weight)));
            }
        }
    }
    let mut nodal: BTreeMap<NodeId, [f64; 3]> = BTreeMap::new();
    if total_area <= 0.0 {
        return nodal;
    }
    for (node, weight) in weighted {
        let share = nodal.entry(node).or_default();
        for k in 0..3 {
            share[k] += force[k] * weight / total_area;
        }
    }
    nodal
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

/// The `*Step` line and the procedure keyword of a static step.
fn static_step(settings: &StaticStep) -> (String, String) {
    let default = settings.incrementation == Incrementation::Default;
    let mut header = String::from("*Step");
    if settings.nlgeom {
        header.push_str(", Nlgeom");
    }
    if !default {
        let _ = write!(header, ", Inc={}", settings.max_increments);
    }
    header.push('\n');
    let mut keyword = String::from("*Static");
    if let Some(solver) = settings.solver.keyword() {
        let _ = write!(keyword, ", Solver={solver}");
    }
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

#[cfg(test)]
mod tests;
