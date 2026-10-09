//! PrePoMax's "Search Contact Pairs": finds the surfaces of different parts that touch and
//! turns them into ties or contact pairs. The pairs found are listed in a table; the ones
//! selected there are shown in the 3D view, master in the primary and slave in the secondary
//! highlight colour, and can be edited together, swapped or merged before OK creates them.

use std::collections::BTreeSet;

use egui::Ui;
use plx_mesh::{GroupBy, MasterSlaveItem, SearchParameters, find_contact_pairs, surface_faces};
use plx_model::{
    Constraint, ContactMethod, ContactPair, FeModel, Quantity, Region, Tie, UnitSystem, next_name,
};

use crate::model::{Highlight, Model};
use crate::numeric;

/// What a pair found becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairType {
    Tie,
    Contact,
}

impl PairType {
    fn label(self) -> &'static str {
        match self {
            PairType::Tie => "Tie",
            PairType::Contact => "Contact",
        }
    }
}

/// The methods PrePoMax offers in the search.
const METHODS: [ContactMethod; 2] = [
    ContactMethod::NodeToSurface,
    ContactMethod::SurfaceToSurface,
];

/// A row of the table: a pair found with the settings it is created with.
#[derive(Clone, Debug)]
struct Row {
    name: String,
    item: MasterSlaveItem,
    kind: PairType,
    interaction: String,
    method: ContactMethod,
    adjust: bool,
    distance: f64,
    selected: bool,
    /// The pair is created on OK.
    checked: bool,
}

impl Row {
    fn geometry(&self) -> &'static str {
        if self.item.unresolved {
            "Solid"
        } else {
            "Solid-Solid"
        }
    }
}

pub struct ContactSearchDialog {
    distance: f64,
    angle: f64,
    group_by: GroupBy,
    ignore_hidden: bool,
    kind: PairType,
    interaction: String,
    method: ContactMethod,
    adjust: bool,
    rows: Vec<Row>,
    /// Names of the parts when the search ran, for naming merged pairs.
    part_names: Vec<String>,
    /// The search ran and found nothing.
    searched: bool,
    error: Option<String>,
}

pub enum SearchResult {
    Open,
    /// Create these ties and contact pairs.
    Ok(Vec<Constraint>, Vec<ContactPair>),
    Cancel,
}

impl ContactSearchDialog {
    pub fn new(fe: &FeModel) -> Self {
        Self {
            // PrePoMax's 0.01 mm.
            distance: UnitSystem::MmTonSC.convert(0.01, Quantity::Length, fe.properties.units),
            angle: 35.0,
            group_by: GroupBy::Parts,
            ignore_hidden: true,
            kind: PairType::Tie,
            interaction: (fe.surface_interactions.first())
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            method: ContactMethod::SurfaceToSurface,
            adjust: false,
            rows: Vec::new(),
            part_names: Vec::new(),
            searched: false,
            error: None,
        }
    }

    /// The pairs selected in the table, for the 3D view.
    pub fn highlight(&self, model: &Model) -> Highlight {
        let mut master = BTreeSet::new();
        let mut slave = BTreeSet::new();
        for row in self.rows.iter().filter(|r| r.selected) {
            master.extend(row.item.master.iter().copied());
            if !row.item.unresolved {
                slave.extend(row.item.slave.iter().copied());
            }
        }
        Highlight {
            faces: surface_faces(&model.mesh, model.skins(), &master)
                .into_iter()
                .collect(),
            secondary_faces: surface_faces(&model.mesh, model.skins(), &slave)
                .into_iter()
                .collect(),
            ..Highlight::default()
        }
    }

