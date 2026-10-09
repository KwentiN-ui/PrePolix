use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cad::CadMap;
use crate::element::{ElementShape, FaceTopology};
use crate::fast_map::FastMap;

pub type NodeId = u32;
pub type ElementId = u32;

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub id: ElementId,
    /// Type name as written in the input file, e.g. `C3D10` or `S8R`.
    pub type_name: String,
    pub shape: ElementShape,
    pub nodes: Vec<NodeId>,
}

impl Element {
    /// Whether this is a plane stress, plane strain or axisymmetric element of a 2D model.
    /// Its faces are its edges then, see [`Self::faces`].
    pub fn is_plane(&self) -> bool {
        let name = self.type_name.as_bytes();
        ["CPS", "CPE", "CAX"]
            .iter()
            .any(|p| name.len() >= 3 && name[..3].eq_ignore_ascii_case(p.as_bytes()))
    }

    /// Faces in CalculiX face order. A 2D element's faces are its edges, as CalculiX numbers
    /// them in surfaces and distributed loads (S1 = P1 = edge from node 1 to 2).
    pub fn faces(&self) -> &'static [FaceTopology] {
        if self.is_plane() {
            self.shape.edges()
        } else {
            self.shape.faces()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SurfaceDefinition {
    /// Element faces as (element, 1-based face number S1…S6).
    ElementFaces(Vec<(ElementId, u8)>),
    Nodes(Vec<NodeId>),
}

/// A named group of elements shown and hidden together, like a part in PrePoMax.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub elements: Vec<ElementId>,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MeshError {
    #[error("Element {id} vom Typ {type_name} braucht {expected} Knoten, hat aber {actual}")]
    WrongNodeCount {
        id: ElementId,
        type_name: String,
        expected: usize,
        actual: usize,
    },
    #[error("Element {0} ist doppelt definiert")]
    DuplicateElement(ElementId),
}

/// Finite element mesh with nodes, elements, sets and parts.
///
/// Set, surface and part names are stored upper case, because CalculiX treats them case-insensitively.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(try_from = "MeshFile", into = "MeshFile")]
pub struct FeMesh {
    node_ids: Vec<NodeId>,
    coords: Vec<[f64; 3]>,
    node_lookup: FastMap<NodeId, usize>,
    elements: Vec<Element>,
    element_lookup: FastMap<ElementId, usize>,
    pub node_sets: BTreeMap<String, Vec<NodeId>>,
    pub element_sets: BTreeMap<String, Vec<ElementId>>,
    pub surfaces: BTreeMap<String, SurfaceDefinition>,
    pub parts: Vec<Part>,
    /// Where the CAD entities lie, for a mesh generated from geometry.
    pub cad: CadMap,
}

impl FeMesh {
    /// Adds a node or moves an existing one; returns true if the id was already defined.
    pub fn set_node(&mut self, id: NodeId, coords: [f64; 3]) -> bool {
        match self.node_lookup.get(&id) {
            Some(&index) => {
                self.coords[index] = coords;
                true
            }
            None => {
                self.node_lookup.insert(id, self.node_ids.len());
                self.node_ids.push(id);
                self.coords.push(coords);
                false
            }
        }
    }

    pub fn add_element(&mut self, element: Element) -> Result<(), MeshError> {
        let expected = element.shape.node_count();
        if element.nodes.len() != expected {
            return Err(MeshError::WrongNodeCount {
                id: element.id,
                type_name: element.type_name,
                expected,
                actual: element.nodes.len(),
            });
        }
        if self.element_lookup.contains_key(&element.id) {
            return Err(MeshError::DuplicateElement(element.id));
        }
        self.element_lookup.insert(element.id, self.elements.len());
        self.elements.push(element);
        Ok(())
    }

    pub fn node_count(&self) -> usize {
        self.node_ids.len()
    }

    pub fn element_count(&self) -> usize {
        self.elements.len()
    }

    pub fn node_ids(&self) -> &[NodeId] {
        &self.node_ids
    }

    /// Coordinates in insertion order, parallel to [`Self::node_ids`].
    pub fn coords(&self) -> &[[f64; 3]] {
        &self.coords
    }

    /// Enlarges the mesh by `factor` about the origin, e.g. when its length unit changes.
    pub fn scale(&mut self, factor: f64) {
        for point in &mut self.coords {
            *point = point.map(|c| c * factor);
        }
    }

