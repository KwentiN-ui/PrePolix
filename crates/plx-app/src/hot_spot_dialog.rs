//! The dialog of a hot spot definition in the Results tree: the toe nodes are picked on the
//! shown results, the paths are drawn while it is open.

use egui::Ui;
use plx_results::hot_spot::{Extrapolation, HotSpot, HotSpotComponent, extrapolation_weights};

use crate::field_output_dialog::DialogAction;
use crate::hot_spots::short;
use crate::model::{Highlight, Hit, Model};
use crate::numeric;
use crate::selection::{Operation, Picker, Target};
use crate::setup::{NODE_SOURCES, RegionDraft, Source};
use crate::viewport::{BoxSelect, Preview};

pub struct HotSpotDialog {
    /// Index of the edited definition; `None` creates a new one.
    pub edit: Option<usize>,
    hot_spot: HotSpot,
    toe: RegionDraft,
    /// Own read-out distances as typed, e.g. "2, 6".
    distances: String,
    /// Names of the other definitions.
    taken: Vec<String>,
    pub error: Option<String>,
    picker: Picker,
}

impl HotSpotDialog {
    /// A new definition next to `existing`; it takes over the settings of the last one, so
    /// several welds of one plate are defined quickly.
    pub fn create(existing: &[HotSpot]) -> Self {
        let taken: Vec<String> = existing.iter().map(|h| h.name.clone()).collect();
        let name = plx_model::next_name("Hot_Spot", taken.iter().map(String::as_str));
        let hot_spot = match existing.last() {
            Some(last) => HotSpot {
                name,
                toe: plx_model::Region::Nodes(Vec::new()),
                ..last.clone()
            },
            None => HotSpot::new(name),
        };
        let distances = match &hot_spot.extrapolation {
            Extrapolation::Custom(d) => format_distances(d),
            _ => String::new(),
        };
        Self {
            edit: None,
            hot_spot,
            toe: RegionDraft::new(NODE_SOURCES, Target::Nodes),
            distances,
            taken,
            error: None,
            picker: Picker::default(),
        }
    }

    pub fn edit(existing: &[HotSpot], index: usize, mesh: &plx_mesh::FeMesh) -> Option<Self> {
        let hot_spot = existing.get(index)?.clone();
        let mut dialog = Self::create(existing);
        dialog.edit = Some(index);
        dialog.taken.remove(index);
        dialog.toe = RegionDraft::from_region(&hot_spot.toe, NODE_SOURCES, Target::Nodes, mesh);
        dialog.distances = match &hot_spot.extrapolation {
            Extrapolation::Custom(d) => format_distances(d),
            _ => String::new(),
        };
        dialog.hot_spot = hot_spot;
        Some(dialog)
    }