    fn search(&mut self, model: &Model) {
        let parameters = SearchParameters {
            distance: self.distance,
            angle_deg: self.angle,
            group_by: self.group_by,
            // Like PrePoMax, only ties are checked for surfaces being a slave twice.
            resolve: self.kind == PairType::Tie,
        };
        let searched: Vec<bool> = (model.parts.iter())
            .map(|p| p.visible || !self.ignore_hidden)
            .collect();
        let items = find_contact_pairs(&model.mesh, model.skins(), &searched, &parameters);
        self.rows = items
            .into_iter()
            .map(|item| Row {
                checked: !item.unresolved,
                name: item.name(),
                item,
                kind: self.kind,
                interaction: self.interaction.clone(),
                method: self.method,
                adjust: self.adjust,
                distance: self.distance,
                selected: false,
            })
            .collect();
        self.part_names = model.parts.iter().map(|p| p.name.clone()).collect();
        self.searched = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, model: &Model) -> SearchResult {
        let mut result = SearchResult::Open;
        let mut open = true;
        egui::Window::new("Kontaktpaare suchen")
            .id(egui::Id::new("contact search"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                self.parameters(ui, model);
                ui.separator();
                ui.label("Kontaktpaare");
                ui.horizontal_top(|ui| {
                    let size = egui::vec2((ui.available_width() - 260.0).max(300.0), 340.0);
                    ui.allocate_ui(size, |ui| self.table(ui));
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.set_width(240.0);
                        self.properties(ui, model);
                    });
                });
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 0, 0), error);
                }
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        result = SearchResult::Cancel;
                    }
                    if ui.button("OK").clicked() {
                        match self.create(model) {
                            Ok((ties, pairs)) => result = SearchResult::Ok(ties, pairs),
                            Err(error) => self.error = Some(error),
                        }
                    }
                });
            });
        if !open {
            result = SearchResult::Cancel;
        }
        result
    }

    /// The four groups above the table, as in PrePoMax.
    fn parameters(&mut self, ui: &mut Ui, model: &Model) {
        ui.horizontal_top(|ui| {
            group(ui, "Suchparameter", |ui| {
                egui::Grid::new("search parameters").show(ui, |ui| {
                    ui.label("Abstand");
                    let units = model.fe.properties.units;
                    ui.add(
                        numeric::quantity(&mut self.distance, units, Quantity::Length)
                            .range(0.0..=f64::MAX)
                            .speed(0.001),
                    );
                    ui.end_row();
                    ui.label("Winkel");
                    ui.add(
                        numeric::drag_value(&mut self.angle)
                            .range(0.0..=180.0)
                            .speed(1.0)
                            .suffix(" °"),
                    );
                    ui.end_row();
                    ui.label("Gruppieren nach");
                    egui::ComboBox::from_id_salt("group by")
                        .selected_text(group_label(self.group_by))
                        .show_ui(ui, |ui| {
                            for choice in [GroupBy::None, GroupBy::Parts, GroupBy::Graph] {
                                ui.selectable_value(
                                    &mut self.group_by,
                                    choice,
                                    group_label(choice),
                                );
                            }
                        });
                    ui.end_row();
                });
            });
            group(ui, "Geometriefilter", |ui| {
                let mut solid = true;
                ui.add_enabled(false, egui::Checkbox::new(&mut solid, "Solid"));
                let mut shell = false;
                let unsupported = "Schalenelemente werden noch nicht durchsucht.";
                ui.add_enabled(false, egui::Checkbox::new(&mut shell, "Shell"))
                    .on_disabled_hover_text(unsupported);
                ui.add_enabled(false, egui::Checkbox::new(&mut shell, "Shell edge"))
                    .on_disabled_hover_text(unsupported);
                ui.checkbox(&mut self.ignore_hidden, "Ausgeblendete Parts ignorieren");
            });
            group(ui, "Kontaktpaar-Parameter", |ui| {
                egui::Grid::new("pair parameters").show(ui, |ui| {
                    ui.label("Typ");
                    egui::ComboBox::from_id_salt("pair type")
                        .selected_text(self.kind.label())
                        .show_ui(ui, |ui| {
                            for kind in [PairType::Tie, PairType::Contact] {
                                ui.selectable_value(&mut self.kind, kind, kind.label());
                            }
                        });
                    ui.end_row();
                    let contact = self.kind == PairType::Contact;
                    ui.label("Surface Interaction");
                    ui.add_enabled_ui(contact, |ui| {
                        interaction_combo(ui, "search interaction", &mut self.interaction, model);
                    });
                    ui.end_row();
                    ui.label("Methode");
                    ui.add_enabled_ui(contact, |ui| {
                        method_combo(ui, "search method", &mut self.method);
                    });
                    ui.end_row();
                    ui.label("Netz anpassen");
                    yes_no(ui, "search adjust", &mut self.adjust);
                    ui.end_row();
                });
            });
            ui.vertical(|ui| {
                ui.add_space(70.0);
                if ui
                    .add_sized([90.0, 24.0], egui::Button::new("Suchen"))
                    .clicked()
                {
                    self.error = None;
                    self.search(model);
                }
            });
        });
    }

    fn table(&mut self, ui: &mut Ui) {
        egui::Frame::new()
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .fill(ui.visuals().extreme_bg_color)
            .inner_margin(4.0)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                // Solid scroll bars take their own space instead of covering the last row.
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.vertical(|ui| self.rows_grid(ui));
                });
            });
    }

    fn rows_grid(&mut self, ui: &mut Ui) {
        // Selected rows are filled across all columns, as in PrePoMax's table.
        let selected: Vec<bool> = self.rows.iter().map(|r| r.selected).collect();
        let fill = ui.visuals().selection.bg_fill;
        let text_color = ui.visuals().selection.stroke.color;
        egui::Grid::new("contact pairs")
            .striped(true)
            .min_col_width(16.0)
            .spacing([12.0, 4.0])
            .with_row_color(move |row, _| {
                let index = row.checked_sub(1)?;
                selected.get(index).copied().filter(|&s| s).map(|_| fill)
            })
            .show(ui, |ui| {
                // The header's checkbox checks or unchecks all pairs.
                let resolved = || self.rows.iter().filter(|r| !r.item.unresolved);
                let mut all = resolved().count() > 0 && resolved().all(|r| r.checked);
                let any = resolved().any(|r| r.checked);
                let partial = any && !all;
                let header = egui::Checkbox::new(&mut all, "").indeterminate(partial);
                if ui
                    .add(header)
                    .on_hover_text("Alle Kontaktpaare an- oder abwählen")
                    .changed()
                {
                    for row in self.rows.iter_mut().filter(|r| !r.item.unresolved) {
                        row.checked = all;
                    }
                }
                for title in [
                    "Name",
                    "Geometry",
                    "Type",
                    "Surface interaction",
                    "Method",
                    "Adjust",
                    "Distance",
                ] {
                    ui.strong(title);
                }
                ui.end_row();
                let mut clicked = None;
                let mut menu = None;
                let mut toggled = None;
                for (i, row) in self.rows.iter().enumerate() {
                    let mut checked = row.checked;
                    let checkbox = ui
                        .add_enabled(
                            !row.item.unresolved,
                            egui::Checkbox::without_text(&mut checked),
                        )
                        .on_hover_text("Beim OK erstellen");
                    if checkbox.changed() {
                        toggled = Some((i, checked));
                    }
                    let contact = row.kind == PairType::Contact;
                    let cells = [
                        row.name.clone(),
                        row.geometry().to_string(),
                        if row.item.unresolved {
                            "Unresolved".to_string()
                        } else {
                            row.kind.label().to_string()
                        },
                        if contact {
                            row.interaction.clone()
                        } else {
                            String::new()
                        },
                        if contact {
                            row.method.name().into()
                        } else {
                            String::new()
                        },
                        if row.adjust { "Yes" } else { "No" }.to_string(),
                        format!("{}", row.distance),
                    ];
                    for cell in cells {
                        let mut text = egui::RichText::new(cell);
                        if row.selected {
                            text = text.color(text_color);
                        }
                        let label = egui::Label::new(text).sense(egui::Sense::click());
                        let response = ui.add(label);
                        if response.clicked() {
                            clicked = Some(i);
                        }
                        if response.secondary_clicked() && !row.selected {
                            clicked = Some(i);
                        }
                        response.context_menu(|ui| menu = self.row_menu(ui));
                    }
                    ui.end_row();
                }
                if let Some((i, checked)) = toggled {
                    self.rows[i].checked = checked;
                }
                if let Some(i) = clicked {
                    let (shift, ctrl) =
                        ui.input(|input| (input.modifiers.shift, input.modifiers.command));
                    self.select(i, shift, ctrl);
                }
                match menu {
                    Some(RowAction::Swap) => self.swap(),
                    Some(RowAction::Merge) => self.merge(),
                    None => {}
                }
            });
        if self.rows.is_empty() {
            ui.weak(if self.searched {
                "Keine Kontaktpaare gefunden."
            } else {
                "Mit \"Suchen\" die Kontaktpaare finden."
            });
        }
    }

    fn row_menu(&self, ui: &mut Ui) -> Option<RowAction> {
        let mut action = None;
        if ui.button("Master/Slave tauschen").clicked() {
            action = Some(RowAction::Swap);
        }
        let several = self.rows.iter().filter(|r| r.selected).count() > 1;
        if ui
            .add_enabled(
                several,
                egui::Button::new("Nach Master/Slave zusammenführen"),
            )
            .clicked()
        {
            action = Some(RowAction::Merge);
        }
        action
    }

    /// A click selects a row; Ctrl toggles it, Shift extends to it from the last selected.
    fn select(&mut self, index: usize, shift: bool, ctrl: bool) {
        if ctrl {
            self.rows[index].selected = !self.rows[index].selected;
        } else if shift && let Some(anchor) = self.rows.iter().position(|r| r.selected) {
            let (a, b) = (anchor.min(index), anchor.max(index));
            for (i, row) in self.rows.iter_mut().enumerate() {
                row.selected = (a..=b).contains(&i);
            }
        } else {
            for (i, row) in self.rows.iter_mut().enumerate() {
                row.selected = i == index;
            }
        }
    }

    fn swap(&mut self) {
        for row in self
            .rows
            .iter_mut()
            .filter(|r| r.selected && !r.item.unresolved)
        {
            row.item.swap();
            row.name = row.item.name();
        }
    }

    /// PrePoMax's "Merge by Master/Slave": the selected pairs become one.
    fn merge(&mut self) {
        let selected: Vec<usize> = (self.rows.iter().enumerate())
            .filter(|(_, r)| r.selected && !r.item.unresolved)
            .map(|(i, _)| i)
            .collect();
        let Some((&first, rest)) = selected.split_first() else {
            return;
        };
        for &i in rest {
            let (master, slave) = (
                self.rows[i].item.master.clone(),
                self.rows[i].item.slave.clone(),
            );
            self.rows[first].item.master.extend(master);
            self.rows[first].item.slave.extend(slave);
        }
        let taken: Vec<String> = (self.rows.iter())
            .flat_map(|r| [r.item.master_name.clone(), r.item.slave_name.clone()])
            .collect();
        let row = &mut self.rows[first];
        row.item.master_name = side_name(&row.item.master, &taken, &self.part_names);
        let mut taken = taken;
        taken.push(row.item.master_name.clone());
        row.item.slave_name = side_name(&row.item.slave, &taken, &self.part_names);
        row.name = row.item.name();
        for &i in rest.iter().rev() {
            self.rows.remove(i);
        }
    }

    /// The properties of the selected rows, edited together like PrePoMax's property grid.
    fn properties(&mut self, ui: &mut Ui, model: &Model) {
        let selected: Vec<usize> = (self.rows.iter().enumerate())
            .filter(|(_, r)| r.selected)
            .map(|(i, _)| i)
            .collect();
        let Some(&first) = selected.first() else {
            ui.weak("Ein Kontaktpaar in der Tabelle wählen, um es zu bearbeiten.");
            return;
        };
        let mut row = self.rows[first].clone();
        egui::Grid::new("pair properties")
            .num_columns(2)
            .show(ui, |ui| {
                if selected.len() == 1 {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut row.name);
                    ui.end_row();
                }
                ui.label("Typ");
                egui::ComboBox::from_id_salt("row type")
                    .selected_text(row.kind.label())
                    .show_ui(ui, |ui| {
                        for kind in [PairType::Tie, PairType::Contact] {
                            ui.selectable_value(&mut row.kind, kind, kind.label());
                        }
                    });
                ui.end_row();
                if row.kind == PairType::Contact {
                    ui.label("Surface Interaction");
                    interaction_combo(ui, "row interaction", &mut row.interaction, model);
                    ui.end_row();
                    ui.label("Methode");
                    method_combo(ui, "row method", &mut row.method);
                    ui.end_row();
                }
                ui.label("Adjust");
                yes_no(ui, "row adjust", &mut row.adjust);
                ui.end_row();
                ui.label("Abstand");
                ui.add(
                    numeric::quantity(
                        &mut row.distance,
                        model.fe.properties.units,
                        Quantity::Length,
                    )
                    .range(0.0..=f64::MAX)
                    .speed(0.001),
                );
                ui.end_row();
            });
        ui.add_space(8.0);
        ui.weak(match row.kind {
            PairType::Tie => "Abstand: Positionstoleranz des Ties.",
            PairType::Contact => "Abstand: Bereich, in dem Adjust die Slave-Knoten verschiebt.",
        });
        let old = self.rows[first].clone();
        for &i in &selected {
            let target = &mut self.rows[i];
            if i == first {
                target.name = row.name.clone();
            }
            if row.kind != old.kind {
                target.kind = row.kind;
            }
            if row.interaction != old.interaction {
                target.interaction = row.interaction.clone();
            }
            if row.method != old.method {
                target.method = row.method;
            }
            if row.adjust != old.adjust {
                target.adjust = row.adjust;
            }
            if row.distance != old.distance {
                target.distance = row.distance;
            }
        }
    }

    /// Ties and contact pairs of the rows, with names not yet taken in the model.
    fn create(&self, model: &Model) -> Result<(Vec<Constraint>, Vec<ContactPair>), String> {
        let fe = &model.fe;
        let mut ties = Vec::new();
        let mut pairs = Vec::new();
        let mut tie_names: Vec<String> = fe
            .constraints
            .iter()
            .map(|c| c.name().to_string())
            .collect();
        let mut pair_names: Vec<String> = fe.contact_pairs.iter().map(|c| c.name.clone()).collect();
        let unique = |name: &str, taken: &mut Vec<String>| {
            let name = if taken.iter().any(|t| t.eq_ignore_ascii_case(name)) {
                next_name(name, taken.iter().map(String::as_str))
            } else {
                name.to_string()
            };
            taken.push(name.clone());
            name
        };
        for row in (self.rows.iter()).filter(|r| r.checked && !r.item.unresolved) {
            // Surfaces of whole CAD faces are kept by geometry, so that they survive
            // remeshing.
            let region = |surface| {
                let faces = surface_faces(&model.mesh, model.skins(), surface);
                match model.mesh.whole_cad_faces(&faces) {
                    Some(entities) => Region::Geometry(entities),
                    None => Region::Faces(faces),
                }
            };
            let (master, slave) = (region(&row.item.master), region(&row.item.slave));
            match row.kind {
                PairType::Tie => ties.push(Constraint::Tie(Tie {
                    position_tolerance: Some(row.distance),
                    adjust: row.adjust,
                    master,
                    slave,
                    ..Tie::new(unique(&row.name, &mut tie_names))
                })),
                PairType::Contact => {
                    if !(fe.surface_interactions.iter()).any(|s| s.name == row.interaction) {
                        return Err(format!(
                            "{}: bitte eine Surface Interaction wählen; zuerst unter Surface \
                             Interactions anlegen.",
                            row.name
                        ));
                    }
                    pairs.push(ContactPair {
                        method: row.method,
                        adjust: row.adjust,
                        adjustment_size: Some(row.distance),
                        master,
                        slave,
                        ..ContactPair::new(unique(&row.name, &mut pair_names), &row.interaction)
                    });
                }
            }
        }
        Ok((ties, pairs))
    }
}

