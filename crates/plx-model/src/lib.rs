//! FE-Modell: Materialien, Sections, Steps, Randbedingungen, Lasten.
//!
//! The model describes an analysis on top of a mesh, the way PrePoMax's FE model does. Regions
//! are what the user picked in the GUI (parts, nodes, element faces) or named sets of the
//! input file; node and element sets that CalculiX needs for them are derived when the input
//! file is written, so the user never has to define sets by hand.

mod constraint;
mod contact;
mod geometry;
mod hot_spot;
pub mod library;
mod region;
mod validity;

pub use constraint::{CompressionOnly, PointSpring, SurfaceSpring, SurfaceToSurfaceSpring};
pub use contact::{
    Constraint, ContactMethod, ContactPair, DEFAULT_SURFACE_COLOR, Friction, GapConductance,
    InteractionProperty, SurfaceBehavior, SurfaceInteraction, Tie,
};
pub use geometry::{
    Algorithm2d, Algorithm3d, Geometry, MeshSetupItem, MeshSetupKind, MeshingParameters,
};
pub use hot_spot::{Extrapolation, HotSpot, HotSpotComponent, extrapolation_weights};
pub use library::MaterialLibrary;
pub use region::Region;
pub use validity::{Invalid, ModelItem};

use serde::{Deserialize, Serialize};

/// Version of the project file format written by this build.
pub const PROJECT_FORMAT: u32 = 1;

/// A prepolix project as saved in a `.plx` file: the mesh and the analysis set up on it.
/// Like PrePoMax's `.pmx` it stores what the user defined, not the CalculiX input file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub format: u32,
    /// CAD geometry the mesh is generated from; projects made from an input file have none.
    #[serde(default)]
    pub geometry: Option<Geometry>,
    pub mesh: plx_mesh::FeMesh,
    #[serde(default)]
    pub model: FeModel,
}

/// Everything besides the mesh that makes up an analysis.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeModel {
    pub materials: Vec<Material>,
    pub sections: Vec<Section>,
    /// Springs, supports and ties, PrePoMax's Constraints.
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub surface_interactions: Vec<SurfaceInteraction>,
    #[serde(default)]
    pub contact_pairs: Vec<ContactPair>,
    pub steps: Vec<Step>,
    /// CalculiX keywords the user added to the input file in the keyword editor, in the order
    /// they appear in it. They cover what the model cannot express yet.
    #[serde(default)]
    pub user_keywords: Vec<UserKeyword>,
    /// Hot spot stress evaluations, done on the results; not part of the input file.
    #[serde(default)]
    pub hot_spots: Vec<HotSpot>,
}

impl FeModel {
    /// Replaces [`EquationSolver::Default`] in all steps by `solver`, the solver the
    /// installed CalculiX should use by default.
    pub fn resolve_default_solver(&mut self, solver: EquationSolver) {
        for step in &mut self.steps {
            let current = step.kind.solver_mut();
            if *current == EquationSolver::Default {
                *current = solver;
            }
        }
    }

    /// Follows a renamed part: regions on the part, or on the element set an input file
    /// defines for it, keep pointing at it.
    pub fn rename_part(&mut self, old: &str, new: &str) {
        let regions = (self.sections.iter_mut().map(|s| &mut s.region))
            .chain(
                self.constraints
                    .iter_mut()
                    .flat_map(Constraint::regions_mut),
            )
            .chain((self.contact_pairs.iter_mut()).flat_map(|c| [&mut c.master, &mut c.slave]))
            .chain(self.steps.iter_mut().flat_map(|step| {
                (step.boundary_conditions.iter_mut().map(|b| &mut b.region))
                    .chain(step.loads.iter_mut().map(|l| &mut l.region))
            }));
        for region in regions {
            match region {
                Region::Parts(names) => {
                    for name in names.iter_mut().filter(|n| n.as_str() == old) {
                        *name = new.to_string();
                    }
                }
                Region::ElementSet(name) if name == old => *name = new.to_string(),
                _ => {}
            }
        }
    }
}

