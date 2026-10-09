//! FE-Modell: Materialien, Sections, Steps, Randbedingungen, Lasten.
//!
//! The model describes an analysis on top of a mesh, the way PrePoMax's FE model does. Regions
//! are what the user picked in the GUI (parts, nodes, element faces) or named sets of the
//! input file; node and element sets that CalculiX needs for them are derived when the input
//! file is written, so the user never has to define sets by hand.

mod amplitude;
mod constraint;
mod contact;
pub mod convert;
mod features;
mod geometry;
pub mod library;
mod properties;
mod region;
mod section;
pub mod units;
mod validity;

pub use amplitude::{Amplitude, AmplitudeTime};
pub use constraint::{CompressionOnly, PointSpring, SurfaceSpring, SurfaceToSurfaceSpring};
pub use contact::{
    Constraint, ContactMethod, ContactPair, DEFAULT_SURFACE_COLOR, Friction, GapConductance,
    InteractionProperty, SurfaceBehavior, SurfaceInteraction, Tie,
};
pub use features::{
    CoordinatePlane, CoordinateSystem, CoordinateSystemKind, GLOBAL, Plane, PlaneSource, PointRef,
    ReferencePoint, ResultPath, ResultPlane,
};
pub use geometry::{
    Algorithm2d, Algorithm3d, Geometry, MeshSetupItem, MeshSetupKind, MeshingParameters,
};
pub use library::MaterialLibrary;
pub use properties::{ModelProperties, ModelSpace};
pub use region::{Region, describe_entities};
pub use section::{
    BeamOrientation, BeamProfile, BeamSection, Section, SectionKind, line_tangent, unit_thickness,
};
pub use units::{BASE_QUANTITIES, DERIVED_QUANTITIES, Quantity, UnitSystem};
pub use validity::{Invalid, ModelItem};

use plx_mesh::CadEntity;
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
    /// Model space and unit system; projects saved before they existed are 3D.
    #[serde(default)]
    pub properties: ModelProperties,
    pub materials: Vec<Material>,
    pub sections: Vec<Section>,
    /// Springs, supports and ties, PrePoMax's Constraints.
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub surface_interactions: Vec<SurfaceInteraction>,
    #[serde(default)]
    pub contact_pairs: Vec<ContactPair>,
    /// Time curves boundary conditions and loads refer to by name.
    #[serde(default)]
    pub amplitudes: Vec<Amplitude>,
    /// State of the model before the first step, such as its initial temperature.
    #[serde(default)]
    pub initial_conditions: Vec<InitialCondition>,
    pub steps: Vec<Step>,
    /// CalculiX keywords the user added to the input file in the keyword editor, in the order
    /// they appear in it. They cover what the model cannot express yet.
    #[serde(default)]
    pub user_keywords: Vec<UserKeyword>,
    /// PrePoMax's features: points and coordinate systems other items refer to by name.
    #[serde(default)]
    pub reference_points: Vec<ReferencePoint>,
    #[serde(default)]
    pub coordinate_systems: Vec<CoordinateSystem>,
    #[serde(default)]
    pub planes: Vec<Plane>,
    /// Results on planes; not part of the input file.
    #[serde(default)]
    pub result_planes: Vec<ResultPlane>,
    /// Straight paths results are read on; not part of the input file.
    #[serde(default)]
    pub result_paths: Vec<ResultPath>,
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

    /// Every region of the model: of sections, constraints, contact pairs, boundary
    /// conditions and loads.
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        (self.sections.iter().map(|s| &s.region))
            .chain(self.constraints.iter().flat_map(Constraint::regions))
            .chain((self.contact_pairs.iter()).flat_map(|c| [&c.master, &c.slave]))
            .chain(self.steps.iter().flat_map(|step| {
                (step.boundary_conditions.iter().map(|b| &b.region))
                    .chain(step.loads.iter().map(|l| &l.region))
            }))
    }

    /// Follows Gmsh's new numbers of the CAD entities after a geometry part was deleted:
    /// `tags` gives the new entity by the old one. Entities without a new one are dropped
    /// from the regions picked on the geometry.
    pub fn renumber_cad(&mut self, tags: &std::collections::BTreeMap<CadEntity, CadEntity>) {
        for region in self.regions_mut() {
            if let Region::Geometry(entities) = region {
                *entities = entities
                    .iter()
                    .filter_map(|e| tags.get(e).copied())
                    .collect();
            }
        }
    }

    fn regions_mut(&mut self) -> impl Iterator<Item = &mut Region> {
        (self.sections.iter_mut().map(|s| &mut s.region))
            .chain(
                self.constraints
                    .iter_mut()
                    .flat_map(Constraint::regions_mut),
            )
            .chain((self.contact_pairs.iter_mut()).flat_map(|c| [&mut c.master, &mut c.slave]))
            .chain(self.steps.iter_mut().flat_map(|step| {
                (step.boundary_conditions.iter_mut().map(|b| &mut b.region))
                    .chain(step.loads.iter_mut().map(|l| &mut l.region))
            }))
    }

    /// Follows a renamed amplitude: boundary conditions and loads keep referring to it.
    pub fn rename_amplitude(&mut self, old: &str, new: &str) {
        for step in &mut self.steps {
            let references = (step.boundary_conditions.iter_mut())
                .map(|b| &mut b.amplitude)
                .chain(
                    (step.loads.iter_mut())
                        .flat_map(|l| [&mut l.amplitude, &mut l.factor_amplitude]),
                );
            for reference in references {
                if reference.as_deref() == Some(old) {
                    *reference = Some(new.to_string());
                }
            }
        }
    }

    /// The amplitude of the name, if the model has it.
    pub fn amplitude(&self, name: &str) -> Option<&Amplitude> {
        self.amplitudes.iter().find(|a| a.name == name)
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
            .chain(self.initial_conditions.iter_mut().map(|i| &mut i.region))
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    pub density: Option<f64>,
    pub elastic: Option<Elastic>,
    /// Thermal conductivity (`*CONDUCTIVITY`), for heat transfer.
    #[serde(default)]
    pub conductivity: Option<f64>,
    /// Specific heat (`*SPECIFIC HEAT`), for transient heat transfer.
    #[serde(default)]
    pub specific_heat: Option<f64>,
    /// Thermal expansion (`*EXPANSION`), for thermal strains.
    #[serde(default)]
    pub expansion: Option<Expansion>,
}