    pub fn node_index(&self, id: NodeId) -> Option<usize> {
        self.node_lookup.get(&id).copied()
    }

    pub fn node(&self, id: NodeId) -> Option<[f64; 3]> {
        self.node_index(id).map(|index| self.coords[index])
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    pub fn element_index(&self, id: ElementId) -> Option<usize> {
        self.element_lookup.get(&id).copied()
    }

    pub fn element(&self, id: ElementId) -> Option<&Element> {
        self.element_index(id).map(|index| &self.elements[index])
    }

    /// Axis-aligned bounding box of all nodes.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let mut coords = self.coords.iter();
        let first = *coords.next()?;
        Some(coords.fold((first, first), |(mut min, mut max), c| {
            for axis in 0..3 {
                min[axis] = min[axis].min(c[axis]);
                max[axis] = max[axis].max(c[axis]);
            }
            (min, max)
        }))
    }

    /// Element node references that point to undefined nodes, as (element, node).
    pub fn missing_nodes(&self) -> Vec<(ElementId, NodeId)> {
        self.elements
            .iter()
            .flat_map(|e| e.nodes.iter().map(move |&n| (e.id, n)))
            .filter(|&(_, n)| !self.node_lookup.contains_key(&n))
            .collect()
    }

    /// Changes the type names of the elements, e.g. when the model space changes.
    pub fn retype_elements(&mut self, mut type_name: impl FnMut(&str, ElementShape) -> String) {
        for element in &mut self.elements {
            element.type_name = type_name(&element.type_name, element.shape);
        }
    }

    /// Reverses the node order of triangles and quadrilaterals for which `flip` holds, which
    /// turns their normal around; corners and midside nodes stay matched.
    pub fn flip_surface_elements(&mut self, mut flip: impl FnMut(&Element) -> bool) {
        for element in &mut self.elements {
            let order: &[usize] = match element.shape {
                ElementShape::Tri3 => &[0, 2, 1],
                ElementShape::Tri6 => &[0, 2, 1, 5, 4, 3],
                ElementShape::Quad4 => &[0, 3, 2, 1],
                ElementShape::Quad8 => &[0, 3, 2, 1, 7, 6, 5, 4],
                _ => continue,
            };
            if flip(element) {
                element.nodes = order.iter().map(|&k| element.nodes[k]).collect();
            }
        }
    }

    /// Replaces nodes by others everywhere they are referred to (elements, sets, surfaces,
    /// the CAD map) and drops them, e.g. to join line parts at a point.
    pub fn merge_nodes(&mut self, replaced: &BTreeMap<NodeId, NodeId>) {
        if replaced.is_empty() {
            return;
        }
        let new = |n: NodeId| replaced.get(&n).copied().unwrap_or(n);
        let relist = |nodes: &[NodeId]| -> Vec<NodeId> {
            let mut nodes: Vec<NodeId> = nodes.iter().map(|&n| new(n)).collect();
            nodes.sort_unstable();
            nodes.dedup();
            nodes
        };
        for element in &mut self.elements {
            for node in &mut element.nodes {
                *node = new(*node);
            }
        }
        for nodes in self.node_sets.values_mut() {
            *nodes = relist(nodes);
        }
        for surface in self.surfaces.values_mut() {
            if let SurfaceDefinition::Nodes(nodes) = surface {
                *nodes = relist(nodes);
            }
        }
        self.cad = CadMap {
            nodes: (self.cad.nodes.iter())
                .map(|(&entity, nodes)| (entity, relist(nodes)))
                .collect(),
            faces: self.cad.faces.clone(),
            segments: (self.cad.segments.iter())
                .map(|(&edge, segments)| (edge, segments.iter().map(|s| s.map(new)).collect()))
                .collect(),
        };
        let (ids, coords): (Vec<NodeId>, Vec<[f64; 3]>) = (self.node_ids.iter().copied())
            .zip(self.coords.iter().copied())
            .filter(|(id, _)| !replaced.contains_key(id))
            .unzip();
        self.node_ids = ids;
        self.coords = coords;
        self.node_lookup = (self.node_ids.iter().copied())
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
    }

