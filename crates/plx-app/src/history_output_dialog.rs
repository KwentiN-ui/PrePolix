//! PrePoMax's "Create History Output" dialog of the Results tree: values of a field at
//! nodes over all increments, an equation of other history outputs, or element sizes.

use egui::{RichText, Ui};
use plx_model::next_name;
use plx_results::history_output::{HistoryOutput, HistoryOutputKind, SizeKind};

use crate::field_output_dialog::DialogAction;
use crate::keywords::{frame, tree_row};
use crate::model::{Highlight, Hit, Model};
use crate::results::ResultsView;
use crate::selection::{Operation, Picker, Target};
use crate::setup::{FACE_SOURCES, NODE_SOURCES, RegionDraft, SOLID_SOURCES, Source};
use crate::viewport::{BoxSelect, Preview};

/// The types of the list, in PrePoMax's order, with the prefix of their default names.
const TYPES: [(&str, &str); 3] = [
    ("Aus Feldausgabe (From Field Output)", "From_Field"),
    (
        "Per Gleichung (From History Output by Equation)",
        "From_Equation",
    ),
    ("Elementgröße (From Element Size)", "From_Element_Size"),
];
const FROM_FIELD: usize = 0;
const EQUATION: usize = 1;
const SIZE: usize = 2;

const NAME: &str = "Name";
const REGION: &str = "Region";
const FIELD: &str = "Feld";
const COMPONENTS: &str = "Komponenten";
const EQUATION_ROW: &str = "Gleichung";
const UNIT: &str = "Einheit";
const SIZE_ROW: &str = "Größe";

fn description(row: &str) -> &'static str {
    match row {
        NAME => "Name der History-Ausgabe.",
        REGION => {
            "Knoten, deren Werte über alle Inkremente ausgegeben werden. Bei \"Auswahl im \
             3D-Fenster\" wird im verformten Modell gewählt."
        }
        FIELD => "Feld, aus dem die Werte stammen.",
        COMPONENTS => "Komponenten des Feldes, die ausgegeben werden.",
        EQUATION_ROW => {
            "Beispiel: =[From_Field-1.DISP.U3] * 2\nEine Komponente wird mit \
             History-Ausgabe.Feld.Komponente angegeben, Namen mit Bindestrich in eckigen \
             Klammern. Die Gleichung wird für jeden Eintrag und jedes Inkrement ausgewertet; \
             alle Komponenten brauchen gleich viele Einträge."
        }
        UNIT => "Benutzerdefinierte Einheit der History-Ausgabe.",
        SIZE_ROW => {
            "Volumen der Elemente oder Fläche der Elementflächen, je Inkrement am verformten \
             Netz (echter Maßstab)."
        }
        _ => "",
    }
}

pub struct HistoryOutputDialog {
    /// Index of the edited output; `None` creates a new one.
    pub edit: Option<usize>,
    kind: usize,
    names: [String; 3],
    nodes: RegionDraft,
    field: String,
    components: Vec<String>,
    equation: String,
    unit: String,
    size: SizeKind,
    elements: RegionDraft,
    focus: &'static str,
    pub error: Option<String>,
    picker: Picker,
    /// Fields of the results with their components.
    fields: Vec<(String, Vec<String>)>,
    /// `Set.Field.Component` of the history outputs an equation may use.
    history: Vec<String>,
    taken: Vec<String>,
}

