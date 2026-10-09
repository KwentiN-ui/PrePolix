//! The forms of PrePoMax's interaction dialogs: tie constraints, surface interactions and
//! contact pairs. Master and slave surfaces are picked in the 3D view one after the other,
//! as in PrePoMax; the master shows in the primary, the slave in the secondary highlight
//! colour.

use egui::Ui;
use plx_mesh::FeMesh;
use plx_model::{
    ContactMethod, ContactPair, FeModel, Friction, GapConductance, InteractionProperty, Region,
    SurfaceBehavior, SurfaceInteraction, Tie,
};

use crate::model::{Highlight, Model};
use crate::numeric;
use crate::selection::Target;
use crate::setup::{FACE_SOURCES, RegionDraft, number, region_highlight};

/// Which of the two regions clicks in the 3D view pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Master,
    Slave,
}

/// Master and slave region of a tie or contact pair while they are edited.
#[derive(Clone, Debug, PartialEq)]
pub struct MasterSlave {
    pub master: RegionDraft,
    pub slave: RegionDraft,
    /// The region picked in the 3D view.
    pub side: Side,
}

impl MasterSlave {
    pub fn new() -> Self {
        Self {
            master: RegionDraft::new(FACE_SOURCES, Target::Faces),
            slave: RegionDraft::new(FACE_SOURCES, Target::Faces),
            side: Side::Master,
        }
    }

    pub fn from_regions(master: &Region, slave: &Region, mesh: &FeMesh) -> Self {
        Self {
            master: RegionDraft::from_region(master, FACE_SOURCES, Target::Faces, mesh),
            slave: RegionDraft::from_region(slave, FACE_SOURCES, Target::Faces, mesh),
            side: Side::Master,
        }
    }

    pub fn current(&self) -> &RegionDraft {
        match self.side {
            Side::Master => &self.master,
            Side::Slave => &self.slave,
        }
    }

    pub fn current_mut(&mut self) -> &mut RegionDraft {
        match self.side {
            Side::Master => &mut self.master,
            Side::Slave => &mut self.slave,
        }
    }

    pub fn regions(&self) -> (Region, Region) {
        (self.master.region(), self.slave.region())
    }

    pub fn ui(&mut self, ui: &mut Ui, model: &Model) {
        ui.label("Auswahl im 3D-Fenster");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.side, Side::Master, "Master");
            ui.radio_value(&mut self.side, Side::Slave, "Slave");
        });
        ui.end_row();
        self.master.ui_labeled(ui, model, "Master-Region", "master");
        self.slave.ui_labeled(ui, model, "Slave-Region", "slave");
    }

    /// The master in the primary, the slave in the secondary highlight colour.
    pub fn highlight(&self, model: &Model) -> Highlight {
        master_slave_highlight(model, &self.master.region(), &self.slave.region())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.master.is_empty() {
            return Err("Die Master-Region ist leer.".into());
        }
        if self.slave.is_empty() {
            return Err("Die Slave-Region ist leer.".into());
        }
        Ok(())
    }
}

/// Master faces in the primary, slave faces in the secondary highlight colour.
pub fn master_slave_highlight(model: &Model, master: &Region, slave: &Region) -> Highlight {
    let mut highlight = region_highlight(model, master);
    let slave = region_highlight(model, slave);
    highlight.secondary_faces = slave.faces;
    highlight.nodes.extend(slave.nodes);
    highlight
}

fn color_row(ui: &mut Ui, label: &str, color: &mut [u8; 3]) {
    ui.label(label);
    ui.color_edit_button_srgb(color);
    ui.end_row();
}

/// A value that may be left to CalculiX: a check box and the number.
fn optional_row(ui: &mut Ui, label: &str, value: &mut Option<f64>, default: f64) {
    let mut set = value.is_some();
    ui.checkbox(&mut set, label);
    let mut number_value = value.unwrap_or(default);
    ui.add_enabled(set, number(&mut number_value));
    *value = set.then_some(number_value);
    ui.end_row();
}

/// The properties of a tie below the constraint dialog's type list.
pub fn tie_form(ui: &mut Ui, model: &Model, tie: &mut Tie, regions: &mut MasterSlave) {
    optional_row(ui, "Positionstoleranz", &mut tie.position_tolerance, 0.05);
    ui.label("");
    ui.checkbox(
        &mut tie.adjust,
        "Slave-Knoten auf Master verschieben (Adjust)",
    );
    ui.end_row();
    regions.ui(ui, model);
    color_row(ui, "Farbe Master", &mut tie.master_color);
    color_row(ui, "Farbe Slave", &mut tie.slave_color);
}

