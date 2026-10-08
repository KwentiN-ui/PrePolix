use std::collections::{BTreeMap, HashMap};

use crate::element::ElementShape;

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

#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceDefinition {
    /// Element faces as (element, 1-based face number S1…S6).
    ElementFaces(Vec<(ElementId, u8)>),
    Nodes(Vec<NodeId>),
}

/// A named group of elements shown and hidden together, like a part in PrePoMax.
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, Default)]
pub struct FeMesh {
    node_ids: Vec<NodeId>,
    coords: Vec<[f64; 3]>,
    node_lookup: HashMap<NodeId, usize>,
    elements: Vec<Element>,
    element_lookup: HashMap<ElementId, usize>,
    pub node_sets: BTreeMap<String, Vec<NodeId>>,
    pub element_sets: BTreeMap<String, Vec<ElementId>>,
    pub surfaces: BTreeMap<String, SurfaceDefinition>,
    pub parts: Vec<Part>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tet(id: ElementId, nodes: Vec<NodeId>) -> Element {
        Element {
            id,
            type_name: "C3D4".into(),
            shape: ElementShape::Tet4,
            nodes,
        }
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
