//! The constraint dialog, PrePoMax's "Create Constraint": a list of constraint types above the
//! properties of the chosen one.

use egui::Ui;
use plx_mesh::FeMesh;
use plx_model::{
    CompressionOnly, Constraint, FeModel, PointSpring, Quantity, Region, RigidBody, SurfaceSpring,
    SurfaceToSurfaceSpring, Tie, UnitSystem, next_name,
};

use crate::contacts::{self, MasterSlave};
use crate::model::{Highlight, Model};
use crate::numeric;
use crate::selection::Target;
use crate::setup::{FACE_SOURCES, NODE_SOURCES, RegionDraft, face_target};

/// PrePoMax's list of constraint types, with prepolix's spring connection added; `None` for
/// those prepolix does not have yet.
const TYPES: [(&str, Option<Type>); 6] = [
    ("Point Spring", Some(Type::PointSpring)),
    ("Surface Spring", Some(Type::SurfaceSpring)),
    ("Compression Only", Some(Type::CompressionOnly)),
    ("Rigid Body", Some(Type::RigidBody)),
    ("Tie", Some(Type::Tie)),
    (
        "Surface To Surface Spring",
        Some(Type::SurfaceToSurfaceSpring),
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    PointSpring,
    SurfaceSpring,
    CompressionOnly,
    RigidBody,
    Tie,
    SurfaceToSurfaceSpring,
}

impl Type {
    fn of(constraint: &Constraint) -> Self {
        match constraint {
            Constraint::PointSpring(_) => Type::PointSpring,
            Constraint::SurfaceSpring(_) => Type::SurfaceSpring,
            Constraint::CompressionOnly(_) => Type::CompressionOnly,
            Constraint::RigidBody(_) => Type::RigidBody,
            Constraint::Tie(_) => Type::Tie,
            Constraint::SurfaceToSurfaceSpring(_) => Type::SurfaceToSurfaceSpring,
            // Moved to the node ties when the project was read; never edited here.
            Constraint::NodeTie(_) => unreachable!("node ties are not constraints"),
        }
    }

    /// PrePoMax's default name without the number.
    fn prefix(self) -> &'static str {
        match self {
            Type::PointSpring => "Point_Spring",
            Type::SurfaceSpring => "Surface_Spring",
            Type::CompressionOnly => "Compression_Only",
            Type::RigidBody => "Rigid_Body",
            Type::Tie => "Tie",
            Type::SurfaceToSurfaceSpring => "Surface_To_Surface_Spring",
        }
    }

    /// Point springs and node ties sit on nodes, all other single-region constraints on
    /// faces.
    fn on_nodes(self) -> bool {
        matches!(self, Type::PointSpring)
    }

    fn has_master_slave(self) -> bool {
        matches!(self, Type::Tie | Type::SurfaceToSurfaceSpring)
    }

    /// The constraint with PrePoMax's default values and empty regions.
    fn create(self, name: String) -> Constraint {
        let faces = || Region::Faces(Vec::new());
        match self {
            Type::PointSpring => Constraint::PointSpring(PointSpring {
                name,
                active: true,
                region: Region::Nodes(Vec::new()),
                stiffness: [0.0; 3],
            }),
            Type::SurfaceSpring => Constraint::SurfaceSpring(SurfaceSpring {
                name,
                active: true,
                region: faces(),
                stiffness: [0.0; 3],
                per_area: false,
            }),
            Type::CompressionOnly => Constraint::CompressionOnly(CompressionOnly {
                name,
                active: true,
                region: faces(),
                clearance: 0.0,
                spring_stiffness: None,
                tensile_force: None,
                offset: 0.0,
                nonlinear: false,
            }),
            Type::RigidBody => Constraint::RigidBody(RigidBody::new(name, "")),
            Type::Tie => Constraint::Tie(Tie::new(name)),
            Type::SurfaceToSurfaceSpring => {
                Constraint::SurfaceToSurfaceSpring(SurfaceToSurfaceSpring {
                    name,
                    active: true,
                    master: faces(),
                    slave: faces(),
                    stiffness: [0.0; 3],
                    per_area: false,
                })
            }
        }
    }

    /// Point springs sit on nodes, all other single-region constraints on `faces`, the
    /// element faces or, in 2D models, the element edges.
    fn region_draft(self, faces: Target) -> RegionDraft {
        if self.on_nodes() {
            RegionDraft::new(NODE_SOURCES, Target::Nodes)
        } else {
            RegionDraft::new(FACE_SOURCES, faces)
        }
    }
}

