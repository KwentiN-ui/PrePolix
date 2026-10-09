//! The constraint dialog, PrePoMax's "Create Constraint": a list of constraint types above the
//! properties of the chosen one.

use egui::Ui;
use plx_mesh::FeMesh;
use plx_model::{
    CompressionOnly, Constraint, FeModel, PointSpring, Region, SurfaceSpring,
    SurfaceToSurfaceSpring, Tie, next_name,
};

use crate::contacts::{self, MasterSlave};
use crate::model::{Highlight, Model};
use crate::numeric;
use crate::selection::Target;
use crate::setup::{FACE_SOURCES, NODE_SOURCES, RegionDraft};

/// PrePoMax's list of constraint types, with prepolix's spring connection added; `None` for
/// those prepolix does not have yet.
const TYPES: [(&str, Option<Type>); 6] = [
    ("Point Spring", Some(Type::PointSpring)),
    ("Surface Spring", Some(Type::SurfaceSpring)),
    ("Compression Only", Some(Type::CompressionOnly)),
    ("Rigid Body", None),
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
    Tie,
    SurfaceToSurfaceSpring,
}

impl Type {
    fn of(constraint: &Constraint) -> Self {
        match constraint {
            Constraint::PointSpring(_) => Type::PointSpring,
            Constraint::SurfaceSpring(_) => Type::SurfaceSpring,
            Constraint::CompressionOnly(_) => Type::CompressionOnly,
            Constraint::Tie(_) => Type::Tie,
            Constraint::SurfaceToSurfaceSpring(_) => Type::SurfaceToSurfaceSpring,
        }
    }

    /// PrePoMax's default name without the number.
    fn prefix(self) -> &'static str {
        match self {
            Type::PointSpring => "Point_Spring",
            Type::SurfaceSpring => "Surface_Spring",
            Type::CompressionOnly => "Compression_Only",
            Type::Tie => "Tie",
            Type::SurfaceToSurfaceSpring => "Surface_To_Surface_Spring",
        }
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

    /// Point springs sit on nodes, all other single-region constraints on faces.
    fn region_draft(self) -> RegionDraft {
        match self {
            Type::PointSpring => RegionDraft::new(NODE_SOURCES, Target::Nodes),
            _ => RegionDraft::new(FACE_SOURCES, Target::Faces),
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
}

impl ConstraintDraft {
    pub(crate) fn new(fe: &FeModel) -> Self {
        let kind = Type::PointSpring;
        Self {
            constraint: kind.create(next_name(kind.prefix(), names(fe))),
            region: kind.region_draft(),
            pair: MasterSlave::new(),
        }
    }

    pub(crate) fn edit(constraint: &Constraint, mesh: &FeMesh) -> Self {
        let kind = Type::of(constraint);
        let regions = constraint.regions();
        let (region, pair) = match constraint.master_slave() {
            Some([master, slave]) => (
                kind.region_draft(),
                MasterSlave::from_regions(master, slave, mesh),
            ),
            None => {
                let (sources, target) = match kind {
                    Type::PointSpring => (NODE_SOURCES, Target::Nodes),
                    _ => (FACE_SOURCES, Target::Faces),
                };
                let region = RegionDraft::from_region(regions[0], sources, target, mesh);
                (region, MasterSlave::new())
            }
        };
        Self {
            constraint: constraint.clone(),
            region,
            pair,
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
        match &mut self.constraint {
            Constraint::PointSpring(spring) => {
                self.region.ui(ui, model);
                stiffness_rows(ui, &mut spring.stiffness, "N/mm");
                hint(
                    ui,
                    "Jeder Knoten der Region erhält diese Federn gegen die Umgebung.",
                );
            }
            Constraint::SurfaceSpring(spring) => {
                self.region.ui(ui, model);
                per_area_row(ui, &mut spring.per_area);
                stiffness_rows(ui, &mut spring.stiffness, unit(spring.per_area));
                hint(
                    ui,
                    "Federn gegen die Umgebung, beim Export flächengewichtet auf die Knoten \
                     verteilt.",
                );
            }
            Constraint::CompressionOnly(support) => {
                self.region.ui(ui, model);
                compression_only_rows(ui, support);
            }
            Constraint::Tie(tie) => contacts::tie_form(ui, model, tie, &mut self.pair),
            Constraint::SurfaceToSurfaceSpring(spring) => {
                self.pair.ui(ui, model);
                per_area_row(ui, &mut spring.per_area);
                stiffness_rows(ui, &mut spring.stiffness, unit(spring.per_area));
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
        if (old == Type::PointSpring) != (kind == Type::PointSpring) {
            self.region = kind.region_draft();
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
    };
    ui.label("Name");
    ui.add(egui::TextEdit::singleline(name).desired_width(200.0));
    ui.end_row();
}

fn unit(per_area: bool) -> &'static str {
    if per_area { "N/mm³" } else { "N/mm" }
}

fn per_area_row(ui: &mut Ui, per_area: &mut bool) {
    ui.label("Steifigkeit");
    ui.horizontal(|ui| {
        ui.radio_value(per_area, false, "Gesamt");
        ui.radio_value(per_area, true, "Pro Fläche");
    });
    ui.end_row();
}

fn stiffness_rows(ui: &mut Ui, stiffness: &mut [f64; 3], unit: &str) {
    for (value, label) in stiffness.iter_mut().zip(["K1", "K2", "K3"]) {
        ui.label(label);
        ui.horizontal(|ui| {
            ui.add(numeric::drag_value(value).speed(1.0).range(0.0..=f64::MAX));
            ui.label(unit);
        });
        ui.end_row();
    }
}

fn compression_only_rows(ui: &mut Ui, support: &mut CompressionOnly) {
    length_row(ui, "Spaltmaß", &mut support.clearance);
    optional_row(
        ui,
        "Federsteifigkeit",
        &mut support.spring_stiffness,
        CompressionOnly::DEFAULT_STIFFNESS,
        "N/mm",
    );
    optional_row(
        ui,
        "Zugkraft",
        &mut support.tensile_force,
        CompressionOnly::DEFAULT_TENSILE_FORCE,
        "N",
    );
    length_row(ui, "Versatz", &mut support.offset);
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

fn length_row(ui: &mut Ui, label: &str, value: &mut f64) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.add(numeric::drag_value(value).speed(0.01));
        ui.label("mm");
    });
    ui.end_row();
}

/// A value PrePoMax leaves at its default until the user enters one.
fn optional_row(ui: &mut Ui, label: &str, value: &mut Option<f64>, default: f64, unit: &str) {
    let mut set = value.is_some();
    ui.checkbox(&mut set, label);
    ui.horizontal(|ui| {
        let mut number = value.unwrap_or(default);
        ui.add_enabled(
            set,
            numeric::drag_value(&mut number)
                .speed(0.0)
                .custom_formatter(|v, _| format!("{v:e}")),
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
            let draft = ConstraintDraft::edit(&constraint, &mesh);
            assert_eq!(draft.validate(), Ok(()));
            assert_eq!(draft.finish(), constraint);
        }
    }
}
