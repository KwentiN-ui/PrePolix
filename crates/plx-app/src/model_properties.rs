//! PrePoMax's "Model Properties" dialog: the model space and the unit system, asked for a
//! new model and editable later from the model's context.

use plx_mesh::{ElementFamily, FeMesh};
use plx_model::{BASE_QUANTITIES, DERIVED_QUANTITIES, ModelProperties, ModelSpace, UnitSystem};

const ERROR: egui::Color32 = egui::Color32::from_rgb(200, 0, 0);

/// The open dialog with its draft.
pub struct ModelPropertiesDialog {
    pub draft: ModelProperties,
    /// Editing the properties of the open model rather than starting a new one.
    pub editing: bool,
    /// Open the geometry import once the new model is created.
    pub then_import: bool,
}

pub enum DialogResult {
    Open,
    Ok(ModelProperties),
    Cancel,
}

impl ModelPropertiesDialog {
    pub fn new_model(properties: ModelProperties, then_import: bool) -> Self {
        Self {
            draft: properties,
            editing: false,
            then_import,
        }
    }

    pub fn edit(properties: ModelProperties) -> Self {
        Self {
            draft: properties,
            editing: true,
            then_import: false,
        }
    }

    /// `mesh` is the mesh of the model being edited, which limits the model spaces.
    pub fn show(&mut self, ctx: &egui::Context, mesh: Option<&FeMesh>) -> DialogResult {
        let mut open = true;
        let mut result = DialogResult::Open;
        let error = mesh.and_then(|mesh| space_error(self.draft.space, mesh));
        egui::Window::new("Modelleigenschaften")
            .id(egui::Id::new("model properties"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.set_width(340.0);
                let draft = &mut self.draft;
                group(ui, "Modellraum", |ui| {
                    egui::Grid::new("model space")
                        .num_columns(2)
                        .spacing([24.0, 4.0])
                        .show(ui, |ui| {
                            for pair in ModelSpace::ALL.chunks(2) {
                                for &space in pair {
                                    ui.radio_value(&mut draft.space, space, space.label());
                                }
                                ui.end_row();
                            }
                        });
                });
                group(ui, "Einheitensystem", |ui| {
                    egui::Frame::new()
                        .fill(crate::style::WINDOW)
                        .stroke(egui::Stroke::new(1.0, crate::style::BORDER))
                        .inner_margin(4)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            for units in UnitSystem::ALL {
                                let selected = draft.units == units;
                                if ui.selectable_label(selected, units.label()).clicked() {
                                    draft.units = units;
                                }
                            }
                        });
                });
                group(ui, "Einheiten", |ui| units_table(ui, draft.units));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Das Einheitensystem wird noch nicht umgerechnet; Werte gelten in \
                             den gewählten Einheiten.",
                        )
                        .weak(),
                    )
                    .wrap(),
                );
                if let Some(error) = &error {
                    ui.add(egui::Label::new(egui::RichText::new(error).color(ERROR)).wrap());
                }
                ui.add_space(4.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        result = DialogResult::Cancel;
                    }
                    let ok = ui.add_enabled(error.is_none(), egui::Button::new("OK"));
                    if ok.clicked() {
                        result = DialogResult::Ok(*draft);
                    }
                });
            });
        if error.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            result = DialogResult::Ok(self.draft);
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            result = DialogResult::Cancel;
        }
        result
    }
}

/// Why the model space does not fit the existing mesh: a 2D model takes no solid elements,
/// a 3D model no plane ones.
pub fn space_error(space: ModelSpace, mesh: &FeMesh) -> Option<String> {
    let has = |family| mesh.elements().iter().any(|e| e.shape.family() == family);
    if space.is_2d() && has(ElementFamily::Solid) {
        Some(
            "Das Netz enthält Volumenelemente; ein 2D-Modell geht nur mit Flächenelementen.".into(),
        )
    } else if !space.is_2d() && mesh.elements().iter().any(|e| e.is_plane()) {
        Some("Das Netz enthält 2D-Elemente; ein 3D-Modell geht damit nicht.".into())
    } else {
        None
    }
}

/// A titled frame like a Windows group box.
fn group(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
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

/// The units of the system, PrePoMax's base and derived units.
fn units_table(ui: &mut egui::Ui, units: UnitSystem) {
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (title, quantities, values) in [
                (
                    "Basiseinheiten",
                    &BASE_QUANTITIES[..],
                    &units.base_units()[..],
                ),
                (
                    "Abgeleitete Einheiten",
                    &DERIVED_QUANTITIES[..],
                    &units.derived_units()[..],
                ),
            ] {
                egui::CollapsingHeader::new(title)
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(title)
                            .num_columns(2)
                            .striped(true)
                            .spacing([24.0, 2.0])
                            .show(ui, |ui| {
                                for (quantity, unit) in quantities.iter().zip(values) {
                                    ui.label(*quantity);
                                    ui.label(*unit);
                                    ui.end_row();
                                }
                            });
                    });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_mesh::{Element, ElementShape};

    #[test]
    fn the_model_space_has_to_fit_the_mesh() {
        let mut mesh = FeMesh::default();
        assert!(space_error(ModelSpace::Axisymmetric, &mesh).is_none());
        mesh.add_element(Element {
            id: 1,
            type_name: "C3D4".into(),
            shape: ElementShape::Tet4,
            nodes: vec![1, 2, 3, 4],
        })
        .unwrap();
        assert!(space_error(ModelSpace::ThreeD, &mesh).is_none());
        assert!(space_error(ModelSpace::PlaneStress, &mesh).is_some());
        let mut plane = FeMesh::default();
        plane
            .add_element(Element {
                id: 1,
                type_name: "CPE3".into(),
                shape: ElementShape::Tri3,
                nodes: vec![1, 2, 3],
            })
            .unwrap();
        assert!(space_error(ModelSpace::Axisymmetric, &plane).is_none());
        assert!(space_error(ModelSpace::ThreeD, &plane).is_some());
    }
}
