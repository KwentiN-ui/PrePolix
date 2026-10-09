//! PrePoMax's "Create Field Output" dialog of the Results tree: a type list, the properties
//! of the chosen type with a description below, and for limits a table of limit values.

use egui::{RichText, Ui};
use plx_mesh::FeMesh;
use plx_model::next_name;
use plx_results::field_output::{FieldOutput, FieldOutputKind, LimitBasis};

use crate::keywords::{frame, tree_row};
use crate::numeric;
use crate::results::ResultsView;

/// The types of the list, in PrePoMax's order, with the prefix of their default names.
const TYPES: [(&str, &str); 4] = [
    ("Grenzwert (Limit)", "Limit"),
    ("Einhüllende (Envelope)", "Envelope"),
    ("Gleichung (Equation)", "Equation"),
    ("Koordinatensystem-Transformation", "Transform"),
];
const LIMIT: usize = 0;
const ENVELOPE: usize = 1;
const EQUATION: usize = 2;
const TRANSFORM: usize = 3;

const NO_FIELDS: &str = "Es gibt keine Feldausgaben oder Komponenten, aus denen eine Feldausgabe \
                         erstellt werden kann.";
const NO_COORDINATE_SYSTEM: &str = "Es ist kein Koordinatensystem definiert, in das die \
                                    Feldausgabe transformiert werden kann.";

const LIMIT_VALUES: &str = "Grenzwert jedes Parts oder Elementsets, z. B. die Streckgrenze. \
                            Er muss ungleich 0 sein.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Properties,
    LimitValues,
}

/// Rows of the property grid; the focused one is described below the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Name,
    Field,
    Component,
    Basis,
    Equation,
    Unit,
}

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Name => "Name",
            Row::Field => "Feld",
            Row::Component => "Komponente",
            Row::Basis => "Grenzwert bezogen auf",
            Row::Equation => "Gleichung",
            Row::Unit => "Einheit",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Row::Name => "Name der Feldausgabe.",
            Row::Field => "Feld, aus dem die Feldausgabe berechnet wird.",
            Row::Component => "Komponente, aus der die Feldausgabe berechnet wird.",
            Row::Basis => {
                "Wofür die Grenzwerte angegeben werden: je Part, je Elementset oder einer für \
                 alle Elemente. Liegt ein Knoten in mehreren, gilt der kleinste Grenzwert. \
                 Berechnet werden RATIO = Wert / Grenzwert und SAFETY_FACTOR = Grenzwert / Wert."
            }
            Row::Equation => {
                "Beispiel: =DISP.ALL\nEine Komponente wird mit ihrem vollen Namen \
                 Feldname.Komponente angegeben, Namen mit Bindestrich in eckigen Klammern: \
                 [Limit-1.RATIO]. Groß- und Kleinschreibung zählt. Funktionen wie Sqrt, Abs, \
                 Max, Min, Pow und If stehen zur Verfügung."
            }
            Row::Unit => "Benutzerdefinierte Einheit der Feldausgabe, in der Legende angezeigt.",
        }
    }
}

/// What the user did in the dialog this frame.
pub enum DialogAction<T> {
    Open,
    Cancel,
    /// Create or replace the output; `next` keeps the dialog open for another one (OK - Neu).
    Ok {
        output: T,
        next: bool,
    },
}

pub struct FieldOutputDialog {
    /// Index of the edited output; `None` creates a new one.
    pub edit: Option<usize>,
    kind: usize,
    /// One draft per type, as PrePoMax keeps the values of each type while switching.
    drafts: [FieldOutput; 4],
    /// Limit values per part, per element set and for all elements.
    part_limits: Vec<(String, f64)>,
    set_limits: Vec<(String, f64)>,
    all_limit: f64,
    tab: Tab,
    focus: Row,
    pub error: Option<String>,
    /// Fields of the results with their components, without the edited output.
    fields: Vec<(String, Vec<String>)>,
    /// Names a new output may not take.
    taken: Vec<String>,
}

impl FieldOutputDialog {
    pub fn create(view: &ResultsView, mesh: &FeMesh) -> Self {
        Self::new(view, mesh, None)
    }