impl HistoryOutputDialog {
    pub fn create(view: &ResultsView) -> Self {
        let fields: Vec<(String, Vec<String>)> = view
            .current_increment()
            .map(|inc| {
                (inc.fields.iter())
                    .filter(|f| !f.components.is_empty())
                    .map(|f| {
                        let components = f.components.iter().map(|c| c.name.clone()).collect();
                        (f.name.clone(), components)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let taken: Vec<String> = view
            .history_outputs
            .iter()
            .map(|o| o.name.clone())
            .collect();
        let history = view
            .history
            .iter()
            .flat_map(|set| {
                set.fields.iter().flat_map(move |f| {
                    (f.components.iter())
                        .map(move |c| format!("{}.{}.{}", set.name, f.name, c.name))
                })
            })
            .collect();
        let (field, components) = fields
            .iter()
            .find(|(f, _)| f == "DISP")
            .or(fields.first())
            .map(|(f, c)| (f.clone(), vec![c[0].clone()]))
            .unwrap_or_default();
        let names = TYPES.map(|(_, prefix)| next_name(prefix, taken.iter().map(String::as_str)));
        Self {
            edit: None,
            kind: FROM_FIELD,
            names,
            nodes: RegionDraft::new(NODE_SOURCES, Target::Nodes),
            field,
            components,
            equation: "=".into(),
            unit: "/".into(),
            size: SizeKind::Volume,
            elements: RegionDraft::new(SOLID_SOURCES, Target::Faces),
            focus: NAME,
            error: None,
            picker: Picker::default(),
            fields,
            history,
            taken,
        }
    }

    pub fn edit(view: &ResultsView, mesh: &plx_mesh::FeMesh, index: usize) -> Option<Self> {
        let output = view.history_outputs.get(index)?.clone();
        let mut dialog = Self::create(view);
        dialog.edit = Some(index);
        dialog.taken.retain(|n| *n != output.name);
        // An equation may only use the outputs before it.
        let before: Vec<&str> = (view.history_outputs[..index].iter())
            .map(|o| o.name.as_str())
            .collect();
        dialog
            .history
            .retain(|h| before.iter().any(|b| h.starts_with(&format!("{b}."))));
        dialog.kind = match output.kind {
            HistoryOutputKind::FromField {
                region,
                field,
                components,
            } => {
                dialog.nodes = RegionDraft::from_region(&region, NODE_SOURCES, Target::Nodes, mesh);
                dialog.field = field;
                dialog.components = components;
                FROM_FIELD
            }
            HistoryOutputKind::FromEquation { equation, unit } => {
                dialog.equation = equation;
                dialog.unit = unit;
                EQUATION
            }
            HistoryOutputKind::ElementSize { region, kind } => {
                dialog.size = kind;
                let sources = match kind {
                    SizeKind::Volume => SOLID_SOURCES,
                    SizeKind::Area => FACE_SOURCES,
                };
                dialog.elements = RegionDraft::from_region(&region, sources, Target::Faces, mesh);
                SIZE
            }
        };
        dialog.names[dialog.kind] = output.name;
        Some(dialog)
    }

    pub fn title(&self) -> &'static str {
        if self.edit.is_some() {
            "History-Ausgabe bearbeiten"
        } else {
            "History-Ausgabe erstellen"
        }
    }

    /// The region being picked in the 3D view, if any.
    fn region(&self) -> Option<&RegionDraft> {
        match self.kind {
            FROM_FIELD => Some(&self.nodes),
            SIZE => Some(&self.elements),
            _ => None,
        }
    }

    fn region_mut(&mut self) -> Option<&mut RegionDraft> {
        match self.kind {
            FROM_FIELD => Some(&mut self.nodes),
            SIZE => Some(&mut self.elements),
            _ => None,
        }
    }

    /// Whether clicks in the 3D view pick for this dialog.
    pub fn picks(&self) -> bool {
        self.region()
            .is_some_and(|r| matches!(r.source, Source::Selection | Source::Parts))
    }

    pub fn click(&mut self, model: &Model, pick: Option<(&Hit, f32)>, operation: Operation) {
        let picker = std::mem::take(&mut self.picker);
        if let Some(region) = self.region_mut() {
            region.click(model, &picker, pick, operation);
        }
        self.picker = picker;
    }

    pub fn box_select(&mut self, model: &Model, area: &BoxSelect, operation: Operation) {
        let picker = std::mem::take(&mut self.picker);
        if let Some(region) = self.region_mut() {
            region.box_select(model, &picker, area, operation);
        }
        self.picker = picker;
    }

    pub fn preview(&self, model: &Model, hit: &Hit, precision: f32) -> Preview {
        self.region()
            .map(|r| r.preview(model, &self.picker, hit, precision))
            .unwrap_or_default()
    }

    pub fn highlight(&self, model: &Model) -> Highlight {
        self.region()
            .map_or_else(Highlight::default, |r| r.highlight(model))
    }

    /// The output as the dialog defines it, or why it cannot be created.
    pub fn output(&self) -> Result<HistoryOutput, String> {
        let name = self.names[self.kind].trim().to_string();
        if name.is_empty() {
            return Err("Bitte einen Namen eingeben.".into());
        }
        if self.taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
            return Err(format!("Der Name {name} ist schon vergeben."));
        }
        if name.contains('.') {
            return Err("Der Name darf keinen Punkt enthalten.".into());
        }
        if self.region().is_some_and(RegionDraft::is_empty) {
            return Err("Die Region ist leer.".into());
        }
        let kind = match self.kind {
            FROM_FIELD => {
                if self.fields.is_empty() {
                    return Err(
                        "Es gibt keine Feldausgaben, aus denen Werte stammen könnten.".into(),
                    );
                }
                HistoryOutputKind::FromField {
                    region: self.nodes.region(),
                    field: self.field.clone(),
                    components: self.components.clone(),
                }
            }
            EQUATION => HistoryOutputKind::FromEquation {
                equation: self.equation.clone(),
                unit: self.unit.clone(),
            },
            _ => HistoryOutputKind::ElementSize {
                region: self.elements.region(),
                kind: self.size,
            },
        };
        Ok(HistoryOutput { name, kind })
    }

    /// Prepares the dialog for the next output after OK - Neu.
    pub fn next(&mut self, created: &HistoryOutput, view: &ResultsView) {
        self.taken.push(created.name.clone());
        self.names =
            TYPES.map(|(_, prefix)| next_name(prefix, self.taken.iter().map(String::as_str)));
        if let Some(set) = view.history.iter().find(|s| s.name == created.name) {
            for f in &set.fields {
                for c in &f.components {
                    self.history
                        .push(format!("{}.{}.{}", set.name, f.name, c.name));
                }
            }
        }
        self.error = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, model: &Model) -> DialogAction<HistoryOutput> {
        let mut action = DialogAction::Open;
        let mut open = true;
        let window = egui::Window::new(self.title())
            .id(egui::Id::new("history output dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                ui.set_width(500.0);
                ui.label("Typ");
                frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(58.0);
                    for (index, (label, _)) in TYPES.iter().enumerate() {
                        let selected = self.kind == index;
                        let row = ui.add_enabled_ui(self.edit.is_none() || selected, |ui| {
                            tree_row(ui, 0, None, selected, RichText::new(*label))
                        });
                        if row.inner.clicked() && !selected {
                            self.kind = index;
                            self.focus = NAME;
                            self.error = None;
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label("Eigenschaften");
                frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(240.0);
                    self.properties(ui, model);
                });
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(76.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.strong(self.focus);
                        ui.label(description(self.focus));
                    });
                });
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 0, 0), error);
                }
                ui.add_space(4.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        action = DialogAction::Cancel;
                    }
                    let ok = ui.button("OK").clicked();
                    let next = self.edit.is_none() && ui.button("OK - Neu").clicked();
                    if ok || next {
                        match self.output() {
                            Ok(output) => action = DialogAction::Ok { output, next },
                            Err(error) => self.error = Some(error),
                        }
                    }
                });
            });
        if let Some(window) = window
            && let Some(region) = self.region()
            && region.source == Source::Selection
        {
            let (target, can_undo) = (region.target, region.can_undo());
            let picked = self
                .picker
                .window(ctx, window.response.rect, target, can_undo);
            if let (Some(picked), Some(region)) = (picked, self.region_mut()) {
                region.action(model, picked);
            }
        }
        if !open {
            action = DialogAction::Cancel;
        }
        action
    }

    fn properties(&mut self, ui: &mut Ui, model: &Model) {
        let header = egui::Frame::new()
            .fill(crate::style::CONTROL)
            .inner_margin(egui::Margin::symmetric(4, 1));
        header.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Daten");
        });
        let focus = &mut self.focus;
        egui::Grid::new("history output properties")
            .num_columns(2)
            .striped(true)
            .min_col_width(100.0)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                row(ui, focus, NAME, |ui| {
                    let edit =
                        egui::TextEdit::singleline(&mut self.names[self.kind]).desired_width(200.0);
                    ui.add(edit).has_focus()
                });
                match self.kind {
                    FROM_FIELD => {
                        // The region editor writes its own row.
                        let before = self.nodes.clone();
                        self.nodes.ui(ui, model);
                        if self.nodes != before {
                            *focus = REGION;
                        }
                        let fields = &self.fields;
                        let field = &mut self.field;
                        let components = &mut self.components;
                        row(ui, focus, FIELD, |ui| {
                            let mut changed = false;
                            let response = egui::ComboBox::from_id_salt("history field")
                                .selected_text(field.as_str())
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    for (name, _) in fields {
                                        changed |= ui
                                            .selectable_value(field, name.clone(), name)
                                            .changed();
                                    }
                                });
                            if changed {
                                let available = fields.iter().find(|(f, _)| f == field);
                                *components = available
                                    .map(|(_, c)| vec![c[0].clone()])
                                    .unwrap_or_default();
                            }
                            response.response.clicked()
                        });
                        row(ui, focus, COMPONENTS, |ui| {
                            let available = fields
                                .iter()
                                .find(|(f, _)| f == field)
                                .map_or(&[][..], |(_, c)| c.as_slice());
                            let mut clicked = false;
                            ui.vertical(|ui| {
                                for name in available {
                                    let mut on = components.contains(name);
                                    if ui.checkbox(&mut on, name).changed() {
                                        clicked = true;
                                        components.retain(|c| c != name);
                                        if on {
                                            components.push(name.clone());
                                            // Keep the order of the field.
                                            components.sort_by_key(|c| {
                                                available.iter().position(|a| a == c)
                                            });
                                        }
                                    }
                                }
                            });
                            clicked
                        });
                    }
                    EQUATION => {
                        row(ui, focus, EQUATION_ROW, |ui| {
                            let edit =
                                egui::TextEdit::singleline(&mut self.equation).desired_width(200.0);
                            ui.add(edit).has_focus()
                        });
                        row(ui, focus, UNIT, |ui| {
                            let edit =
                                egui::TextEdit::singleline(&mut self.unit).desired_width(200.0);
                            ui.add(edit).has_focus()
                        });
                    }
                    _ => {
                        let size = &mut self.size;
                        let elements = &mut self.elements;
                        row(ui, focus, SIZE_ROW, |ui| {
                            let mut clicked = false;
                            ui.horizontal(|ui| {
                                for (kind, label) in
                                    [(SizeKind::Volume, "Volumen"), (SizeKind::Area, "Fläche")]
                                {
                                    if ui.radio(*size == kind, label).clicked() && *size != kind {
                                        *size = kind;
                                        clicked = true;
                                        let sources = match kind {
                                            SizeKind::Volume => SOLID_SOURCES,
                                            SizeKind::Area => FACE_SOURCES,
                                        };
                                        *elements = RegionDraft::new(sources, Target::Faces);
                                    }
                                }
                            });
                            clicked
                        });
                        self.elements.ui(ui, model);
                    }
                }
            });
        if self.kind == EQUATION {
            ui.add_space(4.0);
            let equation = &mut self.equation;
            ui.add_enabled_ui(!self.history.is_empty(), |ui| {
                ui.menu_button("Komponente einfügen …", |ui| {
                    for name in &self.history {
                        if ui.button(name).clicked() {
                            equation.push_str(&format!("[{name}]"));
                            ui.close();
                        }
                    }
                });
            });
            if self.history.is_empty() {
                ui.weak("Es gibt noch keine History-Ausgaben, die die Gleichung verwenden kann.");
            }
        }
    }
}

