use plx_mesh::{ElementShape, FeMesh};
use serde::{Deserialize, Serialize};

use crate::UnitSystem;

/// What the model is about as a whole, PrePoMax's model properties: the model space, the
/// unit system, the physical constants and whether the model is a submodel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelProperties {
    pub space: ModelSpace,
    pub units: UnitSystem,
    /// PrePoMax's model type: a general model or a submodel driven by the results of a
    /// global model.
    pub kind: ModelKind,
    /// Results file (`.frd`) of the global model a submodel reads its boundary displacements
    /// from (`*SUBMODEL, INPUT=`); only used when [`ModelProperties::kind`] is
    /// [`ModelKind::Submodel`].
    pub global_results: Option<std::path::PathBuf>,
    /// Absolute zero on the model's temperature scale (`*PHYSICAL CONSTANTS`); radiation
    /// needs it. `None` leaves it undefined, as in PrePoMax.
    pub absolute_zero: Option<f64>,
    /// Stefan-Boltzmann constant in the model's units; radiation needs it.
    pub stefan_boltzmann: Option<f64>,
}

impl ModelProperties {
    /// The global results file of a submodel; `None` for a general model, as in PrePoMax.
    pub fn submodel_input(&self) -> Option<&std::path::Path> {
        (self.kind == ModelKind::Submodel)
            .then_some(self.global_results.as_deref())
            .flatten()
    }

    /// Absolute zero and the Stefan-Boltzmann constant in the units of `units`; `None`
    /// without units.
    pub fn standard_constants(units: UnitSystem) -> Option<(f64, f64)> {
        use crate::Quantity;
        if !units.has_units() {
            return None;
        }
        let zero = crate::UnitSystem::MKgSC.convert(-273.15, Quantity::Temperature, units);
        let sigma =
            crate::UnitSystem::MKgSC.convert(5.670_374_419e-8, Quantity::StefanBoltzmann, units);
        Some((zero, sigma))
    }
}

/// PrePoMax's model type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    #[default]
    General,
    /// A detailed model of a region of a global model: its cut boundary follows the
    /// displacements the global model computed there (`*SUBMODEL`, `*BOUNDARY, SUBMODEL`).
    Submodel,
}

impl ModelKind {
    pub const ALL: [ModelKind; 2] = [ModelKind::General, ModelKind::Submodel];

    /// Name in the GUI.
    pub fn label(self) -> &'static str {
        match self {
            ModelKind::General => "General model",
            ModelKind::Submodel => "Submodel",
        }
    }
}

/// Whether the model is a solid in space or a cross-section in the x-y plane, PrePoMax's
/// model space.
///
/// As in PrePoMax and CalculiX, 2D models lie in the x-y plane; an axisymmetric model
/// revolves about the y axis, with x as the radius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelSpace {
    #[default]
    ThreeD,
    /// Thin plate loaded in its plane (`CPS` elements, thickness in the section).
    PlaneStress,
    /// Long body loaded across its length (`CPE` elements, thickness in the section).
    PlaneStrain,
    /// Body of revolution about the y axis (`CAX` elements).
    Axisymmetric,
}

impl ModelSpace {
    pub const ALL: [ModelSpace; 4] = [
        ModelSpace::ThreeD,
        ModelSpace::PlaneStress,
        ModelSpace::PlaneStrain,
        ModelSpace::Axisymmetric,
    ];

    pub fn is_2d(self) -> bool {
        self != ModelSpace::ThreeD
    }