/// Lines of the user's own written into the input file at a fixed place, like PrePoMax's
/// `CalculixUserKeyword`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserKeyword {
    /// Place in the keyword tree of the input file: the index of each enclosing keyword among
    /// its siblings, then the index the keyword is inserted at. Keywords are inserted in list
    /// order, so the indices count the user keywords before it as well.
    pub position: Vec<usize>,
    pub text: String,
    /// An inactive keyword is written commented out.
    pub active: bool,
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
    /// A deactivated step is written to the input file as a comment only, as in PrePoMax.
    #[serde(default = "active")]
    pub active: bool,
    pub kind: StepKind,
    pub boundary_conditions: Vec<BoundaryCondition>,
    pub loads: Vec<Load>,
    pub field_outputs: Vec<FieldOutput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StepKind {
    Static(StaticStep),
    /// Eigenfrequencies and mode shapes (`*FREQUENCY`).
    Frequency(FrequencyStep),
}

impl StepKind {
    pub fn solver_mut(&mut self) -> &mut EquationSolver {
        match self {
            StepKind::Static(settings) => &mut settings.solver,
            StepKind::Frequency(settings) => &mut settings.solver,
        }
    }

    /// Whether the step takes loads. A frequency step has none, as in PrePoMax; preloads
    /// come from the previous step with [`FrequencyStep::perturbation`].
    pub fn supports_loads(&self) -> bool {
        matches!(self, StepKind::Static(_))
    }
}

/// How the increments of a step are chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Incrementation {
    /// CalculiX's own defaults; no increment data is written.
    #[default]
    Default,
    /// Automatic increments within the given limits.
    Automatic,
    /// Fixed increments of `initial_increment` (`DIRECT`).
    Direct,
}

/// Equation solver of a step (`SOLVER=`), PrePoMax's choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EquationSolver {
    /// Pardiso where the CalculiX build has it, otherwise CalculiX's own default; resolved
    /// before the input file is written, see [`FeModel::resolve_default_solver`].
    #[default]
    Default,
    Pardiso,
    Spooles,
    PaStiX,
    IterativeScaling,
    IterativeCholesky,
}

impl EquationSolver {
    pub const ALL: [EquationSolver; 6] = [
        EquationSolver::Default,
        EquationSolver::Pardiso,
        EquationSolver::Spooles,
        EquationSolver::PaStiX,
        EquationSolver::IterativeScaling,
        EquationSolver::IterativeCholesky,
    ];

    /// Whether CalculiX can use the solver for an eigenvalue problem; the iterative solvers
    /// only solve static systems.
    pub fn solves_eigenvalues(self) -> bool {
        !matches!(
            self,
            EquationSolver::IterativeScaling | EquationSolver::IterativeCholesky
        )
    }

    /// The value of `SOLVER=` in the input file; `None` leaves the choice to CalculiX.
    pub fn keyword(self) -> Option<&'static str> {
        match self {
            EquationSolver::Default => None,
            EquationSolver::Pardiso => Some("Pardiso"),
            EquationSolver::Spooles => Some("Spooles"),
            EquationSolver::PaStiX => Some("PaStiX"),
            EquationSolver::IterativeScaling => Some("Iterative scaling"),
            EquationSolver::IterativeCholesky => Some("Iterative Cholesky"),
        }
    }
}

/// Settings of a `*STATIC` step, with PrePoMax's defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaticStep {
    /// Geometrically nonlinear (`NLGEOM`).
    pub nlgeom: bool,
    pub incrementation: Incrementation,
    /// Maximum number of increments (`INC`), written unless incrementation is default.
    pub max_increments: u32,
    pub initial_increment: f64,
    pub time_period: f64,
    pub min_increment: f64,
    pub max_increment: f64,
    /// Missing in projects saved before the solver could be chosen.
    #[serde(default)]
    pub solver: EquationSolver,
}

impl Default for StaticStep {
    fn default() -> Self {
        Self {
            nlgeom: false,
            incrementation: Incrementation::Default,
            max_increments: 100,
            initial_increment: 1.0,
            time_period: 1.0,
            min_increment: 1e-5,
            max_increment: 1e30,
            solver: EquationSolver::Default,
        }
    }
}

