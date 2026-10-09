use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::mesh::{ElementId, FeMesh, NodeId};

/// A face, edge or vertex of the CAD geometry, by Gmsh's tag. Tags stay the same each time
/// the geometry is read, so a selection by CAD entity survives remeshing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CadEntity {
    Face(i32),
    Edge(i32),
    Vertex(i32),
}

/// Where the CAD entities lie in a mesh generated from the geometry: kept with the mesh and
/// rebuilt with it, so that selections by CAD entity find their nodes and faces again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CadMap {
    /// Nodes on each CAD face, edge and vertex, with those on its boundary.
    pub nodes: BTreeMap<CadEntity, Vec<NodeId>>,
    /// Element faces on each CAD face: faces of solid elements as (element, face number),
    /// shell elements with face 1.
    pub faces: BTreeMap<i32, Vec<(ElementId, u8)>>,
    /// The mesh segments along each CAD edge, by their end nodes. 2D elements' edges are
    /// found from these, since flipping an element renumbers its edges.
    pub segments: BTreeMap<i32, Vec<[NodeId; 2]>>,
}

impl CadMap {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Whether the mesh has the entity.
    pub fn contains(&self, entity: CadEntity) -> bool {
        self.nodes.contains_key(&entity)
    }

    /// The map with node and element numbers shifted, for a part merged into a mesh.
    pub fn offset(&self, nodes: NodeId, elements: ElementId) -> Self {
        Self {
            nodes: (self.nodes.iter())
                .map(|(&k, v)| (k, v.iter().map(|n| n + nodes).collect()))
                .collect(),
            faces: (self.faces.iter())
                .map(|(&k, v)| (k, v.iter().map(|&(e, f)| (e + elements, f)).collect()))
                .collect(),
            segments: (self.segments.iter())
                .map(|(&k, v)| (k, v.iter().map(|s| s.map(|n| n + nodes)).collect()))
                .collect(),
        }
    }

    /// The map without removed elements and nodes; entities left without any go.
    pub fn without(&self, elements: &BTreeSet<ElementId>, nodes: &BTreeSet<NodeId>) -> Self {
        let mut map = Self::default();
        for (&entity, ids) in &self.nodes {
            let kept: Vec<NodeId> = ids.iter().copied().filter(|n| !nodes.contains(n)).collect();
            if !kept.is_empty() {
                map.nodes.insert(entity, kept);
            }
        }
        for (&face, ids) in &self.faces {
            let kept: Vec<_> = (ids.iter().copied())
                .filter(|(e, _)| !elements.contains(e))
                .collect();
            if !kept.is_empty() {
                map.faces.insert(face, kept);
            }
        }
        for (&edge, segments) in &self.segments {
            let kept: Vec<_> = (segments.iter().copied())
                .filter(|s| !s.iter().any(|n| nodes.contains(n)))
                .collect();
            if !kept.is_empty() {
                map.segments.insert(edge, kept);
            }
        }
        map
    }

    /// Adds the entities of another map; an entity in both gets the union.
    pub fn extend(&mut self, other: Self) {
        for (entity, ids) in other.nodes {
            let list = self.nodes.entry(entity).or_default();
            list.extend(ids);
            list.sort_unstable();
            list.dedup();
        }
        for (face, ids) in other.faces {
            self.faces.entry(face).or_default().extend(ids);
        }
        for (edge, segments) in other.segments {
            self.segments.entry(edge).or_default().extend(segments);
        }
    }

    /// The CAD face of each element face, the reverse of [`Self::faces`].
    pub fn face_of(&self) -> BTreeMap<(ElementId, u8), i32> {
        (self.faces.iter())
            .flat_map(|(&tag, faces)| faces.iter().map(move |&f| (f, tag)))
            .collect()
    }
}

impl FeMesh {
    /// The CAD faces that element faces make up, if they are whole CAD faces and nothing
    /// else, so that a region of them can be kept by geometry.
    pub fn whole_cad_faces(&self, faces: &[(ElementId, u8)]) -> Option<Vec<CadEntity>> {
        if faces.is_empty() || self.cad.is_empty() {
            return None;
        }
        let face_of = self.cad.face_of();
        let tags: BTreeSet<i32> = (faces.iter())
            .map(|f| face_of.get(f).copied())
            .collect::<Option<_>>()?;
        let given: BTreeSet<(ElementId, u8)> = faces.iter().copied().collect();
        let covered: BTreeSet<(ElementId, u8)> = (tags.iter())
            .flat_map(|t| self.cad.faces[t].iter().copied())
            .collect();
        (covered == given).then(|| tags.into_iter().map(CadEntity::Face).collect())
    }

    /// Nodes of CAD entities, in ascending order.
    pub fn cad_nodes(&self, entities: &[CadEntity]) -> Vec<NodeId> {
        let ids: BTreeSet<NodeId> = (entities.iter())
            .filter_map(|e| self.cad.nodes.get(e))
            .flatten()
            .copied()
            .collect();
        ids.into_iter().collect()
    }