    /// Name in the GUI.
    pub fn label(self) -> &'static str {
        match self {
            ModelSpace::ThreeD => "3D",
            ModelSpace::PlaneStress => "2D ebener Spannungszustand",
            ModelSpace::PlaneStrain => "2D ebener Verzerrungszustand",
            ModelSpace::Axisymmetric => "2D rotationssymmetrisch",
        }
    }

    /// Whether the section of a 2D element has a thickness; an axisymmetric one spans the
    /// full revolution instead.
    pub fn has_thickness(self) -> bool {
        matches!(self, ModelSpace::PlaneStress | ModelSpace::PlaneStrain)
    }

    /// Prefix of the CalculiX element types of 2D models.
    fn prefix(self) -> &'static str {
        match self {
            ModelSpace::ThreeD => "S",
            ModelSpace::PlaneStress => "CPS",
            ModelSpace::PlaneStrain => "CPE",
            ModelSpace::Axisymmetric => "CAX",
        }
    }

    /// The model space an input file's elements imply: that of its plane stress, plane
    /// strain or axisymmetric elements, 3D otherwise.
    pub fn of_mesh(mesh: &FeMesh) -> ModelSpace {
        let first_plane = (mesh.elements().iter()).find_map(|e| plane_prefix(&e.type_name));
        match first_plane {
            Some("CPS") => ModelSpace::PlaneStress,
            Some("CPE") => ModelSpace::PlaneStrain,
            Some("CAX") => ModelSpace::Axisymmetric,
            _ => ModelSpace::ThreeD,
        }
    }

    /// The CalculiX type of a surface element in this model space, as PrePoMax switches the
    /// element types when the model space changes: plane elements of a 2D model take its
    /// prefix (CPS6 becomes CAX6), and shells of a 3D model; reduced integration is kept.
    /// Other elements keep their type.
    pub fn element_type(self, type_name: &str, shape: ElementShape) -> String {
        if shape.family() != plx_mesh::ElementFamily::Surface {
            return type_name.to_string();
        }
        let upper = type_name.to_ascii_uppercase();
        let Some(rest) = (["CPS", "CPE", "CAX", "M3D", "S"].iter())
            .find_map(|prefix| upper.strip_prefix(prefix))
        else {
            return type_name.to_string();
        };
        // Membranes and shells of a 3D model stay what they are.
        if !self.is_2d() && plane_prefix(&upper).is_none() {
            return type_name.to_string();
        }
        format!("{}{rest}", self.prefix())
    }

    /// Renames the surface elements of a mesh after [`Self::element_type`]; true when any
    /// changed.
    pub fn convert_mesh(self, mesh: &mut FeMesh) -> bool {
        let mut changed = false;
        mesh.retype_elements(|type_name, shape| {
            let new = self.element_type(type_name, shape);
            changed |= new != type_name;
            new
        });
        changed
    }
}

impl ModelSpace {
    /// Makes a mesh generated from the geometry fit the model space: gives its elements the
    /// model's types and, in 2D models, numbers them counter-clockwise seen from +z, as
    /// CalculiX needs. Fails if the mesh does not fit: 2D models take faces in the x-y
    /// plane only, axisymmetric ones on the side x >= 0 of the axis, 3D models solids only.
    pub fn prepare_generated_mesh(self, mesh: &mut FeMesh) -> Result<(), String> {
        use plx_mesh::ElementFamily;
        let family = |f| mesh.elements().iter().any(|e| e.shape.family() == f);
        if !self.is_2d() {
            if family(ElementFamily::Surface) {
                return Err(
                    "Flächen lassen sich nur in 2D-Modellen vernetzen; der Modellraum ist 3D"
                        .into(),
                );
            }
            return Ok(());
        }
        if family(ElementFamily::Solid) {
            return Err(format!(
                "Ein Modell im Modellraum \"{}\" kann keine Volumenkörper vernetzen",
                self.label()
            ));
        }
        if family(ElementFamily::Line) {
            return Err("Linien (Balken, Stäbe) lassen sich nur in 3D-Modellen vernetzen".into());
        }
        let Some((min, max)) = mesh.bounds() else {
            return Ok(());
        };
        let tolerance = 1e-6 * (0..3).map(|k| max[k] - min[k]).fold(0.0, f64::max);
        if min[2] < -tolerance || max[2] > tolerance {
            return Err("2D-Modelle müssen in der x-y-Ebene liegen (z = 0)".into());
        }
        if self == ModelSpace::Axisymmetric && min[0] < -tolerance {
            return Err(
                "Rotationssymmetrische Modelle müssen bei x >= 0 liegen; die y-Achse ist die \
                 Drehachse"
                    .into(),
            );
        }
        self.convert_mesh(mesh);
        let coords: Vec<[f64; 3]> = mesh.coords().to_vec();
        let index: std::collections::HashMap<_, _> = (mesh.node_ids().iter())
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();
        mesh.flip_surface_elements(|element| {
            let corners = element.shape.edges().len();
            let p: Vec<[f64; 3]> = (element.nodes[..corners].iter())
                .filter_map(|id| index.get(id).map(|&i| coords[i]))
                .collect();
            let twice_area: f64 = (0..p.len())
                .map(|k| {
                    let (a, b) = (p[k], p[(k + 1) % p.len()]);
                    a[0] * b[1] - a[1] * b[0]
                })
                .sum();
            twice_area < 0.0
        });
        Ok(())
    }
}