    pub fn edit(view: &ResultsView, mesh: &FeMesh, index: usize) -> Option<Self> {
        let output = view.field_outputs.get(index)?.clone();
        let mut dialog = Self::new(view, mesh, Some(index));
        dialog.kind = match &output.kind {
            FieldOutputKind::Limit {
                basis,
                limits,
                field,
                component,
            } => {
                let given = |item: &str| limits.iter().find(|(n, _)| n == item).map(|l| l.1);
                match basis {
                    LimitBasis::Parts => (dialog.part_limits.iter_mut())
                        .for_each(|(n, l)| *l = given(n).unwrap_or(*l)),
                    LimitBasis::ElementSets => (dialog.set_limits.iter_mut())
                        .for_each(|(n, l)| *l = given(n).unwrap_or(*l)),
                    LimitBasis::AllElements => {
                        dialog.all_limit = limits.first().map_or(0.0, |l| l.1);
                    }
                }
                // The edited output's own field is not offered, but its source must be.
                dialog.offer(field, component);
                LIMIT
            }
            FieldOutputKind::Envelope { field, component } => {
                dialog.offer(field, component);
                ENVELOPE
            }
            FieldOutputKind::Equation { .. } => EQUATION,
            FieldOutputKind::CoordinateSystemTransform { .. } => TRANSFORM,
        };
        dialog.drafts[dialog.kind] = output;
        Some(dialog)
    }