/// Settings of a `*FREQUENCY` step, with PrePoMax's defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrequencyStep {
    /// Number of eigenfrequencies to compute.
    pub num_frequencies: u32,
    /// Lower bound of the frequency range in cycles per time unit; `None` is CalculiX's 0.
    pub lower_frequency: Option<f64>,
    /// Upper bound of the frequency range; `None` leaves it open.
    pub upper_frequency: Option<f64>,
    /// Writes eigenvalues, mode shapes and the mass and stiffness matrices to the `.eig`
    /// file (`STORAGE=YES`).
    pub storage: bool,
    /// Takes the stiffness of the deformed state of the previous step into account, e.g. the
    /// stiffening by a preload (`*STEP, PERTURBATION`).
    pub perturbation: bool,
    pub solver: EquationSolver,
}

impl Default for FrequencyStep {
    fn default() -> Self {
        Self {
            num_frequencies: 10,
            lower_frequency: None,
            upper_frequency: None,
            storage: false,
            perturbation: false,
            solver: EquationSolver::Default,
        }
    }
}

impl Step {
    /// A static step with PrePoMax's default field outputs.
    pub fn new_static(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            kind: StepKind::Static(StaticStep::default()),
            boundary_conditions: Vec::new(),
            loads: Vec::new(),
            field_outputs: FieldOutput::defaults(),
        }
    }

    /// A frequency step with PrePoMax's default field outputs.
    pub fn new_frequency(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            kind: StepKind::Frequency(FrequencyStep::default()),
            boundary_conditions: Vec::new(),
            loads: Vec::new(),
            field_outputs: FieldOutput::frequency_defaults(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundaryCondition {
    pub name: String,
    /// A deactivated boundary condition is left out of the input file and the 3D view.
    #[serde(default = "active")]
    pub active: bool,
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
    /// A deactivated load is left out of the input file and the 3D view.
    #[serde(default = "active")]
    pub active: bool,
    pub region: Region,
    pub kind: LoadKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LoadKind {
    /// Force applied at every node of the region, as CalculiX's `*CLOAD` on a node set.
    ConcentratedForce([f64; 3]),
    /// Pressure on a surface region; positive pushes into the material.
    Pressure(f64),
    /// Total force on a surface region, spread over its nodes by area when the input file is
    /// written (PrePoMax's surface traction).
    SurfaceTraction([f64; 3]),
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
    /// Field outputs PrePoMax adds to a new static step. `NOE` is added when writing.
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
                variables: vec!["S".into(), "E".into()],
            },
        ]
    }
}

impl FieldOutput {
    /// Field outputs PrePoMax adds to a new frequency step: no reaction forces, as the
    /// mode shapes are scaled arbitrarily.
    pub fn frequency_defaults() -> Vec<Self> {
        let mut outputs = Self::defaults();
        outputs[0].variables = vec!["U".into()];
        outputs
    }
}

/// Items of projects saved before they could be deactivated are active.
fn active() -> bool {
    true
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
    fn regions_follow_renamed_parts() {
        let mut model = FeModel::default();
        model.sections.push(Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: Region::Parts(vec!["A".into(), "B".into()]),
        });
        let mut step = Step::new_static("Step-1");
        step.boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: Region::ElementSet("A".into()),
            kind: BoundaryKind::Fixed,
        });
        step.loads.push(Load {
            name: "Pressure-1".into(),
            active: true,
            region: Region::Surface("A".into()),
            kind: LoadKind::Pressure(1.0),
        });
        model.steps.push(step);
        model.rename_part("A", "C");
        assert_eq!(
            model.sections[0].region,
            Region::Parts(vec!["C".into(), "B".into()])
        );
        let step = &model.steps[0];
        assert_eq!(
            step.boundary_conditions[0].region,
            Region::ElementSet("C".into())
        );
        assert_eq!(step.loads[0].region, Region::Surface("A".into()));
    }

    #[test]
    fn default_names_count_up_per_kind() {
        assert_eq!(next_name("Material", []), "Material-1");
        assert_eq!(
            next_name("Material", ["Material-1", "MATERIAL-2", "Steel"]),
            "Material-3"
        );
    }
}
