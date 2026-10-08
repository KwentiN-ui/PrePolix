//! FE-Modell: Materialien, Sections, Steps, Randbedingungen, Lasten.
//!
//! The model describes an analysis on top of a mesh, the way PrePoMax's FE model does. Regions
//! are what the user picked in the GUI (parts, nodes, element faces) or named sets of the
//! input file; node and element sets that CalculiX needs for them are derived when the input
//! file is written, so the user never has to define sets by hand.

mod region;

pub use region::Region;

use serde::{Deserialize, Serialize};

/// Everything besides the mesh that makes up an analysis.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeModel {
    pub materials: Vec<Material>,
    pub sections: Vec<Section>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    pub density: Option<f64>,
    pub elastic: Option<Elastic>,
}

/// Linear isotropic elasticity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Elastic {
    pub young: f64,
    pub poisson: f64,
}

/// Assigns a material to the solid elements of a region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    pub material: String,
    pub region: Region,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub kind: StepKind,
    pub boundary_conditions: Vec<BoundaryCondition>,
    pub loads: Vec<Load>,
    pub field_outputs: Vec<FieldOutput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StepKind {
    Static(StaticStep),
}

/// Settings of a `*STATIC` step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaticStep {
    /// Geometrically nonlinear (`NLGEOM`).
    pub nlgeom: bool,
    /// Maximum number of increments (`INC`).
    pub max_increments: u32,
    /// Let CalculiX choose the increments; otherwise use `initial` throughout.
    pub automatic_increments: bool,
    pub initial_increment: f64,
    pub time_period: f64,
    pub min_increment: f64,
    pub max_increment: f64,
}

impl Default for StaticStep {
    fn default() -> Self {
        Self {
            nlgeom: false,
            max_increments: 100,
            automatic_increments: true,
            initial_increment: 1.0,
            time_period: 1.0,
            min_increment: 1e-5,
            max_increment: 1.0,
        }
    }
}

impl Step {
    /// A static step with PrePoMax's default field outputs.
    pub fn new_static(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: StepKind::Static(StaticStep::default()),
            boundary_conditions: Vec::new(),
            loads: Vec::new(),
            field_outputs: FieldOutput::defaults(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundaryCondition {
    pub name: String,
    pub region: Region,
    pub kind: BoundaryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BoundaryKind {
    /// All degrees of freedom held at zero.
    Fixed,
    /// Prescribed displacements (U1..U3) and rotations (UR1..UR3); `None` leaves one free.
    Displacement([Option<f64>; 6]),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Load {
    pub name: String,
    pub region: Region,
    pub kind: LoadKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LoadKind {
    /// Total force on a node region, split equally among its nodes.
    ConcentratedForce([f64; 3]),
    /// Pressure on a surface region; positive pushes into the material.
    Pressure(f64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldOutput {
    pub name: String,
    pub kind: OutputKind,
    pub variables: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputKind {
    /// `*NODE FILE`
    Node,
    /// `*EL FILE`
    Element,
}

impl FieldOutput {
    /// Field outputs PrePoMax adds to a new static step.
    pub fn defaults() -> Vec<Self> {
        vec![
            Self {
                name: "NF-Output-1".into(),
                kind: OutputKind::Node,
                variables: vec!["RF".into(), "U".into()],
            },
            Self {
                name: "EF-Output-1".into(),
                kind: OutputKind::Element,
                variables: vec!["E".into(), "ME".into(), "PEEQ".into(), "S".into()],
            },
        ]
    }
}

/// Next free default name such as "Material-2": PrePoMax numbers new items per kind.
pub fn next_name<'a>(prefix: &str, existing: impl IntoIterator<Item = &'a str>) -> String {
    let existing: Vec<&str> = existing.into_iter().collect();
    (1..)
        .map(|n| format!("{prefix}-{n}"))
        .find(|name| !existing.iter().any(|e| e.eq_ignore_ascii_case(name)))
        .expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_count_up_per_kind() {
        assert_eq!(next_name("Material", []), "Material-1");
        assert_eq!(
            next_name("Material", ["Material-1", "MATERIAL-2", "Steel"]),
            "Material-3"
        );
    }
}