    fn new(view: &ResultsView, mesh: &FeMesh, edit: Option<usize>) -> Self {
        let own = edit
            .and_then(|i| view.field_outputs.get(i))
            .map(|o| o.name.clone());
        let fields: Vec<(String, Vec<String>)> = view
            .current_increment()
            .map(|inc| {
                (inc.fields.iter())
                    .filter(|f| Some(&f.name) != own.as_ref() && !f.components.is_empty())
                    .map(|f| {
                        let components = f.components.iter().map(|c| c.name.clone()).collect();
                        (f.name.clone(), components)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut taken: Vec<String> = fields.iter().map(|(f, _)| f.clone()).collect();
        taken.extend(
            (view.field_outputs.iter())
                .map(|o| o.name.clone())
                .filter(|n| Some(n) != own.as_ref()),
        );
        // PrePoMax proposes STRESS.MISES, else the first component there is.
        let (field, component) = fields
            .iter()
            .find(|(f, c)| f == "STRESS" && c.iter().any(|c| c == "MISES"))
            .map(|_| ("STRESS".to_string(), "MISES".to_string()))
            .or_else(|| fields.first().map(|(f, c)| (f.clone(), c[0].clone())))
            .unwrap_or_default();
        let name = |prefix: &str| next_name(prefix, taken.iter().map(String::as_str));
        let drafts = [
            FieldOutput {
                name: name(TYPES[LIMIT].1),
                kind: FieldOutputKind::Limit {
                    field: field.clone(),
                    component: component.clone(),
                    basis: LimitBasis::Parts,
                    limits: Vec::new(),
                },
            },
            FieldOutput {
                name: name(TYPES[ENVELOPE].1),
                kind: FieldOutputKind::Envelope {
                    field: field.clone(),
                    component: component.clone(),
                },
            },
            FieldOutput {
                name: name(TYPES[EQUATION].1),
                kind: FieldOutputKind::Equation {
                    equation: "=".into(),
                    unit: "/".into(),
                },
            },
            FieldOutput {
                name: name(TYPES[TRANSFORM].1),
                kind: FieldOutputKind::CoordinateSystemTransform {
                    field,
                    coordinate_system: String::new(),
                },
            },
        ];
        Self {
            edit,
            kind: LIMIT,
            drafts,
            part_limits: mesh.parts.iter().map(|p| (p.name.clone(), 0.0)).collect(),
            set_limits: mesh.element_sets.keys().map(|n| (n.clone(), 0.0)).collect(),
            all_limit: 0.0,
            tab: Tab::Properties,
            focus: Row::Name,
            error: None,
            fields,
            taken,
        }
    }

    /// Keeps a source that is no longer among the fields selectable, e.g. after a rename.
    fn offer(&mut self, field: &str, component: &str) {
        match self.fields.iter_mut().find(|(f, _)| f == field) {
            Some((_, components)) if !components.iter().any(|c| c == component) => {
                components.push(component.to_string());
            }
            Some(_) => {}
            None => (self.fields).push((field.to_string(), vec![component.to_string()])),
        }
    }

    pub fn title(&self) -> &'static str {
        if self.edit.is_some() {
            "Feldausgabe bearbeiten"
        } else {
            "Feldausgabe erstellen"
        }
    }

    /// Why the chosen type cannot be created, shown instead of its properties.
    fn unavailable(&self) -> Option<&'static str> {
        if self.fields.is_empty() {
            Some(NO_FIELDS)
        } else if self.kind == TRANSFORM {
            // prepolix has no coordinate systems yet.
            Some(NO_COORDINATE_SYSTEM)
        } else {
            None
        }
    }

    /// The output as the dialog defines it, or why it cannot be created.
    pub fn output(&self) -> Result<FieldOutput, String> {
        if let Some(reason) = self.unavailable() {
            return Err(reason.into());
        }
        let mut output = self.drafts[self.kind].clone();
        output.name = output.name.trim().to_string();
        if output.name.is_empty() {
            return Err("Bitte einen Namen eingeben.".into());
        }
        if self
            .taken
            .iter()
            .any(|t| t.eq_ignore_ascii_case(&output.name))
        {
            return Err(format!("Der Name {} ist schon vergeben.", output.name));
        }
        if output.name.contains('.') {
            return Err("Der Name darf keinen Punkt enthalten.".into());
        }
        if let FieldOutputKind::Limit { basis, limits, .. } = &mut output.kind {
            *limits = match basis {
                LimitBasis::Parts => self.part_limits.clone(),
                LimitBasis::ElementSets => self.set_limits.clone(),
                LimitBasis::AllElements => {
                    vec![(LimitBasis::AllElements.name().to_string(), self.all_limit)]
                }
            };
            if limits.is_empty() {
                return Err("Es gibt keine Elementsets, für die Grenzwerte gelten könnten.".into());
            }
            if let Some((item, _)) = limits.iter().find(|(_, l)| *l == 0.0) {
                return Err(format!(
                    "Bitte unter Grenzwerte einen Grenzwert ungleich 0 für {item} eingeben."
                ));
            }
        }
        Ok(output)
    }

    /// Prepares the dialog for the next output after OK - Neu: the created name is taken
    /// and every type proposes its next free name.
    pub fn next(&mut self, created: &str) {
        self.taken.push(created.to_string());
        for (draft, (_, prefix)) in self.drafts.iter_mut().zip(TYPES) {
            draft.name = next_name(prefix, self.taken.iter().map(String::as_str));
        }
        self.error = None;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> DialogAction<FieldOutput> {
        let mut action = DialogAction::Open;
        let mut open = true;
        egui::Window::new(self.title())
            .id(egui::Id::new("field output dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            // Anchored at the top, so that a longer description or an error never moves it.
            .default_pos(ctx.content_rect().center() - egui::vec2(175.0, 290.0))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label("Typ");
                frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(76.0);
                    for (index, (label, _)) in TYPES.iter().enumerate() {
                        let selected = self.kind == index;
                        let row = ui.add_enabled_ui(self.edit.is_none() || selected, |ui| {
                            tree_row(ui, 0, None, selected, RichText::new(*label))
                        });
                        if row.inner.clicked() && !selected {
                            self.kind = index;
                            self.tab = Tab::Properties;
                            self.focus = Row::Name;
                            self.error = None;
                        }
                    }
                });
                ui.add_space(6.0);
                self.tabs(ui);
                frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(220.0);
                    match self.tab {
                        Tab::Properties => self.properties(ui),
                        Tab::LimitValues => self.limit_values(ui),
                    }
                });
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(76.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.description(ui);
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
        if !open {
            action = DialogAction::Cancel;
        }
        action
    }

    /// PrePoMax's help box below the property grid: what the focused row means.
    fn description(&self, ui: &mut Ui) {
        if self.unavailable().is_some() {
            return;
        }
        if self.tab == Tab::LimitValues {
            ui.strong("Grenzwerte");
            ui.label(LIMIT_VALUES);
        } else {
            ui.strong(self.focus.label());
            ui.label(self.focus.description());
        }
    }

    fn tabs(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut tabs = vec![(Tab::Properties, "Eigenschaften")];
            if self.kind == LIMIT && self.unavailable().is_none() {
                tabs.push((Tab::LimitValues, "Grenzwerte"));
            }
            for (tab, label) in tabs {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    self.tab = tab;
                }
            }
        });
    }

    fn properties(&mut self, ui: &mut Ui) {
        if let Some(reason) = self.unavailable() {
            ui.label(reason);
            return;
        }
        // Category header like PrePoMax's property grid.
        let header = egui::Frame::new()
            .fill(crate::style::CONTROL)
            .inner_margin(egui::Margin::symmetric(4, 1));
        header.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Daten");
        });
        let fields = &self.fields;
        let focus = &mut self.focus;
        let draft = &mut self.drafts[self.kind];
        egui::Grid::new("field output properties")
            .num_columns(2)
            .striped(true)
            .min_col_width(130.0)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                prop_row(ui, focus, Row::Name, |ui| {
                    let edit = egui::TextEdit::singleline(&mut draft.name).desired_width(180.0);
                    ui.add(edit).has_focus()
                });
                match &mut draft.kind {
                    FieldOutputKind::Limit {
                        field,
                        component,
                        basis,
                        ..
                    } => {
                        source_rows(ui, focus, fields, field, component);
                        prop_row(ui, focus, Row::Basis, |ui| {
                            let response = egui::ComboBox::from_id_salt("limit basis")
                                .selected_text(basis_label(*basis))
                                .width(180.0)
                                .show_ui(ui, |ui| {
                                    for choice in LimitBasis::ALL {
                                        ui.selectable_value(basis, choice, basis_label(choice));
                                    }
                                });
                            response.response.clicked()
                        });
                    }
                    FieldOutputKind::Envelope { field, component } => {
                        source_rows(ui, focus, fields, field, component);
                    }
                    FieldOutputKind::Equation { equation, unit } => {
                        prop_row(ui, focus, Row::Equation, |ui| {
                            let edit = egui::TextEdit::singleline(equation).desired_width(180.0);
                            ui.add(edit).has_focus()
                        });
                        prop_row(ui, focus, Row::Unit, |ui| {
                            let edit = egui::TextEdit::singleline(unit).desired_width(180.0);
                            ui.add(edit).has_focus()
                        });
                    }
                    FieldOutputKind::CoordinateSystemTransform { .. } => {}
                }
            });
        if let FieldOutputKind::Equation { equation, .. } = &mut draft.kind {
            ui.add_space(4.0);
            ui.menu_button("Komponente einfügen …", |ui| {
                insert_menu(ui, fields, equation);
            });
        }
    }

    fn limit_values(&mut self, ui: &mut Ui) {
        let FieldOutputKind::Limit { basis, .. } = &self.drafts[LIMIT].kind else {
            return;
        };
        let (header, limits): (&str, Vec<(String, &mut f64)>) = match basis {
            LimitBasis::Parts => (
                "Part",
                (self.part_limits.iter_mut())
                    .map(|(n, l)| (n.clone(), l))
                    .collect(),
            ),
            LimitBasis::ElementSets => (
                "Elementset",
                (self.set_limits.iter_mut())
                    .map(|(n, l)| (n.clone(), l))
                    .collect(),
            ),
            LimitBasis::AllElements => (
                "Elemente",
                vec![(
                    basis_label(LimitBasis::AllElements).into(),
                    &mut self.all_limit,
                )],
            ),
        };
        if limits.is_empty() {
            ui.weak("Das Ergebnis enthält keine Elementsets.");
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(210.0)
            .show(ui, |ui| {
                egui::Grid::new("limit values")
                    .num_columns(2)
                    .striped(true)
                    .min_col_width(150.0)
                    .show(ui, |ui| {
                        ui.strong(header);
                        ui.strong("Grenzwert");
                        ui.end_row();
                        for (name, limit) in limits {
                            ui.label(name);
                            let height = ui.spacing().interact_size.y;
                            ui.add_sized([120.0, height], numeric::drag_value(limit).speed(1.0));
                            ui.end_row();
                        }
                    });
            });
    }
}

