//! Joining line parts at their ends. Beams and trusses meshed from separate CAD edges meet
//! at the same points but get nodes of their own, and CalculiX connects elements only
//! through shared nodes: a truss of loose bars is a set of mechanisms.

use std::collections::{BTreeMap, BTreeSet};

use crate::cad::CadMap;
use crate::element::ElementShape;
use crate::mesh::{FeMesh, NodeId, SurfaceDefinition};

impl FeMesh {
    /// Joins the ends of line elements (beams, trusses) that lie within `tolerance` of each
    /// other into one node, so that lines meeting at a point form a frame or truss. Ends on a
    /// node that a solid or shell element uses stay as they are, so lines are not glued to a
    /// solid by accident. Returns the number of nodes that went.
    pub fn join_line_ends(&mut self, tolerance: f64) -> usize {
        let mut other_nodes = BTreeSet::new();
        let mut ends = BTreeSet::new();
        for element in &self.elements {
            match element.shape {
                ElementShape::Line2 | ElementShape::Line3 => {
                    ends.insert(element.nodes[0]);
                    ends.insert(element.nodes[element.nodes.len() - 1]);
                }
                _ => other_nodes.extend(element.nodes.iter().copied()),
            }
        }
        let mut candidates: Vec<(NodeId, [f64; 3])> = (ends.into_iter())
            .filter(|n| !other_nodes.contains(n))
            .filter_map(|n| Some((n, self.node(n)?)))
            .collect();
        // A sweep along x: only ends within the tolerance in x can be within it in space.
        candidates.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]).then(a.0.cmp(&b.0)));
        let mut replaced: BTreeMap<NodeId, NodeId> = BTreeMap::new();
        for (i, &(id, p)) in candidates.iter().enumerate() {
            if replaced.contains_key(&id) {
                continue;
            }
            for &(other, q) in &candidates[i + 1..] {
                if q[0] - p[0] > tolerance {
                    break;
                }
                if !replaced.contains_key(&other) && distance(p, q) <= tolerance {
                    replaced.insert(other, id);
                }
            }
        }
        self.replace_nodes(&replaced);
        replaced.len()
    }

    /// A tolerance for [`Self::join_line_ends`]: a millionth of the mesh's extent, but well
    /// below the shortest line element, so that no element collapses.
    pub fn line_join_tolerance(&self) -> f64 {
        let diagonal = self.bounds().map_or(0.0, |(min, max)| distance(min, max));
        let shortest = (self.elements.iter())
            .filter(|e| matches!(e.shape, ElementShape::Line2 | ElementShape::Line3))
            .filter_map(|e| {
                let a = self.node(e.nodes[0])?;
                let b = self.node(e.nodes[e.nodes.len() - 1])?;
                Some(distance(a, b))
            })
            .fold(f64::INFINITY, f64::min);
        (1e-6 * diagonal).min(0.01 * shortest)
    }

    /// Replaces nodes by others everywhere they are referred to, and drops them.
    fn replace_nodes(&mut self, replaced: &BTreeMap<NodeId, NodeId>) {
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
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cad::CadEntity;
    use crate::mesh::{Element, Part};

    fn line(id: u32, nodes: Vec<NodeId>) -> Element {
        Element {
            id,
            type_name: "B31".into(),
            shape: ElementShape::Line2,
            nodes,
        }
    }

    /// Two bars meeting at (10, 0, 0) with nodes of their own there, and a tetrahedron with
    /// a corner at the far end of the second bar.
    fn loose_bars() -> FeMesh {
        let mut mesh = FeMesh::default();
        mesh.set_node(1, [0.0, 0.0, 0.0]);
        mesh.set_node(2, [10.0, 0.0, 0.0]);
        mesh.set_node(3, [10.0, 0.0, 1e-9]);
        mesh.set_node(4, [10.0, 10.0, 0.0]);
        for (id, coords) in [
            (5, [10.0, 10.0, 0.0]),
            (6, [11.0, 10.0, 0.0]),
            (7, [10.0, 11.0, 0.0]),
            (8, [10.0, 10.0, 1.0]),
        ] {
            mesh.set_node(id, coords);
        }
        mesh.add_element(line(1, vec![1, 2])).unwrap();
        mesh.add_element(line(2, vec![3, 4])).unwrap();
        mesh.add_element(Element {
            id: 3,
            type_name: "C3D4".into(),
            shape: ElementShape::Tet4,
            nodes: vec![5, 6, 7, 8],
        })
        .unwrap();
        mesh.parts = vec![
            Part {
                name: "LINE-1".into(),
                elements: vec![1],
            },
            Part {
                name: "LINE-2".into(),
                elements: vec![2],
            },
            Part {
                name: "SOLID-1".into(),
                elements: vec![3],
            },
        ];
        mesh.node_sets.insert("ENDS".into(), vec![2, 3, 4]);
        mesh.cad.nodes.insert(CadEntity::Vertex(2), vec![2]);
        mesh.cad.nodes.insert(CadEntity::Vertex(3), vec![3]);
        mesh.cad.nodes.insert(CadEntity::Edge(2), vec![3, 4]);
        mesh.cad.segments.insert(2, vec![[3, 4]]);
        mesh
    }

    #[test]
    fn bars_meeting_at_a_point_share_a_node() {
        let mut mesh = loose_bars();
        let tolerance = mesh.line_join_tolerance();
        assert!(tolerance > 1e-9 && tolerance < 0.1, "{tolerance}");
        assert_eq!(mesh.join_line_ends(tolerance), 1);
        assert_eq!(mesh.node_count(), 7);
        assert!(mesh.node(3).is_none());
        assert_eq!(mesh.element(2).unwrap().nodes, [2, 4]);
        assert_eq!(mesh.node_sets["ENDS"], [2, 4]);
        assert_eq!(mesh.cad.nodes[&CadEntity::Vertex(3)], [2]);
        assert_eq!(mesh.cad.nodes[&CadEntity::Edge(2)], [2, 4]);
        assert_eq!(mesh.cad.segments[&2], [[2, 4]]);
        // The bar's end on the tetrahedron's corner is left alone.
        assert_eq!(mesh.element(3).unwrap().nodes, [5, 6, 7, 8]);
        assert!(mesh.missing_nodes().is_empty());
        assert_eq!(mesh.join_line_ends(tolerance), 0);
    }

    #[test]
    fn ends_apart_stay_apart() {
        let mut mesh = loose_bars();
        mesh.set_node(3, [10.0, 0.5, 0.0]);
        assert_eq!(mesh.join_line_ends(mesh.line_join_tolerance()), 0);
        assert_eq!(mesh.node_count(), 8);
    }
}