    pub fn title(&self) -> &'static str {
        if self.edit.is_some() {
            "Hot Spot bearbeiten"
        } else {
            "Hot Spot erstellen"
        }
    }

    /// Whether clicks in the 3D view pick toe nodes.
    pub fn picks(&self) -> bool {
        self.toe.source == Source::Selection
    }

    pub fn click(&mut self, model: &Model, pick: Option<(&Hit, f32)>, operation: Operation) {
        self.toe.click(model, &self.picker, pick, operation);
    }

    pub fn box_select(&mut self, model: &Model, area: &BoxSelect, operation: Operation) {
        self.toe.box_select(model, &self.picker, area, operation);
    }

    pub fn preview(&self, model: &Model, hit: &Hit, precision: f32) -> Preview {
        self.toe.preview(model, &self.picker, hit, precision)
    }

    pub fn highlight(&self, model: &Model) -> Highlight {
        self.toe.highlight(model)
    }

    /// The definition as currently entered, for drawing its paths.
    pub fn hot_spot(&self) -> HotSpot {
        HotSpot {
            toe: self.toe.region(),
            ..self.hot_spot.clone()
        }
    }

    /// The definition, or why it cannot be created.
    pub fn output(&self) -> Result<HotSpot, String> {
        let hot_spot = self.hot_spot();
        let name = hot_spot.name.trim();
        if name.is_empty() {
            return Err("Bitte einen Namen eingeben.".into());
        }
        if self.taken.iter().any(|t| t.eq_ignore_ascii_case(name)) {
            return Err(format!("Der Name {name} ist schon vergeben."));
        }
        if self.toe.is_empty() {
            return Err("Bitte Knoten am Nahtübergang wählen.".into());
        }
        validate(&hot_spot)?;
        Ok(HotSpot {
            name: name.to_string(),
            ..hot_spot
        })
    }

    /// Prepares the dialog for the next definition after OK - Neu: same settings, new toe.
    pub fn next(&mut self, created: &HotSpot) {
        self.taken.push(created.name.clone());
        self.hot_spot.name =
            plx_model::next_name("Hot_Spot", self.taken.iter().map(String::as_str));
        self.toe = RegionDraft::new(NODE_SOURCES, Target::Nodes);
        self.error = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, model: &Model) -> DialogAction<HotSpot> {
        let mut action = DialogAction::Open;
        let mut open = true;
        let window = egui::Window::new(self.title())
            .id(egui::Id::new("hot spot dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                // A fixed width, so the selection window opens right next to the dialog.
                ui.set_width(380.0);
                egui::Grid::new("hot spot form")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        form(
                            ui,
                            model,
                            &mut self.hot_spot,
                            &mut self.toe,
                            &mut self.distances,
                        )
                    });
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 0, 0), error);
                }
                ui.separator();
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
            && self.toe.source == Source::Selection
        {
            let can_undo = self.toe.can_undo();
            let picked = self
                .picker
                .window(ctx, window.response.rect, Target::Nodes, can_undo);
            if let Some(picked) = picked {
                self.toe.action(model, picked);
            }
        }
        if !open {
            action = DialogAction::Cancel;
        }
        action
    }
}

fn form(
    ui: &mut Ui,
    model: &Model,
    hot_spot: &mut HotSpot,
    toe: &mut RegionDraft,
    text: &mut String,
) {
    ui.label("Name");
    ui.add(egui::TextEdit::singleline(&mut hot_spot.name).desired_width(200.0));
    ui.end_row();
    toe.ui_labeled(ui, model, "Nahtübergang", "toe");
    ui.label("");
    ui.weak("Knoten am Nahtübergang, z. B. als Kante.");
    ui.end_row();
    ui.label("Extrapolation");
    let custom = matches!(hot_spot.extrapolation, Extrapolation::Custom(_));
    egui::ComboBox::from_id_salt("hot spot extrapolation")
        .selected_text(hot_spot.extrapolation.label())
        .width(240.0)
        .show_ui(ui, |ui| {
            for method in Extrapolation::IIW {
                let label = method.label();
                ui.selectable_value(&mut hot_spot.extrapolation, method, label);
            }
            if ui.selectable_label(custom, "Eigene Lesepunkte").clicked() && !custom {
                let distances = hot_spot.distances();
                *text = format_distances(&distances);
                hot_spot.extrapolation = Extrapolation::Custom(distances);
            }
        });
    ui.end_row();
    if let Extrapolation::Custom(distances) = &mut hot_spot.extrapolation {
        ui.label("Abstände");
        let edit = egui::TextEdit::singleline(text)
            .hint_text("z. B. 2, 6, 10")
            .desired_width(200.0);
        if ui.add(edit).changed() {
            *distances = parse_distances(text);
        }
        ui.end_row();
    }
    ui.label("Blechdicke t");
    ui.add_enabled(
        hot_spot.extrapolation.uses_thickness(),
        numeric::drag_value(&mut hot_spot.thickness)
            .range(0.0..=f64::MAX)
            .speed(0.1),
    );
    ui.end_row();
    ui.label("Lesepunkte");
    let distances = hot_spot.distances();
    let weights = extrapolation_weights(&distances);
    let mut formula = String::from("S_hs =");
    for (i, (d, w)) in distances.iter().zip(&weights).enumerate() {
        let sign = match (i, *w < 0.0) {
            (0, false) => "",
            (0, true) => " -",
            (_, false) => " +",
            (_, true) => " -",
        };
        formula += &format!("{sign} {:.3} S({})", w.abs(), short(*d));
    }
    ui.vertical(|ui| {
        ui.label(formula);
        if matches!(
            hot_spot.extrapolation,
            Extrapolation::IiwTypeBFine | Extrapolation::IiwTypeBCoarse
        ) {
            ui.weak("Abstände in mm: das Modell muss in mm sein.");
        }
    });
    ui.end_row();
    ui.label("Spannung");
    egui::ComboBox::from_id_salt("hot spot component")
        .selected_text(hot_spot.component.label())
        .width(240.0)
        .show_ui(ui, |ui| {
            for component in HotSpotComponent::ALL {
                ui.selectable_value(&mut hot_spot.component, component, component.label());
            }
        });
    ui.end_row();
    ui.label("Pfadrichtung");
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            for (value, label) in hot_spot.direction.iter_mut().zip(["X", "Y", "Z"]) {
                ui.label(label);
                ui.add(numeric::drag_value(value).speed(0.05));
            }
        });
        ui.weak("Vom Nahtübergang weg; wird quer zur Naht in die Blechoberfläche gedreht.");
    });
    ui.end_row();
}

