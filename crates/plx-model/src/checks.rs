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

use crate::{BoundaryKind, Constraint, FeModel, LoadKind, ModelItem, ModelSpace, StepKind};

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
    NoConductivity,
    NoSpecificHeat,
    NoInitialTemperature,
    InvalidElastic,
    DistortedElements,
    RigidBodyMotion,
    HeldOnlyByContact,
    ConflictingBoundaries,
    LoadOnFixedNodes,
    RotationsIgnored,
    IncrementExceedsStep,
    NoLoad,
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
            | Problem::NoLoad => Severity::Warning,
            _ => Severity::Error,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Problem::MissingReference => "Fehlende Referenz",
            Problem::NoSection => "Elemente ohne Material",
            Problem::NoElastic => "Material ohne Elastizität",
            Problem::NoDensity => "Material ohne Dichte",
            Problem::NoConductivity => "Material ohne Wärmeleitfähigkeit",
            Problem::NoSpecificHeat => "Material ohne Wärmekapazität",
            Problem::NoInitialTemperature => "Keine Anfangstemperatur",
            Problem::InvalidElastic => "Ungültige Materialkonstanten",
            Problem::DistortedElements => "Verzerrte Elemente",
            Problem::RigidBodyMotion => "Starrkörperbewegung möglich",
            Problem::HeldOnlyByContact => "Nur über Kontakt gehalten",
            Problem::ConflictingBoundaries => "Widersprüchliche Randbedingungen",
            Problem::LoadOnFixedNodes => "Last auf festgehaltenen Knoten",
            Problem::RotationsIgnored => "Rotationen ohne Wirkung",
            Problem::IncrementExceedsStep => "Inkrement größer als der Step",
            Problem::NoLoad => "Keine Last",
            Problem::NoConvergence => "Keine Konvergenz",
            Problem::MpcAndSpc => "Freiheitsgrad doppelt gebunden",
            Problem::RotationIn2d => "Rotation in einem 2D-Modell",
            Problem::SolverError => "Fehler von CalculiX",
        }
    }

    /// What goes wrong and why CalculiX cannot cope with it.
    pub fn explanation(self) -> &'static str {
        match self {
            Problem::MissingReference => {
                "Das Element verweist auf etwas, das es nicht mehr gibt, zum Beispiel ein \
                 gelöschtes Material oder ein Set, das das Netz nicht mehr hat. Die \
                 Eingabedatei kann so nicht geschrieben werden."
            }
            Problem::NoSection => {
                "CalculiX braucht für jedes Element ein Material. Elemente, die keine Section \
                 erfasst, bringen den Lauf mit \"no material was assigned to element\" zum \
                 Abbruch."
            }
            Problem::NoElastic => {
                "Eine Section verwendet dieses Material, es hat aber keinen \
                 Elastizitätsmodul und keine Querkontraktionszahl. Ohne sie gibt es keine \
                 Steifigkeit; CalculiX bricht mit \"no elastic constants were assigned\" ab."
            }
            Problem::NoDensity => {
                "Ein Frequency Step und eine instationäre Wärmeübertragung brauchen die \
                 Masse des Modells. Ohne Dichte bricht CalculiX mit \"no density was \
                 assigned\" ab."
            }
            Problem::NoConductivity => {
                "Eine Wärmeübertragung braucht die Wärmeleitfähigkeit jedes Materials. \
                 CalculiX warnt nur (\"no conductivity constants were assigned\") und \
                 stürzt danach beim Lösen ab."
            }
            Problem::NoSpecificHeat => {
                "Eine instationäre Wärmeübertragung braucht die spezifische \
                 Wärmekapazität. CalculiX bricht mit \"no specific heat was assigned\" ab."
            }
            Problem::NoInitialTemperature => {
                "Eine instationäre Wärmeübertragung startet von einer Anfangstemperatur. \
                 Ohne sie bricht CalculiX mit \"please define initial conditions for the \
                 temperature\" ab."
            }
            Problem::InvalidElastic => {
                "Der Elastizitätsmodul muss größer als 0 sein und die Querkontraktionszahl \
                 zwischen -1 und 0,5 liegen (0,5 wäre ein inkompressibles Material, das ein \
                 linear-elastisches Modell nicht abbilden kann). CalculiX lehnt das Material \
                 beim Einlesen ab."
            }
            Problem::DistortedElements => {
                "Diese Elemente sind umgestülpt oder so stark verzerrt, dass ihre \
                 Jacobi-Determinante an einem Integrationspunkt nicht positiv ist. CalculiX \
                 bricht mit \"nonpositive jacobian determinant in element\" ab. Typische \
                 Ursachen sind quadratische Tetraeder, deren Mittelknoten auf eine stark \
                 gekrümmte Fläche verschoben wurden, sehr dünne Bereiche, oder 2D-Elemente, \
                 deren Knoten im Uhrzeigersinn nummeriert sind."
            }
            Problem::RigidBodyMotion => {
                "Diese Teile können sich als starrer Körper bewegen: Die Randbedingungen \
                 und Federn halten nicht alle Verschiebungen und Drehungen fest, und keine \
                 Tie verbindet sie mit gehaltenen Teilen. Die Steifigkeitsmatrix ist dann \
                 singulär. Je nach Gleichungslöser bricht CalculiX mit \"zero pivot\" ab \
                 oder liefert riesige, unbrauchbare Verschiebungen ohne Fehlermeldung."
            }
            Problem::HeldOnlyByContact => {
                "Diese Teile werden nur über Kontaktpaare gehalten. Zu Beginn der Rechnung \
                 trägt ein Kontakt erst, wenn er geschlossen ist; bis dahin können die Teile \
                 frei wegdriften. Das führt oft zu \"zero pivot\", zu \"too many cutbacks\" \
                 oder zu sehr langen Rechnungen."
            }
            Problem::ConflictingBoundaries => {
                "Dieselben Knoten werden in diesem Step von mehreren Randbedingungen mit \
                 verschiedenen Werten festgelegt, etwa eine Fläche fest eingespannt und \
                 eine angrenzende Kante mit Verschiebung. CalculiX meldet das nicht, sondern \
                 verwendet den zuletzt geschriebenen Wert; das Ergebnis hängt dann von der \
                 Reihenfolge im Baum ab."
            }
            Problem::LoadOnFixedNodes => {
                "Alle Knoten dieser Last sind in Lastrichtung festgehalten. Die Last geht \
                 direkt in die Lagerreaktion und verformt das Modell nicht."
            }
            Problem::RotationsIgnored => {
                "Drehungen (UR1 bis UR3) haben nur Balken- und Schalenknoten. Volumen- und \
                 2D-Elemente haben nur Verschiebungen; vorgegebene Drehungen werden \
                 ignoriert oder führen in 2D-Modellen zum Abbruch (\"mpc of type is \
                 unknown\")."
            }
            Problem::IncrementExceedsStep => {
                "Das Anfangsinkrement ist größer als die Dauer des Steps. CalculiX lehnt \
                 den Step mit \"initial increment size exceeds step size\" ab."
            }
            Problem::NoLoad => {
                "Der Step hat weder eine aktive Last noch eine vorgegebene Verschiebung. \
                 Die Rechnung läuft, alle Ergebnisse sind aber null."
            }
            Problem::NoConvergence => {
                "Die Newton-Iteration ist nicht konvergiert; CalculiX hat das Inkrement \
                 immer weiter verkleinert und aufgegeben (\"too many cutbacks\" oder \
                 \"increment size smaller than minimum\"). Meist liegt es an Kontakten, an \
                 schlecht gehaltenen Teilen oder an zu großen Lasten in einer nichtlinearen \
                 Rechnung. Ein noch kleineres Mindestinkrement hilft selten."
            }
            Problem::MpcAndSpc => {
                "Ein Freiheitsgrad ist zugleich durch eine Randbedingung festgehalten und \
                 als abhängiger Freiheitsgrad einer Gleichung (MPC) gebunden, etwa durch \
                 eigene *EQUATION-Keywords. CalculiX bricht dann ab."
            }
            Problem::RotationIn2d => {
                "In einem 2D-Modell wurden Drehungen (Freiheitsgrade 4 bis 6) festgehalten. \
                 CalculiX erweitert 2D-Elemente intern zu Volumenelementen und kann dafür \
                 keine Drehungen vorgeben; der Lauf bricht mit \"mpc of type is unknown\" \
                 oder einem Gleitkommafehler ab."
            }
            Problem::SolverError => {
                "CalculiX hat mit einer Fehlermeldung abgebrochen, die prepolix nicht \
                 kennt. Der genaue Text steht im Monitor."
            }
        }
    }

    /// How to fix it.
    pub fn fix(self) -> &'static str {
        match self {
            Problem::MissingReference => {
                "Das Element bearbeiten und eine vorhandene Auswahl oder ein vorhandenes \
                 Material wählen, oder es löschen."
            }
            Problem::NoSection => {
                "Unter Sections eine Section für den Part erstellen oder eine bestehende um \
                 ihn erweitern."
            }
            Problem::NoElastic => {
                "Das Material bearbeiten und Elastizitätsmodul und Querkontraktionszahl \
                 eintragen, oder ein Material aus der Materialbibliothek nehmen."
            }
            Problem::NoDensity => "Das Material bearbeiten und eine Dichte eintragen.",
            Problem::NoConductivity => {
                "Das Material bearbeiten und eine Wärmeleitfähigkeit eintragen."
            }
            Problem::NoSpecificHeat => {
                "Das Material bearbeiten und eine spezifische Wärmekapazität eintragen, \
                 oder den Step stationär rechnen."
            }
            Problem::NoInitialTemperature => {
                "Unter Initial Conditions eine Anfangstemperatur für das Modell erstellen, \
                 oder den Step stationär rechnen."
            }
            Problem::InvalidElastic => {
                "Elastizitätsmodul größer als 0 und Querkontraktionszahl kleiner als 0,5 \
                 eintragen (Stahl: 210000 MPa, 0,3)."
            }
            Problem::DistortedElements => {
                "Den Part neu vernetzen: kleinere Elementgröße an engen Radien oder dünnen \
                 Wänden, oder lineare statt quadratischer Elemente. Bei importierten Netzen \
                 die Knotenreihenfolge prüfen."
            }
            Problem::RigidBodyMotion => {
                "Eine Randbedingung ergänzen, die die genannten Bewegungen sperrt, die Teile \
                 mit einer Tie an gehaltene Teile binden, oder schwache Federn (Constraints \
                 > Point Spring) gegen Masse setzen."
            }
            Problem::HeldOnlyByContact => {
                "Die Teile zusätzlich schwach lagern (Point Spring mit kleiner Steifigkeit), \
                 die Kontaktflächen zu Beginn berühren lassen (Adjust im Kontaktpaar) oder \
                 die Teile in einem ersten Step über eine kleine Verschiebung in Kontakt \
                 bringen."
            }
            Problem::ConflictingBoundaries => {
                "Die Auswahl der Randbedingungen so ändern, dass sich keine Knoten \
                 überschneiden, oder beiden Randbedingungen denselben Wert geben."
            }
            Problem::LoadOnFixedNodes => {
                "Die Last auf eine Fläche oder Knoten legen, die sich bewegen können, oder \
                 die Randbedingung dort entfernen."
            }
            Problem::RotationsIgnored => {
                "UR1 bis UR3 in der Randbedingung frei lassen. Drehungen von Volumenkörpern \
                 über Verschiebungen mehrerer Knoten vorgeben."
            }
            Problem::IncrementExceedsStep => {
                "Im Step das Anfangsinkrement höchstens so groß wie die Step-Dauer wählen."
            }
            Problem::NoLoad => "Unter Loads eine Last erstellen oder eine deaktivierte aktivieren.",
            Problem::NoConvergence => {
                "Kontakte prüfen (Steifigkeit der Surface Interaction, Adjust, Lage der \
                 Flächen), Teile ausreichend lagern, die Last auf mehrere Inkremente \
                 verteilen (Inkrementierung Automatisch mit kleinerem Anfangsinkrement) und \
                 das Netz an den Kontaktflächen verfeinern."
            }
            Problem::MpcAndSpc => {
                "Die Randbedingung von den abhängigen Knoten der Gleichung entfernen oder \
                 die Gleichung umformulieren."
            }
            Problem::RotationIn2d => {
                "In 2D-Modellen nur U1 und U2 vorgeben; eigene *BOUNDARY-Keywords im \
                 Keyword-Editor prüfen."
            }
            Problem::SolverError => {
                "Die Meldung im Monitor lesen; oft hilft \"Modell prüfen\" im Kontextmenü der \
                 Analyse."
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
    /// Whether the mesh has beams or shells, the only elements with rotations.
    has_rotations: bool,
    /// Whether each node, by node index, belongs to a beam or shell and so has rotations.
    rotational: Vec<bool>,
}

impl MeshCheck {
    pub fn new(mesh: &FeMesh, space: ModelSpace) -> Self {
        let mut union = UnionFind::new(mesh.node_count());
        let mut used = vec![false; mesh.node_count()];
        let mut rotational = vec![false; mesh.node_count()];
        for element in mesh.elements() {
            // Beams (B31, B32) and shells (S3 to S8R) have rotations; trusses, plane and
            // solid elements only displacements.
            let rotations = match element.shape.family() {
                ElementFamily::Line => element.type_name.to_ascii_uppercase().starts_with('B'),
                ElementFamily::Surface => {
                    !element.is_plane() && element.type_name.to_ascii_uppercase().starts_with('S')
                }
                ElementFamily::Solid => false,
            };
            let mut first = None;
            for index in element.nodes.iter().filter_map(|&n| mesh.node_index(n)) {
                used[index] = true;
                rotational[index] |= rotations;
                match first {
                    None => first = Some(index),
                    Some(first) => union.join(first, index),
                }
            }
        }
        let has_rotations = rotational.iter().any(|&r| r);
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
            has_rotations,
            rotational,
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
                    "{} Element(e) mit nicht positiver Jacobi-Determinante, z. B. {}",
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
        let initial_temperature = self.initial_conditions.iter().any(|c| c.active);
        for (s, step) in self.steps.iter().enumerate().filter(|(_, s)| s.active) {
            self.check_step(s, mesh, mesh_check, &mut findings);
            if let StepKind::HeatTransfer(h) | StepKind::CoupledTempDisp(h) = &step.kind
                && !h.steady_state
                && !initial_temperature
            {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoInitialTemperature,
                    format!("{} ist instationär", step.name),
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
                        "Anfangsinkrement {} > Step-Dauer {}",
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
        let frequency = active().any(|s| matches!(s.kind, StepKind::Frequency(_)));
        for (i, material) in self.materials.iter().enumerate() {
            if !self.sections.iter().any(|s| s.material == material.name) {
                continue;
            }
            let item = ModelItem::Material(i);
            match material.elastic {
                None if mechanical => findings.push(Finding::new(
                    item,
                    Problem::NoElastic,
                    format!("{} hat keine Elastizität", material.name),
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
            if thermal && material.conductivity.is_none() {
                findings.push(Finding::new(
                    item,
                    Problem::NoConductivity,
                    format!("{} hat keine Wärmeleitfähigkeit", material.name),
                ));
            }
            if transient && material.specific_heat.is_none() {
                findings.push(Finding::new(
                    item,
                    Problem::NoSpecificHeat,
                    format!("{} hat keine spezifische Wärmekapazität", material.name),
                ));
            }
            if (frequency || transient) && material.density.is_none_or(|d| d <= 0.0) {
                findings.push(Finding::new(
                    item,
                    Problem::NoDensity,
                    format!("{} hat keine Dichte", material.name),
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
                        "{missing} von {} Elementen von {} ohne Section",
                        part.elements.len(),
                        part.name
                    ),
                ));
            }
        }
    }

    fn check_step(&self, s: usize, mesh: &FeMesh, check: &MeshCheck, findings: &mut Vec<Finding>) {
        let step = &self.steps[s];
        let space = self.properties.space;
        let dofs = if space.is_2d() { 2 } else { 6 };
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
            };
            if let BoundaryKind::Displacement(values) = bc.kind
                && values[3..].iter().any(Option::is_some)
                && (space.is_2d() || !check.has_rotations)
            {
                findings.push(Finding::new(
                    ModelItem::BoundaryCondition(s, i),
                    Problem::RotationsIgnored,
                    format!("{} gibt Drehungen vor", bc.name),
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
                        "{count} Freiheitsgrade auch in {} mit anderem Wert; es gilt {}",
                        step.boundary_conditions[other].name, bc.name
                    ),
                ));
            }
        }
        let user_keywords = self.user_keywords.iter().any(|k| k.active);
        if step.kind.supports_loads() {
            let mut loaded = false;
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
                    LoadKind::Pressure(_) => (0..dofs.min(3)).collect(),
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
                        format!("{}: alle {} Knoten festgehalten", load.name, nodes.len()),
                    ));
                }
            }
            let displaced = fixed.values().any(|&(_, v)| v != 0.0);
            if !loaded && !displaced && !user_keywords {
                findings.push(Finding::new(
                    ModelItem::Step(s),
                    Problem::NoLoad,
                    format!("{} hat keine aktive Last", step.name),
                ));
            }
        }
        // Free-free eigenfrequencies are fine; a static step needs every part held. Own
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
        let static_mechanical = matches!(
            step.kind,
            StepKind::Static(_) | StepKind::CoupledTempDisp(_)
        );
        if static_mechanical && !constrained_by_keywords {
            self.check_rigid_body(s, mesh, check, &fixed, findings);
        }
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
                && (mesh.node_index(node)).is_some_and(|i| check.rotational[i])
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
            }
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
                "Elemente ohne Part".to_string()
            } else if names.len() > 5 {
                format!("{} und {} weitere", names[..5].join(", "), names.len() - 5)
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
}