pub fn contact_pair_form(
    ui: &mut Ui,
    model: &Model,
    pair: &mut ContactPair,
    regions: &mut MasterSlave,
) {
    ui.label("Surface Interaction");
    egui::ComboBox::from_id_salt("contact interaction")
        .selected_text(pair.interaction.as_str())
        .width(200.0)
        .show_ui(ui, |ui| {
            for interaction in &model.fe.surface_interactions {
                let name = interaction.name.clone();
                ui.selectable_value(&mut pair.interaction, name, &interaction.name);
            }
        });
    ui.end_row();
    ui.label("Methode");
    egui::ComboBox::from_id_salt("contact method")
        .selected_text(pair.method.name())
        .width(200.0)
        .show_ui(ui, |ui| {
            for method in ContactMethod::ALL {
                ui.selectable_value(&mut pair.method, method, method.name());
            }
        });
    ui.end_row();
    ui.label("");
    let node_to_surface = pair.method == ContactMethod::NodeToSurface;
    ui.add_enabled(
        node_to_surface,
        egui::Checkbox::new(&mut pair.small_sliding, "Small sliding"),
    )
    .on_hover_text("Paarung nur zu Beginn jedes Inkrements; nur Node to surface.");
    if !node_to_surface {
        pair.small_sliding = false;
    }
    ui.end_row();
    ui.label("");
    ui.checkbox(
        &mut pair.adjust,
        "Slave-Knoten auf Master verschieben (Adjust)",
    );
    ui.end_row();
    if pair.adjust {
        optional_row(ui, "Abstand für Adjust", &mut pair.adjustment_size, 0.01);
    }
    regions.ui(ui, model);
    color_row(ui, "Farbe Master", &mut pair.master_color);
    color_row(ui, "Farbe Slave", &mut pair.slave_color);
}

pub fn validate_contact_pair(pair: &ContactPair, fe: &FeModel) -> Result<(), String> {
    if !(fe.surface_interactions.iter()).any(|s| s.name == pair.interaction) {
        return Err(
            "Bitte eine Surface Interaction wählen; zuerst unter Surface Interactions anlegen."
                .into(),
        );
    }
    Ok(())
}

/// The interaction models of a surface interaction: PrePoMax's lists of available and
/// selected models, and the properties of the selected one below.
pub fn interaction_form(ui: &mut Ui, interaction: &mut SurfaceInteraction, selected: &mut usize) {
    ui.label("Interaction Models");
    ui.horizontal_top(|ui| {
        let list = |ui: &mut Ui, title: &str, body: &mut dyn FnMut(&mut Ui)| {
            ui.vertical(|ui| {
                ui.label(title);
                egui::Frame::new()
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(140.0, 70.0));
                        body(ui);
                    });
            });
        };
        let mut add = None;
        list(ui, "Verfügbar", &mut |ui| {
            for model in InteractionProperty::all() {
                let taken = (interaction.properties.iter())
                    .any(|p| std::mem::discriminant(p) == std::mem::discriminant(&model));
                let response = ui.add_enabled(!taken, egui::Button::new(model.name()).frame(false));
                if response
                    .on_hover_text("Doppelklick fügt hinzu")
                    .double_clicked()
                {
                    add = Some(model);
                }
            }
        });
        ui.vertical(|ui| {
            ui.add_space(18.0);
            let next = InteractionProperty::all().into_iter().find(|model| {
                !(interaction.properties.iter())
                    .any(|p| std::mem::discriminant(p) == std::mem::discriminant(model))
            });
            if ui
                .add_enabled(next.is_some(), egui::Button::new("Hinzufügen >"))
                .on_hover_text("Fügt das nächste verfügbare Modell hinzu")
                .clicked()
            {
                add = next;
            }
            let can_remove = *selected < interaction.properties.len();
            if ui
                .add_enabled(can_remove, egui::Button::new("< Entfernen"))
                .clicked()
            {
                interaction.properties.remove(*selected);
                *selected = selected.saturating_sub(1);
            }
        });
        if let Some(model) = add {
            interaction.properties.push(model);
            *selected = interaction.properties.len() - 1;
        }
        list(ui, "Gewählt", &mut |ui| {
            for (i, property) in interaction.properties.iter().enumerate() {
                if ui
                    .selectable_label(*selected == i, property.name())
                    .clicked()
                {
                    *selected = i;
                }
            }
        });
    });
    ui.end_row();
    let Some(property) = interaction.properties.get_mut(*selected) else {
        return;
    };
    ui.label("");
    ui.strong(property.name());
    ui.end_row();
    match property {
        InteractionProperty::SurfaceBehavior(behavior) => surface_behavior_form(ui, behavior),
        InteractionProperty::Friction(friction) => friction_form(ui, friction),
        InteractionProperty::GapConductance(conductance) => gap_conductance_form(ui, conductance),
    }
}

