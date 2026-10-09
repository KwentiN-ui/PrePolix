//! The forms of PrePoMax's interaction dialogs: tie constraints, surface interactions and
//! contact pairs. Clicks in the 3D view fill the master or the slave surface, whichever
//! field's "..." button is pressed; in 2D models the surfaces are element edges. The master
//! shows in the primary, the slave in the secondary highlight colour.

use egui::{RichText, Ui};
use plx_mesh::FeMesh;
use plx_model::{
    ContactMethod, ContactPair, FeModel, Friction, GapConductance, InteractionProperty, Quantity,
    Region, SurfaceBehavior, SurfaceInteraction, Tie, UnitSystem,
};

use crate::icons::{self, Icon};
use crate::keywords::{frame, tree_row};
use crate::model::{Highlight, Model};
use crate::numeric;
use crate::selection::Target;
use crate::setup::{FACE_SOURCES, RegionDraft, region_highlight};

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
    /// `faces` is what the 3D view picks: element faces, or element edges in 2D models.
    pub fn new(faces: Target) -> Self {
        Self {
            master: RegionDraft::new(FACE_SOURCES, faces),
            slave: RegionDraft::new(FACE_SOURCES, faces),
            side: Side::Master,
        }
    }

    pub fn from_regions(master: &Region, slave: &Region, faces: Target, mesh: &FeMesh) -> Self {
        Self {
            master: RegionDraft::from_region(master, FACE_SOURCES, faces, mesh),
            slave: RegionDraft::from_region(slave, FACE_SOURCES, faces, mesh),
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
        let master = self.side == Side::Master;
        if (self.master).ui_labeled(ui, model, "Master-Region", "master", master) {
            self.side = Side::Master;
        }
        if (self.slave).ui_labeled(ui, model, "Slave-Region", "slave", !master) {
            self.side = Side::Slave;
        }
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
    highlight.secondary_lines = slave.lines;
    highlight.nodes.extend(slave.nodes);
    highlight
}

fn color_row(ui: &mut Ui, label: &str, color: &mut [u8; 3]) {
    ui.label(label);
    ui.color_edit_button_srgb(color);
    ui.end_row();
}

/// A value that may be left to CalculiX: a check box and the number.
fn optional_row(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<f64>,
    default: f64,
    (units, quantity): (UnitSystem, Quantity),
) {
    let mut set = value.is_some();
    ui.checkbox(&mut set, label);
    let mut number_value = value.unwrap_or(default);
    ui.add_enabled(set, numeric::physical(&mut number_value, units, quantity));
    *value = set.then_some(number_value);
    ui.end_row();
}

/// The properties of a tie below the constraint dialog's type list.
pub fn tie_form(ui: &mut Ui, model: &Model, tie: &mut Tie, regions: &mut MasterSlave) {
    let length = (model.fe.properties.units, Quantity::Length);
    optional_row(
        ui,
        "Positionstoleranz",
        &mut tie.position_tolerance,
        0.05,
        length,
    );
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
        let length = (model.fe.properties.units, Quantity::Length);
        optional_row(
            ui,
            "Abstand für Adjust",
            &mut pair.adjustment_size,
            0.01,
            length,
        );
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

/// A row's title and description, shown below the property grid like in PrePoMax.
type Description = (&'static str, &'static str);

/// What the surface interaction dialog shows: the model selected in each list and the
/// focused property.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InteractionView {
    /// The model selected in the Selected list, whose properties are shown.
    selected: Option<usize>,
    /// The model selected in the Available list, added by the arrow button.
    available: Option<usize>,
    focus: Option<Description>,
}

impl InteractionView {
    pub fn new(interaction: &SurfaceInteraction) -> Self {
        Self {
            selected: (!interaction.properties.is_empty()).then_some(0),
            ..Self::default()
        }
    }
}

/// PrePoMax's "Create surface interaction" dialog: the name, the lists of available and
/// selected interaction models, the properties of the selected model and their description.
pub fn interaction_dialog(
    ui: &mut Ui,
    interaction: &mut SurfaceInteraction,
    view: &mut InteractionView,
    units: UnitSystem,
) {
    // As wide as the title, which grows with the name.
    ui.set_min_width(420.0);
    group(ui, "Daten", |ui| {
        let mut focus = None;
        egui::Grid::new("interaction data")
            .num_columns(2)
            .min_col_width(110.0)
            .show(ui, |ui| {
                let label = ui.add(egui::Label::new("Name").sense(egui::Sense::click()));
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut interaction.name)
                        .desired_width(ui.available_width()),
                );
                if label.clicked() || edit.has_focus() {
                    focus = Some(("Name", "Name der Surface Interaction."));
                }
            });
        if focus.is_some() {
            view.focus = focus;
        }
    });
    group(ui, "Interaction Models", |ui| {
        model_lists(ui, interaction, view)
    });
    ui.add_space(4.0);
    ui.strong("Eigenschaften");
    frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(150.0);
        match view
            .selected
            .and_then(|i| interaction.properties.get_mut(i))
        {
            Some(property) => {
                category(ui, property.name());
                let mut rows = Rows {
                    focus: &mut view.focus,
                    units,
                };
                egui::Grid::new(("interaction properties", property.name()))
                    .num_columns(2)
                    .striped(true)
                    .min_col_width(110.0)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| match property {
                        InteractionProperty::SurfaceBehavior(b) => rows.surface_behavior(ui, b),
                        InteractionProperty::Friction(f) => rows.friction(ui, f),
                        InteractionProperty::GapConductance(g) => rows.gap_conductance(ui, g),
                    });
            }
            None => {
                ui.weak("Ein Modell aus der Liste Gewählt zeigt hier seine Eigenschaften.");
            }
        }
    });
    ui.add_space(4.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(60.0);
        let selected = view.selected.and_then(|i| interaction.properties.get(i));
        if let Some((title, text)) = view.focus.or_else(|| selected.map(model_description)) {
            ui.strong(title);
            ui.label(text);
        }
    });
}