    /// Whether a part, set or surface already has this name; CalculiX ignores case.
    pub fn name_in_use(&self, name: &str) -> bool {
        let same = |n: &String| n.eq_ignore_ascii_case(name);
        self.parts.iter().any(|p| same(&p.name))
            || self.node_sets.keys().any(same)
            || self.element_sets.keys().any(same)
            || self.surfaces.keys().any(same)
    }

    /// Renames a part together with the element set of the same name that an input file
    /// defines for it. Returns the old name.
    pub fn rename_part(&mut self, index: usize, name: &str) -> Option<String> {
        let part = self.parts.get_mut(index)?;
        let old = std::mem::replace(&mut part.name, name.to_string());
        if let Some(elements) = self.element_sets.remove(&old) {
            self.element_sets.insert(name.to_string(), elements);
        }
        Some(old)
    }
}

/// How a mesh is stored in project files: nodes and elements without lookup tables; element
/// shapes follow from the type names.
#[derive(Serialize, Deserialize)]
struct MeshFile {
    node_ids: Vec<NodeId>,
    coords: Vec<[f64; 3]>,
    elements: Vec<(ElementId, String, Vec<NodeId>)>,
    #[serde(default)]
    node_sets: BTreeMap<String, Vec<NodeId>>,
    #[serde(default)]
    element_sets: BTreeMap<String, Vec<ElementId>>,
    #[serde(default)]
    surfaces: BTreeMap<String, SurfaceDefinition>,
    #[serde(default)]
    parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "CadMap::is_empty")]
    cad: CadMap,
}

impl From<FeMesh> for MeshFile {
    fn from(mesh: FeMesh) -> Self {
        Self {
            node_ids: mesh.node_ids,
            coords: mesh.coords,
            elements: (mesh.elements.into_iter())
                .map(|e| (e.id, e.type_name, e.nodes))
                .collect(),
            node_sets: mesh.node_sets,
            element_sets: mesh.element_sets,
            surfaces: mesh.surfaces,
            parts: mesh.parts,
            cad: mesh.cad,
        }
    }
}

impl TryFrom<MeshFile> for FeMesh {
    type Error = String;

