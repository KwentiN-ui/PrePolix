//! Table of one history output component, like PrePoMax's history output view: a row per
//! increment, a column per node, element or face.

use plx_results::AnalysisKind;
use plx_results::history_output::{HistoryComponent, HistorySet};

use crate::results::format_value;

/// Which component is shown: indices of set, field and component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryTable {
    pub set: usize,
    pub field: usize,
    pub component: usize,
}

impl HistoryTable {
    fn component<'a>(
        &self,
        sets: &'a [HistorySet],
    ) -> Option<(&'a HistorySet, &'a str, &'a HistoryComponent)> {
        let set = sets.get(self.set)?;
        let field = set.fields.get(self.field)?;
        Some((set, &field.name, field.components.get(self.component)?))
    }

    /// The table as tab separated text, e.g. for a spreadsheet.
    pub fn text(&self, sets: &[HistorySet], kind: AnalysisKind) -> String {
        let Some((set, _, component)) = self.component(sets) else {
            return String::new();
        };
        let mut lines = vec![
            ["Step", "Increment", kind.value_label()]
                .into_iter()
                .map(String::from)
                .chain(component.entries.iter().map(|e| e.name.clone()))
                .collect::<Vec<_>>()
                .join("\t"),
        ];
        for (row, (step, increment, value)) in set.rows.iter().enumerate() {
            let values = component
                .entries
                .iter()
                .map(|e| e.values.get(row).map_or(String::new(), |v| v.to_string()));
            let line: Vec<String> = [step.to_string(), increment.to_string(), value.to_string()]
                .into_iter()
                .chain(values)
                .collect();
            lines.push(line.join("\t"));
        }
        lines.join("\n") + "\n"
    }

    /// Shows the window; returns false when it is closed.
    pub fn show(
        &self,
        ctx: &egui::Context,
        sets: &[HistorySet],
        kind: AnalysisKind,
        unit: Option<&str>,
    ) -> bool {
        let Some((set, field, component)) = self.component(sets) else {
            return false;
        };
        let mut open = true;
        let title = format!("History-Ausgabe: {}.{field}.{}", set.name, component.name);
        egui::Window::new(title)
            .id(egui::Id::new("history table"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 380.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} Inkremente, {} Einträge",
                        set.rows.len(),
                        component.entries.len()
                    ));
                    if let Some(unit) = unit.filter(|u| !u.trim().is_empty() && u.trim() != "/") {
                        ui.label(format!("Einheit: {}", unit.trim()));
                    }
                    if ui.button("In Zwischenablage kopieren").clicked() {
                        ui.ctx().copy_text(self.text(sets, kind));
                    }
                });
                ui.separator();
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    egui::Grid::new("history table grid")
                        .striped(true)
                        .min_col_width(60.0)
                        .spacing([16.0, 2.0])
                        .show(ui, |ui| {
                            for title in ["Step", "Increment", kind.value_label()] {
                                ui.strong(title);
                            }
                            for entry in &component.entries {
                                ui.strong(&entry.name);
                            }
                            ui.end_row();
                            for (index, (step, increment, value)) in set.rows.iter().enumerate() {
                                ui.label(step.to_string());
                                ui.label(increment.to_string());
                                ui.label(format_value(*value as f32));
                                for entry in &component.entries {
                                    let value = entry.values.get(index).copied();
                                    ui.label(
                                        value.map_or(String::new(), |v| format_value(v as f32)),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                });
            });
        open
    }
}