/// Linear isotropic thermal expansion.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expansion {
    pub coefficient: f64,
    /// Temperature at which the expansion is zero (`ZERO=`); PrePoMax's default is 20.
    pub zero_temperature: f64,
}

impl Default for Expansion {
    fn default() -> Self {
        Self {
            coefficient: 0.0,
            zero_temperature: 20.0,
        }
    }
}

/// Linear isotropic elasticity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Elastic {
    pub young: f64,
    pub poisson: f64,
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
    /// Temperatures only (`*HEAT TRANSFER`).
    HeatTransfer(HeatTransferStep),
    /// Temperatures and displacements solved together
    /// (`*COUPLED TEMPERATURE-DISPLACEMENT`).
    CoupledTempDisp(HeatTransferStep),
}

impl StepKind {
    pub fn solver_mut(&mut self) -> &mut EquationSolver {
        match self {
            StepKind::Static(settings) => &mut settings.solver,
            StepKind::Frequency(settings) => &mut settings.solver,
            StepKind::HeatTransfer(settings) | StepKind::CoupledTempDisp(settings) => {
                &mut settings.increments.solver
            }
        }
    }

    /// Whether the step takes loads. A frequency step has none, as in PrePoMax; preloads
    /// come from the previous step with [`FrequencyStep::perturbation`].
    pub fn supports_loads(&self) -> bool {
        !matches!(self, StepKind::Frequency(_))
    }

    /// Whether the step solves for displacements.
    pub fn is_mechanical(&self) -> bool {
        !matches!(self, StepKind::HeatTransfer(_))
    }

    /// Whether the step solves for temperatures.
    pub fn is_thermal(&self) -> bool {
        matches!(
            self,
            StepKind::HeatTransfer(_) | StepKind::CoupledTempDisp(_)
        )
    }

    /// Whether a boundary condition of this kind acts in the step, PrePoMax's
    /// `IsBoundaryConditionSupported`: temperatures in thermal steps, displacements in
    /// mechanical ones.
    pub fn supports_boundary(&self, kind: &BoundaryKind) -> bool {
        if kind.is_thermal() {
            self.is_thermal()
        } else {
            self.is_mechanical()
        }
    }

