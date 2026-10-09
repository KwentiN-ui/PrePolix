//! Section view: what the plane is (a principal plane at a coordinate, or a point with a
//! normal) and the dialog that edits it. Every way of defining the plane ends in the same
//! point and normal, so the renderer, the cut and the manipulator only know those.

use std::collections::BTreeSet;

use egui::Ui;
use glam::{DMat3, DVec3};
use plx_mesh::NodeId;

use crate::gizmo::{GizmoDrag, PlaneGizmo};
use crate::model::{Highlight, Hit, Model};
use crate::selection::{Items, Operation, Picker, PickerAction, Target};
use crate::viewport::Preview;

/// The three planes of the global coordinate system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrincipalPlane {
    Xy,
    Yz,
    Xz,
}

impl PrincipalPlane {
    pub const ALL: [PrincipalPlane; 3] = [Self::Xy, Self::Yz, Self::Xz];

    fn label(self) -> &'static str {
        match self {
            Self::Xy => "XY",
            Self::Yz => "YZ",
            Self::Xz => "XZ",
        }
    }

    /// The coordinate axis normal to the plane.
    fn axis(self) -> usize {
        match self {
            Self::Yz => 0,
            Self::Xz => 1,
            Self::Xy => 2,
        }
    }

    fn axis_label(self) -> &'static str {
        ["X", "Y", "Z"][self.axis()]
    }

    fn unit_normal(self) -> DVec3 {
        let mut n = DVec3::ZERO;
        n[self.axis()] = 1.0;
        n
    }
}

/// How the user defines the plane.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaneDefinition {
    /// A principal plane at a coordinate, e.g. XY at z = `offset`.
    Principal { plane: PrincipalPlane, offset: f64 },
    /// Any plane through a point, as in PrePoMax. The normal need not be a unit vector.
    PointNormal { point: DVec3, normal: DVec3 },
}

/// A section view: the plane in model coordinates and how the cut faces look. The side the
/// normal points to stays visible, as in PrePoMax.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionView {
    pub definition: PlaneDefinition,
    /// Keeps the other side of a principal plane; a plane through a point is turned by
    /// reversing its normal instead.
    pub flipped: bool,
    /// PrePoMax's "lighten section colors".
    pub lighten: bool,
}

impl SectionView {
    /// A principal plane through the centre of the model, facing away from the viewer like
    /// PrePoMax's default section: the half towards the camera is cut off.
    pub fn facing(view_direction: DVec3, center: DVec3) -> Self {
        let axis = (0..3)
            .max_by(|&a, &b| view_direction[a].abs().total_cmp(&view_direction[b].abs()))
            .unwrap_or(2);
        let plane = PrincipalPlane::ALL
            .into_iter()
            .find(|p| p.axis() == axis)
            .unwrap_or(PrincipalPlane::Xy);
        Self {
            definition: PlaneDefinition::Principal {
                plane,
                offset: center[axis],
            },
            flipped: view_direction[axis] < 0.0,
            lighten: true,
        }
    }

    /// Unit normal pointing into the visible half.
    pub fn normal(&self) -> DVec3 {
        let normal = match &self.definition {
            PlaneDefinition::Principal { plane, .. } => plane.unit_normal(),
            PlaneDefinition::PointNormal { normal, .. } => normal.normalize_or(DVec3::Z),
        };
        if self.flipped { -normal } else { normal }
    }

    /// Where the manipulator sits: the defined point, or for a principal plane the centre of
    /// the model projected onto it.
    pub fn anchor(&self, center: DVec3) -> DVec3 {
        match &self.definition {
            PlaneDefinition::Principal { plane, offset } => {
                let mut p = center;
                p[plane.axis()] = *offset;
                p
            }
            PlaneDefinition::PointNormal { point, .. } => *point,
        }
    }

    /// Shifts the plane along its normal.
    pub fn translate(&mut self, distance: f64) {
        let shift = self.normal() * distance;
        match &mut self.definition {
            PlaneDefinition::Principal { plane, offset } => *offset += shift[plane.axis()],
            PlaneDefinition::PointNormal { point, .. } => *point += shift,
        }
    }

    /// Tilts the plane around an axis through its anchor. A principal plane becomes a plane
    /// through a point, since it no longer lies in a coordinate plane.
    pub fn rotate(&mut self, axis: DVec3, angle: f64, center: DVec3) {
        if axis == DVec3::ZERO || !angle.is_finite() {
            return;
        }
        let rotation = DMat3::from_axis_angle(axis.normalize(), angle);
        let point = self.anchor(center);
        let normal = rotation * self.normal();
        self.set_point_normal(point, normal);
    }