/// One row of the property grid; clicking its label or using its editor focuses it.
fn row(
    ui: &mut Ui,
    focus: &mut &'static str,
    label: &'static str,
    editor: impl FnOnce(&mut Ui) -> bool,
) {
    let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
    if editor(ui) || response.clicked() {
        *focus = label;
    }
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_model::Region;

    fn results() -> Model {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/kragbalken_c3d8.frd");
        crate::model::load(&path).unwrap().model
    }

    #[test]
    fn values_at_picked_nodes_then_an_equation_of_them() {
        let mut model = results();
        let view = model.results.as_ref().unwrap();
        let mut dialog = HistoryOutputDialog::create(view);
        assert_eq!(dialog.names[FROM_FIELD], "From_Field-1");
        assert_eq!(dialog.field, "DISP");
        assert!(dialog.output().is_err(), "nothing picked yet");
        dialog.nodes = RegionDraft::from_region(
            &Region::Nodes(vec![1, 99]),
            NODE_SOURCES,
            Target::Nodes,
            &model.mesh,
        );
        let output = dialog.output().unwrap();
        let mesh = model.mesh.clone();
        let view = model.results.as_mut().unwrap();
        view.set_history_output(None, output.clone(), &mesh)
            .unwrap();
        assert_eq!(view.history[0].fields[0].components[0].entries.len(), 2);

        dialog.next(&output, view);
        dialog.kind = EQUATION;
        assert_eq!(dialog.names[EQUATION], "From_Equation-1");
        assert_eq!(dialog.history, ["From_Field-1.DISP.ALL"]);
        dialog.equation = "=[From_Field-1.DISP.ALL] * 2".into();
        view.set_history_output(None, dialog.output().unwrap(), &mesh)
            .unwrap();
        assert_eq!(view.history.len(), 2);

        // Editing the first one keeps its region; deleting it breaks the equation.
        let edit = HistoryOutputDialog::edit(view, &mesh, 0).unwrap();
        assert_eq!(edit.nodes.region(), Region::Nodes(vec![1, 99]));
        assert!(edit.history.is_empty());
        let warnings = view.remove_history_output(0, &mesh);
        assert_eq!(warnings.len(), 1);
        assert!(view.history.is_empty());
    }
}