/// `CPS`, `CPE` or `CAX` when the type is a plane element.
fn plane_prefix(type_name: &str) -> Option<&'static str> {
    let upper = type_name.to_ascii_uppercase();
    ["CPS", "CPE", "CAX"]
        .into_iter()
        .find(|prefix| upper.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_mesh::{Element, ElementShape};

    #[test]
    fn surface_elements_follow_the_model_space() {
        use ElementShape::*;
        let axi = ModelSpace::Axisymmetric;
        assert_eq!(axi.element_type("CPS6", Tri6), "CAX6");
        assert_eq!(axi.element_type("S8R", Quad8), "CAX8R");
        assert_eq!(axi.element_type("C3D10", Tet10), "C3D10");
        assert_eq!(ModelSpace::PlaneStrain.element_type("cps4", Quad4), "CPE4");
        assert_eq!(ModelSpace::ThreeD.element_type("CAX8R", Quad8), "S8R");
        assert_eq!(ModelSpace::ThreeD.element_type("M3D4", Quad4), "M3D4");
        assert_eq!(ModelSpace::ThreeD.element_type("S3", Tri3), "S3");
    }

    #[test]
    fn input_files_tell_their_model_space() {
        let mut mesh = FeMesh::default();
        assert_eq!(ModelSpace::of_mesh(&mesh), ModelSpace::ThreeD);
        mesh.add_element(Element {
            id: 1,
            type_name: "CAX4".into(),
            shape: ElementShape::Quad4,
            nodes: vec![1, 2, 3, 4],
        })
        .unwrap();
        assert_eq!(ModelSpace::of_mesh(&mesh), ModelSpace::Axisymmetric);
        assert!(ModelSpace::PlaneStress.convert_mesh(&mut mesh));
        assert_eq!(mesh.elements()[0].type_name, "CPS4");
        assert!(!ModelSpace::PlaneStress.convert_mesh(&mut mesh));
    }

    fn square(z: f64, clockwise: bool) -> FeMesh {
        let mut mesh = FeMesh::default();
        for (id, [x, y]) in [
            (1, [0.0, 0.0]),
            (2, [1.0, 0.0]),
            (3, [1.0, 1.0]),
            (4, [0.0, 1.0]),
        ] {
            mesh.set_node(id, [x - 0.5, y, z]);
        }
        let nodes = if clockwise {
            vec![1, 4, 3, 2]
        } else {
            vec![1, 2, 3, 4]
        };
        mesh.add_element(Element {
            id: 1,
            type_name: "S4".into(),
            shape: ElementShape::Quad4,
            nodes,
        })
        .unwrap();
        mesh
    }

    #[test]
    fn generated_meshes_are_checked_and_oriented_for_the_model_space() {
        let mut mesh = square(0.0, true);
        ModelSpace::PlaneStress
            .prepare_generated_mesh(&mut mesh)
            .unwrap();
        let element = &mesh.elements()[0];
        assert_eq!(
            (element.type_name.as_str(), &element.nodes[..]),
            ("CPS4", &[1, 2, 3, 4][..])
        );
        assert!(
            ModelSpace::ThreeD
                .prepare_generated_mesh(&mut square(0.0, false))
                .is_err()
        );
        assert!(
            ModelSpace::PlaneStrain
                .prepare_generated_mesh(&mut square(1.0, false))
                .is_err()
        );
        // The square reaches to x = -0.5, across the axis.
        assert!(
            ModelSpace::Axisymmetric
                .prepare_generated_mesh(&mut square(0.0, false))
                .is_err()
        );
    }

    #[test]
    fn every_unit_system_names_all_units() {
        for units in UnitSystem::ALL {
            let named = units.base_units().iter().all(|u| !u.is_empty());
            assert_eq!(named, units != UnitSystem::Unitless, "{units:?}");
        }
        let properties: ModelProperties = ron::from_str("()").unwrap();
        assert_eq!(properties.space, ModelSpace::ThreeD);
        assert_eq!(properties.units, UnitSystem::MmTonSC);
    }
}
