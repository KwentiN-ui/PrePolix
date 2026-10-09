use std::collections::BTreeSet;

use plx_mesh::{ElementId, FeMesh, NodeId, SurfaceDefinition};
use serde::{Deserialize, Serialize};

/// Where a section, boundary condition or load applies.
///
/// Selections made in the GUI are stored as they were picked; the sets CalculiX needs are
/// derived from them when writing the input file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Region {
    /// Whole parts by name.
    Parts(Vec<String>),
    /// Picked nodes.
    Nodes(Vec<NodeId>),
    /// Picked element faces as (element, CalculiX face number 1..6).
    Faces(Vec<(ElementId, u8)>),
    /// A node set of the input file.
    NodeSet(String),
    /// An element set of the input file.
    ElementSet(String),
    /// A surface of the input file.
    Surface(String),
}

impl Region {
    /// Elements of the region, for sections. Node and face selections have none.
    pub fn elements(&self, mesh: &FeMesh) -> Vec<ElementId> {
        let mut ids: BTreeSet<ElementId> = BTreeSet::new();
        match self {
            Region::Parts(names) => {
                for part in mesh.parts.iter().filter(|p| names.contains(&p.name)) {
                    ids.extend(part.elements.iter().copied());
                }
            }
            Region::ElementSet(name) => {
                ids.extend(mesh.element_sets.get(name).into_iter().flatten().copied());
            }
            Region::Faces(faces) => ids.extend(faces.iter().map(|(e, _)| *e)),
            Region::Nodes(_) | Region::NodeSet(_) | Region::Surface(_) => {}
        }
        ids.into_iter().collect()
    }

    /// Nodes of the region in ascending order, for boundary conditions and point loads.
    pub fn nodes(&self, mesh: &FeMesh) -> Vec<NodeId> {
        let mut ids: BTreeSet<NodeId> = BTreeSet::new();
        let element_nodes = |ids: &mut BTreeSet<NodeId>, elements: &[ElementId]| {
            for element in elements.iter().filter_map(|&e| mesh.element(e)) {
                ids.extend(element.nodes.iter().copied());
            }
        };
        match self {
            Region::Parts(_) | Region::ElementSet(_) => {
                element_nodes(&mut ids, &self.elements(mesh));
            }
            Region::Nodes(nodes) => ids.extend(nodes.iter().copied()),
            Region::NodeSet(name) => {
                ids.extend(mesh.node_sets.get(name).into_iter().flatten().copied());
            }
            Region::Faces(faces) => ids.extend(face_nodes(mesh, faces)),
            Region::Surface(name) => match mesh.surfaces.get(name) {
                Some(SurfaceDefinition::Nodes(nodes)) => ids.extend(nodes.iter().copied()),
                Some(SurfaceDefinition::ElementFaces(faces)) => {
                    ids.extend(face_nodes(mesh, faces));
                }
                None => {}
            },
        }
        ids.into_iter().collect()
    }

    /// Element faces of the region, for pressure loads.
    pub fn faces(&self, mesh: &FeMesh) -> Vec<(ElementId, u8)> {
        match self {
            Region::Faces(faces) => faces.clone(),
            Region::Surface(name) => match mesh.surfaces.get(name) {
                Some(SurfaceDefinition::ElementFaces(faces)) => faces.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Short description for the GUI, e.g. "3 Knoten".
    pub fn describe(&self) -> String {
        match self {
            Region::Parts(names) => names.join(", "),
            Region::Nodes(nodes) => format!("{} Knoten", nodes.len()),
            Region::Faces(faces) => format!("{} Elementflächen", faces.len()),
            Region::NodeSet(name) | Region::ElementSet(name) | Region::Surface(name) => {
                name.clone()
            }
        }
    }
}

/// All nodes (corners and midside) of element faces.
fn face_nodes<'a>(
    mesh: &'a FeMesh,
    faces: &'a [(ElementId, u8)],
) -> impl Iterator<Item = NodeId> + 'a {
    faces.iter().flat_map(move |&(element, face)| {
        let element = mesh.element(element);
        let nodes: Vec<NodeId> = element
            .and_then(|e| {
                let topology = e.faces().get(usize::from(face).checked_sub(1)?)?;
                let quadratic = e.shape.is_quadratic();
                Some(
                    topology
                        .corners
                        .iter()
                        .chain(if quadratic { topology.mids } else { &[] })
                        .filter_map(|&local| e.nodes.get(local).copied())
                        .collect(),
                )
            })
            .unwrap_or_default();
        nodes
    })
}