/// The Available list, the add and remove buttons and the Selected list.
fn model_lists(ui: &mut Ui, interaction: &mut SurfaceInteraction, view: &mut InteractionView) {
    let height = 72.0;
    let all = InteractionProperty::all();
    let taken = |interaction: &SurfaceInteraction, model: &InteractionProperty| {
        (interaction.properties.iter())
            .any(|p| std::mem::discriminant(p) == std::mem::discriminant(model))
    };
    let mut add = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label("Verfügbar");
            frame().show(ui, |ui| {
                ui.set_width(140.0);
                ui.set_height(height);
                for (i, model) in all.iter().enumerate() {
                    let used = taken(interaction, model);
                    let selected = view.available == Some(i);
                    let row = ui
                        .add_enabled_ui(!used, |ui| {
                            tree_row(ui, 0, None, selected, RichText::new(model.name()))
                        })
                        .inner;
                    if row.clicked() {
                        view.available = Some(i);
                        view.focus = None;
                    }
                    if row.double_clicked() {
                        add = true;
                    }
                }
            });
        });
        ui.vertical(|ui| {
            ui.add_space(22.0);
            let can_add = view
                .available
                .and_then(|i| all.get(i))
                .is_some_and(|model| !taken(interaction, model));
            let arrow = Icon::Arrow(egui::vec2(1.0, 0.0));
            if icons::dialog_button(ui, arrow, "Hinzufügen", can_add).clicked() {
                add = true;
            }
            let can_remove = view.selected.is_some();
            if icons::dialog_button(ui, Icon::Remove, "Entfernen", can_remove).clicked()
                && let Some(index) = view.selected
            {
                interaction.properties.remove(index);
                let len = interaction.properties.len();
                view.selected = (len > 0).then(|| index.min(len - 1));
                view.focus = None;
            }
        });
        if add
            && let Some(model) = view.available.and_then(|i| all.get(i))
            && !taken(interaction, model)
        {
            interaction.properties.push(model.clone());
            view.selected = Some(interaction.properties.len() - 1);
            // The next model still available is ready for the next click on the arrow.
            view.available = (all.iter()).position(|model| !taken(interaction, model));
            view.focus = None;
        }
        ui.vertical(|ui| {
            ui.label("Gewählt");
            frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(height);
                for (i, property) in interaction.properties.iter().enumerate() {
                    let selected = view.selected == Some(i);
                    if tree_row(ui, 0, None, selected, RichText::new(property.name())).clicked() {
                        view.selected = Some(i);
                        view.focus = None;
                    }
                }
            });
        });
    });
}

fn model_description(property: &InteractionProperty) -> Description {
    match property {
        InteractionProperty::SurfaceBehavior(_) => (
            "Surface Behavior",
            "Kontaktdruck in Abhängigkeit von der Eindringung der Flächen.",
        ),
        InteractionProperty::Friction(_) => (
            "Friction",
            "Reibung zwischen den Kontaktflächen nach Coulomb.",
        ),
        InteractionProperty::GapConductance(_) => (
            "Gap Conductance",
            "Wärmeleitung über den Spalt zwischen den Kontaktflächen.",
        ),
    }
}

