//! Items whose references are gone, like PrePoMax's invalid features: a section whose material
//! was deleted, or a load on a node set the mesh no longer has. Validity is derived from the
//! model each time, so an item becomes valid again as soon as its reference is back.

use plx_mesh::FeMesh;

use crate::{FeModel, LoadKind, Region, Section, SectionKind, line_tangent};

/// An item of the model that can refer to something else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelItem {
    Section(usize),
    Constraint(usize),
    ContactPair(usize),
    /// Boundary condition by step and index.
    BoundaryCondition(usize, usize),
    /// Load by step and index.
    Load(usize, usize),
    Material(usize),
    /// Part of the mesh by index.
    Part(usize),
    Step(usize),
    /// The boundary conditions of a step as a whole.
    BoundaryConditions(usize),
    /// The analysis, for problems CalculiX reported.
    Analysis,
    InitialCondition(usize),
    Amplitude(usize),
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
                (section.region.missing_reference(mesh)).or_else(|| section.kind_problem(mesh))
            };
            if let Some(reason) = reason {
                invalid.push(Invalid {
                    item: ModelItem::Section(i),
                    reason,
                });
            }
        }
        for (i, constraint) in self.constraints.iter().enumerate() {
            let reason = match constraint.master_slave() {
                Some([master, slave]) => master_slave_reference(master, slave, mesh),
                None => (constraint.regions().into_iter())
                    .find_map(|region| region.missing_reference(mesh)),
            };
            if let Some(reason) = reason {
                invalid.push(Invalid {
                    item: ModelItem::Constraint(i),
                    reason,
                });
            }
        }
        for (i, pair) in self.contact_pairs.iter().enumerate() {
            let reason = if !(self.surface_interactions.iter()).any(|s| s.name == pair.interaction)
            {
                Some(format!(
                    "Surface Interaction {} existiert nicht",
                    pair.interaction
                ))
            } else {
                master_slave_reference(&pair.master, &pair.slave, mesh)
            };
            if let Some(reason) = reason {
                invalid.push(Invalid {
                    item: ModelItem::ContactPair(i),
                    reason,
                });
            }
        }
        for (s, step) in self.steps.iter().enumerate() {
            for (i, bc) in step.boundary_conditions.iter().enumerate() {
                let reason = (bc.region.missing_reference(mesh))
                    .or_else(|| self.missing_amplitude([&bc.amplitude]));
                if let Some(reason) = reason {
                    invalid.push(Invalid {
                        item: ModelItem::BoundaryCondition(s, i),
                        reason,
                    });
                }
            }
            for (i, load) in step.loads.iter().enumerate() {
                let radiation = matches!(load.kind, LoadKind::Radiation { .. });
                let constants = self.properties.absolute_zero.is_some()
                    && self.properties.stefan_boltzmann.is_some();
                let reason = if radiation && !constants {
                    Some(
                        "Strahlung braucht den absoluten Nullpunkt und die \
                         Stefan-Boltzmann-Konstante (Modelleigenschaften)"
                            .into(),
                    )
                } else {
                    (load.region.missing_reference(mesh)).or_else(|| {
                        self.missing_amplitude([&load.amplitude, &load.factor_amplitude])
                    })
                };
                if let Some(reason) = reason {
                    invalid.push(Invalid {
                        item: ModelItem::Load(s, i),
                        reason,
                    });
                }
            }
        }
        for (i, condition) in self.initial_conditions.iter().enumerate() {
            if let Some(reason) = condition.region.missing_reference(mesh) {
                invalid.push(Invalid {
                    item: ModelItem::InitialCondition(i),
                    reason,
                });
            }
        }
        for (i, amplitude) in self.amplitudes.iter().enumerate() {
            if let Some(reason) = amplitude.points_problem() {
                invalid.push(Invalid {
                    item: ModelItem::Amplitude(i),
                    reason,
                });
            }
        }
        invalid
    }

    /// The first of the amplitude references that names no amplitude of the model.
    fn missing_amplitude<'a>(
        &self,
        references: impl IntoIterator<Item = &'a Option<String>>,
    ) -> Option<String> {
        (references.into_iter().flatten())
            .find(|name| self.amplitude(name).is_none())
            .map(|name| format!("Amplitude {name} existiert nicht"))
    }
}