/// Whether two prescribed values are the same up to rounding.
fn same(a: f64, b: f64) -> bool {
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

/// Names the free motions, e.g. "Verschiebung in X, Drehung um Z".
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
        return "ganz frei, nichts hält sie".into();
    }
    let mut parts = Vec::new();
    let mut found = 0;
    for &(i, name) in translations {
        if project(&unit(i)) > 0.99 {
            parts.push(format!("Verschiebung in {name}"));
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
        parts.push(format!("Drehung um {axis}"));
    }
    if parts.is_empty() {
        parts.push(format!("{} Bewegung(en)", free.len()));
    }
    format!("{} frei", parts.join(", "))
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
                "{} Element(e), z. B. {}",
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
                    format!("Laut CalculiX: Element(e) {}", list(in_part.iter().take(5))),
                ));
            }
        }
    }
    let known: [(Problem, &[&str]); 11] = [
        (Problem::NoElastic, &["no elastic constants"]),
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
                format!("CalculiX meldet \"{marker}\""),
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
        BoundaryCondition, ContactPair, Elastic, Load, Material, Region, Section, Step,
        SurfaceInteraction, Tie,
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
        });
        step.loads.push(Load {
            name: "Force-1".into(),
            active: true,
            region: Region::Nodes(vec![7]),
            kind: LoadKind::ConcentratedForce([0.0, 0.0, -1.0]),
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
        assert!(findings[0].detail.contains("Drehung um"), "{findings:?}");
        assert!(!findings[0].detail.contains("Verschiebung"), "{findings:?}");
        model.steps[0].boundary_conditions.clear();
        let detail = &check(&model, &mesh)[0].detail;
        assert!(detail.ends_with(": ganz frei, nichts hält sie"), "{detail}");
        // Two corners fixed leave the rotation about the line through them.
        model.steps[0].boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: Region::Nodes(vec![1, 2]),
            kind: BoundaryKind::Fixed,
        });
        let detail = &check(&model, &mesh)[0].detail;
        assert!(detail.ends_with(": Drehung um X frei"), "{detail}");
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
        model.constraints.push(Constraint::Tie(tie));
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
    fn overlapping_boundary_conditions_and_held_loads_are_found() {
        let mesh = cubes(1, false);
        let mut model = model(&mesh);
        let step = &mut model.steps[0];
        step.boundary_conditions.push(BoundaryCondition {
            name: "Displacement-1".into(),
            active: true,
            region: Region::Nodes(vec![1, 2]),
            kind: BoundaryKind::Displacement([Some(0.1), None, None, None, None, None]),
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
                .starts_with("1 Freiheitsgrade auch in Fixed-1")
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
        assert!(findings[0].detail.ends_with("z. B. 1"));
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
            findings[0].detail.ends_with(": Drehung um Z frei"),
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
        });
        step.loads.push(Load {
            name: "Flux-1".into(),
            active: true,
            region: Region::Nodes(vec![7]),
            kind: LoadKind::ConcentratedFlux(10.0),
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
