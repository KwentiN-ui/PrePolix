//! Checks of the model before CalculiX runs, for the mistakes that most often make a run abort
//! or give useless results: parts that are not held, elements without a material, materials
//! without the constants a step needs, distorted elements, contradicting boundary conditions.
//! Each finding names the tree item it belongs to and explains the problem and its fix.
//!
//! The checks are derived from the model like [`crate::validity`]; the expensive part that
//! only depends on the mesh (connectivity and element distortion) is computed once in
//! [`MeshCheck`]. CalculiX's own error messages after a failed run are mapped to the same
//! explanations by [`diagnose_solver_output`].

use std::collections::{HashMap, HashSet};

use plx_mesh::{ElementFamily, ElementId, FeMesh, NodeId};

use crate::Region;

use crate::{
    BoundaryKind, Constraint, DefinedFieldKind, FeModel, LoadKind, Material, ModelItem, ModelSpace,
    SectionKind, StepKind,
};

/// Whether CalculiX aborts or the results are likely wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// CalculiX aborts, or the results are meaningless.
    Error,
    /// The run may work, but the model probably does not do what was intended.
    Warning,
}

/// A kind of problem, with the explanation shown when its warning sign is clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Problem {
    MissingReference,
    NoSection,
    NoElastic,
    NoDensity,
    /// A boundary condition or load on a reference point that no active rigid body is driven
    /// by: the point has no node in the input file.
    NoRigidBody,
    /// A plastic hardening curve CalculiX rejects: no elasticity, a row without a yield
    /// stress, or plastic strains that do not start at 0 and grow.
    InvalidPlastic,
    NoConductivity,
    NoSpecificHeat,
    NoInitialTemperature,
    InvalidElastic,
    DistortedElements,
    RigidBodyMotion,
    /// Truss nodes that no bar or support holds in some direction.
    Mechanism,
    HeldOnlyByContact,
    ConflictingBoundaries,
    LoadOnFixedNodes,
    RotationsIgnored,
    /// A submodel boundary condition in a model without a global results file.
    NoGlobalResults,
    IncrementExceedsStep,
    NoLoad,
    /// A defined temperature in a step whose materials have no thermal expansion.
    NoExpansion,
    /// A step that builds on stored eigenmodes (modal dynamics, steady state dynamics,
    /// complex frequency) without a frequency step before it that stores them.
    NoStoredModes,
    /// An initial velocity in a model without a Dynamic step.
    VelocityIgnored,
    /// A Coriolis complex frequency step without a centrifugal load before it.
    NoRotation,
    /// Found in the solver output only.
    NoConvergence,
    /// Found in the solver output only.
    MpcAndSpc,
    /// Found in the solver output only.
    RotationIn2d,
    /// Any other `*ERROR` of the solver output.
    SolverError,
}

impl Problem {
    pub fn severity(self) -> Severity {
        match self {
            Problem::HeldOnlyByContact
            | Problem::ConflictingBoundaries
            | Problem::LoadOnFixedNodes
            | Problem::RotationsIgnored
            | Problem::VelocityIgnored
            | Problem::NoLoad
            | Problem::NoExpansion
            | Problem::NoRotation => Severity::Warning,
            _ => Severity::Error,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Problem::MissingReference => "Missing reference",
            Problem::NoSection => "Elements without material",
            Problem::NoElastic => "Material without elasticity",
            Problem::NoDensity => "Material without density",
            Problem::NoConductivity => "Material without conductivity",
            Problem::NoSpecificHeat => "Material without specific heat",
            Problem::NoInitialTemperature => "No initial temperature",
            Problem::InvalidElastic => "Invalid material constants",
            Problem::DistortedElements => "Distorted elements",
            Problem::RigidBodyMotion => "Rigid body motion possible",
            Problem::Mechanism => "Truss structure is a mechanism",
            Problem::HeldOnlyByContact => "Held only by contact",
            Problem::ConflictingBoundaries => "Conflicting boundary conditions",
            Problem::LoadOnFixedNodes => "Load on fixed nodes",
            Problem::RotationsIgnored => "Rotations without effect",
            Problem::NoGlobalResults => "No global results",
            Problem::IncrementExceedsStep => "Increment larger than the step",
            Problem::NoLoad => "No load",
            Problem::NoRigidBody => "Reference point without rigid body",
            Problem::InvalidPlastic => "Invalid plasticity",
            Problem::NoExpansion => "Temperature without thermal expansion",
            Problem::NoStoredModes => "No stored eigenmodes",
            Problem::VelocityIgnored => "Initial velocity without a Dynamic step",
            Problem::NoRotation => "No rotation",
            Problem::NoConvergence => "No convergence",
            Problem::MpcAndSpc => "Degree of freedom bound twice",
            Problem::RotationIn2d => "Rotation in a 2D model",
            Problem::SolverError => "CalculiX error",
        }
    }

    /// What goes wrong and why CalculiX cannot cope with it.
    pub fn explanation(self) -> &'static str {
        match self {
            Problem::MissingReference => {
                "The item refers to something that no longer exists, for example a \
                 deleted material or a set the mesh no longer has. The input file \
                 cannot be written like this."
            }
            Problem::NoSection => {
                "CalculiX needs a material for every element. Elements that no section \
                 covers abort the run with \"no material was assigned to element\"."
            }
            Problem::NoElastic => {
                "A section uses this material, but it has no Young's modulus and no \
                 Poisson's ratio. Without them there is no stiffness; CalculiX aborts \
                 with \"no elastic constants were assigned\"."
            }
            Problem::NoDensity => {
                "A Frequency step, a Dynamic step, a transient heat transfer as well as \
                 gravity and centrifugal loads need the mass of the model. Without a \
                 density CalculiX aborts with \"no density was assigned\"."
            }
            Problem::NoRigidBody => {
                "A boundary condition or load on a reference point acts on the rigid body the \
                 point drives. Without an active rigid body constraint with this reference \
                 point there is no node to apply it to; the input file would be incomplete."
            }
            Problem::InvalidPlastic => {
                "The hardening curve of a plastic material lists the yield stress over the \
                 plastic strain. Every row needs a stress above 0, the first row of each \
                 temperature starts at plastic strain 0 and the strain grows from row to \
                 row; plasticity also needs the elastic constants. Otherwise CalculiX stops \
                 with \"*PLASTIC: the plastic strain must be increasing\" or gives a wrong \
                 stiffness."
            }
            Problem::NoConductivity => {
                "A heat transfer needs the thermal conductivity of every material. \
                 CalculiX only warns (\"no conductivity constants were assigned\") and \
                 then crashes while solving."
            }
            Problem::NoSpecificHeat => {
                "A transient heat transfer needs the specific heat. CalculiX aborts \
                 with \"no specific heat was assigned\"."
            }
            Problem::NoInitialTemperature => {
                "A transient heat transfer starts from an initial temperature, and a \
                 prescribed temperature (Defined Field) expands the model relative to it. \
                 Without it CalculiX aborts with \"please define initial conditions for \
                 the temperature\" or \"no thermal *INITIAL CONDITIONS are given\"."
            }
            Problem::InvalidElastic => {
                "Young's modulus must be greater than 0 and Poisson's ratio must lie \
                 between -1 and 0.5 (0.5 would be an incompressible material, which a \
                 linear elastic model cannot represent). CalculiX rejects the material \
                 when reading the input."
            }
            Problem::DistortedElements => {
                "These elements are inverted or so badly distorted that their Jacobian \
                 determinant is not positive at an integration point. CalculiX aborts \
                 with \"nonpositive jacobian determinant in element\". Typical causes \
                 are quadratic tetrahedra whose midside nodes were moved onto a strongly \
                 curved face, very thin regions, or 2D elements whose nodes are numbered \
                 clockwise."
            }
            Problem::RigidBodyMotion => {
                "These parts can move as a rigid body: the boundary conditions and \
                 springs do not fix all displacements and rotations, and no Tie \
                 connects them to supported parts. The stiffness matrix is then \
                 singular. Depending on the matrix solver CalculiX aborts with \"zero \
                 pivot\" or returns huge, useless displacements without an error."
            }
            Problem::Mechanism => {
                "Trusses (Truss Section) carry axial force only. A node at which all \
                 trusses lie on one line or in one plane is not held across it: a truss \
                 of several elements buckles at every inner node without resistance, and \
                 a plane truss can move perpendicular to its plane. The stiffness matrix \
                 is singular; CalculiX does not report this but returns huge, useless \
                 displacements."
            }
            Problem::HeldOnlyByContact => {
                "These parts are held only by contact pairs. At the start of the \
                 analysis a contact carries load only once it is closed; until then the \
                 parts can drift away freely. This often leads to \"zero pivot\", to \
                 \"too many cutbacks\" or to very long analyses."
            }
            Problem::ConflictingBoundaries => {
                "In this step the same nodes are prescribed by several boundary \
                 conditions with different values, e.g. a face fixed and an adjacent \
                 edge with a displacement. CalculiX does not report this but uses the \
                 value written last; the result then depends on the order in the tree."
            }
            Problem::LoadOnFixedNodes => {
                "All nodes of this load are fixed in the load direction. The load goes \
                 directly into the reaction force and does not deform the model."
            }
            Problem::RotationsIgnored => {
                "Only beam and shell nodes have rotations (UR1 to UR3). Solid and 2D \
                 elements have displacements only; prescribed rotations are ignored or \
                 abort the run in 2D models (\"mpc of type is unknown\")."
            }
            Problem::NoGlobalResults => {
                "A submodel boundary condition takes its displacements from the results \
                 of a global model (*SUBMODEL). Without the global results file the input \
                 file cannot be written."
            }
            Problem::IncrementExceedsStep => {
                "The initial increment is larger than the time period of the step. \
                 CalculiX rejects the step with \"initial increment size exceeds step \
                 size\"."
            }
            Problem::NoLoad => {
                "The step has neither an active load nor a prescribed displacement. The \
                 analysis runs, but all results are zero."
            }
            Problem::NoExpansion => {
                "A defined temperature (Defined Field) deforms the model only through the \
                 thermal expansion of the materials. Without an expansion coefficient \
                 CalculiX runs, but the temperature has no effect."
            }
            Problem::NoStoredModes => {
                "Modal Dynamics, Steady State Dynamics and Complex Frequency steps work on \
                 the eigenmodes a previous Frequency step with Storage wrote to the .eig \
                 file. Without it CalculiX stops with \"error opening the eigenvalue file\" \
                 or \"the eigenvalue file does not exist\"."
            }
            Problem::VelocityIgnored => {
                "Only a Dynamic step (time integration) starts from an initial velocity; \
                 static, frequency and thermal steps ignore it without a message."
            }
            Problem::NoRotation => {
                "The Coriolis forces of a Complex Frequency step come from the centrifugal \
                 load of a static step before the Frequency step. Without a rotation the \
                 complex eigenfrequencies are those of the Frequency step."
            }
            Problem::NoConvergence => {
                "The Newton iteration did not converge; CalculiX kept reducing the \
                 increment and gave up (\"too many cutbacks\" or \"increment size \
                 smaller than minimum\"). The cause is usually contacts, poorly supported \
                 parts or too large loads in a nonlinear analysis. An even smaller \
                 minimum increment rarely helps."
            }
            Problem::MpcAndSpc => {
                "A degree of freedom is both fixed by a boundary condition and bound as \
                 the dependent degree of freedom of an equation (MPC), e.g. by user \
                 *EQUATION keywords. CalculiX then aborts."
            }
            Problem::RotationIn2d => {
                "Rotations (degrees of freedom 4 to 6) were fixed in a 2D model. \
                 CalculiX expands 2D elements internally into solid elements and cannot \
                 prescribe rotations for them; the run aborts with \"mpc of type is \
                 unknown\" or a floating point error."
            }
            Problem::SolverError => {
                "CalculiX aborted with an error message that prepolix does not know. \
                 The exact text is shown in the Monitor."
            }
        }
    }

    /// How to fix it.
    pub fn fix(self) -> &'static str {
        match self {
            Problem::MissingReference => {
                "Edit the item and select an existing selection or an existing \
                 material, or delete it."
            }
            Problem::NoSection => {
                "Under Sections create a section for the part or extend an existing one \
                 to it."
            }
            Problem::NoElastic => {
                "Edit the material and enter Young's modulus and Poisson's ratio, or \
                 take a material from the material library."
            }
            Problem::NoDensity => "Edit the material and enter a density.",
            Problem::NoConductivity => "Edit the material and enter a thermal conductivity.",
            Problem::NoSpecificHeat => {
                "Edit the material and enter a specific heat, or compute the step as \
                 steady state."
            }
            Problem::NoInitialTemperature => {
                "Under Initial Conditions create an initial temperature for the model \
                 (for thermal expansion the temperature of the stress-free state), or \
                 compute the step as steady state."
            }
            Problem::InvalidElastic => {
                "Enter a Young's modulus greater than 0 and a Poisson's ratio less than \
                 0.5 (steel: 210000 MPa, 0.3)."
            }
            Problem::NoRigidBody => {
                "Create a Rigid Body constraint on the surface or nodes the reference point \
                 shall drive, or put the boundary condition or load on nodes of the mesh."
            }
            Problem::InvalidPlastic => {
                "Edit the material: enter the elastic constants and a hardening curve that \
                 starts at plastic strain 0 with the yield stress, e.g. 235 MPa at 0 and \
                 400 MPa at 0.2."
            }
            Problem::DistortedElements => {
                "Remesh the part: a smaller element size at tight radii or thin walls, \
                 or linear instead of quadratic elements. For imported meshes check the \
                 node order."
            }
            Problem::RigidBodyMotion => {
                "Add a boundary condition that blocks the listed motions, tie the parts \
                 to supported parts with a Tie, or add weak springs (Constraints \
                 > Point Spring) to ground."
            }
            Problem::Mechanism => {
                "Mesh every truss with exactly one element (Mesh Setup > Meshing \
                 Parameters: max element size larger than the longest truss). Also fix \
                 a plane truss at all nodes perpendicular to its plane. If the trusses \
                 should carry bending, use a Beam Section instead of the Truss \
                 Section."
            }
            Problem::HeldOnlyByContact => {
                "Additionally support the parts weakly (Point Spring with a small \
                 stiffness), let the contact surfaces touch at the start (Adjust in the \
                 contact pair) or bring the parts into contact with a small \
                 displacement in a first step."
            }
            Problem::ConflictingBoundaries => {
                "Change the selection of the boundary conditions so that no nodes \
                 overlap, or give both boundary conditions the same value."
            }
            Problem::LoadOnFixedNodes => {
                "Apply the load to a surface or nodes that can move, or remove the \
                 boundary condition there."
            }
            Problem::RotationsIgnored => {
                "Leave UR1 to UR3 free in the boundary condition. Prescribe rotations of \
                 solids by displacements of several nodes."
            }
            Problem::NoGlobalResults => {
                "Open Model > Model Properties, set the model type to Submodel and pick the results \
                 file (.frd) of the global model."
            }
            Problem::IncrementExceedsStep => {
                "In the step choose an initial increment no larger than the time period."
            }
            Problem::NoLoad => "Under Loads create a load or activate a deactivated one.",
            Problem::NoExpansion => {
                "Edit the material and enter a thermal expansion coefficient \
                 (steel: 1.2e-5 1/K)."
            }
            Problem::NoStoredModes => {
                "Add a Frequency step with \"Store eigenmodes (Storage)\" before it, with \
                 the same supports (for Complex Frequency as a perturbation step after a \
                 static step with the centrifugal load)."
            }
            Problem::VelocityIgnored => "Add a Dynamic step or deactivate the initial condition.",
            Problem::NoRotation => {
                "Create a Centrifugal load in the static step before the Frequency step."
            }
            Problem::NoConvergence => {
                "Check the contacts (stiffness of the Surface Interaction, Adjust, \
                 position of the surfaces), support the parts sufficiently, spread the \
                 load over several increments (incrementation Automatic with a smaller \
                 initial increment) and refine the mesh at the contact surfaces."
            }
            Problem::MpcAndSpc => {
                "Remove the boundary condition from the dependent nodes of the equation \
                 or reformulate the equation."
            }
            Problem::RotationIn2d => {
                "In 2D models prescribe only U1 and U2; check user *BOUNDARY keywords in \
                 the Keyword Editor."
            }
            Problem::SolverError => {
                "Read the message in the Monitor; \"Check Model\" in the context menu of \
                 the analysis often helps."
            }
        }
    }
}