    /// Shows the other side.
    pub fn flip(&mut self) {
        match &mut self.definition {
            PlaneDefinition::Principal { .. } => self.flipped = !self.flipped,
            // Adding zero turns -0 into 0, so the dialog shows no "-0.00".
            PlaneDefinition::PointNormal { normal, .. } => *normal = -*normal + DVec3::ZERO,
        }
    }

    /// Makes the plane one through a point, with the normal pointing into the visible half.
    fn set_point_normal(&mut self, point: DVec3, normal: DVec3) {
        self.definition = PlaneDefinition::PointNormal {
            point,
            normal: normal + DVec3::ZERO,
        };
        self.flipped = false;
    }

    /// The plane through the anchor with the normal into the visible half.
    fn as_point_normal(&self, center: DVec3) -> (DVec3, DVec3) {
        (self.anchor(center), self.normal())
    }
}

/// What the next click in the 3D view sets.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Picking {
    Point,
    /// The first point of the normal, then the second one.
    Normal(Option<DVec3>),
}

/// How the dialog was closed.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionResult {
    Open,
    /// Keep the section view as it is now.
    Ok,
    /// Restore the section view from before the dialog.
    Cancel,
    /// Turn the section view off.
    Disable,
}

/// PrePoMax's section view dialog, extended by principal planes and the manipulator.
pub struct SectionDialog {
    pub draft: SectionView,
    /// The section view before the dialog opened, restored on cancel.
    pub before: Option<SectionView>,
    picking: Option<Picking>,
    picker: Picker,
    /// Nodes of the picked first point of a normal, shown highlighted.
    marked: BTreeSet<NodeId>,
    /// Centre and half diagonal of the model, for the slider and the manipulator.
    center: DVec3,
    half_diagonal: DVec3,
}

impl SectionDialog {
    /// Edits the current section view, or starts one facing away from the viewer.
    pub fn new(current: Option<SectionView>, model: &Model, view_direction: DVec3) -> Self {
        let (min, max) = model
            .mesh
            .bounds()
            .map_or((DVec3::ZERO, DVec3::ZERO), |(a, b)| (a.into(), b.into()));
        let center = (min + max) * 0.5;
        let draft = current
            .clone()
            .unwrap_or_else(|| SectionView::facing(view_direction, center));
        Self {
            draft,
            before: current,
            picking: None,
            picker: Picker::default(),
            marked: BTreeSet::new(),
            center,
            half_diagonal: ((max - min) * 0.5).max(DVec3::splat(1e-9)),
        }
    }

    /// Whether a click in the 3D view picks a point.
    pub fn picks(&self) -> bool {
        self.picking.is_some()
    }

    /// The manipulator in render coordinates of the model.
    pub fn gizmo(&self, model: &Model) -> PlaneGizmo {
        let origin = model.origin();
        PlaneGizmo {
            point: (self.draft.anchor(self.center) - origin).as_vec3(),
            normal: self.draft.normal().as_vec3(),
            // A square a little larger than the model in every direction of the plane.
            half_size: self.half_diagonal.length() as f32 * 0.8,
        }
    }

    pub fn drag(&mut self, drag: GizmoDrag) {
        match drag {
            GizmoDrag::Translate(distance) => self.draft.translate(distance as f64),
            GizmoDrag::Rotate { axis, angle } => {
                self.draft
                    .rotate(axis.as_dvec3(), angle as f64, self.center);
            }
        }
    }

    pub fn highlight(&self) -> Highlight {
        Highlight {
            nodes: self.marked.iter().copied().collect(),
            ..Highlight::default()
        }
    }

    /// A click in the 3D view while picking: the centre of what it selects becomes the point.
    pub fn click(&mut self, model: &Model, pick: Option<(&Hit, f32)>) {
        let Some((hit, precision)) = pick else {
            return;
        };
        let picked = self.picker.pick(model, hit, Target::Nodes, precision);
        if let Items::Nodes(nodes) = picked.resolved(&model.mesh, Target::Nodes) {
            self.take(model, nodes);
        }
    }

    pub fn preview(&self, model: &Model, hit: &Hit, precision: f32) -> Preview {
        let items = self.picker.pick(model, hit, Target::Nodes, precision);
        crate::selection::preview(model, &items)
    }

