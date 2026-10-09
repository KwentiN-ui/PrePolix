//! PrePoMax's exploded view dialog: the method and its parameters, the direction, the
//! magnification and a scroll bar between the assembled and the exploded state. Every change
//! is previewed in the 3D view at once.

use egui::Ui;
use glam::DVec3;

use super::{Direction, Method, Parameters};

/// Width of the label column, the same in both groups so that the fields line up.
const LABEL_WIDTH: f32 = 130.0;

/// How the dialog was closed.
#[derive(Clone, Debug, PartialEq)]
pub enum ExplodedResult {
    Open,
    /// Apply the edited exploded view.
    Ok,
    /// Restore the exploded view from before the dialog.
    Cancel,
    /// Show the model assembled.
    Disable,
}

/// What the user changed in this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// A parameter; the preview animates towards it like in PrePoMax.
    Parameter,
    /// The scroll bar is dragged; the preview follows it directly.
    Position,
}

pub struct ExplodedDialog {
    pub draft: Parameters,
    /// The applied exploded view when the dialog opened, restored on cancel.
    pub before: Option<Parameters>,
    /// Centre of the model, offered as the centre point.
    model_center: DVec3,
    /// Steps of a sequential disassembly, shown as a hint.
    pub step_count: usize,
}

impl ExplodedDialog {
    /// Edits the applied exploded view, or starts from the last used one.
    pub fn new(applied: Option<Parameters>, last: &Parameters, model_center: DVec3) -> Self {
        let draft = applied.clone().unwrap_or_else(|| last.clone());
        Self {
            draft,
            before: applied,
            model_center,
            step_count: 1,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> (ExplodedResult, Option<Change>) {
        let mut result = ExplodedResult::Open;
        let mut change = None;
        let mut open = true;
        egui::Window::new("Explosionsansicht")
            .id(egui::Id::new("exploded view"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                change = self.form(ui);
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        result = ExplodedResult::Cancel;
                    }
                    if ui.button("Deaktivieren").clicked() {
                        result = ExplodedResult::Disable;
                    }
                    if ui.button("OK").clicked() {
                        result = ExplodedResult::Ok;
                    }
                });
            });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            result = ExplodedResult::Cancel;
        }
        (result, change)
    }

    fn form(&mut self, ui: &mut Ui) -> Option<Change> {
        let before = self.draft.clone();
        let method = self.draft.method;
        ui.strong("Methode");
        egui::Grid::new("exploded method")
            .num_columns(2)
            .min_col_width(LABEL_WIDTH)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Explosionsmethode");
                egui::ComboBox::from_id_salt("exploded method combo")
                    .selected_text(method.label())
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for candidate in Method::ALL {
                            ui.selectable_value(
                                &mut self.draft.method,
                                candidate,
                                candidate.label(),
                            )
                            .on_hover_text(candidate.description());
                        }
                    });
                ui.end_row();
                if method == Method::CenterPoint {
                    for (axis, label) in ["X-Koordinate", "Y-Koordinate", "Z-Koordinate"]
                        .into_iter()
                        .enumerate()
                    {
                        ui.label(label);
                        ui.add(
                            egui::DragValue::new(&mut self.draft.center[axis])
                                .speed(self.model_center.length().max(1.0) * 0.005)
                                .max_decimals(6),
                        );
                        ui.end_row();
                    }
                    ui.label("");
                    if ui
                        .button("Modellmitte")
                        .on_hover_text("Den Mittelpunkt in die Mitte des Modells legen")
                        .clicked()
                    {
                        self.draft.center = self.model_center;
                    }
                    ui.end_row();
                }
                if method == Method::Disassembly {
                    ui.label("Kontakttoleranz").on_hover_text(
                        "Abstand, unter dem sich Flächen zweier Parts berühren; \
                             0 nimmt ein Tausendstel der Diagonale der Baugruppe.",
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.draft.tolerance)
                            .speed(0.001)
                            .range(0.0..=f64::MAX)
                            .max_decimals(6),
                    );
                    ui.end_row();
                }
            });
        ui.weak(method.description());
        ui.add_space(6.0);
        ui.strong("Richtung und Skalierung");
        egui::Grid::new("exploded scaling")
            .num_columns(2)
            .min_col_width(LABEL_WIDTH)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Richtung");
                egui::ComboBox::from_id_salt("exploded direction combo")
                    .selected_text(self.draft.direction.label())
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for candidate in Direction::ALL {
                            ui.selectable_value(
                                &mut self.draft.direction,
                                candidate,
                                candidate.label(),
                            );
                        }
                    });
                ui.end_row();
                ui.label("Vergrößerung")
                    .on_hover_text("Wie weit die Parts beim Skalierungsfaktor 1 auseinander sind");
                ui.add(
                    egui::DragValue::new(&mut self.draft.magnification)
                        .speed(0.05)
                        .range(1.0..=method.max_magnification())
                        .max_decimals(3),
                );
                ui.end_row();
                ui.label("Skalierungsfaktor");
                ui.add(
                    egui::DragValue::new(&mut self.draft.scale_factor)
                        .speed(0.005)
                        .range(0.0..=1.0)
                        .max_decimals(3),
                );
                ui.end_row();
                if method == Method::Disassembly {
                    ui.label("Schrittweise").on_hover_text(
                        "Die Baugruppe Ebene für Ebene zerlegen: erst kommen die Parts ab, die \
                         auf dem Grundkörper sitzen, und nehmen mit, was auf ihnen sitzt, dann \
                         diese Parts und so weiter.",
                    );
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.draft.sequential, true, "Ja");
                        ui.radio_value(&mut self.draft.sequential, false, "Nein");
                        if self.draft.sequential && self.step_count > 1 {
                            ui.weak(format!("{} Ebenen", self.step_count));
                        }
                    });
                    ui.end_row();
                }
            });
        self.draft.clamp();
        let parameter_changed = self.draft != before;
        // PrePoMax's scroll bar from the assembled to the exploded state.
        ui.add_space(6.0);
        let mut position = self.draft.scale_factor;
        let dragged = ui
            .horizontal(|ui| {
                ui.label("Zusammengebaut");
                ui.spacing_mut().slider_width = 200.0;
                let response =
                    ui.add(egui::Slider::new(&mut position, 0.0..=1.0).show_value(false));
                ui.label("Explodiert");
                response.changed()
            })
            .inner;
        if dragged {
            self.draft.scale_factor = position;
            return Some(Change::Position);
        }
        parameter_changed.then_some(Change::Parameter)
    }
}