fn validate(hot_spot: &HotSpot) -> Result<(), String> {
    let distances = hot_spot.distances();
    if hot_spot.extrapolation.uses_thickness()
        && hot_spot.thickness.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
    {
        return Err("Die Blechdicke muss größer als null sein.".into());
    }
    if distances.len() < 2 {
        return Err("Mindestens zwei Lesepunkte angeben.".into());
    }
    let mut sorted = distances.clone();
    sorted.sort_by(f64::total_cmp);
    if sorted[0] <= 0.0 || sorted.windows(2).any(|w| w[0] == w[1]) {
        return Err("Die Abstände müssen positiv und verschieden sein.".into());
    }
    if hot_spot.direction.iter().all(|&v| v == 0.0) {
        return Err("Bitte eine Pfadrichtung angeben.".into());
    }
    Ok(())
}

fn format_distances(distances: &[f64]) -> String {
    let parts: Vec<String> = distances.iter().map(|&d| short(d)).collect();
    parts.join(", ")
}

/// Distances separated by spaces or semicolons; a comma right after a number separates
/// as well, a comma inside one is a decimal comma ("2, 6" are two, "2,5" is one).
fn parse_distances(text: &str) -> Vec<f64> {
    text.split(|c: char| c == ';' || c.is_whitespace())
        .map(|part| part.trim_end_matches(','))
        .filter(|part| !part.is_empty())
        .filter_map(numeric::parse_number)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn definitions_are_checked() {
        let mut dialog = HotSpotDialog::create(&[]);
        assert_eq!(dialog.hot_spot.name, "Hot_Spot-1");
        assert!(dialog.output().is_err(), "no toe nodes");
        dialog.toe.take(
            crate::selection::Items::Nodes(BTreeSet::from([7])),
            Operation::Replace,
        );
        dialog.hot_spot.extrapolation = Extrapolation::Custom(vec![3.0, 3.0]);
        assert!(dialog.output().is_err(), "equal distances");
        dialog.hot_spot.extrapolation = Extrapolation::IiwTypeBCoarse;
        let created = dialog.output().unwrap();
        assert_eq!(created.toe, plx_model::Region::Nodes(vec![7]));
        // OK - Neu keeps the settings for the next weld.
        dialog.next(&created);
        assert_eq!(dialog.hot_spot.name, "Hot_Spot-2");
        assert_eq!(dialog.hot_spot.extrapolation, Extrapolation::IiwTypeBCoarse);
        assert!(dialog.toe.is_empty());
        let edit = HotSpotDialog::edit(&[created], 0, &plx_mesh::FeMesh::default()).unwrap();
        assert!(edit.output().is_ok(), "its own name is no duplicate");
    }

    #[test]
    fn distances_accept_lists_and_decimal_commas() {
        assert_eq!(parse_distances("2, 6, 10"), [2.0, 6.0, 10.0]);
        assert_eq!(parse_distances("0,4; 1,5"), [0.4, 1.5]);
        assert_eq!(parse_distances("4 8 x 12"), [4.0, 8.0, 12.0]);
        assert_eq!(format_distances(&[0.4 * 12.0, 12.0]), "4.8, 12");
    }
}