    fn try_from(file: MeshFile) -> Result<Self, String> {
        if file.node_ids.len() != file.coords.len() {
            return Err("Knotennummern und Koordinaten passen nicht zusammen".into());
        }
        let mut mesh = FeMesh::default();
        for (id, coords) in file.node_ids.into_iter().zip(file.coords) {
            mesh.set_node(id, coords);
        }
        for (id, type_name, nodes) in file.elements {
            let shape = ElementShape::from_type_name(&type_name)
                .ok_or_else(|| format!("Element {id}: unbekannter Typ {type_name}"))?;
            mesh.add_element(Element {
                id,
                type_name,
                shape,
                nodes,
            })
            .map_err(|e| e.to_string())?;
        }
        mesh.node_sets = file.node_sets;
        mesh.element_sets = file.element_sets;
        mesh.surfaces = file.surfaces;
        mesh.parts = file.parts;
        mesh.cad = file.cad;
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_nodes_vanish_everywhere() {
        use crate::cad::CadEntity;
        let mut mesh = FeMesh::default();
        for (id, x) in [(1, 0.0), (2, 10.0), (3, 10.0), (4, 20.0)] {
            mesh.set_node(id, [x, 0.0, 0.0]);
        }
        for (id, nodes) in [(1, vec![1, 2]), (2, vec![3, 4])] {
            mesh.add_element(Element {
                id,
                type_name: "B31".into(),
                shape: ElementShape::Line2,
                nodes,
            })
            .unwrap();
        }
        mesh.node_sets.insert("ENDS".into(), vec![2, 3, 4]);
        mesh.surfaces
            .insert("S".into(), SurfaceDefinition::Nodes(vec![3]));
        mesh.cad.nodes.insert(CadEntity::Vertex(2), vec![2, 3]);
        mesh.cad.segments.insert(2, vec![[3, 4]]);
        mesh.merge_nodes(&BTreeMap::from([(3, 2)]));
        assert_eq!(mesh.node_ids(), [1, 2, 4]);
        assert_eq!(mesh.node_index(4), Some(2));
        assert!(mesh.node(3).is_none());
        assert_eq!(mesh.element(2).unwrap().nodes, [2, 4]);
        assert_eq!(mesh.node_sets["ENDS"], [2, 4]);
        assert_eq!(mesh.surfaces["S"], SurfaceDefinition::Nodes(vec![2]));
        assert_eq!(mesh.cad.nodes[&CadEntity::Vertex(2)], [2]);
        assert_eq!(mesh.cad.segments[&2], [[2, 4]]);
        assert!(mesh.missing_nodes().is_empty());
    }

    #[test]
    fn meshes_survive_serialization() {
        let mut mesh = FeMesh::default();
        for (id, x) in [(1, 0.0), (2, 1.0), (3, 0.5), (4, 0.25)] {
            mesh.set_node(id, [x, x * 2.0, 0.1]);
        }
        mesh.add_element(tet(7, vec![1, 2, 3, 4])).unwrap();
        mesh.parts.push(Part {
            name: "SOLID".into(),
            elements: vec![7],
        });
        mesh.surfaces
            .insert("TOP".into(), SurfaceDefinition::ElementFaces(vec![(7, 2)]));
        mesh.cad
            .nodes
            .insert(crate::CadEntity::Face(3), vec![1, 2, 3]);
        mesh.cad.faces.insert(3, vec![(7, 1)]);
        let text = ron::to_string(&mesh).unwrap();
        let read: FeMesh = ron::from_str(&text).unwrap();
        assert_eq!(read.node_ids(), mesh.node_ids());
        assert_eq!(read.coords(), mesh.coords());
        assert_eq!(read.elements(), mesh.elements());
        assert_eq!(read.element_index(7), Some(0));
        assert_eq!(read.cad, mesh.cad);
        assert_eq!((read.parts, read.surfaces), (mesh.parts, mesh.surfaces));
        let broken = text.replace("C3D4", "XYZ");
        assert!(ron::from_str::<FeMesh>(&broken).is_err());
    }

    fn tet(id: ElementId, nodes: Vec<NodeId>) -> Element {
        Element {
            id,
            type_name: "C3D4".into(),
            shape: ElementShape::Tet4,
            nodes,
        }
    }

    #[test]
    fn renamed_parts_take_their_element_set_along() {
        let mut mesh = FeMesh::default();
        mesh.parts.push(Part {
            name: "SOLID".into(),
            elements: vec![7],
        });
        mesh.element_sets.insert("SOLID".into(), vec![7]);
        mesh.node_sets.insert("FIX".into(), vec![1]);
        assert!(mesh.name_in_use("fix") && mesh.name_in_use("Solid"));
        assert_eq!(mesh.rename_part(0, "BRACKET").as_deref(), Some("SOLID"));
        assert_eq!(mesh.parts[0].name, "BRACKET");
        assert_eq!(mesh.element_sets.get("BRACKET"), Some(&vec![7]));
        assert!(!mesh.name_in_use("SOLID"));
        assert_eq!(mesh.rename_part(1, "X"), None);
    }

    #[test]
    fn nodes_are_found_by_id() {
        let mut mesh = FeMesh::default();
        assert!(!mesh.set_node(10, [1.0, 2.0, 3.0]));
        assert!(!mesh.set_node(3, [0.0, 0.0, 0.0]));
        assert!(mesh.set_node(10, [1.0, 2.0, 4.0]));
        assert_eq!(mesh.node_count(), 2);
        assert_eq!(mesh.node(10), Some([1.0, 2.0, 4.0]));
        assert_eq!(mesh.node(7), None);
        assert_eq!(mesh.bounds(), Some(([0.0, 0.0, 0.0], [1.0, 2.0, 4.0])));
    }

    #[test]
    fn elements_are_validated() {
        let mut mesh = FeMesh::default();
        mesh.add_element(tet(1, vec![1, 2, 3, 4])).unwrap();
        assert_eq!(
            mesh.add_element(tet(1, vec![1, 2, 3, 4])),
            Err(MeshError::DuplicateElement(1))
        );
        assert!(matches!(
            mesh.add_element(tet(2, vec![1, 2, 3])),
            Err(MeshError::WrongNodeCount {
                expected: 4,
                actual: 3,
                ..
            })
        ));
        for (id, x) in [(1, 0.0), (2, 1.0), (3, 2.0)] {
            mesh.set_node(id, [x, 0.0, 0.0]);
        }
        assert_eq!(mesh.missing_nodes(), vec![(1, 4)]);
        assert_eq!(mesh.element(1).map(|e| e.shape), Some(ElementShape::Tet4));
    }
}