/// A problem found at an item of the model.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub item: ModelItem,
    pub problem: Problem,
    /// What exactly is affected, e.g. the parts or the number of nodes.
    pub detail: String,
}

impl Finding {
    fn new(item: ModelItem, problem: Problem, detail: impl Into<String>) -> Self {
        Self {
            item,
            problem,
            detail: detail.into(),
        }
    }

    pub fn severity(&self) -> Severity {
        self.problem.severity()
    }
}

/// What the checks need of the mesh, computed once per mesh: which nodes are connected and
/// which elements are distorted.
#[derive(Clone, Debug)]
pub struct MeshCheck {
    /// Node count, element count, part count and model space it was made for.
    stamp: (usize, usize, usize, ModelSpace),
    /// Connected piece of each node by node index; `u32::MAX` for nodes of no element.
    piece: Vec<u32>,
    pieces: usize,
    /// Parts by index that each piece contains.
    piece_parts: Vec<Vec<usize>>,
    /// Elements with a nonpositive Jacobian determinant of each part, by part index.
    distorted: Vec<Vec<ElementId>>,
}

impl MeshCheck {
    pub fn new(mesh: &FeMesh, space: ModelSpace) -> Self {
        let mut union = UnionFind::new(mesh.node_count());
        let mut used = vec![false; mesh.node_count()];
        for element in mesh.elements() {
            let mut first = None;
            for index in element.nodes.iter().filter_map(|&n| mesh.node_index(n)) {
                used[index] = true;
                match first {
                    None => first = Some(index),
                    Some(first) => union.join(first, index),
                }
            }
        }
        let mut label = vec![u32::MAX; mesh.node_count()];
        let mut pieces = 0;
        let piece: Vec<u32> = (0..mesh.node_count())
            .map(|i| {
                if !used[i] {
                    return u32::MAX;
                }
                let root = union.root(i);
                if label[root] == u32::MAX {
                    label[root] = pieces as u32;
                    pieces += 1;
                }
                label[root]
            })
            .collect();
        let mut piece_parts = vec![Vec::new(); pieces];
        let mut distorted = Vec::with_capacity(mesh.parts.len());
        for (index, part) in mesh.parts.iter().enumerate() {
            let mut bad = Vec::new();
            let mut seen = HashSet::new();
            for element in part.elements.iter().filter_map(|&e| mesh.element(e)) {
                if let Some(p) = (element.nodes.first())
                    .and_then(|&n| mesh.node_index(n))
                    .map(|i| piece[i])
                    && seen.insert(p)
                {
                    piece_parts[p as usize].push(index);
                }
                if element.min_jacobian(mesh).is_some_and(|j| j <= 0.0) {
                    bad.push(element.id);
                }
            }
            distorted.push(bad);
        }
        Self {
            stamp: Self::stamp(mesh, space),
            piece,
            pieces,
            piece_parts,
            distorted,
        }
    }

    fn stamp(mesh: &FeMesh, space: ModelSpace) -> (usize, usize, usize, ModelSpace) {
        (
            mesh.node_count(),
            mesh.element_count(),
            mesh.parts.len(),
            space,
        )
    }

    /// Whether it was made for this mesh; a mesh with other counts needs a new check.
    pub fn is_current(&self, mesh: &FeMesh, space: ModelSpace) -> bool {
        self.stamp == Self::stamp(mesh, space)
    }

    fn piece_of(&self, mesh: &FeMesh, node: NodeId) -> Option<u32> {
        let piece = *self.piece.get(mesh.node_index(node)?)?;
        (piece != u32::MAX).then_some(piece)
    }
}