    /// Element faces on CAD entities: the faces on CAD faces of solids and shells, and the
    /// edges of 2D elements (their faces, see [`crate::Element::faces`]) along CAD edges.
    pub fn cad_faces(&self, entities: &[CadEntity]) -> Vec<(ElementId, u8)> {
        let mut faces = BTreeSet::new();
        let mut segments: BTreeSet<[NodeId; 2]> = BTreeSet::new();
        for entity in entities {
            match *entity {
                CadEntity::Face(tag) => {
                    let on_face = self.cad.faces.get(&tag).into_iter().flatten();
                    // A 2D element's face numbers name its edges, not the element itself.
                    faces.extend(
                        on_face.filter(|(e, _)| self.element(*e).is_some_and(|e| !e.is_plane())),
                    );
                }
                CadEntity::Edge(tag) => {
                    let along = self.cad.segments.get(&tag).into_iter().flatten();
                    segments.extend(along.map(|&[a, b]| [a.min(b), a.max(b)]));
                }
                CadEntity::Vertex(_) => {}
            }
        }
        if !segments.is_empty() {
            for element in self.elements().iter().filter(|e| e.is_plane()) {
                for (k, edge) in element.faces().iter().enumerate() {
                    let (Some(&a), Some(&b)) = (
                        edge.corners.first().and_then(|&i| element.nodes.get(i)),
                        edge.corners.get(1).and_then(|&i| element.nodes.get(i)),
                    ) else {
                        continue;
                    };
                    if segments.contains(&[a.min(b), a.max(b)]) {
                        faces.insert((element.id, (k + 1) as u8));
                    }
                }
            }
        }
        faces.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, ElementShape};

    /// Two plane stress quadrilaterals side by side, 1-2-5-4 and 2-3-6-5, meshed from CAD
    /// face 1 with the bottom edge 7 (nodes 1, 2, 3).
    fn strip() -> FeMesh {
        let mut mesh = FeMesh::default();
        for (id, x, y) in [
            (1, 0., 0.),
            (2, 1., 0.),
            (3, 2., 0.),
            (4, 0., 1.),
            (5, 1., 1.),
            (6, 2., 1.),
        ] {
            mesh.set_node(id, [x, y, 0.0]);
        }
        for (id, nodes) in [(1, vec![1, 2, 5, 4]), (2, vec![2, 3, 6, 5])] {
            mesh.add_element(Element {
                id,
                type_name: "CPS4".into(),
                shape: ElementShape::Quad4,
                nodes,
            })
            .unwrap();
        }
        mesh.cad
            .nodes
            .insert(CadEntity::Face(1), vec![1, 2, 3, 4, 5, 6]);
        mesh.cad.nodes.insert(CadEntity::Edge(7), vec![1, 2, 3]);
        mesh.cad.faces.insert(1, vec![(1, 1), (2, 1)]);
        mesh.cad.segments.insert(7, vec![[1, 2], [2, 3]]);
        mesh
    }

    #[test]
    fn edges_of_2d_elements_are_found_after_flipping() {
        let mut mesh = strip();
        let edge = [CadEntity::Edge(7)];
        assert_eq!(mesh.cad_faces(&edge), [(1, 1), (2, 1)]);
        assert_eq!(mesh.cad_nodes(&edge), [1, 2, 3]);
        // A 2D element's face numbers name edges, so a CAD face gives none.
        assert!(mesh.cad_faces(&[CadEntity::Face(1)]).is_empty());
        // Flipped, the bottom edge runs from node 4 to 1: edge 4 of the elements.
        mesh.flip_surface_elements(|_| true);
        assert_eq!(mesh.cad_faces(&edge), [(1, 4), (2, 4)]);
    }

    #[test]
    fn maps_follow_merged_and_removed_parts() {
        let mesh = strip();
        let moved = mesh.cad.offset(10, 100);
        assert_eq!(moved.faces[&1], [(101, 1), (102, 1)]);
        assert_eq!(moved.segments[&7], [[11, 12], [12, 13]]);
        let kept = mesh
            .cad
            .without(&BTreeSet::from([2]), &BTreeSet::from([3, 6]));
        assert_eq!(kept.faces[&1], [(1, 1)]);
        assert_eq!(kept.segments[&7], [[1, 2]]);
        assert_eq!(kept.nodes[&CadEntity::Edge(7)], [1, 2]);
        let gone = mesh
            .cad
            .without(&BTreeSet::from([1, 2]), &BTreeSet::from([1, 2, 3, 4, 5, 6]));
        assert!(gone.is_empty() && gone.faces.is_empty());
        let mut both = kept.clone();
        both.extend(moved);
        assert!(both.contains(CadEntity::Face(1)));
        assert_eq!(both.faces[&1].len(), 3);
    }

    #[test]
    fn whole_cad_faces_are_recognised() {
        let mut mesh = strip();
        mesh.cad.faces.insert(2, vec![(5, 3)]);
        assert_eq!(
            mesh.whole_cad_faces(&[(2, 1), (1, 1)]),
            Some(vec![CadEntity::Face(1)])
        );
        assert_eq!(
            mesh.whole_cad_faces(&[(1, 1)]),
            None,
            "only part of the face"
        );
        assert_eq!(mesh.whole_cad_faces(&[(1, 2)]), None, "not on the geometry");
        assert_eq!(mesh.whole_cad_faces(&[]), None);
    }
}