/// Width of the number fields of the property grid.
fn field_size(ui: &Ui) -> egui::Vec2 {
    egui::vec2(120.0, ui.spacing().interact_size.y)
}

/// Category header like PrePoMax's property grid.
fn category(ui: &mut Ui, title: &str) {
    let header = egui::Frame::new()
        .fill(crate::style::CONTROL)
        .inner_margin(egui::Margin::symmetric(4, 1));
    header.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.strong(title);
    });
}

/// A titled frame like a Windows group box.
fn group(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui)) {
    ui.add_space(4.0);
    ui.strong(title);
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, crate::style::BORDER))
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            contents(ui);
        });
}

/// Rows of the property grid; a click on a row or its editor shows its description.
struct Rows<'a> {
    focus: &'a mut Option<Description>,
    units: UnitSystem,
}

impl Rows<'_> {
    fn row(
        &mut self,
        ui: &mut Ui,
        description: Description,
        editor: impl FnOnce(&mut Ui) -> egui::Response,
    ) {
        let label = ui.add(egui::Label::new(description.0).sense(egui::Sense::click()));
        let response = editor(ui);
        if label.clicked() || response.clicked() || response.has_focus() || response.changed() {
            *self.focus = Some(description);
        }
        ui.end_row();
    }

    fn value(&mut self, ui: &mut Ui, description: Description, value: &mut f64, of: Quantity) {
        let units = self.units;
        self.row(ui, description, |ui| {
            ui.add_sized(field_size(ui), numeric::physical(value, units, of))
        });
    }

    /// A value CalculiX may leave out; the checkbox writes it.
    fn optional(
        &mut self,
        ui: &mut Ui,
        description: Description,
        value: &mut Option<f64>,
        default: f64,
        of: Quantity,
    ) {
        let units = self.units;
        let mut set = value.is_some();
        let mut number_value = value.unwrap_or(default);
        let label = ui.checkbox(&mut set, description.0);
        let size = field_size(ui);
        let response = ui
            .add_enabled_ui(set, |ui| {
                ui.add_sized(size, numeric::physical(&mut number_value, units, of))
            })
            .inner;
        if label.changed() || response.has_focus() || response.changed() {
            *self.focus = Some(description);
        }
        *value = set.then_some(number_value);
        ui.end_row();
    }

    fn surface_behavior(&mut self, ui: &mut Ui, behavior: &mut SurfaceBehavior) {
        let description = (
            "Druck-Eindringung",
            "Kennlinie des Kontaktdrucks über der Eindringung (Pressure-overclosure).",
        );
        self.row(ui, description, |ui| {
            egui::ComboBox::from_id_salt("pressure overclosure")
                .selected_text(behavior.keyword())
                .show_ui(ui, |ui| {
                    for kind in SurfaceBehavior::kinds() {
                        let same =
                            std::mem::discriminant(&kind) == std::mem::discriminant(behavior);
                        if ui.selectable_label(same, kind.keyword()).clicked() && !same {
                            // Switching the kind starts with its default values, as in PrePoMax.
                            *behavior = kind;
                        }
                    }
                })
                .response
        });
        match behavior {
            SurfaceBehavior::Hard => {}
            SurfaceBehavior::Linear { k, sigma_inf, c0 } => {
                let k_text = "Steigung der Kennlinie, etwa 5- bis 50-mal der E-Modul je Länge.";
                self.value(ui, ("K", k_text), k, Quantity::ForcePerVolume);
                let sigma_text = "Zugspannung bei großem Spalt, etwa 0,25 % der größten \
                                  erwarteten Vergleichsspannung.";
                self.value(
                    ui,
                    ("Sigma unendlich", sigma_text),
                    sigma_inf,
                    Quantity::Pressure,
                );
                let c0_text = "Optionaler Parameter c0 der linearen Kennlinie, siehe \
                               *SURFACE BEHAVIOR im CalculiX-Handbuch.";
                self.optional(ui, ("c0", c0_text), c0, 1.0, Quantity::Length);
            }
            SurfaceBehavior::Exponential { c0, p0 } => {
                let c0_text = "Spalt, bei dem der Kontaktdruck auf 1 % von p0 gefallen ist.";
                self.value(ui, ("c0", c0_text), c0, Quantity::Length);
                self.value(
                    ui,
                    ("p0", "Kontaktdruck bei Spalt null."),
                    p0,
                    Quantity::Pressure,
                );
            }
            SurfaceBehavior::Tabular(rows) => {
                let text = "Je Zeile ein Kontaktdruck und die zugehörige Eindringung.";
                self.table(
                    ui,
                    ("Tabelle", text),
                    rows,
                    [
                        ("Druck", Quantity::Pressure),
                        ("Eindringung", Quantity::Length),
                    ],
                );
            }
            SurfaceBehavior::Tied { k } => {
                self.value(
                    ui,
                    ("K", "Steifigkeit der Verbindung."),
                    k,
                    Quantity::ForcePerVolume,
                );
            }
        }
    }

    fn friction(&mut self, ui: &mut Ui, friction: &mut Friction) {
        let text = "Reibungskoeffizient mu, größer als null.";
        self.row(ui, ("Reibungskoeffizient", text), |ui| {
            let value = numeric::drag_value(&mut friction.coefficient)
                .range(0.0..=f64::MAX)
                .speed(0.01);
            ui.add_sized(field_size(ui), value)
        });
        let text = "Steigung lambda der Schubspannung über dem Schlupf im Haftbereich; \
                    ohne Angabe wählt CalculiX sie selbst.";
        self.optional(
            ui,
            ("Haftsteigung", text),
            &mut friction.stick_slope,
            1e5,
            Quantity::ForcePerVolume,
        );
    }

    fn gap_conductance(&mut self, ui: &mut Ui, conductance: &mut GapConductance) {
        let text = "Konstanter Leitwert oder Tabelle über Druck und Temperatur.";
        self.row(ui, ("Art", text), |ui| {
            ui.horizontal(|ui| {
                let constant = matches!(conductance, GapConductance::Constant(_));
                let mut response = ui.radio(constant, "Konstant");
                if response.clicked() && !constant {
                    *conductance = GapConductance::Constant(0.0);
                }
                let table = ui.radio(!constant, "Tabelle");
                if table.clicked() && constant {
                    *conductance = GapConductance::Tabular(vec![[0.0; 3]]);
                }
                response |= table;
                response
            })
            .inner
        });
        match conductance {
            GapConductance::Constant(value) => {
                let text = "Wärmestrom je Fläche und Temperaturdifferenz über den Spalt.";
                self.value(
                    ui,
                    ("Leitwert", text),
                    value,
                    Quantity::HeatTransferCoefficient,
                );
            }
            GapConductance::Tabular(rows) => {
                let text = "Je Zeile ein Leitwert mit dem Kontaktdruck und der Temperatur, \
                            für die er gilt.";
                self.table(
                    ui,
                    ("Tabelle", text),
                    rows,
                    [
                        ("Leitwert", Quantity::HeatTransferCoefficient),
                        ("Druck", Quantity::Pressure),
                        ("Temperatur", Quantity::Temperature),
                    ],
                );
            }
        }
    }

    /// A small editable table with a row to add and buttons to remove rows.
    fn table<const N: usize>(
        &mut self,
        ui: &mut Ui,
        description: Description,
        rows: &mut Vec<[f64; N]>,
        columns: [(&str, Quantity); N],
    ) {
        let units = self.units;
        self.row(ui, description, |ui| {
            ui.vertical(|ui| {
                let mut response = ui.response();
                egui::Grid::new(("interaction table", description.1)).show(ui, |ui| {
                    for (title, quantity) in columns {
                        let unit = units.unit(quantity);
                        if unit.is_empty() {
                            ui.strong(title);
                        } else {
                            ui.strong(format!("{title} [{unit}]"));
                        }
                    }
                    ui.end_row();
                    let mut remove = None;
                    let rows_len = rows.len();
                    for (i, row) in rows.iter_mut().enumerate() {
                        for (value, (_, quantity)) in row.iter_mut().zip(columns) {
                            // The unit stands in the column title.
                            let field = numeric::without_unit(value, units, quantity)
                                .speed(0.0)
                                .custom_formatter(|v, _| numeric::format_physical(v));
                            response |= ui.add(field);
                        }
                        if ui
                            .add_enabled(rows_len > 1, egui::Button::new("x").small())
                            .on_hover_text("Zeile entfernen")
                            .clicked()
                        {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                    if let Some(i) = remove.filter(|_| rows.len() > 1) {
                        rows.remove(i);
                    }
                });
                if ui.small_button("Zeile hinzufügen").clicked() {
                    let last = rows.last().copied().unwrap_or([0.0; N]);
                    rows.push(last);
                }
                response
            })
            .inner
        });
    }
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