impl FeModel {
    /// All problems of the model, invalid references included, errors first.
    pub fn check(&self, mesh: &FeMesh, mesh_check: &MeshCheck) -> Vec<Finding> {
        let mut findings: Vec<Finding> = (self.invalid_items(mesh).into_iter())
            .map(|i| Finding::new(i.item, Problem::MissingReference, i.reason))
            .collect();
        self.check_materials(&mut findings);
        self.check_sections(mesh, &mut findings);
        for (part, bad) in mesh_check.distorted.iter().enumerate() {
            if !bad.is_empty() {
                let detail = format!(
                    "{} element(s) with nonpositive Jacobian determinant, e.g. {}",
                    bad.len(),
                    list(bad.iter().take(5))
                );
                findings.push(Finding::new(
                    ModelItem::Part(part),
                    Problem::DistortedElements,
                    detail,
                ));
            }
        }
        let initial_temperature =
            (self.initial_conditions.iter()).any(|c| c.active && !c.kind.is_velocity());
        let dynamic =
            (self.steps.iter()).any(|s| s.active && matches!(s.kind, StepKind::Dynamic(_)));
        for (i, condition) in self.initial_conditions.iter().enumerate() {
            if condition.active && condition.kind.is_velocity() && !dynamic {
                findings.push(Finding::new(
                    ModelItem::InitialCondition(i),
                    Problem::VelocityIgnored,
                    format!("{} needs a Dynamic step", condition.name),
                ));
            }
        }
        let trusses = Trusses::new(self, mesh);
        for (s, step) in self.steps.iter().enumerate().filter(|(_, s)| s.active) {
            self.check_step(s, mesh, mesh_check, &trusses, &mut findings);
            let stored = (self.steps[..s].iter().filter(|p| p.active))
                .any(|p| matches!(&p.kind, StepKind::Frequency(f) if f.storage));
            if step.kind.uses_stored_modes() && !stored {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoStoredModes,
                    format!("{} needs the eigenmodes of a Frequency step", step.name),
                ));
            }
            if let StepKind::HeatTransfer(h) | StepKind::CoupledTempDisp(h) = &step.kind
                && !h.steady_state
                && !initial_temperature
            {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoInitialTemperature,
                    format!("{} is transient", step.name),
                ));
            }
            // CalculiX refuses a *Temperature without an initial temperature.
            let defined =
                step.kind.supports_defined_fields() && step.defined_fields.iter().any(|f| f.active);
            if defined && !initial_temperature {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoInitialTemperature,
                    format!("{} prescribes temperatures (Defined Field)", step.name),
                ));
            }
            if let StepKind::Static(settings) = &step.kind
                && settings.incrementation != crate::Incrementation::Default
                && settings.initial_increment > settings.time_period
            {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::IncrementExceedsStep,
                    format!(
                        "Initial increment {} > time period {}",
                        settings.initial_increment, settings.time_period
                    ),
                ));
            }
        }
        findings.sort_by_key(|f| f.severity());
        findings
    }

    fn check_materials(&self, findings: &mut Vec<Finding>) {
        let active = || self.steps.iter().filter(|s| s.active);
        let mechanical = active().any(|s| s.kind.is_mechanical());
        let thermal = active().any(|s| s.kind.is_thermal());
        let transient = active().any(|s| match &s.kind {
            StepKind::HeatTransfer(h) | StepKind::CoupledTempDisp(h) => !h.steady_state,
            _ => false,
        });
        // Eigenfrequencies and inertia need the mass.
        let frequency = active().any(|s| {
            matches!(s.kind, StepKind::Frequency(_) | StepKind::Dynamic(_))
                || s.kind.uses_stored_modes()
        });
        // Gravity and centrifugal loads act on the mass of the elements.
        let body_force = active().any(|s| {
            s.kind.supports_loads() && (s.loads.iter()).any(|l| l.active && l.kind.is_body_force())
        });
        for (i, material) in self.materials.iter().enumerate() {
            if !self.sections.iter().any(|s| s.material == material.name) {
                continue;
            }
            let item = ModelItem::Material(i);
            match material.elastic {
                None if mechanical => findings.push(Finding::new(
                    item,
                    Problem::NoElastic,
                    format!("{} has no elasticity", material.name),
                )),
                Some(e) if e.young <= 0.0 || e.poisson >= 0.5 || e.poisson <= -1.0 => {
                    findings.push(Finding::new(
                        item,
                        Problem::InvalidElastic,
                        format!("E = {}, nu = {}", e.young, e.poisson),
                    ));
                }
                _ => {}
            }
            if let Some(plastic) = &material.plastic
                && mechanical
            {
                let detail = match plastic.invalid_row() {
                    _ if material.elastic.is_none() => {
                        Some(format!("{} is plastic but not elastic", material.name))
                    }
                    Some(row) => Some(format!(
                        "{}: row {} of the hardening curve",
                        material.name,
                        row + 1
                    )),
                    None => None,
                };
                if let Some(detail) = detail {
                    findings.push(Finding::new(item, Problem::InvalidPlastic, detail));
                }
            }
            if thermal && material.conductivity.is_none() {
                findings.push(Finding::new(
                    item,
                    Problem::NoConductivity,
                    format!("{} has no thermal conductivity", material.name),
                ));
            }
            if transient && material.specific_heat.is_none() {
                findings.push(Finding::new(
                    item,
                    Problem::NoSpecificHeat,
                    format!("{} has no specific heat", material.name),
                ));
            }
            if (frequency || transient || body_force) && material.density.is_none_or(|d| d <= 0.0) {
                findings.push(Finding::new(
                    item,
                    Problem::NoDensity,
                    format!("{} has no density", material.name),
                ));
            }
        }
    }

    /// Parts with elements no section covers.
    fn check_sections(&self, mesh: &FeMesh, findings: &mut Vec<Finding>) {
        let whole: HashSet<&str> = (self.sections.iter())
            .filter_map(|s| match &s.region {
                crate::Region::Parts(names) => Some(names.iter().map(String::as_str)),
                _ => None,
            })
            .flatten()
            .collect();
        let mut covered: Option<HashSet<ElementId>> = None;
        for (i, part) in mesh.parts.iter().enumerate() {
            if whole.contains(part.name.as_str()) || part.elements.is_empty() {
                continue;
            }
            // Element sets and faces are resolved only when a part is not covered as a whole.
            let covered = covered.get_or_insert_with(|| {
                (self.sections.iter())
                    .filter(|s| !matches!(s.region, crate::Region::Parts(_)))
                    .flat_map(|s| s.region.elements(mesh))
                    .collect()
            });
            let missing = (part.elements.iter())
                .filter(|e| !covered.contains(e))
                .count();
            if missing > 0 {
                findings.push(Finding::new(
                    ModelItem::Part(i),
                    Problem::NoSection,
                    format!(
                        "{missing} of {} elements of {} without section",
                        part.elements.len(),
                        part.name
                    ),
                ));
            }
        }
    }

    fn check_step(
        &self,
        s: usize,
        mesh: &FeMesh,
        check: &MeshCheck,
        trusses: &Trusses,
        findings: &mut Vec<Finding>,
    ) {
        let step = &self.steps[s];
        let space = self.properties.space;
        let dofs = if space.is_2d() { 2 } else { 6 };
        let driven = |region: &Region| {
            region.reference_point().is_none_or(|point| {
                self.constraints.iter().any(|c| {
                    matches!(c, Constraint::RigidBody(body) if body.active && body.reference_point == point)
                })
            })
        };
        for (i, bc) in step.boundary_conditions.iter().enumerate() {
            if bc.active && step.kind.supports_boundary(&bc.kind) && !driven(&bc.region) {
                findings.push(Finding::new(
                    ModelItem::BoundaryCondition(s, i),
                    Problem::NoRigidBody,
                    format!("{}: {}", bc.name, bc.region.describe()),
                ));
            }
        }
        for (i, load) in step.loads.iter().enumerate() {
            if load.active && step.kind.supports_load(&load.kind) && !driven(&load.region) {
                findings.push(Finding::new(
                    ModelItem::Load(s, i),
                    Problem::NoRigidBody,
                    format!("{}: {}", load.name, load.region.describe()),
                ));
            }
        }
        // Value of each constrained node and degree of freedom, with the boundary condition.
        let mut fixed: HashMap<(NodeId, usize), (usize, f64)> = HashMap::new();
        for (i, bc) in step.boundary_conditions.iter().enumerate() {
            // A step leaves out what it cannot take, as the input file does.
            if !bc.active
                || !step.kind.supports_boundary(&bc.kind)
                || bc.region.missing_reference(mesh).is_some()
            {
                continue;
            }
            let values: Vec<(usize, f64)> = match bc.kind {
                BoundaryKind::Fixed => (0..dofs).map(|d| (d, 0.0)).collect(),
                BoundaryKind::Displacement(values) => (values.iter().enumerate())
                    .filter_map(|(d, v)| v.map(|v| (d, v)))
                    .collect(),
                // Degree of freedom 11.
                BoundaryKind::Temperature(t) => vec![(10, t)],
                // The values come from the global model; NaN stands for them.
                BoundaryKind::Submodel { dofs: held, .. } => (0..dofs)
                    .filter(|&d| held[d])
                    .map(|d| (d, f64::NAN))
                    .collect(),
            };
            if let BoundaryKind::Submodel { .. } = bc.kind
                && self.properties.submodel_input().is_none()
            {
                findings.push(Finding::new(
                    ModelItem::BoundaryCondition(s, i),
                    Problem::NoGlobalResults,
                    format!(
                        "{} needs the results of a global model (Model > Model Properties: model type \
                         Submodel and global results file)",
                        bc.name
                    ),
                ));
            }
            // The rotations of a reference point turn its rigid body.
            if let BoundaryKind::Displacement(values) = bc.kind
                && values[3..].iter().any(Option::is_some)
                && bc.region.reference_point().is_none()
                && (space.is_2d() || !trusses.rotational.iter().any(|&r| r))
            {
                findings.push(Finding::new(
                    ModelItem::BoundaryCondition(s, i),
                    Problem::RotationsIgnored,
                    format!("{} prescribes rotations", bc.name),
                ));
            }
            let mut conflicts: HashMap<usize, usize> = HashMap::new();
            for node in bc.region.nodes(mesh) {
                for &(dof, value) in &values {
                    match fixed.insert((node, dof), (i, value)) {
                        Some((other, old)) if other != i && !same(old, value) => {
                            *conflicts.entry(other).or_default() += 1;
                        }
                        _ => {}
                    }
                }
            }
            for (other, count) in conflicts {
                findings.push(Finding::new(
                    ModelItem::BoundaryCondition(s, i),
                    Problem::ConflictingBoundaries,
                    format!(
                        "{count} degrees of freedom also in {} with a different value; {} applies",
                        step.boundary_conditions[other].name, bc.name
                    ),
                ));
            }
        }
        let user_keywords = self.user_keywords.iter().any(|k| k.active);
        // A defined temperature strains the model like a load, if the materials expand.
        let mut temperatures = false;
        if step.kind.supports_defined_fields() {
            for (i, field) in step.defined_fields.iter().enumerate() {
                let missing = match &field.kind {
                    DefinedFieldKind::Temperature(_) => field.region.missing_reference(mesh),
                    DefinedFieldKind::TemperatureFromFile { .. } => None,
                };
                if !field.active || missing.is_some() {
                    continue;
                }
                temperatures = true;
                let used = |m: &&Material| self.sections.iter().any(|s| s.material == m.name);
                let expands = (self.materials.iter().filter(used)).any(|m| m.expansion.is_some());
                if !expands {
                    findings.push(Finding::new(
                        ModelItem::DefinedField(s, i),
                        Problem::NoExpansion,
                        format!("{}: no material with thermal expansion", field.name),
                    ));
                }
            }
        }
        if step.kind.supports_loads() {
            let mut loaded = temperatures;
            for (i, load) in step.loads.iter().enumerate() {
                if !load.active
                    || !step.kind.supports_load(&load.kind)
                    || load.region.missing_reference(mesh).is_some()
                {
                    continue;
                }
                loaded = true;
                let directions: Vec<usize> = match load.kind {
                    LoadKind::ConcentratedForce(f) | LoadKind::SurfaceTraction(f) => {
                        (0..3).filter(|&d| f[d] != 0.0 && d < dofs).collect()
                    }
                    LoadKind::Moment(m) => (0..3)
                        .filter(|&d| m[d] != 0.0 && d + 3 < dofs)
                        .map(|d| d + 3)
                        .collect(),
                    // Acts on its own node, which nothing holds.
                    LoadKind::PreTension { .. } => Vec::new(),
                    LoadKind::Pressure(_) | LoadKind::Centrifugal { .. } => {
                        (0..dofs.min(3)).collect()
                    }
                    LoadKind::Gravity(g) => (0..3).filter(|&d| g[d] != 0.0 && d < dofs).collect(),
                    // Heat flows go into temperatures, which nothing but a temperature holds.
                    _ => Vec::new(),
                };
                let nodes = load.region.nodes(mesh);
                let held = !nodes.is_empty()
                    && !directions.is_empty()
                    && nodes
                        .iter()
                        .all(|&n| directions.iter().all(|&d| fixed.contains_key(&(n, d))));
                if held {
                    findings.push(Finding::new(
                        ModelItem::Load(s, i),
                        Problem::LoadOnFixedNodes,
                        format!("{}: all {} nodes fixed", load.name, nodes.len()),
                    ));
                }
            }
            let displaced = fixed.values().any(|&(_, v)| v != 0.0);
            // A Dynamic step may start from an initial velocity instead of a load.
            let started = matches!(step.kind, StepKind::Dynamic(_))
                && (self.initial_conditions.iter()).any(|c| c.active && c.kind.is_velocity());
            if !loaded && !displaced && !started && !user_keywords {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoLoad,
                    format!("{} has no active load", step.name),
                ));
            }
        }
        // Free-free eigenfrequencies are fine; a static or buckle step needs every part held. Own
        // keywords may hold parts in ways the model does not know of.
        let constrained_by_keywords = self.user_keywords.iter().any(|k| {
            let text = k.text.to_ascii_uppercase();
            k.active
                && [
                    "*BOUNDARY",
                    "*EQUATION",
                    "*RIGID",
                    "*COUPLING",
                    "*MPC",
                    "*SPRING",
                ]
                .iter()
                .any(|w| text.contains(w))
        });
        if let StepKind::ComplexFrequency(settings) = &step.kind {
            let before = || self.steps[..s].iter().filter(|s| s.active);
            let stored = before().rev().find_map(|s| match &s.kind {
                StepKind::Frequency(f) => Some(f.storage),
                _ => None,
            });
            if stored != Some(true) {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoStoredModes,
                    format!("{}: no Frequency step with Storage before it", step.name),
                ));
            }
            let rotating = before().any(|s| {
                s.kind.supports_loads()
                    && (s.loads.iter())
                        .any(|l| l.active && matches!(l.kind, LoadKind::Centrifugal { .. }))
            });
            if settings.coriolis && !rotating {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoRotation,
                    format!("{}: no centrifugal load in a step before it", step.name),
                ));
            }
        }
        // A buckle step solves the static state of its loads first.
        let static_mechanical = matches!(
            step.kind,
            StepKind::Static(_) | StepKind::Buckle(_) | StepKind::CoupledTempDisp(_)
        );
        if static_mechanical && !constrained_by_keywords {
            self.check_rigid_body(s, mesh, check, trusses, &fixed, findings);
            self.check_trusses(s, mesh, trusses, &fixed, findings);
        }
    }

    /// Degrees of freedom (0..6) the active boundary conditions of the step prescribe at a
    /// reference point.
    fn point_dofs(&self, s: usize, point: &str) -> Vec<usize> {
        let step = &self.steps[s];
        let dofs = if self.properties.space.is_2d() { 2 } else { 6 };
        let mut out = Vec::new();
        for bc in &step.boundary_conditions {
            if !bc.active
                || !step.kind.supports_boundary(&bc.kind)
                || bc.region.reference_point() != Some(point)
            {
                continue;
            }
            match bc.kind {
                BoundaryKind::Fixed => out.extend(0..dofs),
                BoundaryKind::Displacement(values) => {
                    out.extend((0..dofs).filter(|&d| values[d].is_some()));
                }
                BoundaryKind::Submodel { dofs: driven, .. } => {
                    out.extend((0..dofs).filter(|&d| driven[d]));
                }
                BoundaryKind::Temperature(_) => {}
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Finds pieces of the model that can move as a rigid body in a static step: the rigid
    /// body motions a support at point x in direction n prevents are those with
    /// n . (t + w x (x - c)) != 0, so a piece is held when the rows [n, (x - c) x n] of its
    /// supports have full rank. Ties join pieces; contact pairs join them only loosely.
    fn check_rigid_body(
        &self,
        s: usize,
        mesh: &FeMesh,
        check: &MeshCheck,
        trusses: &Trusses,
        fixed: &HashMap<(NodeId, usize), (usize, f64)>,
        findings: &mut Vec<Finding>,
    ) {
        let Some((min, max)) = mesh.bounds() else {
            return;
        };
        let center: [f64; 3] = std::array::from_fn(|i| 0.5 * (min[i] + max[i]));
        let size = (0..3)
            .map(|i| max[i] - min[i])
            .fold(0.0, f64::max)
            .max(1e-30);
        let space = self.properties.space;
        let mut grams = vec![[[0.0; 6]; 6]; check.pieces];
        // A support holds its node in a direction, or a beam or shell node about an axis.
        let mut support = |node: NodeId, direction: [f64; 3], rotation: bool| {
            let (Some(piece), Some(x)) = (check.piece_of(mesh, node), mesh.node(node)) else {
                return;
            };
            let row = if rotation {
                [0.0, 0.0, 0.0, direction[0], direction[1], direction[2]]
            } else {
                let r: [f64; 3] = std::array::from_fn(|i| (x[i] - center[i]) / size);
                rigid_row(space, r, direction)
            };
            let gram = &mut grams[piece as usize];
            for a in 0..6 {
                for b in 0..6 {
                    gram[a][b] += row[a] * row[b];
                }
            }
        };
        let axis = |d: usize| std::array::from_fn(|i| if i == d { 1.0 } else { 0.0 });
        for &(node, dof) in fixed.keys() {
            if dof < 3 {
                support(node, axis(dof), false);
            } else if dof < 6
                && space == ModelSpace::ThreeD
                && (mesh.node_index(node)).is_some_and(|i| trusses.rotational[i])
            {
                support(node, axis(dof - 3), true);
            }
        }
        let mut ties = UnionFind::new(check.pieces);
        let mut contacts = UnionFind::new(check.pieces);
        let pieces = |region: &crate::Region| -> Vec<usize> {
            let mut pieces: Vec<usize> = (region.nodes(mesh).into_iter())
                .filter_map(|n| check.piece_of(mesh, n))
                .map(|p| p as usize)
                .collect();
            pieces.sort_unstable();
            pieces.dedup();
            pieces
        };
        let join = |union: &mut UnionFind, a: &[usize], b: &[usize]| {
            for &x in a.iter().chain(b) {
                union.join(a.first().copied().unwrap_or(x), x);
            }
        };
        for constraint in self.constraints.iter().filter(|c| c.active()) {
            match constraint {
                Constraint::PointSpring(c) => {
                    for node in c.region.nodes(mesh) {
                        for d in (0..3).filter(|&d| c.stiffness[d] > 0.0) {
                            support(node, axis(d), false);
                        }
                    }
                }
                Constraint::SurfaceSpring(c) => {
                    for node in c.region.nodes(mesh) {
                        for d in (0..3).filter(|&d| c.stiffness[d] > 0.0) {
                            support(node, axis(d), false);
                        }
                    }
                }
                Constraint::CompressionOnly(c) => {
                    for (node, normal) in face_normals(mesh, &c.region) {
                        support(node, normal, false);
                    }
                }
                Constraint::Tie(_) | Constraint::SurfaceToSurfaceSpring(_) => {
                    if let Some([master, slave]) = constraint.master_slave() {
                        let (a, b) = (pieces(master), pieces(slave));
                        join(&mut ties, &a, &b);
                        join(&mut contacts, &a, &b);
                    }
                }
                Constraint::NodeTie(c) => {
                    let tied = pieces(&c.region);
                    join(&mut ties, &tied, &[]);
                    join(&mut contacts, &tied, &[]);
                }
                Constraint::RigidBody(body) => {
                    let tied = pieces(&body.region);
                    join(&mut ties, &tied, &[]);
                    join(&mut contacts, &tied, &[]);
                    // A boundary condition on the reference point holds the body: its
                    // translations like a support at the node nearest the point, its
                    // rotations like those of a beam node.
                    let Some(point) = self.reference_point(&body.reference_point) else {
                        continue;
                    };
                    let nearest = (body.region.nodes(mesh).into_iter())
                        .filter_map(|n| mesh.node(n).map(|x| (n, x)))
                        .min_by(|a, b| {
                            let d = |x: &[f64; 3]| {
                                (0..3)
                                    .map(|i| (x[i] - point.position[i]).powi(2))
                                    .sum::<f64>()
                            };
                            d(&a.1).total_cmp(&d(&b.1))
                        })
                        .map(|(n, _)| n);
                    let Some(node) = nearest else {
                        continue;
                    };
                    for dof in self.point_dofs(s, &body.reference_point) {
                        if dof < 3 {
                            support(node, axis(dof), false);
                        } else if dof < 6 && space == ModelSpace::ThreeD {
                            support(node, axis(dof - 3), true);
                        }
                    }
                }
            }
        }
        for tie in self.node_ties.iter().filter(|t| t.active) {
            let tied = pieces(&tie.region);
            join(&mut ties, &tied, &[]);
            join(&mut contacts, &tied, &[]);
        }
        for tie in self.ties.iter().filter(|t| t.active) {
            let (a, b) = (pieces(&tie.master), pieces(&tie.slave));
            join(&mut ties, &a, &b);
            join(&mut contacts, &a, &b);
        }
        for pair in self.contact_pairs.iter().filter(|c| c.active) {
            join(&mut contacts, &pieces(&pair.master), &pieces(&pair.slave));
        }
        let sum = |union: &mut UnionFind| {
            let mut sums: HashMap<usize, [[f64; 6]; 6]> = HashMap::new();
            for (piece, gram) in grams.iter().enumerate() {
                let total = sums.entry(union.root(piece)).or_insert([[0.0; 6]; 6]);
                for a in 0..6 {
                    for b in 0..6 {
                        total[a][b] += gram[a][b];
                    }
                }
            }
            sums
        };
        let held = sum(&mut ties);
        let held_by_contact = sum(&mut contacts);
        let unknowns = match space {
            ModelSpace::ThreeD => 6,
            ModelSpace::Axisymmetric => 1,
            _ => 3,
        };
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for piece in 0..check.pieces {
            groups.entry(ties.root(piece)).or_default().push(piece);
        }
        let mut groups: Vec<(usize, Vec<usize>)> = groups.into_iter().collect();
        groups.sort_by_key(|(_, pieces)| pieces[0]);
        for (root, group) in groups {
            let free = free_motions(&held[&root], unknowns);
            if free.is_empty() {
                continue;
            }
            let mut parts: Vec<usize> = (group.iter())
                .flat_map(|&p| check.piece_parts[p].iter().copied())
                .collect();
            parts.sort_unstable();
            parts.dedup();
            let names: Vec<&str> = (parts.iter())
                .filter_map(|&p| mesh.parts.get(p).map(|p| p.name.as_str()))
                .collect();
            let contact_root = contacts.root(group[0]);
            let problem = if free_motions(&held_by_contact[&contact_root], unknowns).is_empty() {
                Problem::HeldOnlyByContact
            } else {
                Problem::RigidBodyMotion
            };
            let what = if names.is_empty() {
                "Elements without part".to_string()
            } else if names.len() > 5 {
                format!("{} and {} more", names[..5].join(", "), names.len() - 5)
            } else {
                names.join(", ")
            };
            let detail = format!(
                "{}: {what}: {}",
                self.steps[s].name,
                describe_motions(space, &free)
            );
            findings.push(Finding::new(
                ModelItem::BoundaryConditions(s),
                problem,
                detail.clone(),
            ));
            for part in parts {
                findings.push(Finding::new(ModelItem::Part(part), problem, detail.clone()));
            }
        }
    }

    /// Finds truss nodes that can move without resistance in a static step. A bar holds its
    /// end nodes along its axis only, so a node is held when its bars, supports and springs
    /// span all three directions; tied nodes are written as one node and count together.
    /// Inner nodes of a bar meshed with several elements and the nodes of a plane truss
    /// fail this, and CalculiX solves the singular system without a word.
    fn check_trusses(
        &self,
        s: usize,
        mesh: &FeMesh,
        trusses: &Trusses,
        fixed: &HashMap<(NodeId, usize), (usize, f64)>,
        findings: &mut Vec<Finding>,
    ) {
        if trusses.bars.is_empty() || self.properties.space != ModelSpace::ThreeD {
            return;
        }
        let mut joined = UnionFind::new(mesh.node_count());
        // Nodes held by something this check does not follow, such as a tie to a solid.
        let mut held_otherwise = vec![false; mesh.node_count()];
        let mark = |region: &crate::Region, held: &mut Vec<bool>| {
            for index in region
                .nodes(mesh)
                .iter()
                .filter_map(|&n| mesh.node_index(n))
            {
                held[index] = true;
            }
        };
        let mut gram = vec![[[0.0; 6]; 6]; mesh.node_count()];
        let add = |gram: &mut Vec<[[f64; 6]; 6]>, node: NodeId, d: [f64; 3]| {
            if let Some(index) = mesh.node_index(node) {
                for a in 0..3 {
                    for b in 0..3 {
                        gram[index][a][b] += d[a] * d[b];
                    }
                }
            }
        };
        let axis =
            |d: usize| -> [f64; 3] { std::array::from_fn(|i| if i == d { 1.0 } else { 0.0 }) };
        for tie in self.node_ties.iter().filter(|t| t.active) {
            let nodes: Vec<usize> = (tie.region.nodes(mesh).into_iter())
                .filter_map(|n| mesh.node_index(n))
                .collect();
            for pair in nodes.windows(2) {
                joined.join(pair[0], pair[1]);
            }
        }
        for constraint in self.constraints.iter().filter(|c| c.active()) {
            match constraint {
                // Moved to the node ties when the project was read.
                Constraint::NodeTie(_) => {}
                Constraint::PointSpring(c) => {
                    for node in c.region.nodes(mesh) {
                        for d in (0..3).filter(|&d| c.stiffness[d] > 0.0) {
                            add(&mut gram, node, axis(d));
                        }
                    }
                }
                Constraint::SurfaceSpring(c) => {
                    for node in c.region.nodes(mesh) {
                        for d in (0..3).filter(|&d| c.stiffness[d] > 0.0) {
                            add(&mut gram, node, axis(d));
                        }
                    }
                }
                Constraint::CompressionOnly(c) => mark(&c.region, &mut held_otherwise),
                Constraint::Tie(_) | Constraint::SurfaceToSurfaceSpring(_) => {
                    for region in constraint.master_slave().into_iter().flatten() {
                        mark(region, &mut held_otherwise);
                    }
                }
                Constraint::RigidBody(body) => {
                    if !self.point_dofs(s, &body.reference_point).is_empty() {
                        mark(&body.region, &mut held_otherwise);
                    }
                }
            }
        }
        for tie in self.ties.iter().filter(|t| t.active) {
            mark(&tie.master, &mut held_otherwise);
            mark(&tie.slave, &mut held_otherwise);
        }
        for pair in self.contact_pairs.iter().filter(|c| c.active) {
            mark(&pair.master, &mut held_otherwise);
            mark(&pair.slave, &mut held_otherwise);
        }
        for &(node, dof) in fixed.keys() {
            if dof < 3 {
                add(&mut gram, node, axis(dof));
            }
        }
        for &(_, [a, b]) in &trusses.bars {
            let (Some(x), Some(y)) = (mesh.node(a), mesh.node(b)) else {
                continue;
            };
            let d: [f64; 3] = std::array::from_fn(|i| y[i] - x[i]);
            let length = d.iter().map(|v| v * v).sum::<f64>().sqrt();
            if length > 0.0 {
                let d = d.map(|v| v / length);
                add(&mut gram, a, d);
                add(&mut gram, b, d);
            }
        }
        // Sum over the tied nodes; a group with a node of another element is held by it.
        let mut groups: HashMap<usize, ([[f64; 6]; 6], bool, NodeId)> = HashMap::new();
        for (index, &id) in mesh.node_ids().iter().enumerate() {
            let (truss, other) = (trusses.truss[index], trusses.other[index]);
            if !truss && !other {
                continue;
            }
            let entry = (groups.entry(joined.root(index))).or_insert(([[0.0; 6]; 6], false, id));
            for (sum, row) in entry.0.iter_mut().zip(&gram[index]) {
                for (sum, value) in sum.iter_mut().zip(row) {
                    *sum += value;
                }
            }
            entry.1 |= other || held_otherwise[index];
            entry.2 = entry.2.min(id);
        }
        let mut free: Vec<(NodeId, Vec<[f64; 6]>)> = (groups.into_values())
            .filter(|(_, held, _)| !held)
            .filter_map(|(gram, _, node)| {
                let (values, vectors) = eigen(&gram, 3);
                let free: Vec<[f64; 6]> = (0..3)
                    .filter(|&i| values[i] <= 1e-9)
                    .map(|i| vectors[i])
                    .collect();
                (!free.is_empty()).then_some((node, free))
            })
            .collect();
        if free.is_empty() {
            return;
        }
        free.sort_by_key(|(node, _)| *node);
        let examples: Vec<String> = (free.iter().take(3))
            .map(|(node, directions)| format!("{node} {}", describe_directions(directions)))
            .collect();
        let detail = format!(
            "{}: {} truss nodes not held, e.g. node {}",
            self.steps[s].name,
            free.len(),
            examples.join(", ")
        );
        let free_nodes: HashSet<NodeId> = free.iter().map(|(node, _)| *node).collect();
        findings.push(Finding::new(
            ModelItem::BoundaryConditions(s),
            Problem::Mechanism,
            detail.clone(),
        ));
        for (part, p) in mesh.parts.iter().enumerate() {
            let touched = (p.elements.iter().filter_map(|&e| mesh.element(e)))
                .any(|e| e.nodes.iter().any(|n| free_nodes.contains(n)));
            if touched {
                findings.push(Finding::new(
                    ModelItem::Part(part),
                    Problem::Mechanism,
                    detail.clone(),
                ));
            }
        }
    }
}

/// Whether the nodes of an element have rotations: beams (B31, B32) and shells (S3 to S8R)
/// have them; trusses, plane and solid elements only displacements. A line element of a
/// truss section is a truss whatever its type name says; see [`Trusses`].
fn has_rotations(element: &plx_mesh::Element) -> bool {
    match element.shape.family() {
        ElementFamily::Line => element.type_name.to_ascii_uppercase().starts_with('B'),
        ElementFamily::Surface => {
            !element.is_plane() && element.type_name.to_ascii_uppercase().starts_with('S')
        }
        ElementFamily::Solid => false,
    }
}

/// The trusses of the model. The mesh stores beams and trusses alike as line elements
/// (meshed lines are `B32`); only a truss section makes one a truss, which has no rotations
/// and is written as `T3D2` between its end nodes.
struct Trusses {
    /// Each truss element with its two end nodes.
    bars: Vec<(ElementId, [NodeId; 2])>,
    /// Whether each node, by node index, is an end node of a truss.
    truss: Vec<bool>,
    /// Whether each node, by node index, belongs to an element other than a truss.
    other: Vec<bool>,
    /// Whether each node, by node index, has rotations.
    rotational: Vec<bool>,
}

impl Trusses {
    fn new(model: &FeModel, mesh: &FeMesh) -> Self {
        let ids: HashSet<ElementId> = (model.sections.iter())
            .filter(|s| matches!(s.kind, SectionKind::Truss { .. }))
            .flat_map(|s| s.region.elements(mesh))
            .collect();
        let mut bars = Vec::new();
        let mut truss = vec![false; mesh.node_count()];
        let mut other = vec![false; mesh.node_count()];
        let mut rotational = vec![false; mesh.node_count()];
        for element in mesh.elements() {
            if element.shape.family() == ElementFamily::Line && ids.contains(&element.id) {
                // The midside node of a 3-node line is left out of the written truss.
                if let (Some(&a), Some(&b)) = (element.nodes.first(), element.nodes.last()) {
                    bars.push((element.id, [a, b]));
                    for index in [a, b].into_iter().filter_map(|n| mesh.node_index(n)) {
                        truss[index] = true;
                    }
                }
                continue;
            }
            let rotations = has_rotations(element);
            for index in element.nodes.iter().filter_map(|&n| mesh.node_index(n)) {
                other[index] = true;
                rotational[index] |= rotations;
            }
        }
        Self {
            bars,
            truss,
            other,
            rotational,
        }
    }
}

/// Names the directions a truss node is free in: two free directions are across a single
/// bar, one is out of the plane of its bars.
fn describe_directions(free: &[[f64; 6]]) -> String {
    match free {
        [f] => match (0..3).find(|&i| f[i].abs() > 0.99) {
            Some(i) => format!("free in {}", ["X", "Y", "Z"][i]),
            None => "free perpendicular to the plane of the trusses".into(),
        },
        [_, _] => "free across the truss".into(),
        _ => "completely free".into(),
    }
}

/// Whether two prescribed values are the same up to rounding.
fn same(a: f64, b: f64) -> bool {
    // Values read from the global model of a submodel.
    if a.is_nan() && b.is_nan() {
        return true;
    }
    (a - b).abs() <= 1e-12 * a.abs().max(b.abs())
}

fn list<'a>(ids: impl Iterator<Item = &'a ElementId>) -> String {
    ids.map(u32::to_string).collect::<Vec<_>>().join(", ")
}

/// The rigid body motions a support in `direction` at `r` (relative to the centre) prevents,
/// as a row over the model's rigid body unknowns: translations, then rotations.
fn rigid_row(space: ModelSpace, r: [f64; 3], n: [f64; 3]) -> [f64; 6] {
    match space {
        ModelSpace::ThreeD => [
            n[0],
            n[1],
            n[2],
            r[1] * n[2] - r[2] * n[1],
            r[2] * n[0] - r[0] * n[2],
            r[0] * n[1] - r[1] * n[0],
        ],
        // A body of revolution can only move along its axis.
        ModelSpace::Axisymmetric => [n[1], 0.0, 0.0, 0.0, 0.0, 0.0],
        _ => [n[0], n[1], r[0] * n[1] - r[1] * n[0], 0.0, 0.0, 0.0],
    }
}

/// Nodes of the faces of a region with the face normal, the direction a compression only
/// support holds them in.
fn face_normals(mesh: &FeMesh, region: &crate::Region) -> Vec<(NodeId, [f64; 3])> {
    let mut out = Vec::new();
    for (element, face) in region.faces(mesh) {
        let Some(element) = mesh.element(element) else {
            continue;
        };
        let Some(topology) = element.faces().get(usize::from(face).wrapping_sub(1)) else {
            continue;
        };
        let corners: Vec<(NodeId, [f64; 3])> = (topology.corners.iter())
            .filter_map(|&l| element.nodes.get(l))
            .filter_map(|&n| Some((n, mesh.node(n)?)))
            .collect();
        let normal = match corners.as_slice() {
            [(_, a), (_, b)] => [-(b[1] - a[1]), b[0] - a[0], 0.0],
            [(_, a), (_, b), (_, c), ..] => {
                let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
                let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
                [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ]
            }
            _ => continue,
        };
        let length = normal.iter().map(|x| x * x).sum::<f64>().sqrt();
        if length > 0.0 {
            let normal = normal.map(|x| x / length);
            out.extend(corners.iter().map(|&(n, _)| (n, normal)));
        }
    }
    out
}

/// The rigid body motions the supports summed up in `gram` leave free, as an orthonormal
/// basis of the first `unknowns` rigid body unknowns.
fn free_motions(gram: &[[f64; 6]; 6], unknowns: usize) -> Vec<[f64; 6]> {
    let (values, vectors) = eigen(gram, unknowns);
    let largest = values.iter().copied().fold(0.0, f64::max);
    (0..unknowns)
        .filter(|&i| values[i] <= 1e-9 * largest.max(1e-300) || largest == 0.0)
        .map(|i| vectors[i])
        .collect()
}

/// Names the free motions, e.g. "translation in X, rotation about Z".
fn describe_motions(space: ModelSpace, free: &[[f64; 6]]) -> String {
    let project = |v: &[f64; 6]| -> f64 {
        free.iter()
            .map(|f| (0..6).map(|i| f[i] * v[i]).sum::<f64>().powi(2))
            .sum()
    };
    let unit = |i: usize| -> [f64; 6] { std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }) };
    type Axes<'a> = &'a [(usize, &'a str)];
    let (translations, rotations): (Axes, Axes) = match space {
        ModelSpace::ThreeD => (
            &[(0, "X"), (1, "Y"), (2, "Z")],
            &[(3, "X"), (4, "Y"), (5, "Z")],
        ),
        ModelSpace::Axisymmetric => (&[(0, "Y")], &[]),
        _ => (&[(0, "X"), (1, "Y")], &[(2, "Z")]),
    };
    if free.len() == translations.len() + rotations.len() {
        return "completely free, nothing holds them".into();
    }
    let mut parts = Vec::new();
    let mut found = 0;
    for &(i, name) in translations {
        if project(&unit(i)) > 0.99 {
            parts.push(format!("displacement in {name}"));
            found += 1;
        }
    }
    // The remaining free motions are rotations about axes through some point; each axis is
    // named by the largest component of its rotation.
    let mut axes: Vec<&str> = Vec::new();
    for f in free {
        let translation: f64 = (translations.iter())
            .filter(|(i, _)| project(&unit(*i)) > 0.99)
            .map(|&(i, _)| f[i] * f[i])
            .sum();
        if translation > 0.99 {
            continue;
        }
        if let Some(&(_, name)) =
            (rotations.iter()).max_by(|a, b| f[a.0].abs().total_cmp(&f[b.0].abs()))
            && !axes.contains(&name)
        {
            axes.push(name);
        }
    }
    let rotations_free = free.len().saturating_sub(found);
    for axis in axes.iter().take(rotations_free) {
        parts.push(format!("rotation about {axis}"));
    }
    if parts.is_empty() {
        parts.push(format!("{} motion(s)", free.len()));
    }
    format!("{} free", parts.join(", "))
}

/// Eigenvalues and eigenvectors of the upper left `n` x `n` block of a symmetric matrix by
/// Jacobi rotations; small enough to be exact for the six rigid body unknowns.
fn eigen(matrix: &[[f64; 6]; 6], n: usize) -> ([f64; 6], [[f64; 6]; 6]) {
    let mut a = *matrix;
    let mut v = [[0.0; 6]; 6];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..100 {
        let off: f64 = (0..n)
            .flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j)))
            .map(|(i, j)| a[i][j] * a[i][j])
            .sum();
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut().take(n) {
                    let (akp, akq) = (row[p], row[q]);
                    row[p] = c * akp - s * akq;
                    row[q] = s * akp + c * akq;
                }
                let (row_p, row_q) = (a[p], a[q]);
                for k in 0..n {
                    a[p][k] = c * row_p[k] - s * row_q[k];
                    a[q][k] = s * row_p[k] + c * row_q[k];
                }
                for row in v.iter_mut().take(n) {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let values = std::array::from_fn(|i| a[i][i]);
    // Columns of v are the eigenvectors.
    let vectors = std::array::from_fn(|i| std::array::from_fn(|k| v[k][i]));
    (values, vectors)
}