fn basis_label(basis: LimitBasis) -> &'static str {
    match basis {
        LimitBasis::Parts => "Parts",
        LimitBasis::ElementSets => "Elementsets",
        LimitBasis::AllElements => "Alle Elemente",
    }
}

/// One row of the property grid; clicking its label or using its editor focuses it.
fn prop_row(ui: &mut Ui, focus: &mut Row, row: Row, editor: impl FnOnce(&mut Ui) -> bool) {
    let label = ui.add(egui::Label::new(row.label()).sense(egui::Sense::click()));
    if editor(ui) || label.clicked() {
        *focus = row;
    }
    ui.end_row();
}

/// Field and component rows; changing the field keeps the component if the field has it.
fn source_rows(
    ui: &mut Ui,
    focus: &mut Row,
    fields: &[(String, Vec<String>)],
    field: &mut String,
    component: &mut String,
) {
    prop_row(ui, focus, Row::Field, |ui| {
        let mut changed = false;
        let response = egui::ComboBox::from_id_salt("field output field")
            .selected_text(field.as_str())
            .width(180.0)
            .show_ui(ui, |ui| {
                for (name, _) in fields {
                    changed |= ui.selectable_value(field, name.clone(), name).changed();
                }
            });
        if changed
            && let Some((_, components)) = fields.iter().find(|(f, _)| f == field)
            && !components.contains(component)
        {
            *component = components[0].clone();
        }
        response.response.clicked()
    });
    prop_row(ui, focus, Row::Component, |ui| {
        let components = fields
            .iter()
            .find(|(f, _)| f == field)
            .map_or(&[][..], |(_, c)| c.as_slice());
        let response = egui::ComboBox::from_id_salt("field output component")
            .selected_text(component.as_str())
            .width(180.0)
            .show_ui(ui, |ui| {
                for name in components {
                    ui.selectable_value(component, name.clone(), name);
                }
            });
        response.response.clicked()
    });
}

