//! Writer for CalculiX input files, following the layout of PrePoMax's `CalculixFileWriter`.
//!
//! The mesh is written together with the analysis of a [`FeModel`]. Regions the user picked in
//! the GUI become node sets, element sets and surfaces named like PrePoMax's internal
//! selections (`Internal_Selection-1_Fixed-1`), so the user never defines sets by hand.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use plx_mesh::{ElementId, FeMesh, NodeId, SurfaceDefinition};
use plx_model::{
    BoundaryKind, FeModel, FieldOutput, Incrementation, LoadKind, OutputKind, Region, StaticStep,
    Step, StepKind,
};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WriteError {
    #[error("{item}: Bereich enthält keine {what}")]
    EmptyRegion { item: String, what: &'static str },
    #[error("{item}: Material {material} existiert nicht")]
    UnknownMaterial { item: String, material: String },
}

/// Writes the mesh and the analysis as the text of an input file.
pub fn write_inp(mesh: &FeMesh, model: &FeModel, heading: &str) -> Result<String, WriteError> {
    let mut sets = Sets::new(mesh);
    // Regions are resolved first: their sets must precede the materials and steps.
    let analysis = analysis(&mut sets, model)?;

    let mut out = String::new();
    title(&mut out, "Heading");
    out.push_str("*Heading\n");
    out.push_str(heading.lines().next().unwrap_or_default());
    out.push('\n');
    title(&mut out, "Nodes");
    out.push_str("*Node\n");
    for (id, &[x, y, z]) in mesh.node_ids().iter().zip(mesh.coords()) {
        let _ = writeln!(out, "{id}, {:.8E}, {:.8E}, {:.8E}", x, y, z);
    }
    title(&mut out, "Elements");
    elements(&mut out, mesh);
    title(&mut out, "Node sets");
    for (name, ids) in &sets.node_sets {
        write_list(&mut out, &format!("*Nset, Nset={name}"), ids);
    }
    title(&mut out, "Element sets");
    for (name, members) in &sets.element_sets {
        let _ = writeln!(out, "*Elset, Elset={name}");
        match members {
            Members::Ids(ids) => write_ids(&mut out, ids),
            Members::Parts(parts) => {
                let _ = writeln!(out, "{}", parts.join(",\n"));
            }
        }
    }
    title(&mut out, "Surfaces");
    for (name, surface) in &sets.surfaces {
        match surface {
            Surface::Faces(faces) => {
                let _ = writeln!(out, "*Surface, Name={name}, Type=Element");
                for (set, face) in faces {
                    let _ = writeln!(out, "{set}, S{face}");
                }
            }
            Surface::Nodes(set) => {
                let _ = writeln!(out, "*Surface, Name={name}, Type=Node\n{set}");
            }
        }
    }
    out.push_str(&analysis);
    Ok(out)
}

/// Section banner as PrePoMax writes it: `** Nodes ++++…`, 60 characters wide.
fn title(out: &mut String, title: &str) {
    let _ = writeln!(out, "**\n{:+<60}\n**", format!("** {title} "));
}

/// One `*Element` block per part and element type; the part name is the element set.
fn elements(out: &mut String, mesh: &FeMesh) {
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
    for (set, type_name, mut ids) in groups {
        let _ = match set {
            Some(set) => writeln!(out, "*Element, Type={type_name}, Elset={set}"),
            None => writeln!(out, "*Element, Type={type_name}"),
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
    }
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

fn analysis(sets: &mut Sets, model: &FeModel) -> Result<String, WriteError> {
    let mut out = String::new();
    title(&mut out, "Materials");
    for material in &model.materials {
        let _ = writeln!(out, "*Material, Name={}", name(&material.name));
        if let Some(density) = material.density {
            let _ = writeln!(out, "*Density\n{}", number(density));
        }
        if let Some(elastic) = material.elastic {
            let (young, poisson) = (number(elastic.young), number(elastic.poisson));
            let _ = writeln!(out, "*Elastic\n{young}, {poisson}");
        }
    }
    title(&mut out, "Sections");
    for section in &model.sections {
        if !model.materials.iter().any(|m| m.name == section.material) {
            return Err(WriteError::UnknownMaterial {
                item: section.name.clone(),
                material: section.material.clone(),
            });
        }
        let set = sets.element_set(&section.name, &section.region)?;
        let material = name(&section.material);
        let _ = writeln!(out, "** Name: {}", section.name);
        let _ = writeln!(out, "*Solid section, Elset={set}, Material={material}");
    }
    title(&mut out, "Steps");
    for step in &model.steps {
        write_step(&mut out, sets, step)?;
    }
    Ok(out)
}

fn write_step(out: &mut String, sets: &mut Sets, step: &Step) -> Result<(), WriteError> {
    title(out, &step.name);
    match &step.kind {
        StepKind::Static(settings) => static_step(out, settings),
    }
    title(out, "Boundary conditions");
    out.push_str("*Boundary, op=New\n");
    for bc in &step.boundary_conditions {
        let set = sets.node_set(&bc.name, &bc.region)?;
        let _ = writeln!(out, "** Name: {}\n*Boundary", bc.name);
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
    }
    title(out, "Loads");
    out.push_str("*Cload, op=New\n*Dload, op=New\n");
    for load in &step.loads {
        let _ = writeln!(out, "** Name: {}", load.name);
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
        }
    }
    title(out, "Field outputs");
    for output in &step.field_outputs {
        field_output(out, output);
    }
    title(out, "End step");
    out.push_str("*End step\n");
    Ok(())
}

fn static_step(out: &mut String, settings: &StaticStep) {
    let default = settings.incrementation == Incrementation::Default;
    out.push_str("*Step");
    if settings.nlgeom {
        out.push_str(", Nlgeom");
    }
    if !default {
        let _ = write!(out, ", Inc={}", settings.max_increments);
    }
    out.push_str("\n*Static");
    match settings.incrementation {
        Incrementation::Default => out.push('\n'),
        Incrementation::Automatic => {
            let _ = writeln!(
                out,
                "\n{}, {}, {}, {}",
                number(settings.initial_increment),
                number(settings.time_period),
                number(settings.min_increment),
                number(settings.max_increment)
            );
        }
        Incrementation::Direct => {
            let _ = writeln!(
                out,
                ", Direct\n{}, {}",
                number(settings.initial_increment),
                number(settings.time_period)
            );
        }
    }
}

fn field_output(out: &mut String, output: &FieldOutput) {
    if output.variables.is_empty() {
        return;
    }
    let _ = writeln!(out, "** Name: {}", output.name);
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
    let _ = writeln!(out, "{keyword}\n{variables}");
}

/// Writes ids with 16 per line, the most CalculiX reads on one line.
fn write_list(out: &mut String, header: &str, ids: &[u32]) {
    let _ = writeln!(out, "{header}");
    write_ids(out, ids);
}

fn write_ids(out: &mut String, ids: &[u32]) {
    let lines: Vec<String> = ids
        .chunks(16)
        .map(|chunk| {
            let ids: Vec<String> = chunk.iter().map(u32::to_string).collect();
            ids.join(", ")
        })
        .collect();
    let _ = writeln!(out, "{}", lines.join(",\n"));
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
