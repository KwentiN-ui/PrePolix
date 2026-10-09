//! Items whose references are gone, like PrePoMax's invalid features: a section whose material
//! was deleted, or a load on a node set the mesh no longer has. Validity is derived from the
//! model each time, so an item becomes valid again as soon as its reference is back.

use plx_mesh::FeMesh;

use crate::{FeModel, Region};

/// An item of the model that can refer to something else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelItem {
    Section(usize),
    /// Boundary condition by step and index.
    BoundaryCondition(usize, usize),
    /// Load by step and index.
    Load(usize, usize),
}

/// An item with a missing reference and why it is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    pub item: ModelItem,
    pub reason: String,
}

impl FeModel {
    /// Items that refer to materials, parts, sets or mesh entities that do not exist.
    pub fn invalid_items(&self, mesh: &FeMesh) -> Vec<Invalid> {
        let mut invalid = Vec::new();
        for (i, section) in self.sections.iter().enumerate() {
            let reason = if !self.materials.iter().any(|m| m.name == section.material) {
                Some(format!("Material {} existiert nicht", section.material))
            } else {
                section.region.missing_reference(mesh)
            };
            if let Some(reason) = reason {
                invalid.push(Invalid {
                    item: ModelItem::Section(i),
                    reason,
                });
            }
        }
        for (s, step) in self.steps.iter().enumerate() {
            for (i, bc) in step.boundary_conditions.iter().enumerate() {
                if let Some(reason) = bc.region.missing_reference(mesh) {
                    invalid.push(Invalid {
                        item: ModelItem::BoundaryCondition(s, i),
                        reason,
                    });
                }
            }
            for (i, load) in step.loads.iter().enumerate() {
                if let Some(reason) = load.region.missing_reference(mesh) {
                    invalid.push(Invalid {
                        item: ModelItem::Load(s, i),
                        reason,
                    });
                }
            }
        }
        invalid
    }
}

impl Region {
    /// What the region refers to that the mesh does not have, e.g. "Node Set FIX existiert
    /// nicht".
    pub fn missing_reference(&self, mesh: &FeMesh) -> Option<String> {
        match self {
            Region::Parts(names) => names
                .iter()
                .find(|name| !mesh.parts.iter().any(|p| &p.name == *name))
                .map(|name| format!("Part {name} existiert nicht")),
            Region::NodeSet(name) => (!mesh.node_sets.contains_key(name))
                .then(|| format!("Node Set {name} existiert nicht")),
            Region::ElementSet(name) => (!mesh.element_sets.contains_key(name))
                .then(|| format!("Element Set {name} existiert nicht")),
            Region::Surface(name) => (!mesh.surfaces.contains_key(name))
                .then(|| format!("Surface {name} existiert nicht")),
            Region::Nodes(nodes) => {
                let missing = nodes.iter().filter(|&&n| mesh.node(n).is_none()).count();
                (missing > 0).then(|| format!("{missing} Knoten existieren nicht"))
            }
            Region::Faces(faces) => {
                let missing = faces
                    .iter()
                    .filter(|&&(element, face)| {
                        mesh.element(element)
                            .is_none_or(|e| face == 0 || usize::from(face) > e.shape.faces().len())
                    })
                    .count();
                (missing > 0).then(|| format!("{missing} Elementflächen existieren nicht"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use plx_mesh::{Element, ElementShape, Part};

    use super::*;
    use crate::{BoundaryCondition, BoundaryKind, Material, Section, Step};

    fn mesh() -> FeMesh {
        let mut mesh = FeMesh::default();
        for (id, coords) in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ]
        .into_iter()
        .enumerate()
        {
            mesh.set_node(id as u32 + 1, coords);
        }
        mesh.add_element(Element {
            id: 1,
            type_name: "C3D4".into(),
            shape: ElementShape::Tet4,
            nodes: vec![1, 2, 3, 4],
        })
        .unwrap();
        mesh.parts.push(Part {
            name: "PART-1".into(),
            elements: vec![1],
        });
        mesh.node_sets.insert("FIX".into(), vec![1]);
        mesh
    }

    fn model() -> FeModel {
        let mut step = Step::new_static("Step-1");
        step.boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            region: Region::NodeSet("FIX".into()),
            kind: BoundaryKind::Fixed,
        });
        FeModel {
            materials: vec![Material {
                name: "Steel".into(),
                density: None,
                elastic: None,
            }],
            sections: vec![Section {
                name: "Section-1".into(),
                material: "Steel".into(),
                region: Region::Parts(vec!["PART-1".into()]),
            }],
            steps: vec![step],
            user_keywords: Vec::new(),
        }
    }

    #[test]
    fn complete_model_is_valid() {
        assert_eq!(model().invalid_items(&mesh()), []);
    }

    #[test]
    fn section_without_its_material_is_invalid_until_it_is_back() {
        let mut model = model();
        let steel = model.materials.remove(0);
        let invalid = model.invalid_items(&mesh());
        assert_eq!(invalid.len(), 1);
        assert_eq!(invalid[0].item, ModelItem::Section(0));
        assert!(invalid[0].reason.contains("Steel"));
        model.materials.push(steel);
        assert_eq!(model.invalid_items(&mesh()), []);
    }

    #[test]
    fn missing_sets_and_mesh_entities_are_found() {
        let mesh = mesh();
        let mut model = model();
        model.steps[0].boundary_conditions[0].region = Region::NodeSet("GONE".into());
        assert_eq!(
            model.invalid_items(&mesh)[0].item,
            ModelItem::BoundaryCondition(0, 0)
        );
        assert!(
            Region::Nodes(vec![1, 99])
                .missing_reference(&mesh)
                .is_some()
        );
        assert!(
            Region::Faces(vec![(1, 4)])
                .missing_reference(&mesh)
                .is_none()
        );
        assert!(
            Region::Faces(vec![(1, 5)])
                .missing_reference(&mesh)
                .is_some()
        );
        assert!(
            Region::Faces(vec![(2, 1)])
                .missing_reference(&mesh)
                .is_some()
        );
        assert!(
            Region::Parts(vec!["PART-2".into()])
                .missing_reference(&mesh)
                .is_some()
        );
    }
}