impl Section {
    /// Why the section's kind does not fit its elements or its own values, if it does not:
    /// a beam section on solids, a solid section on lines, a pipe on 2-node lines, a normal
    /// parallel to a beam, or dimensions CalculiX would reject.
    pub fn kind_problem(&self, mesh: &FeMesh) -> Option<String> {
        match &self.kind {
            SectionKind::Solid => {}
            SectionKind::Truss { area } => {
                if !(area.is_finite() && *area > 0.0) {
                    return Some("Die Querschnittsfläche muss größer als 0 sein".into());
                }
            }
            SectionKind::Beam(beam) => {
                if !beam.profile.is_valid() {
                    return Some("Die Profilmaße sind ungültig".into());
                }
            }
        }
        let elements = self.region.elements(mesh);
        for element in elements.iter().filter_map(|&id| mesh.element(id)) {
            if let Some(reason) = self.kind.rejects(element) {
                return Some(reason);
            }
            if let SectionKind::Beam(beam) = &self.kind
                && beam
                    .orientation
                    .normal_for(line_tangent(mesh, element))
                    .is_none()
            {
                return Some(format!(
                    "Die Normale ist parallel zur Achse von Element {}",
                    element.id
                ));
            }
        }
        None
    }
}

/// What the master or slave region of a contact or tie refers to that the mesh lacks.
fn master_slave_reference(master: &Region, slave: &Region, mesh: &FeMesh) -> Option<String> {
    (master
        .missing_reference(mesh)
        .map(|r| format!("Master: {r}")))
    .or_else(|| slave.missing_reference(mesh).map(|r| format!("Slave: {r}")))
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
                            .is_none_or(|e| face == 0 || usize::from(face) > e.faces().len())
                    })
                    .count();
                (missing > 0).then(|| format!("{missing} Elementflächen existieren nicht"))
            }
            Region::Geometry(entities) => {
                if mesh.cad.is_empty() {
                    return Some("Das Netz ist nicht aus der Geometrie erzeugt".into());
                }
                let missing = entities.iter().filter(|&&e| !mesh.cad.contains(e)).count();
                (missing > 0).then(|| format!("{missing} Geometrieelemente sind nicht vernetzt"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use plx_mesh::{Element, ElementShape, Part};

    use super::*;
    use crate::{
        BoundaryCondition, BoundaryKind, ContactPair, Material, Section, Step, SurfaceInteraction,
    };

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
            active: true,
            region: Region::NodeSet("FIX".into()),
            kind: BoundaryKind::Fixed,
            amplitude: None,
        });
        FeModel {
            properties: Default::default(),
            materials: vec![Material {
                name: "Steel".into(),
                density: None,
                elastic: None,
                ..Default::default()
            }],
            sections: vec![Section {
                name: "Section-1".into(),
                material: "Steel".into(),
                region: Region::Parts(vec!["PART-1".into()]),
                thickness: 1.0,
                kind: SectionKind::Solid,
            }],
            steps: vec![step],
            user_keywords: Vec::new(),
            ..FeModel::default()
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
    fn references_follow_amplitudes() {
        let mesh = mesh();
        let mut model = model();
        model.steps[0].boundary_conditions[0].amplitude = Some("Ramp".into());
        let invalid = model.invalid_items(&mesh);
        assert_eq!(invalid[0].item, ModelItem::BoundaryCondition(0, 0));
        assert!(invalid[0].reason.contains("Ramp"));
        model.amplitudes.push(crate::Amplitude::new("Ramp"));
        assert_eq!(model.invalid_items(&mesh), []);
        model.rename_amplitude("Ramp", "Ramp-2");
        model.amplitudes[0].name = "Ramp-2".into();
        assert_eq!(model.invalid_items(&mesh), []);
        model.amplitudes[0].points.clear();
        assert_eq!(model.invalid_items(&mesh)[0].item, ModelItem::Amplitude(0));
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

    #[test]
    fn contact_pair_needs_its_interaction_and_surfaces() {
        let mesh = mesh();
        let mut model = model();
        let mut pair = ContactPair::new("Contact_Pair-1", "Surface_Interaction-1");
        pair.master = Region::Faces(vec![(1, 1)]);
        pair.slave = Region::Faces(vec![(1, 2)]);
        model.contact_pairs.push(pair);
        let invalid = model.invalid_items(&mesh);
        assert_eq!(invalid[0].item, ModelItem::ContactPair(0));
        assert!(invalid[0].reason.contains("Surface_Interaction-1"));
        model.surface_interactions.push(SurfaceInteraction {
            name: "Surface_Interaction-1".into(),
            properties: Vec::new(),
        });
        assert_eq!(model.invalid_items(&mesh), []);
        model.contact_pairs[0].slave = Region::Surface("GONE".into());
        assert!(model.invalid_items(&mesh)[0].reason.starts_with("Slave"));
    }
}