/// Problems CalculiX reports in its output after a failed run, mapped to the explanations of
/// the checks. Distorted elements and elements without material are also attached to their
/// parts.
pub fn diagnose_solver_output(lines: &[String], mesh: &FeMesh) -> Vec<Finding> {
    let text = lines.join(" ").to_ascii_lowercase();
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut findings = Vec::new();
    let add = |findings: &mut Vec<Finding>, problem: Problem, detail: String| {
        if !findings.iter().any(|f| f.problem == problem) {
            findings.push(Finding::new(ModelItem::Analysis, problem, detail));
        }
    };
    let elements_after = |marker: &str| -> Vec<ElementId> {
        let mut ids: Vec<ElementId> = (text.match_indices(marker))
            .filter_map(|(at, _)| {
                let rest = &text[at + marker.len()..];
                rest.split_whitespace().next()?.parse().ok()
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let mut part_findings = Vec::new();
    for (problem, marker) in [
        (
            Problem::DistortedElements,
            "nonpositive jacobian determinant in element",
        ),
        (Problem::NoSection, "no material was assigned to element"),
    ] {
        let ids = elements_after(marker);
        if ids.is_empty() {
            continue;
        }
        add(
            &mut findings,
            problem,
            format!(
                "{} element(s), e.g. {}",
                ids.len(),
                list(ids.iter().take(5))
            ),
        );
        for (p, part) in mesh.parts.iter().enumerate() {
            let in_part: Vec<ElementId> = (ids.iter())
                .filter(|id| part.elements.contains(id))
                .copied()
                .collect();
            if !in_part.is_empty() {
                part_findings.push(Finding::new(
                    ModelItem::Part(p),
                    problem,
                    format!(
                        "According to CalculiX: element(s) {}",
                        list(in_part.iter().take(5))
                    ),
                ));
            }
        }
    }
    let known: [(Problem, &[&str]); 12] = [
        (Problem::NoElastic, &["no elastic constants"]),
        (
            Problem::InvalidPlastic,
            &["reading *PLASTIC", "plastic strain must be"],
        ),
        (Problem::NoDensity, &["no density was assigned"]),
        (Problem::NoConductivity, &["no conductivity constants"]),
        (Problem::NoSpecificHeat, &["no specific heat was assigned"]),
        (
            Problem::NoInitialTemperature,
            &["define initial conditions for the temperature"],
        ),
        (
            Problem::InvalidElastic,
            &["poisson coefficient should be less than 0.5"],
        ),
        (
            Problem::RigidBodyMotion,
            &["zero pivot", "matrix is singular", "singular matrix"],
        ),
        (
            Problem::NoConvergence,
            &[
                "too many cutbacks",
                "increment size smaller than minimum",
                "increment size is smaller than minimum",
                "too far apart",
            ],
        ),
        (
            Problem::IncrementExceedsStep,
            &["initial increment size exceeds step size"],
        ),
        (Problem::MpcAndSpc, &["dependent side of a mpc and a spc"]),
        (Problem::RotationIn2d, &["usermpc: mpc of type"]),
    ];
    for (problem, markers) in known {
        if let Some(marker) = markers.iter().find(|m| text.contains(*m)) {
            add(
                &mut findings,
                problem,
                format!("CalculiX reports \"{marker}\""),
            );
        }
    }
    if findings.is_empty()
        && let Some(line) = lines.iter().find(|l| l.contains("*ERROR"))
    {
        add(&mut findings, Problem::SolverError, line.trim().to_string());
    }
    findings.extend(part_findings);
    findings
}

/// Disjoint sets with path halving.
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        if a != b {
            self.parent[b] = a;
        }
    }
}

#[cfg(test)]
mod tests {
    use plx_mesh::{Element, ElementShape, Part};

    use super::*;
    use crate::{
        BoundaryCondition, ContactPair, Elastic, Hardening, Load, Material, Plastic, PlasticPoint,
        ReferencePoint, Region, RigidBody, Section, Step, SurfaceInteraction, Tie,
    };

    /// Unit cubes side by side along x, one C3D8 and one part each; neighbours share nodes
    /// when `joined`.
    fn cubes(count: u32, joined: bool) -> FeMesh {
        let mut mesh = FeMesh::default();
        let mut next = 1;
        for c in 0..count {
            let x0 = if joined { c as f64 } else { 1.5 * c as f64 };
            let first = next;
            for [x, y, z] in [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ] {
                mesh.set_node(next, [x0 + x, y, z]);
                next += 1;
            }
            let mut nodes: Vec<NodeId> = (first..first + 8).collect();
            if joined && c > 0 {
                // The left face is the right face of the previous cube.
                let previous = first - 8;
                for (left, right) in [(0, 1), (3, 2), (4, 5), (7, 6)] {
                    nodes[left] = previous + right;
                }
            }
            mesh.add_element(Element {
                id: c + 1,
                type_name: "C3D8".into(),
                shape: ElementShape::Hex8,
                nodes,
            })
            .unwrap();
            mesh.parts.push(Part {
                name: format!("PART-{}", c + 1),
                elements: vec![c + 1],
            });
        }
        mesh
    }

    fn steel() -> Material {
        Material {
            name: "Steel".into(),
            density: Some(7.85e-9),
            elastic: Some(Elastic {
                young: 210000.0,
                poisson: 0.3,
            }),
            ..Material::default()
        }
    }

    /// Steel on all parts, a static step with the left face of the first cube fixed and a
    /// force on node 7.
    fn model(mesh: &FeMesh) -> FeModel {
        let mut step = Step::new_static("Step-1");
        step.boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: Region::Nodes(vec![1, 4, 5, 8]),
            kind: BoundaryKind::Fixed,
            amplitude: None,
        });
        step.loads.push(Load {
            name: "Force-1".into(),
            active: true,
            region: Region::Nodes(vec![7]),
            kind: LoadKind::ConcentratedForce([0.0, 0.0, -1.0]),
            amplitude: None,
            factor_amplitude: None,
        });
        FeModel {
            materials: vec![steel()],
            sections: vec![Section {
                name: "Section-1".into(),
                material: "Steel".into(),
                region: Region::Parts(mesh.parts.iter().map(|p| p.name.clone()).collect()),
                thickness: 1.0,
                kind: Default::default(),
            }],
            steps: vec![step],
            ..FeModel::default()
        }
    }

    fn check(model: &FeModel, mesh: &FeMesh) -> Vec<Finding> {
        model.check(mesh, &MeshCheck::new(mesh, model.properties.space))
    }

    fn problems(findings: &[Finding]) -> Vec<(ModelItem, Problem)> {
        findings.iter().map(|f| (f.item, f.problem)).collect()
    }

    #[test]
    fn a_held_and_loaded_model_has_no_findings() {
        let mesh = cubes(2, true);
        assert_eq!(check(&model(&mesh), &mesh), []);
    }

    #[test]
    fn an_initial_velocity_needs_a_dynamic_step() {
        let mesh = cubes(2, true);
        let mut model = model(&mesh);
        model.initial_conditions.push(crate::InitialCondition {
            name: "Initial_Velocity-1".into(),
            active: true,
            region: Region::Nodes(vec![1]),
            kind: crate::InitialConditionKind::Velocity([1.0, 0.0, 0.0]),
        });
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::InitialCondition(0), Problem::VelocityIgnored)]
        );
        // A Dynamic step takes it up, even without a load.
        model.steps[0].kind = StepKind::Dynamic(crate::DynamicStep::default());
        model.steps[0].loads.clear();
        assert_eq!(check(&model, &mesh), []);
        model.initial_conditions[0].active = false;
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::Step(0), Problem::NoLoad)]
        );
    }

    #[test]
    fn submodel_boundaries_load_the_step_and_need_the_global_results() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        let step = &mut model.steps[0];
        step.loads.clear();
        step.boundary_conditions.push(BoundaryCondition {
            name: "Submodel-1".into(),
            active: true,
            region: Region::Nodes(vec![2, 3, 6, 7]),
            kind: BoundaryKind::Submodel {
                step: 1,
                dofs: [true, true, true, false, false, false],
            },
            amplitude: None,
        });
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::BoundaryCondition(0, 1), Problem::NoGlobalResults)]
        );
        model.properties.kind = crate::ModelKind::Submodel;
        model.properties.global_results = Some("global.frd".into());
        assert_eq!(check(&model, &mesh), []);
        assert!(model.uses_global_results());
        model.steps[0].kind = StepKind::Frequency(Default::default());
        assert!(!model.uses_global_results());
    }

    #[test]
    fn missing_supports_name_the_free_motions() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        model.steps[0].boundary_conditions[0].region = Region::Nodes(vec![1]);
        let findings = check(&model, &mesh);
        assert_eq!(
            problems(&findings),
            [
                (ModelItem::BoundaryConditions(0), Problem::RigidBodyMotion),
                (ModelItem::Part(0), Problem::RigidBodyMotion),
            ]
        );
        assert!(
            findings[0].detail.contains("rotation about"),
            "{findings:?}"
        );
        assert!(!findings[0].detail.contains("displacement"), "{findings:?}");
        model.steps[0].boundary_conditions.clear();
        let detail = &check(&model, &mesh)[0].detail;
        assert!(
            detail.ends_with(": completely free, nothing holds them"),
            "{detail}"
        );
        // Two corners fixed leave the rotation about the line through them.
        model.steps[0].boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: Region::Nodes(vec![1, 2]),
            kind: BoundaryKind::Fixed,
            amplitude: None,
        });
        let detail = &check(&model, &mesh)[0].detail;
        assert!(detail.ends_with(": rotation about X free"), "{detail}");
        // A spring on a third corner holds it.
        model
            .constraints
            .push(Constraint::PointSpring(crate::PointSpring {
                name: "Spring-1".into(),
                active: true,
                region: Region::Nodes(vec![7]),
                stiffness: [0.0, 0.0, 1.0],
            }));
        assert_eq!(check(&model, &mesh), []);
    }

    #[test]
    fn unconnected_parts_need_a_tie_and_contact_holds_them_only_loosely() {
        let mut mesh = cubes(2, false);
        mesh.surfaces.insert(
            "RIGHT".into(),
            plx_mesh::SurfaceDefinition::ElementFaces(vec![(1, 4)]),
        );
        mesh.surfaces.insert(
            "LEFT".into(),
            plx_mesh::SurfaceDefinition::ElementFaces(vec![(2, 6)]),
        );
        let mut model = model(&mesh);
        assert_eq!(
            problems(&check(&model, &mesh)),
            [
                (ModelItem::BoundaryConditions(0), Problem::RigidBodyMotion),
                (ModelItem::Part(1), Problem::RigidBodyMotion),
            ]
        );
        let mut pair = ContactPair::new("Contact_Pair-1", "Surface_Interaction-1");
        pair.master = Region::Surface("RIGHT".into());
        pair.slave = Region::Surface("LEFT".into());
        model.contact_pairs.push(pair);
        model.surface_interactions.push(SurfaceInteraction {
            name: "Surface_Interaction-1".into(),
            properties: Vec::new(),
        });
        assert_eq!(
            problems(&check(&model, &mesh))[1],
            (ModelItem::Part(1), Problem::HeldOnlyByContact)
        );
        let mut tie = Tie::new("Tie-1");
        tie.master = Region::Surface("RIGHT".into());
        tie.slave = Region::Surface("LEFT".into());
        model.ties.push(tie);
        assert_eq!(check(&model, &mesh), []);
    }

    #[test]
    fn frequency_steps_may_be_free_but_need_a_density() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        model.steps = vec![Step::new_frequency("Step-1")];
        assert_eq!(check(&model, &mesh), []);
        model.materials[0].density = None;
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::Material(0), Problem::NoDensity)]
        );
    }

    #[test]
    fn body_loads_need_a_density() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        model.materials[0].density = None;
        assert_eq!(check(&model, &mesh), []);
        model.steps[0].loads.push(Load {
            name: "Gravity-1".into(),
            active: true,
            region: Region::Parts(mesh.parts.iter().map(|p| p.name.clone()).collect()),
            kind: LoadKind::Gravity([0.0, 0.0, -9.81]),
            amplitude: None,
            factor_amplitude: None,
        });
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::Material(0), Problem::NoDensity)]
        );
    }

    #[test]
    fn materials_and_sections_are_checked() {
        let mesh = cubes(2, true);
        let mut model = model(&mesh);
        model.materials[0].elastic = None;
        model.sections[0].region = Region::Parts(vec!["PART-1".into()]);
        assert_eq!(
            problems(&check(&model, &mesh)),
            [
                (ModelItem::Material(0), Problem::NoElastic),
                (ModelItem::Part(1), Problem::NoSection),
            ]
        );
        model.materials[0].elastic = Some(Elastic {
            young: 1.0,
            poisson: 0.5,
        });
        model.sections[0].region = Region::ElementSet("ALL".into());
        let mut mesh = mesh;
        mesh.element_sets.insert("ALL".into(), vec![1, 2]);
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::Material(0), Problem::InvalidElastic)]
        );
    }

    #[test]
    fn a_reference_point_needs_an_active_rigid_body_and_holds_it() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        model.reference_points.push(ReferencePoint {
            name: "RP-1".into(),
            position: [0.0, 0.5, 0.5],
        });
        // The support moves from the nodes to the reference point: without a rigid body
        // the point has no node, the cube is loose.
        model.steps[0].boundary_conditions[0].region = Region::ReferencePoint("RP-1".into());
        let found = problems(&check(&model, &mesh));
        assert!(
            found.contains(&(ModelItem::BoundaryCondition(0, 0), Problem::NoRigidBody)),
            "{found:?}"
        );
        assert!(
            found.contains(&(ModelItem::Part(0), Problem::RigidBodyMotion)),
            "{found:?}"
        );
        let mut body = RigidBody::new("Rigid_Body-1", "RP-1");
        body.region = Region::Nodes(vec![1, 4, 5, 8]);
        model.constraints.push(Constraint::RigidBody(body));
        assert_eq!(problems(&check(&model, &mesh)), []);
        // An inactive body does not count.
        *model.constraints[0].active_mut() = false;
        let found = problems(&check(&model, &mesh));
        assert!(
            found.contains(&(ModelItem::BoundaryCondition(0, 0), Problem::NoRigidBody)),
            "{found:?}"
        );
        *model.constraints[0].active_mut() = true;
        // Only the translations held: the cube can still turn about the point.
        model.steps[0].boundary_conditions[0].kind =
            BoundaryKind::Displacement([Some(0.0), Some(0.0), Some(0.0), None, None, None]);
        let found = problems(&check(&model, &mesh));
        assert!(
            found.contains(&(ModelItem::Part(0), Problem::RigidBodyMotion)),
            "{found:?}"
        );
        // A moment on the point without the body is flagged too.
        model.steps[0].boundary_conditions[0].kind = BoundaryKind::Fixed;
        model.steps[0].loads[0].region = Region::ReferencePoint("RP-1".into());
        model.steps[0].loads[0].kind = LoadKind::Moment([1.0, 0.0, 0.0]);
        assert_eq!(problems(&check(&model, &mesh)), []);
        model.constraints.clear();
        let found = problems(&check(&model, &mesh));
        assert!(
            found.contains(&(ModelItem::Load(0, 0), Problem::NoRigidBody)),
            "{found:?}"
        );
    }

    #[test]
    fn a_plastic_material_needs_elasticity_and_a_growing_hardening_curve() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        let plastic = |rows: &[(f64, f64, f64)]| {
            Some(Plastic {
                hardening: Hardening::Isotropic,
                points: rows
                    .iter()
                    .map(|&(stress, plastic_strain, temperature)| PlasticPoint {
                        stress,
                        plastic_strain,
                        temperature,
                    })
                    .collect(),
            })
        };
        model.materials[0].plastic = plastic(&[(235.0, 0.0, 0.0), (400.0, 0.2, 0.0)]);
        assert_eq!(problems(&check(&model, &mesh)), []);
        // Rows at a second temperature start at plastic strain 0 again.
        model.materials[0].plastic = plastic(&[
            (235.0, 0.0, 20.0),
            (400.0, 0.2, 20.0),
            (200.0, 0.0, 300.0),
            (300.0, 0.2, 300.0),
        ]);
        assert_eq!(problems(&check(&model, &mesh)), []);
        for rows in [
            &[(235.0, 0.1, 0.0)][..],
            &[(235.0, 0.0, 0.0), (400.0, 0.0, 0.0)],
            &[(0.0, 0.0, 0.0)],
            &[],
        ] {
            model.materials[0].plastic = plastic(rows);
            assert_eq!(
                problems(&check(&model, &mesh)),
                [(ModelItem::Material(0), Problem::InvalidPlastic)],
                "{rows:?}"
            );
        }
        model.materials[0].plastic = plastic(&[(235.0, 0.0, 0.0)]);
        model.materials[0].elastic = None;
        assert_eq!(
            problems(&check(&model, &mesh)),
            [
                (ModelItem::Material(0), Problem::NoElastic),
                (ModelItem::Material(0), Problem::InvalidPlastic),
            ]
        );
    }

    #[test]
    fn overlapping_boundary_conditions_and_held_loads_are_found() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        let step = &mut model.steps[0];
        step.boundary_conditions.push(BoundaryCondition {
            name: "Displacement-1".into(),
            active: true,
            region: Region::Nodes(vec![1, 2]),
            kind: BoundaryKind::Displacement([Some(0.1), None, None, None, None, None]),
            amplitude: None,
        });
        step.loads[0].region = Region::Nodes(vec![4]);
        let findings = check(&model, &mesh);
        assert_eq!(
            problems(&findings),
            [
                (
                    ModelItem::BoundaryCondition(0, 1),
                    Problem::ConflictingBoundaries
                ),
                (ModelItem::Load(0, 0), Problem::LoadOnFixedNodes),
            ]
        );
        assert!(
            findings[0]
                .detail
                .starts_with("1 degrees of freedom also in Fixed-1")
        );
        // The same value is no conflict.
        model.steps[0].boundary_conditions[1].kind =
            BoundaryKind::Displacement([Some(0.0), None, None, None, None, None]);
        model.steps[0].loads.clear();
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::Step(0), Problem::NoLoad)]
        );
    }

    #[test]
    fn a_defined_temperature_is_a_load_and_needs_thermal_expansion() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        let parts: Vec<String> = mesh.parts.iter().map(|p| p.name.clone()).collect();
        model.steps[0].loads.clear();
        model.initial_conditions.push(crate::InitialCondition {
            name: "Initial_Temperature-1".into(),
            active: true,
            region: Region::Parts(parts.clone()),
            kind: crate::InitialConditionKind::Temperature(20.0),
        });
        model.steps[0].defined_fields.push(crate::DefinedField {
            name: "Defined_Temperature-1".into(),
            active: true,
            region: Region::Parts(parts),
            kind: DefinedFieldKind::Temperature(100.0),
            amplitude: None,
        });
        let findings = check(&model, &mesh);
        assert_eq!(
            problems(&findings),
            [(ModelItem::DefinedField(0, 0), Problem::NoExpansion)]
        );
        assert_eq!(
            findings[0].detail,
            "Defined_Temperature-1: no material with thermal expansion"
        );
        model.materials[0].expansion = Some(crate::Expansion::default());
        assert_eq!(check(&model, &mesh), []);
        model.initial_conditions[0].active = false;
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::Step(0), Problem::NoInitialTemperature)]
        );
        model.initial_conditions[0].active = true;
        // A thermal step solves for the temperatures and takes no defined field.
        model.steps[0].kind = StepKind::CoupledTempDisp(Default::default());
        model.materials[0].conductivity = Some(50.0);
        model.steps[0].defined_fields[0].region = Region::Parts(vec!["Nowhere".into()]);
        assert!(
            check(&model, &mesh)
                .iter()
                .all(|f| f.problem != Problem::NoExpansion)
        );
    }

    #[test]
    fn rotations_on_solids_are_ignored() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        model.steps[0].boundary_conditions[0].kind =
            BoundaryKind::Displacement([Some(0.0), Some(0.0), Some(0.0), Some(0.0), None, None]);
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(
                ModelItem::BoundaryCondition(0, 0),
                Problem::RotationsIgnored
            )]
        );
    }

    #[test]
    fn distorted_elements_are_attached_to_their_part() {
        let mut mesh = cubes(2, true);
        // Node 7 pushed through the cube, like a node moved by hand.
        mesh.set_node(7, [-1.0, -1.0, -1.0]);
        let model = model(&mesh);
        let findings = check(&model, &mesh);
        assert_eq!(
            problems(&findings),
            [
                (ModelItem::Part(0), Problem::DistortedElements),
                (ModelItem::Part(1), Problem::DistortedElements),
            ]
        );
        assert!(findings[0].detail.ends_with("e.g. 1"));
    }

    #[test]
    fn plane_models_rotate_about_z_only() {
        let mut mesh = FeMesh::default();
        for (id, c) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
            .into_iter()
            .enumerate()
        {
            mesh.set_node(id as u32 + 1, [c[0], c[1], 0.0]);
        }
        mesh.add_element(Element {
            id: 1,
            type_name: "CPS4".into(),
            shape: ElementShape::Quad4,
            nodes: vec![1, 2, 3, 4],
        })
        .unwrap();
        mesh.parts.push(Part {
            name: "PART-1".into(),
            elements: vec![1],
        });
        let mut model = model(&mesh);
        model.properties.space = ModelSpace::PlaneStress;
        model.steps[0].boundary_conditions[0].region = Region::Nodes(vec![1]);
        model.steps[0].loads[0].region = Region::Nodes(vec![3]);
        model.steps[0].loads[0].kind = LoadKind::ConcentratedForce([1.0, 0.0, 0.0]);
        let findings = check(&model, &mesh);
        assert_eq!(findings[0].problem, Problem::RigidBodyMotion);
        assert!(
            findings[0].detail.ends_with(": rotation about Z free"),
            "{findings:?}"
        );
        // Axisymmetric: only the axial motion is a rigid body motion.
        model.properties.space = ModelSpace::Axisymmetric;
        model.steps[0].boundary_conditions[0].kind =
            BoundaryKind::Displacement([None, Some(0.0), None, None, None, None]);
        assert_eq!(check(&model, &mesh), []);
    }

    #[test]
    fn a_beam_fixed_at_one_end_is_held_by_its_rotations() {
        let line = |type_name: &str| {
            let mut mesh = FeMesh::default();
            for (id, x) in [0.0, 1.0, 2.0].into_iter().enumerate() {
                mesh.set_node(id as u32 + 1, [x, 0.0, 0.0]);
            }
            for (id, nodes) in [(1, vec![1, 2]), (2, vec![2, 3])] {
                mesh.add_element(Element {
                    id,
                    type_name: type_name.into(),
                    shape: ElementShape::Line2,
                    nodes,
                })
                .unwrap();
            }
            mesh.parts.push(Part {
                name: "BEAM".into(),
                elements: vec![1, 2],
            });
            mesh
        };
        let mesh = line("B31");
        let mut model = model(&mesh);
        model.sections[0].kind = crate::SectionKind::Beam(crate::BeamSection::DEFAULT);
        model.steps[0].boundary_conditions[0].region = Region::Nodes(vec![1]);
        model.steps[0].loads[0].region = Region::Nodes(vec![3]);
        assert_eq!(check(&model, &mesh), []);
        // Trusses have no rotations: the same support leaves them free to turn.
        let trusses = line("T3D2");
        model.sections[0].kind = crate::SectionKind::Truss { area: 1.0 };
        assert_eq!(check(&model, &trusses)[0].problem, Problem::RigidBodyMotion);
        // Meshed lines are B31 or B32 whatever their section; a truss section makes them
        // trusses all the same.
        assert_eq!(check(&model, &mesh)[0].problem, Problem::RigidBodyMotion);
    }

    /// A plane truss triangle in the XZ plane, as meshed lines (`B31`) with a truss section:
    /// nodes 1 (0, 0, 0), 2 (2, 0, 0) and 3 (1, 0, 1), the bottom bar split into
    /// `bottom` elements, held statically determinate in the plane and at every node in Y.
    /// With `tied` the left bar ends in node 9 at node 3 instead, joined by a node tie.
    fn truss_triangle(bottom: u32, tied: bool) -> (FeMesh, FeModel) {
        let mut mesh = FeMesh::default();
        mesh.set_node(1, [0.0, 0.0, 0.0]);
        mesh.set_node(2, [2.0, 0.0, 0.0]);
        mesh.set_node(3, [1.0, 0.0, 1.0]);
        let mut bottom_nodes = vec![1];
        for i in 1..bottom {
            mesh.set_node(3 + i, [2.0 * f64::from(i) / f64::from(bottom), 0.0, 0.0]);
            bottom_nodes.push(3 + i);
        }
        bottom_nodes.push(2);
        let mut bars: Vec<Vec<Vec<NodeId>>> =
            vec![(bottom_nodes.windows(2)).map(<[NodeId]>::to_vec).collect()];
        bars.push(vec![vec![2, 3]]);
        let top = if tied { 9 } else { 3 };
        if tied {
            mesh.set_node(9, [1.0, 0.0, 1.0]);
        }
        bars.push(vec![vec![top, 1]]);
        let mut id = 1;
        for (b, elements) in bars.into_iter().enumerate() {
            let mut part = Vec::new();
            for nodes in elements {
                mesh.add_element(Element {
                    id,
                    type_name: "B31".into(),
                    shape: ElementShape::Line2,
                    nodes,
                })
                .unwrap();
                part.push(id);
                id += 1;
            }
            mesh.parts.push(Part {
                name: format!("LINE-{}", b + 1),
                elements: part,
            });
        }
        let mut model = model(&mesh);
        model.sections[0].kind = crate::SectionKind::Truss { area: 1.0 };
        let bcs = &mut model.steps[0].boundary_conditions;
        bcs[0].region = Region::Nodes(vec![1]);
        bcs[0].kind = BoundaryKind::Displacement([Some(0.0), None, Some(0.0), None, None, None]);
        bcs.push(BoundaryCondition {
            name: "Roller".into(),
            active: true,
            region: Region::Nodes(vec![2]),
            kind: BoundaryKind::Displacement([None, None, Some(0.0), None, None, None]),
            amplitude: None,
        });
        bcs.push(BoundaryCondition {
            name: "Plane".into(),
            active: true,
            region: Region::Nodes(mesh.node_ids().to_vec()),
            kind: BoundaryKind::Displacement([None, Some(0.0), None, None, None, None]),
            amplitude: None,
        });
        model.steps[0].loads[0].region = Region::Nodes(vec![3]);
        if tied {
            model.node_ties.push(crate::NodeTie {
                name: "Node_Tie-1".into(),
                active: true,
                region: Region::Nodes(vec![3, 9]),
                rotations: true,
            });
        }
        (mesh, model)
    }

    #[test]
    fn truss_nodes_need_bars_or_supports_in_every_direction() {
        let (mesh, model) = truss_triangle(1, false);
        assert_eq!(check(&model, &mesh), []);
        // A bar of two elements buckles at its inner node without resistance.
        let (mesh, model) = truss_triangle(2, false);
        let findings = check(&model, &mesh);
        assert_eq!(
            problems(&findings),
            [
                (ModelItem::BoundaryConditions(0), Problem::Mechanism),
                (ModelItem::Part(0), Problem::Mechanism),
            ]
        );
        assert_eq!(
            findings[0].detail,
            "Step-1: 1 truss nodes not held, e.g. node 4 free in Z"
        );
        // Without the supports in Y the plane truss moves out of its plane.
        let (mesh, mut model) = truss_triangle(1, false);
        model.steps[0].boundary_conditions[2].region = Region::Nodes(vec![1, 2]);
        let findings = check(&model, &mesh);
        assert!(findings.iter().any(|f| f.problem == Problem::Mechanism
            && f.detail == "Step-1: 1 truss nodes not held, e.g. node 3 free in Y"));
        // Tied nodes count as one node, as they are written.
        let (mesh, model) = truss_triangle(1, true);
        assert_eq!(check(&model, &mesh), []);
    }

    #[test]
    fn heat_transfer_needs_thermal_constants() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        let mut step = Step::new_heat_transfer("Step-1");
        step.boundary_conditions.push(BoundaryCondition {
            name: "Temperature-1".into(),
            active: true,
            region: Region::Nodes(vec![1]),
            kind: BoundaryKind::Temperature(20.0),
            amplitude: None,
        });
        step.loads.push(Load {
            name: "Flux-1".into(),
            active: true,
            region: Region::Nodes(vec![7]),
            kind: LoadKind::ConcentratedFlux(10.0),
            amplitude: None,
            factor_amplitude: None,
        });
        model.steps = vec![step];
        // Displacements are free in a heat transfer, and no elasticity is needed.
        model.materials[0].elastic = None;
        assert_eq!(
            problems(&check(&model, &mesh)),
            [(ModelItem::Material(0), Problem::NoConductivity)]
        );
        model.materials[0].conductivity = Some(50.0);
        assert_eq!(check(&model, &mesh), []);
        if let StepKind::HeatTransfer(settings) = &mut model.steps[0].kind {
            settings.steady_state = false;
        }
        assert_eq!(
            problems(&check(&model, &mesh)),
            [
                (ModelItem::Material(0), Problem::NoSpecificHeat),
                (ModelItem::Step(0), Problem::NoInitialTemperature),
            ]
        );
    }

    #[test]
    fn solver_errors_are_explained() {
        let mesh = cubes(2, true);
        let lines: Vec<String> = [
            " *ERROR in e_c3d: nonpositive jacobian",
            "        determinant in element           2",
            "",
            " *ERROR: too many cutbacks",
        ]
        .map(String::from)
        .to_vec();
        let findings = diagnose_solver_output(&lines, &mesh);
        assert_eq!(
            problems(&findings),
            [
                (ModelItem::Analysis, Problem::DistortedElements),
                (ModelItem::Analysis, Problem::NoConvergence),
                (ModelItem::Part(1), Problem::DistortedElements),
            ]
        );
        let unknown = [" *ERROR in something: new message".to_string()];
        assert_eq!(
            diagnose_solver_output(&unknown, &mesh)[0].problem,
            Problem::SolverError
        );
    }
}