/// A constraint while its dialog is open.
pub(crate) struct ConstraintDraft {
    pub(crate) constraint: Constraint,
    /// The region of a spring or support.
    region: RegionDraft,
    /// Master and slave of a tie or spring connection.
    pair: MasterSlave,
    /// What the faces of the model are: element faces, or element edges in 2D models.
    faces: Target,
}

impl ConstraintDraft {
    pub(crate) fn new(fe: &FeModel) -> Self {
        let kind = Type::PointSpring;
        let faces = face_target(fe);
        Self {
            constraint: kind.create(next_name(kind.prefix(), names(fe))),
            region: kind.region_draft(faces),
            pair: MasterSlave::new(faces),
            faces,
        }
    }

    /// `faces` is what the faces of the model are, see [`face_target`].
    pub(crate) fn edit(constraint: &Constraint, faces: Target, mesh: &FeMesh) -> Self {
        let kind = Type::of(constraint);
        let regions = constraint.regions();
        let (region, pair) = match constraint.master_slave() {
            Some([master, slave]) => (
                kind.region_draft(faces),
                MasterSlave::from_regions(master, slave, faces, mesh),
            ),
            None => {
                let (sources, target) = if kind.on_nodes() {
                    (NODE_SOURCES, Target::Nodes)
                } else {
                    (FACE_SOURCES, faces)
                };
                let region = RegionDraft::from_region(regions[0], sources, target, mesh);
                (region, MasterSlave::new(faces))
            }
        };
        Self {
            constraint: constraint.clone(),
            region,
            pair,
            faces,
        }
    }

    fn kind(&self) -> Type {
        Type::of(&self.constraint)
    }

    pub(crate) fn name(&self) -> &str {
        self.constraint.name()
    }

    /// The region that clicks in the 3D view pick.
    pub(crate) fn region(&self) -> &RegionDraft {
        if self.kind().has_master_slave() {
            self.pair.current()
        } else {
            &self.region
        }
    }