enum RowAction {
    Swap,
    Merge,
}

/// Name of a merged side: its part, or the next free "Merged-n".
fn side_name(ids: &BTreeSet<plx_mesh::SurfaceId>, taken: &[String], parts: &[String]) -> String {
    let of: BTreeSet<usize> = ids.iter().map(|id| id.0).collect();
    if let (1, Some(name)) = (of.len(), of.first().and_then(|&p| parts.get(p))) {
        return name.clone();
    }
    (1..)
        .map(|n| format!("Merged-{n}"))
        .find(|name| !taken.contains(name))
        .expect("unbounded range")
}

fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.strong(title);
            body(ui);
        });
    });
}

fn group_label(group_by: GroupBy) -> &'static str {
    match group_by {
        GroupBy::None => "Keine",
        GroupBy::Parts => "Parts",
        GroupBy::Graph => "Graph",
    }
}

fn interaction_combo(ui: &mut Ui, id: &str, interaction: &mut String, model: &Model) {
    let text = if interaction.is_empty() {
        "Fehlt"
    } else {
        interaction.as_str()
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for candidate in &model.fe.surface_interactions {
                let name = candidate.name.clone();
                ui.selectable_value(interaction, name, &candidate.name);
            }
        });
}

fn method_combo(ui: &mut Ui, id: &str, method: &mut ContactMethod) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(method.name())
        .show_ui(ui, |ui| {
            for choice in METHODS {
                ui.selectable_value(method, choice, choice.name());
            }
        });
}

fn yes_no(ui: &mut Ui, id: &str, value: &mut bool) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(if *value { "Ja" } else { "Nein" })
        .show_ui(ui, |ui| {
            ui.selectable_value(value, true, "Ja");
            ui.selectable_value(value, false, "Nein");
        });
}
