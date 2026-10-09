//! PrePoMax's "Model Properties" dialog: the model space, the unit system and the physical
//! constants, asked for a new model and editable later from the model's context.

use std::collections::BTreeMap;

use crate::model::Model;
use plx_mesh::{ElementFamily, ElementShape, FeMesh};
use plx_mesher::CadEntity;
use plx_model::convert::Conversion;
use plx_model::{
    BASE_QUANTITIES, DERIVED_QUANTITIES, ModelProperties, ModelSpace, Quantity, UnitSystem,
};

const ERROR: egui::Color32 = egui::Color32::from_rgb(200, 0, 0);

/// The open dialog with its draft.
pub struct ModelPropertiesDialog {
    pub draft: ModelProperties,
    /// Editing the properties of the open model rather than starting a new one.
    pub editing: bool,
    /// The properties of the open model being edited.
    original: ModelProperties,
    /// Convert the model's values when its unit system changes, so that it stays the same
    /// physically; otherwise its numbers are taken in the new units.
    pub convert: bool,
    /// Open the geometry import once the new model is created.
    pub then_import: bool,
    /// The units the draft's physical constants are given in.
    constants_units: UnitSystem,
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
            original: properties,
            convert: true,
            then_import,
            constants_units: properties.units,
        }
    }

    pub fn edit(properties: ModelProperties) -> Self {
        Self {
            draft: properties,
            editing: true,
            original: properties,
            convert: true,
            then_import: false,
            constants_units: properties.units,
        }
    }

    /// `mesh` and `geometry`, the display of the CAD geometry, are those of the model being
    /// edited, which limit the model spaces.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        mesh: Option<&FeMesh>,
        geometry: Option<&Model>,
    ) -> DialogResult {
        let mut open = true;
        let mut result = DialogResult::Open;
        let error = (mesh.and_then(|mesh| space_error(self.draft.space, mesh)))
            .or_else(|| geometry.and_then(|g| geometry_check(self.draft.space, g).err()));
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
                // The constants follow a changed unit system when the model is converted
                // or is new, so that they stay the same physically.
                if self.constants_units != draft.units {
                    if self.convert || !self.editing {
                        let c = Conversion::new(self.constants_units, draft.units);
                        c.option(&mut draft.absolute_zero, Quantity::Temperature);
                        c.option(&mut draft.stefan_boltzmann, Quantity::StefanBoltzmann);
                    }
                    self.constants_units = draft.units;
                }
                group(ui, "Physikalische Konstanten", |ui| constants(ui, draft));
                let (from, to) = (self.original.units, draft.units);
                if self.editing && from != to {
                    if from.has_units() && to.has_units() {
                        ui.checkbox(&mut self.convert, "Werte des Modells umrechnen");
                    }
                    let note = if self.convert && from.has_units() && to.has_units() {
                        "Netz, Geometrie, Materialien, Lasten und alle anderen Werte werden \
                         umgerechnet; das Modell bleibt physikalisch gleich."
                    } else {
                        "Die Zahlenwerte bleiben und gelten in den neuen Einheiten."
                    };
                    ui.add(egui::Label::new(egui::RichText::new(note).weak()).wrap());
                }
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

/// Checks CAD geometry for a 2D model space: it has to be faces in the x-y plane, for an
/// axisymmetric model on the side x >= 0 of the axis of revolution. Returns the faces whose
/// normal points to -z; their elements are turned round when meshed.
pub fn geometry_check(space: ModelSpace, geometry: &Model) -> Result<Vec<i32>, String> {
    if !space.is_2d() {
        return Ok(Vec::new());
    }
    if geometry.geometry_solids > 0 {
        return Err(format!(
            "Die Geometrie enthält {} Volumenkörper; ein 2D-Modell braucht Flächen in der \
             x-y-Ebene.",
            geometry.geometry_solids
        ));
    }
    let Some((min, max)) = geometry.mesh.bounds() else {
        return Ok(Vec::new());
    };
    let diagonal = (0..3)
        .map(|k| (max[k] - min[k]).powi(2))
        .sum::<f64>()
        .sqrt();
    let tolerance = 1e-6 * diagonal.max(f64::MIN_POSITIVE);
    if min[2].abs() > tolerance || max[2].abs() > tolerance {
        return Err(format!(
            "Die Geometrie liegt nicht in der x-y-Ebene (z von {:.4} bis {:.4}); ein \
             2D-Modell braucht Flächen bei z = 0.",
            min[2], max[2]
        ));
    }
    if space == ModelSpace::Axisymmetric && min[0] < -tolerance {
        return Err(format!(
            "Die Geometrie reicht bis x = {:.4}; ein rotationssymmetrisches Modell liegt ganz \
             bei x >= 0, die y-Achse ist die Drehachse.",
            min[0]
        ));
    }
    // The display triangles of a face follow its orientation.
    let mut turn: BTreeMap<i32, f64> = BTreeMap::new();
    for element in geometry.mesh.elements() {
        let (Some(CadEntity::Face(face)), ElementShape::Tri3) =
            (geometry.cad_entity(element.id), element.shape)
        else {
            continue;
        };
        let points: Vec<[f64; 3]> = (element.nodes.iter())
            .filter_map(|&id| geometry.mesh.node(id))
            .collect();
        if let [a, b, c] = points[..] {
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            *turn.entry(face).or_default() += cross;
        }
    }
    Ok((turn.into_iter())
        .filter(|&(_, area)| area < 0.0)
        .map(|(face, _)| face)
        .collect())
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

/// Absolute zero and the Stefan-Boltzmann constant, which radiation needs; undefined as in
/// PrePoMax until the user sets them.
fn constants(ui: &mut egui::Ui, draft: &mut ModelProperties) {
    let units = draft.units;
    egui::Grid::new("physical constants")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for (label, value, quantity) in [
                (
                    "Absoluter Nullpunkt",
                    &mut draft.absolute_zero,
                    Quantity::Temperature,
                ),
                (
                    "Stefan-Boltzmann",
                    &mut draft.stefan_boltzmann,
                    Quantity::StefanBoltzmann,
                ),
            ] {
                let mut set = value.is_some();
                ui.checkbox(&mut set, label);
                let mut number = value.unwrap_or(0.0);
                ui.add_enabled(set, crate::numeric::physical(&mut number, units, quantity));
                *value = set.then_some(number);
                ui.end_row();
            }
        });
    if let Some((zero, sigma)) = ModelProperties::standard_constants(units)
        && ui
            .button("Standardwerte")
            .on_hover_text("Absoluter Nullpunkt und Stefan-Boltzmann-Konstante im Einheitensystem")
            .clicked()
    {
        draft.absolute_zero = Some(zero);
        draft.stefan_boltzmann = Some(sigma);
    }
    ui.add(egui::Label::new(egui::RichText::new("Nur für Wärmestrahlung nötig.").weak()).wrap());
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
                                    ui.label(quantity.label());
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

    /// A geometry display of two triangles, face 1 counter-clockwise and face 2 clockwise
    /// seen from +z, moved by `shift`.
    fn two_faces(shift: [f64; 3], solids: usize) -> Model {
        let mut mesh = FeMesh::default();
        let points = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        for (id, [x, y]) in (1..).zip(points) {
            mesh.set_node(id, [x + shift[0], y + shift[1], shift[2]]);
        }
        for (id, nodes) in [(1, vec![1, 2, 3]), (2, vec![2, 3, 4])] {
            mesh.add_element(Element {
                id,
                type_name: "S3".into(),
                shape: ElementShape::Tri3,
                nodes,
            })
            .unwrap();
        }
        let display = plx_mesher::GeometryDisplay {
            mesh,
            entities: vec![CadEntity::Face(1), CadEntity::Face(2)],
            solids,
            faces: 2,
            edges: 0,
        };
        Model::geometry_view(std::path::Path::new("flaechen.brep"), display)
    }

    #[test]
    fn two_d_geometry_lies_in_the_x_y_plane_and_reversed_faces_are_found() {
        let flat = two_faces([0.0; 3], 0);
        assert_eq!(geometry_check(ModelSpace::PlaneStress, &flat), Ok(vec![2]));
        assert_eq!(geometry_check(ModelSpace::ThreeD, &flat), Ok(Vec::new()));
        let raised = two_faces([0.0, 0.0, 0.5], 0);
        assert!(geometry_check(ModelSpace::PlaneStrain, &raised).is_err());
        let left = two_faces([-0.5, 0.0, 0.0], 0);
        assert!(geometry_check(ModelSpace::PlaneStress, &left).is_ok());
        assert!(geometry_check(ModelSpace::Axisymmetric, &left).is_err());
        let solid = two_faces([0.0; 3], 1);
        assert!(geometry_check(ModelSpace::Axisymmetric, &solid).is_err());
        assert!(geometry_check(ModelSpace::ThreeD, &solid).is_ok());
    }
}