    /// Takes picked nodes: one node is itself the point, an edge or a face is replaced by
    /// its centre, e.g. the centre of a hole.
    fn take(&mut self, model: &Model, nodes: BTreeSet<NodeId>) {
        // Where the nodes are drawn, which is where the plane cuts, e.g. with an exploded
        // view.
        let positions: Vec<DVec3> = nodes
            .iter()
            .filter_map(|&id| model.node_position(model.mesh.node_index(id)?))
            .map(|p| p.as_dvec3() + model.origin())
            .collect();
        if positions.is_empty() {
            return;
        }
        let picked = positions.iter().sum::<DVec3>() / positions.len() as f64;
        let (_, normal) = self.draft.as_point_normal(self.center);
        match self.picking {
            Some(Picking::Point) => {
                self.draft.set_point_normal(picked, normal);
                self.picking = None;
            }
            Some(Picking::Normal(None)) => {
                self.picking = Some(Picking::Normal(Some(picked)));
                self.marked = nodes;
            }
            Some(Picking::Normal(Some(first))) => {
                let direction = picked - first;
                if direction.length() > 1e-12 * self.half_diagonal.length() {
                    // As in PrePoMax the plane goes through the first point, the normal
                    // points from it to the second.
                    self.draft.set_point_normal(first, direction);
                    self.picking = None;
                    self.marked.clear();
                }
            }
            None => {}
        }
    }