fn surface_behavior_form(ui: &mut Ui, behavior: &mut SurfaceBehavior) {
    ui.label("Druck-Eindringung");
    egui::ComboBox::from_id_salt("pressure overclosure")
        .selected_text(behavior.keyword())
        .show_ui(ui, |ui| {
            for kind in SurfaceBehavior::kinds() {
                let same = std::mem::discriminant(&kind) == std::mem::discriminant(behavior);
                if ui.selectable_label(same, kind.keyword()).clicked() && !same {
                    // Switching the kind starts with its default values, as in PrePoMax.
                    *behavior = kind;
                }
            }
        });
    ui.end_row();
    match behavior {
        SurfaceBehavior::Hard => {
            ui.label("");
            ui.weak("Keine Durchdringung der Flächen.");
            ui.end_row();
        }
        SurfaceBehavior::Linear { k, sigma_inf, c0 } => {
            value_row(ui, "K", k, "Steigung, etwa 5- bis 50-mal der E-Modul");
            value_row(
                ui,
                "Sigma unendlich",
                sigma_inf,
                "Zugspannung bei großem Spalt, etwa 0,25 % der größten Vergleichsspannung",
            );
            optional_row(ui, "c0", c0, 1.0);
        }
        SurfaceBehavior::Exponential { c0, p0 } => {
            value_row(
                ui,
                "c0",
                c0,
                "Spalt, bei dem der Druck auf 1 % von p0 fällt",
            );
            value_row(ui, "p0", p0, "Kontaktdruck bei Spalt null");
        }
        SurfaceBehavior::Tabular(rows) => {
            table(
                ui,
                rows,
                ["Druck", "Eindringung"],
                "pressure overclosure table",
            );
        }
        SurfaceBehavior::Tied { k } => {
            value_row(ui, "K", k, "Steifigkeit der Verbindung");
        }
    }
}

fn friction_form(ui: &mut Ui, friction: &mut Friction) {
    ui.label("Reibungskoeffizient");
    ui.add(
        numeric::drag_value(&mut friction.coefficient)
            .range(0.0..=f64::MAX)
            .speed(0.01),
    );
    ui.end_row();
    optional_row(ui, "Haftsteigung", &mut friction.stick_slope, 1e5);
}

fn gap_conductance_form(ui: &mut Ui, conductance: &mut GapConductance) {
    ui.label("Art");
    ui.horizontal(|ui| {
        let constant = matches!(conductance, GapConductance::Constant(_));
        if ui.radio(constant, "Konstant").clicked() && !constant {
            *conductance = GapConductance::Constant(0.0);
        }
        if ui.radio(!constant, "Tabelle").clicked() && constant {
            *conductance = GapConductance::Tabular(vec![[0.0; 3]]);
        }
    });
    ui.end_row();
    match conductance {
        GapConductance::Constant(value) => {
            value_row(ui, "Leitwert", value, "Wärmestrom je Temperaturdifferenz");
        }
        GapConductance::Tabular(rows) => {
            table(
                ui,
                rows,
                ["Leitwert", "Druck", "Temperatur"],
                "conductance table",
            );
        }
    }
}

fn value_row(ui: &mut Ui, label: &str, value: &mut f64, hint: &str) {
    ui.label(label).on_hover_text(hint);
    ui.add(number(value)).on_hover_text(hint);
    ui.end_row();
}

/// A small editable table with a row to add and buttons to remove rows.
fn table<const N: usize>(ui: &mut Ui, rows: &mut Vec<[f64; N]>, header: [&str; N], id: &str) {
    ui.label("Tabelle");
    ui.vertical(|ui| {
        egui::Grid::new(id).striped(true).show(ui, |ui| {
            for title in header {
                ui.strong(title);
            }
            ui.label("");
            ui.end_row();
            let mut remove = None;
            for (i, row) in rows.iter_mut().enumerate() {
                for value in row.iter_mut() {
                    ui.add(number(value));
                }
                if ui.small_button("Entfernen").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
            if let Some(i) = remove.filter(|_| rows.len() > 1) {
                rows.remove(i);
            }
        });
        if ui.button("Zeile hinzufügen").clicked() {
            let last = rows.last().copied().unwrap_or([0.0; N]);
            rows.push(last);
        }
    });
    ui.end_row();
}

pub fn validate_interaction(interaction: &SurfaceInteraction) -> Result<(), String> {
    if interaction.properties.is_empty() {
        return Err("Bitte mindestens ein Interaction Model hinzufügen.".into());
    }
    for property in &interaction.properties {
        if let InteractionProperty::Friction(friction) = property
            && friction.coefficient <= 0.0
        {
            return Err("Der Reibungskoeffizient muss größer als 0 sein.".into());
        }
    }
    Ok(())
}