    /// Whether a load of this kind acts in the step, PrePoMax's `IsLoadTypeSupported`.
    pub fn supports_load(&self, kind: &LoadKind) -> bool {
        self.supports_loads()
            && if kind.is_thermal() {
                self.is_thermal()
            } else {
                self.is_mechanical()
            }
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

/// Settings of a `*HEAT TRANSFER` or `*COUPLED TEMPERATURE-DISPLACEMENT` step, with
/// PrePoMax's defaults; like PrePoMax's `HeatTransferStep` it extends the static step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeatTransferStep {
    /// Increments, time period and solver as in a static step. `nlgeom` only applies to a
    /// coupled step.
    pub increments: StaticStep,
    /// Steady state (`STEADY STATE`): the temperatures the loads settle to, without heat
    /// capacity; otherwise transient over the time period.
    pub steady_state: bool,
    /// Largest temperature change allowed in an increment of a transient analysis
    /// (`DELTMX`); `None` leaves it open.
    pub deltmx: Option<f64>,
}

impl Default for HeatTransferStep {
    fn default() -> Self {
        Self {
            increments: StaticStep::default(),
            steady_state: true,
            deltmx: None,
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

    /// A heat transfer step with PrePoMax's default field outputs.
    pub fn new_heat_transfer(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            kind: StepKind::HeatTransfer(HeatTransferStep::default()),
            boundary_conditions: Vec::new(),
            loads: Vec::new(),
            field_outputs: FieldOutput::heat_transfer_defaults(),
        }
    }

    /// A coupled temperature-displacement step with PrePoMax's default field outputs.
    pub fn new_coupled(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            kind: StepKind::CoupledTempDisp(HeatTransferStep::default()),
            boundary_conditions: Vec::new(),
            loads: Vec::new(),
            field_outputs: FieldOutput::coupled_defaults(),
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
    /// Amplitude the values follow over time; `None` is CalculiX's default ramp or step.
    #[serde(default)]
    pub amplitude: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BoundaryKind {
    /// All degrees of freedom held at zero.
    Fixed,
    /// Prescribed displacements (U1..U3) and rotations (UR1..UR3); `None` leaves one free.
    Displacement([Option<f64>; 6]),
    /// Prescribed temperature (degree of freedom 11), PrePoMax's `TemperatureBC`.
    Temperature(f64),
}

impl BoundaryKind {
    /// Whether an amplitude can scale the boundary condition; fixed supports stay zero.
    pub fn takes_amplitude(&self) -> bool {
        !matches!(self, BoundaryKind::Fixed)
    }

    pub fn is_thermal(&self) -> bool {
        matches!(self, BoundaryKind::Temperature(_))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Load {
    pub name: String,
    /// A deactivated load is left out of the input file and the 3D view.
    #[serde(default = "active")]
    pub active: bool,
    pub region: Region,
    pub kind: LoadKind,
    /// Amplitude the value follows over time, of a film or radiation its sink temperature;
    /// `None` is CalculiX's default ramp or step.
    #[serde(default)]
    pub amplitude: Option<String>,
    /// Amplitude of the film coefficient or the emissivity (`FILM AMPLITUDE`,
    /// `RADIATION AMPLITUDE`); other loads have none.
    #[serde(default)]
    pub factor_amplitude: Option<String>,
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
    /// Heat flow into every node of the region (`*CFLUX`), PrePoMax's concentrated flux.
    ConcentratedFlux(f64),
    /// Heat flow per area into a surface (`*DFLUX`, `S`), PrePoMax's surface flux.
    SurfaceFlux(f64),
    /// Heat generated per volume in the elements of the region (`*DFLUX`, `BF`).
    BodyFlux(f64),
    /// Convection to the surroundings at the sink temperature (`*FILM`).
    Film { sink: f64, coefficient: f64 },
    /// Radiation to the surroundings at the sink temperature (`*RADIATE`); needs the
    /// physical constants of the model.
    Radiation { sink: f64, emissivity: f64 },
}

impl LoadKind {
    /// What the second amplitude of a film or radiation scales, if the load has one.
    pub fn factor_amplitude_label(&self) -> Option<&'static str> {
        match self {
            LoadKind::Film { .. } => Some("Wärmeübergangskoeffizient"),
            LoadKind::Radiation { .. } => Some("Emissionsgrad"),
            _ => None,
        }
    }

    /// Whether the load is a heat flow rather than a force.
    pub fn is_thermal(&self) -> bool {
        !matches!(
            self,
            LoadKind::ConcentratedForce(_) | LoadKind::Pressure(_) | LoadKind::SurfaceTraction(_)
        )
    }
}

/// A state of the model before the first step, PrePoMax's initial conditions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InitialCondition {
    pub name: String,
    #[serde(default = "active")]
    pub active: bool,
    pub region: Region,
    pub kind: InitialConditionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum InitialConditionKind {
    /// `*INITIAL CONDITIONS, TYPE=TEMPERATURE`
    Temperature(f64),
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

    /// Field outputs PrePoMax adds to a new heat transfer step: temperatures, reaction heat
    /// flows and heat fluxes.
    pub fn heat_transfer_defaults() -> Vec<Self> {
        let mut outputs = Self::defaults();
        outputs[0].variables = vec!["NT".into(), "RFL".into()];
        outputs[1].variables = vec!["HFL".into()];
        outputs
    }

    /// Field outputs PrePoMax adds to a new coupled temperature-displacement step.
    pub fn coupled_defaults() -> Vec<Self> {
        let mut outputs = Self::defaults();
        outputs[0].variables = ["RF", "U", "NT", "RFL"].map(String::from).to_vec();
        outputs[1].variables = ["S", "E", "HFL"].map(String::from).to_vec();
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
            thickness: 1.0,
            kind: SectionKind::Solid,
        });
        let mut step = Step::new_static("Step-1");
        step.boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: Region::ElementSet("A".into()),
            kind: BoundaryKind::Fixed,
            amplitude: None,
        });
        step.loads.push(Load {
            name: "Pressure-1".into(),
            active: true,
            region: Region::Surface("A".into()),
            kind: LoadKind::Pressure(1.0),
            amplitude: None,
            factor_amplitude: None,
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