    pub(crate) fn region_mut(&mut self) -> &mut RegionDraft {
        if self.kind().has_master_slave() {
            self.pair.current_mut()
        } else {
            &mut self.region
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.kind().has_master_slave() {
            self.pair.validate()
        } else if self.region.is_empty() {
            Err("Die Region ist leer.".into())
        } else if matches!(&self.constraint, Constraint::RigidBody(b) if b.reference_point.is_empty())
        {
            Err("Choose the reference point that drives the rigid body.".into())
        } else {
            Ok(())
        }
    }

    /// Master in the primary, slave in the secondary highlight colour.
    pub(crate) fn highlight(&self, model: &Model) -> Highlight {
        if self.kind().has_master_slave() {
            self.pair.highlight(model)
        } else {
            self.region.highlight(model)
        }
    }

    /// The constraint with the regions as entered.
    pub(crate) fn finish(mut self) -> Constraint {
        let regions = if self.kind().has_master_slave() {
            let (master, slave) = self.pair.regions();
            vec![master, slave]
        } else {
            vec![self.region.region()]
        };
        for (slot, region) in self.constraint.regions_mut().into_iter().zip(regions) {
            *slot = region;
        }
        self.constraint
    }

    pub(crate) fn form(&mut self, ui: &mut Ui, model: &Model, taken: &[&str], creating: bool) {
        let current = self.kind();
        ui.label("Typ");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(200.0);
            ui.vertical(|ui| {
                for (label, kind) in TYPES {
                    let selected = kind == Some(current);
                    let enabled = creating && kind.is_some();
                    let response =
                        ui.add_enabled(enabled, egui::Button::selectable(selected, label));
                    let response = match kind {
                        None => response.on_disabled_hover_text("Noch nicht verfügbar"),
                        Some(_) if !selected => response.on_disabled_hover_text(
                            "Der Typ eines Constraints bleibt beim Bearbeiten.",
                        ),
                        Some(_) => response,
                    };
                    if let Some(kind) = kind
                        && response.clicked()
                        && !selected
                    {
                        self.switch(kind, taken);
                    }
                }
            });
        });
        ui.end_row();
        name_row(ui, &mut self.constraint);
        let units = model.fe.properties.units;
        match &mut self.constraint {
            Constraint::PointSpring(spring) => {
                self.region.ui(ui, model);
                stiffness_rows(ui, &mut spring.stiffness, units, stiffness(false));
                hint(
                    ui,
                    "Jeder Knoten der Region erhält diese Federn gegen die Umgebung.",
                );
            }
            Constraint::SurfaceSpring(spring) => {
                self.region.ui(ui, model);
                per_area_row(ui, &mut spring.per_area);
                stiffness_rows(ui, &mut spring.stiffness, units, stiffness(spring.per_area));
                hint(
                    ui,
                    "Federn gegen die Umgebung, beim Export flächengewichtet auf die Knoten \
                     verteilt.",
                );
            }
            Constraint::CompressionOnly(support) => {
                self.region.ui(ui, model);
                compression_only_rows(ui, support, units);
            }
            Constraint::RigidBody(body) => {
                self.region.ui(ui, model);
                ui.label("Reference point");
                let points = &model.fe.reference_points;
                if body.reference_point.is_empty()
                    && let Some(first) = points.first()
                {
                    body.reference_point = first.name.clone();
                }
                if points.is_empty() {
                    ui.weak("The model has no reference points (Features).");
                } else {
                    egui::ComboBox::from_id_salt("rigid body point")
                        .selected_text(body.reference_point.as_str())
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for point in points {
                                ui.selectable_value(
                                    &mut body.reference_point,
                                    point.name.clone(),
                                    &point.name,
                                );
                            }
                        });
                }
                ui.end_row();
                hint(
                    ui,
                    "The nodes of the region move as one rigid body with the reference \
                     point. Boundary conditions, forces and moments on the point drive the \
                     body.",
                );
            }
            Constraint::Tie(tie) => contacts::tie_form(ui, model, tie, &mut self.pair),
            Constraint::NodeTie(_) => unreachable!("node ties are not constraints"),
            Constraint::SurfaceToSurfaceSpring(spring) => {
                self.pair.ui(ui, model);
                per_area_row(ui, &mut spring.per_area);
                stiffness_rows(ui, &mut spring.stiffness, units, stiffness(spring.per_area));
                hint(
                    ui,
                    "Jeder Knoten der Slave-Fläche wird über Federn in den globalen Richtungen \
                     mit dem nächstgelegenen Punkt der Master-Fläche verbunden. Die Steifigkeit \
                     gilt für die ganze Verbindung und wird flächengewichtet auf die Knoten \
                     verteilt.",
                );
            }
        }
    }

    /// Another type was picked in the list: its defaults, keeping a name the user chose.
    fn switch(&mut self, kind: Type, taken: &[&str]) {
        let old = self.kind();
        let default_name = (self.constraint.name().strip_prefix(old.prefix()))
            .and_then(|n| n.strip_prefix('-'))
            .is_some_and(|n| n.parse::<u32>().is_ok());
        let name = if default_name {
            next_name(kind.prefix(), taken.iter().copied())
        } else {
            self.constraint.name().to_owned()
        };
        self.constraint = kind.create(name);
        if old.on_nodes() != kind.on_nodes() {
            self.region = kind.region_draft(self.faces);
        }
    }
}

fn names(fe: &FeModel) -> impl Iterator<Item = &str> {
    fe.constraints.iter().map(Constraint::name)
}

fn name_row(ui: &mut Ui, constraint: &mut Constraint) {
    let name = match constraint {
        Constraint::PointSpring(c) => &mut c.name,
        Constraint::SurfaceSpring(c) => &mut c.name,
        Constraint::CompressionOnly(c) => &mut c.name,
        Constraint::Tie(c) => &mut c.name,
        Constraint::SurfaceToSurfaceSpring(c) => &mut c.name,
        Constraint::RigidBody(c) => &mut c.name,
        Constraint::NodeTie(_) => unreachable!("node ties are not constraints"),
    };
    ui.label("Name");
    ui.add(egui::TextEdit::singleline(name).desired_width(200.0));
    ui.end_row();
}

/// Springs have a stiffness per length, or per length and area.
fn stiffness(per_area: bool) -> Quantity {
    if per_area {
        Quantity::ForcePerVolume
    } else {
        Quantity::ForcePerLength
    }
}

fn per_area_row(ui: &mut Ui, per_area: &mut bool) {
    ui.label("Steifigkeit");
    ui.horizontal(|ui| {
        ui.radio_value(per_area, false, "Gesamt");
        ui.radio_value(per_area, true, "Pro Fläche");
    });
    ui.end_row();
}

fn stiffness_rows(ui: &mut Ui, stiffness: &mut [f64; 3], units: UnitSystem, of: Quantity) {
    for (value, label) in stiffness.iter_mut().zip(["K1", "K2", "K3"]) {
        ui.label(label);
        ui.add(
            numeric::quantity(value, units, of)
                .speed(1.0)
                .range(0.0..=f64::MAX),
        );
        ui.end_row();
    }
}