    fn action(&mut self, model: &Model, action: PickerAction) {
        match action {
            PickerAction::Undo | PickerAction::Clear => {
                if let Some(Picking::Normal(_)) = self.picking {
                    self.picking = Some(Picking::Normal(None));
                }
                self.marked.clear();
            }
            PickerAction::Ids(Operation::Add, ids) => {
                let nodes = ids
                    .into_iter()
                    .filter(|&id| model.mesh.node_index(id).is_some())
                    .collect();
                self.take(model, nodes);
            }
            _ => {}
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, model: &Model) -> SectionResult {
        let mut result = SectionResult::Open;
        let mut open = true;
        let window = egui::Window::new("Schnittansicht")
            .id(egui::Id::new("section view"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                self.form(ui);
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        result = SectionResult::Cancel;
                    }
                    if ui.button("Deaktivieren").clicked() {
                        result = SectionResult::Disable;
                    }
                    if ui.button("OK").clicked() {
                        result = SectionResult::Ok;
                    }
                });
            });
        if let Some(window) = window
            && self.picking.is_some()
        {
            let can_undo = !self.marked.is_empty();
            if let Some(action) =
                self.picker
                    .window(ctx, window.response.rect, Target::Nodes, can_undo)
            {
                self.action(model, action);
            }
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            result = SectionResult::Cancel;
        }
        result
    }

    fn form(&mut self, ui: &mut Ui) {
        let speed = self.half_diagonal.length() * 0.005;
        let principal = matches!(self.draft.definition, PlaneDefinition::Principal { .. });
        ui.horizontal(|ui| {
            ui.label("Ebene:");
            if ui.radio(principal, "Grundebene").clicked() && !principal {
                let visible = self.draft.normal();
                let plane = closest_principal(visible);
                let offset = self.draft.anchor(self.center)[plane.axis()];
                self.draft.definition = PlaneDefinition::Principal { plane, offset };
                self.draft.flipped = visible[plane.axis()] < 0.0;
                self.picking = None;
            }
            if ui.radio(!principal, "Punkt und Normale").clicked() && principal {
                let (point, normal) = self.draft.as_point_normal(self.center);
                self.draft.set_point_normal(point, normal);
            }
        });
        ui.add_space(4.0);
        egui::Grid::new("section form")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| match &mut self.draft.definition {
                PlaneDefinition::Principal { plane, offset } => {
                    ui.label("Grundebene");
                    ui.horizontal(|ui| {
                        for candidate in PrincipalPlane::ALL {
                            // Framed also when inactive, so hovering does not widen a button
                            // and push its neighbours.
                            let button =
                                egui::Button::selectable(*plane == candidate, candidate.label())
                                    .frame_when_inactive(true);
                            if ui.add(button).clicked() && *plane != candidate {
                                *plane = candidate;
                                *offset = self.center[candidate.axis()];
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(format!("Lage {}", plane.axis_label()));
                    ui.add(egui::DragValue::new(offset).speed(speed));
                    ui.end_row();
                }
                PlaneDefinition::PointNormal { point, normal } => {
                    ui.label("Punkt");
                    vector_row(ui, point, speed);
                    ui.end_row();
                    ui.label("");
                    let label = if self.picking == Some(Picking::Point) {
                        "Punkt im 3D-Fenster wählen …"
                    } else {
                        "Punkt wählen"
                    };
                    if ui
                        .selectable_label(self.picking == Some(Picking::Point), label)
                        .clicked()
                    {
                        self.picking = match self.picking {
                            Some(Picking::Point) => None,
                            _ => Some(Picking::Point),
                        };
                        self.marked.clear();
                    }
                    ui.end_row();
                    ui.label("Normale");
                    vector_row(ui, normal, 0.01);
                    ui.end_row();
                    ui.label("");
                    let normal_picking = matches!(self.picking, Some(Picking::Normal(_)));
                    let label = match self.picking {
                        Some(Picking::Normal(None)) => "Ersten Punkt wählen …",
                        Some(Picking::Normal(Some(_))) => "Zweiten Punkt wählen …",
                        _ => "Aus zwei Punkten",
                    };
                    if ui.selectable_label(normal_picking, label).clicked() {
                        self.picking = if normal_picking {
                            None
                        } else {
                            Some(Picking::Normal(None))
                        };
                        self.marked.clear();
                    }
                    ui.end_row();
                    ui.label("");
                    ui.horizontal(|ui| {
                        for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                            if ui
                                .button(label)
                                .on_hover_text("Normale entlang der Achse")
                                .clicked()
                            {
                                *normal = DVec3::ZERO;
                                normal[axis] = 1.0;
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        // PrePoMax's position scroll bar: the plane between both ends of the model.
        let normal = self.draft.normal();
        let reach = (self.half_diagonal.dot(normal.abs()) * 1.05).max(1e-9);
        let current = normal.dot(self.draft.anchor(self.center) - self.center);
        let mut position = current;
        ui.horizontal(|ui| {
            ui.label("Position");
            ui.spacing_mut().slider_width = 220.0;
            ui.add(
                egui::Slider::new(&mut position, -reach..=reach)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Never),
            );
        });
        if position != current {
            self.draft.translate(position - current);
        }
        ui.horizontal(|ui| {
            if ui
                .button("Umkehren")
                .on_hover_text("Die andere Seite des Schnitts zeigen")
                .clicked()
            {
                self.draft.flip();
            }
            ui.checkbox(&mut self.draft.lighten, "Schnittflächen aufhellen");
        });
        ui.weak("Pfeil ziehen: verschieben, Bögen ziehen: kippen");
    }
}

/// The principal plane whose normal is closest to a direction.
fn closest_principal(normal: DVec3) -> PrincipalPlane {
    PrincipalPlane::ALL
        .into_iter()
        .max_by(|a, b| normal[a.axis()].abs().total_cmp(&normal[b.axis()].abs()))
        .unwrap_or(PrincipalPlane::Xy)
}

fn vector_row(ui: &mut Ui, value: &mut DVec3, speed: f64) {
    ui.horizontal(|ui| {
        for (k, label) in ["X", "Y", "Z"].into_iter().enumerate() {
            ui.label(label);
            ui.add(
                egui::DragValue::new(&mut value[k])
                    .speed(speed)
                    .max_decimals(6),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    #[test]
    fn default_section_cuts_off_the_half_towards_the_viewer() {
        // Front view: the camera looks along -Z.
        let section = SectionView::facing(DVec3::NEG_Z, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            section.definition,
            PlaneDefinition::Principal {
                plane: PrincipalPlane::Xy,
                offset: 3.0
            }
        );
        assert!((section.normal() - DVec3::NEG_Z).length() < EPS);
        let top = SectionView::facing(DVec3::new(0.1, -0.9, 0.2), DVec3::ZERO);
        assert!((top.normal() - DVec3::NEG_Y).length() < EPS);
    }

    #[test]
    fn translating_moves_along_the_visible_normal() {
        let mut section = SectionView::facing(DVec3::NEG_Z, DVec3::ZERO);
        section.translate(2.0);
        assert_eq!(
            section.definition,
            PlaneDefinition::Principal {
                plane: PrincipalPlane::Xy,
                offset: -2.0
            }
        );
        let mut tilted = SectionView {
            definition: PlaneDefinition::PointNormal {
                point: DVec3::ZERO,
                normal: DVec3::new(0.0, 3.0, 4.0),
            },
            flipped: false,
            lighten: true,
        };
        tilted.translate(5.0);
        assert!((tilted.anchor(DVec3::ZERO) - DVec3::new(0.0, 3.0, 4.0)).length() < EPS);
    }

    #[test]
    fn rotating_a_principal_plane_keeps_its_anchor_and_side() {
        let center = DVec3::new(1.0, 1.0, 1.0);
        let mut section = SectionView::facing(DVec3::X, center);
        section.flipped = true;
        let before = section.normal();
        section.rotate(DVec3::Z, std::f64::consts::FRAC_PI_2, center);
        assert!(matches!(
            section.definition,
            PlaneDefinition::PointNormal { .. }
        ));
        assert!((section.anchor(DVec3::ZERO) - center).length() < EPS);
        let expected = DMat3::from_rotation_z(std::f64::consts::FRAC_PI_2) * before;
        assert!((section.normal() - expected).length() < 1e-9);
    }

    #[test]
    fn closest_principal_plane_follows_the_normal() {
        assert_eq!(
            closest_principal(DVec3::new(0.1, -0.9, 0.0)),
            PrincipalPlane::Xz
        );
        assert_eq!(
            closest_principal(DVec3::new(0.7, 0.1, 0.5)),
            PrincipalPlane::Yz
        );
    }
}