/// Menu entries that append `FIELD.COMPONENT` to the equation.
fn insert_menu(ui: &mut Ui, fields: &[(String, Vec<String>)], equation: &mut String) {
    for (field, components) in fields {
        ui.menu_button(field, |ui| {
            for component in components {
                if ui.button(component).clicked() {
                    let name = if field.contains(|c: char| !c.is_alphanumeric() && c != '_') {
                        format!("[{field}.{component}]")
                    } else {
                        format!("{field}.{component}")
                    };
                    equation.push_str(&name);
                    ui.close();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn results() -> (ResultsView, FeMesh) {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/kragbalken_c3d8.frd");
        let import = plx_io::frd::read_frd(&path).unwrap();
        let view = ResultsView::new(import.increments, import.mesh.bounds());
        (view, import.mesh)
    }

    #[test]
    fn limit_proposes_stress_and_needs_limit_values() {
        let (mut view, mesh) = results();
        let mut dialog = FieldOutputDialog::create(&view, &mesh);
        assert_eq!(dialog.drafts[LIMIT].name, "Limit-1");
        let FieldOutputKind::Limit {
            field, component, ..
        } = &dialog.drafts[LIMIT].kind
        else {
            panic!()
        };
        assert_eq!((field.as_str(), component.as_str()), ("STRESS", "MISES"));
        assert!(dialog.output().is_err(), "limits are still 0");
        for (_, limit) in &mut dialog.part_limits {
            *limit = 235.0;
        }
        let output = dialog.output().unwrap();
        view.set_field_output(None, output, &mesh).unwrap();
        let field = view.current_increment().unwrap().field("Limit-1").unwrap();
        assert_eq!(field.components[0].name, "RATIO");

        // OK - Neu proposes the next free name; editing keeps the type and values.
        dialog.next("Limit-1");
        assert_eq!(dialog.drafts[LIMIT].name, "Limit-2");
        let index = view.field_output_index(view.current_increment().unwrap().fields.len() - 1);
        let edit = FieldOutputDialog::edit(&view, &mesh, index.unwrap()).unwrap();
        assert_eq!(edit.kind, LIMIT);
        assert_eq!(edit.part_limits[0].1, 235.0);
        assert!(edit.output().is_ok(), "its own name is free");
        assert!(edit.fields.iter().all(|(f, _)| f != "Limit-1"));
    }

    #[test]
    fn equation_output_keeps_the_shown_component() {
        let (mut view, mesh) = results();
        let shown = view
            .current()
            .map(|(f, c)| (f.name.clone(), c.name.clone()));
        let mut dialog = FieldOutputDialog::create(&view, &mesh);
        dialog.kind = EQUATION;
        assert!(dialog.output().is_ok(), "the dialog checks names only");
        assert!(
            view.set_field_output(None, dialog.output().unwrap(), &mesh)
                .is_err()
        );
        dialog.drafts[EQUATION].kind = FieldOutputKind::Equation {
            equation: "=DISP.ALL * 1000".into(),
            unit: "um".into(),
        };
        view.set_field_output(None, dialog.output().unwrap(), &mesh)
            .unwrap();
        let now = view
            .current()
            .map(|(f, c)| (f.name.clone(), c.name.clone()));
        assert_eq!(now, shown);
        let fields = &view.current_increment().unwrap().fields;
        let index = fields.iter().position(|f| f.name == "Equation-1").unwrap();
        (view.field, view.component) = (index, 0);
        assert!(view.legend().unwrap().title.contains("Unit: um"));
        view.remove_field_output(0, &mesh);
        assert!(
            view.current_increment()
                .unwrap()
                .field("Equation-1")
                .is_none()
        );
        assert_eq!(view.field, 0);
    }

    #[test]
    fn transform_needs_a_coordinate_system() {
        let (view, mesh) = results();
        let mut dialog = FieldOutputDialog::create(&view, &mesh);
        dialog.kind = TRANSFORM;
        assert_eq!(dialog.output().unwrap_err(), NO_COORDINATE_SYSTEM);
    }
}