fn compression_only_rows(ui: &mut Ui, support: &mut CompressionOnly, units: UnitSystem) {
    length_row(ui, "Spaltmaß", &mut support.clearance, units);
    optional_row(
        ui,
        "Federsteifigkeit",
        &mut support.spring_stiffness,
        CompressionOnly::DEFAULT_STIFFNESS,
        (units, Quantity::ForcePerLength),
    );
    optional_row(
        ui,
        "Zugkraft",
        &mut support.tensile_force,
        CompressionOnly::DEFAULT_TENSILE_FORCE,
        (units, Quantity::Force),
    );
    length_row(ui, "Versatz", &mut support.offset, units);
    ui.label("Nichtlinear");
    ui.checkbox(&mut support.nonlinear, "")
        .on_hover_text("Sonst wird die Stützung in einem linearen Step linearisiert.");
    ui.end_row();
    hint(
        ui,
        "Spaltelemente nehmen nur Druck auf; die kleine Zugkraft im offenen Zustand hält das \
         Modell lösbar. Steifigkeit und Zugkraft gelten für die ganze Fläche.",
    );
}

fn length_row(ui: &mut Ui, label: &str, value: &mut f64, units: UnitSystem) {
    ui.label(label);
    ui.add(numeric::quantity(value, units, Quantity::Length).speed(0.01));
    ui.end_row();
}

/// A value PrePoMax leaves at its default until the user enters one.
fn optional_row(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<f64>,
    default: f64,
    (units, quantity): (UnitSystem, Quantity),
) {
    let mut set = value.is_some();
    ui.checkbox(&mut set, label);
    ui.horizontal(|ui| {
        let mut number = value.unwrap_or(default);
        let unit = units.unit(quantity);
        ui.add_enabled(
            set,
            numeric::without_unit(&mut number, units, quantity)
                .speed(0.0)
                .custom_formatter(|v, _| numeric::format_physical(v)),
        );
        ui.label(if set { unit } else { "Standard" });
        *value = set.then_some(number);
    });
    ui.end_row();
}

fn hint(ui: &mut Ui, text: &str) {
    ui.label("");
    ui.add(egui::Label::new(egui::RichText::new(text).weak()).wrap());
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_the_type_proposes_its_default_name() {
        let mut fe = FeModel::default();
        fe.constraints
            .push(Type::SurfaceSpring.create("Surface_Spring-1".into()));
        let mut draft = ConstraintDraft::new(&fe);
        assert_eq!(draft.name(), "Point_Spring-1");
        assert_eq!(draft.region.target, Target::Nodes);
        draft.switch(Type::SurfaceSpring, &["Surface_Spring-1"]);
        assert_eq!(draft.name(), "Surface_Spring-2");
        assert_eq!(draft.region.target, Target::Faces);
        draft.switch(Type::Tie, &[]);
        assert_eq!(draft.name(), "Tie-1");
        assert!(matches!(draft.constraint, Constraint::Tie(_)));
        let Constraint::Tie(tie) = &mut draft.constraint else {
            unreachable!()
        };
        tie.name = "Lager".into();
        draft.switch(Type::SurfaceToSurfaceSpring, &[]);
        assert_eq!(draft.name(), "Lager");
    }

    #[test]
    fn faces_of_2d_models_are_picked_as_edges() {
        let mut fe = FeModel::default();
        fe.properties.space = plx_model::ModelSpace::PlaneStrain;
        let mut draft = ConstraintDraft::new(&fe);
        draft.switch(Type::SurfaceSpring, &[]);
        assert_eq!(draft.region.target, Target::Edges);
        draft.switch(Type::Tie, &[]);
        assert_eq!(draft.pair.master.target, Target::Edges);
        assert_eq!(draft.pair.slave.target, Target::Edges);
    }

    #[test]
    fn regions_survive_the_dialog() {
        let mesh = FeMesh::default();
        let connection = Constraint::SurfaceToSurfaceSpring(SurfaceToSurfaceSpring {
            name: "Lager".into(),
            active: false,
            master: Region::Surface("BORE".into()),
            slave: Region::Faces(vec![(1, 2)]),
            stiffness: [1.0, 2.0, 3.0],
            per_area: true,
        });
        let spring = Constraint::PointSpring(PointSpring {
            name: "Point_Spring-1".into(),
            active: true,
            region: Region::NodeSet("FIX".into()),
            stiffness: [1.0, 0.0, 0.0],
        });
        for constraint in [connection, spring] {
            let draft = ConstraintDraft::edit(&constraint, Target::Faces, &mesh);
            assert_eq!(draft.validate(), Ok(()));
            assert_eq!(draft.finish(), constraint);
        }
    }
}
