//! Dialogs of PrePoMax's features, reference points, coordinate systems and planes, and of
//! results on them: result paths with the values of the shown result along a straight line
//! through the model, as a plot and a table, and plane results with the values where a plane
//! cuts the model.
//!
//! Points are typed in or picked in the 3D view: one node is itself the point, an edge or a
//! face gives its centre, e.g. that of a hole. The FE model and every results file have
//! their own features, as in PrePoMax.

use std::collections::BTreeSet;

use egui::{Color32, Pos2, Rect, RichText, Stroke, Ui, pos2, vec2};
use glam::Vec3;
use plx_mesh::NodeId;
use plx_model::{
    CoordinatePlane, CoordinateSystem, CoordinateSystemKind, FeModel, GLOBAL, Plane, PlaneSource,
    PointRef, ReferencePoint, ResultPath, ResultPlane, next_name,
};
use plx_render::SectionValues;
use plx_results::path::{self, PathPoint};

use crate::model::{Highlight, Hit, Model};
use crate::numeric;
use crate::overlay::FeatureMark;
use crate::results::format_value;
use crate::selection::{Items, Operation, Picker, PickerAction, Target};
use crate::viewport::Preview;

/// Kinds of features the trees create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    ReferencePoint,
    CoordinateSystem,
    Plane,
    ResultPath,
    ResultPlane,
}

/// A feature of a model, by kind and index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureItem {
    pub kind: FeatureKind,
    pub index: usize,
}

/// The point a click in the 3D view sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Position,
    Origin,
    PointX,
    PointXy,
    Start,
    End,
    /// A point of a plane, by its place in the definition.
    PlanePoint(usize),
}

enum Draft {
    ReferencePoint(ReferencePoint),
    CoordinateSystem(CoordinateSystem),
    Plane(Plane),
    Path(ResultPath),
    ResultPlane(ResultPlane),
}

pub enum FeatureResult {
    Open,
    Ok,
    Cancel,
}

/// The read-out points of a path for one definition; located again when it changes.
struct Located {
    path: ResultPath,
    /// Read-out points, or why there are none.
    points: Result<Vec<PathPoint>, String>,
}

pub struct FeatureDialog {
    /// The results file whose features are edited; `None` for the FE model.
    pub results: Option<usize>,
    draft: Draft,
    /// Index of the edited item; `None` creates a new one.
    index: Option<usize>,
    picking: Option<Slot>,
    picker: Picker,
    /// Nodes of the last picked point, shown highlighted.
    marked: BTreeSet<NodeId>,
    error: Option<String>,
    /// Drag speed of coordinates, from the model size.
    speed: f64,
    located: Option<Located>,
}

impl FeatureDialog {
    pub fn create(kind: FeatureKind, results: Option<usize>, model: &Model) -> Self {
        let fe = &model.fe;
        let (min, max) = model.mesh.bounds().unwrap_or(([0.0; 3], [1.0; 3]));
        let center = [0, 1, 2].map(|k| (min[k] + max[k]) * 0.5);
        let draft = match kind {
            FeatureKind::ReferencePoint => Draft::ReferencePoint(ReferencePoint {
                name: next_name("RP", fe.reference_points.iter().map(|r| r.name.as_str())),
                position: center,
            }),
            FeatureKind::CoordinateSystem => {
                let names = fe.coordinate_systems.iter().map(|c| c.name.as_str());
                Draft::CoordinateSystem(CoordinateSystem::new(next_name(
                    "Coordinate_System",
                    names,
                )))
            }
            FeatureKind::ResultPath => {
                let names = fe.result_paths.iter().map(|p| p.name.as_str());
                let mut path = ResultPath::new(next_name("Path", names));
                // Through the middle of the model along its longest side.
                let axis = (0..3)
                    .max_by(|&a, &b| (max[a] - min[a]).total_cmp(&(max[b] - min[b])))
                    .unwrap_or(0);
                let (mut start, mut end) = (center, center);
                start[axis] = min[axis];
                end[axis] = max[axis];
                path.start = PointRef::Coordinates(start);
                path.end = PointRef::Coordinates(end);
                Draft::Path(path)
            }
            FeatureKind::Plane => {
                let mut plane = Plane::new(next_name("Plane", fe.planes.iter().map(|p| &*p.name)));
                // The global XY plane through the middle of the model.
                plane.source = PlaneSource::CoordinateSystem {
                    system: GLOBAL.into(),
                    plane: CoordinatePlane::Xy,
                    offset: center[2],
                };
                Draft::Plane(plane)
            }
            FeatureKind::ResultPlane => {
                let names = fe.result_planes.iter().map(|p| p.name.as_str());
                Draft::ResultPlane(ResultPlane {
                    name: next_name("Plane_Result", names),
                    plane: fe
                        .planes
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_default(),
                })
            }
        };
        Self::new(draft, None, results, model)
    }

    pub fn edit(item: FeatureItem, results: Option<usize>, model: &Model) -> Option<Self> {
        let fe = &model.fe;
        let draft = match item.kind {
            FeatureKind::ReferencePoint => {
                Draft::ReferencePoint(fe.reference_points.get(item.index)?.clone())
            }
            FeatureKind::CoordinateSystem => {
                Draft::CoordinateSystem(fe.coordinate_systems.get(item.index)?.clone())
            }
            FeatureKind::ResultPath => Draft::Path(fe.result_paths.get(item.index)?.clone()),
            FeatureKind::Plane => Draft::Plane(fe.planes.get(item.index)?.clone()),
            FeatureKind::ResultPlane => {
                Draft::ResultPlane(fe.result_planes.get(item.index)?.clone())
            }
        };
        Some(Self::new(draft, Some(item.index), results, model))
    }

    fn new(draft: Draft, index: Option<usize>, results: Option<usize>, model: &Model) -> Self {
        let size = model.mesh.bounds().map_or(1.0, |(min, max)| {
            (0..3).map(|k| max[k] - min[k]).fold(0.0, f64::max)
        });
        Self {
            results,
            draft,
            index,
            picking: None,
            picker: Picker::default(),
            marked: BTreeSet::new(),
            error: None,
            speed: (size * 0.005).max(1e-9),
            located: None,
        }
    }

    pub fn kind(&self) -> FeatureKind {
        match self.draft {
            Draft::ReferencePoint(_) => FeatureKind::ReferencePoint,
            Draft::CoordinateSystem(_) => FeatureKind::CoordinateSystem,
            Draft::Path(_) => FeatureKind::ResultPath,
            Draft::Plane(_) => FeatureKind::Plane,
            Draft::ResultPlane(_) => FeatureKind::ResultPlane,
        }
    }

    /// The plane whose cut the edited plane result shows.
    pub fn result_plane(&self) -> Option<&str> {
        match &self.draft {
            Draft::ResultPlane(result) => Some(&result.plane),
            _ => None,
        }
    }

    /// The edited item, to draw it as selected.
    pub fn item(&self) -> Option<FeatureItem> {
        Some(FeatureItem {
            kind: self.kind(),
            index: self.index?,
        })
    }

    /// Whether a click in the 3D view picks a point.
    pub fn picks(&self) -> bool {
        self.picking.is_some()
    }

    pub fn highlight(&self) -> Highlight {
        Highlight {
            nodes: self.marked.iter().copied().collect(),
            ..Highlight::default()
        }
    }

    pub fn click(&mut self, model: &Model, pick: Option<(&Hit, f32)>) {
        let Some((hit, precision)) = pick else {
            return;
        };
        if let Items::Nodes(nodes) = self.picker.pick(model, hit, Target::Nodes, precision) {
            self.take(model, nodes);
        }
    }

    pub fn preview(&self, model: &Model, hit: &Hit, precision: f32) -> Preview {
        let items = self.picker.pick(model, hit, Target::Nodes, precision);
        crate::selection::preview(model, &items)
    }

    fn take(&mut self, model: &Model, nodes: BTreeSet<NodeId>) {
        let positions: Vec<[f64; 3]> = nodes.iter().filter_map(|&id| model.mesh.node(id)).collect();
        let (Some(slot), false) = (self.picking, positions.is_empty()) else {
            return;
        };
        let n = positions.len() as f64;
        let picked = [0, 1, 2].map(|k| positions.iter().map(|p| p[k]).sum::<f64>() / n);
        match (&mut self.draft, slot) {
            (Draft::ReferencePoint(r), _) => r.position = picked,
            (Draft::CoordinateSystem(c), Slot::Origin) => c.origin = picked,
            (Draft::CoordinateSystem(c), Slot::PointX) => c.point_x = picked,
            (Draft::CoordinateSystem(c), _) => c.point_xy = picked,
            (Draft::Path(p), Slot::Start) => p.start = PointRef::Coordinates(picked),
            (Draft::Path(p), _) => p.end = PointRef::Coordinates(picked),
            (Draft::Plane(plane), Slot::PlanePoint(k)) => match &mut plane.source {
                PlaneSource::PointNormal { point, .. } => *point = PointRef::Coordinates(picked),
                PlaneSource::ThreePoints { points } => {
                    points[k.min(2)] = PointRef::Coordinates(picked);
                }
                PlaneSource::CoordinateSystem { .. } => {}
            },
            (Draft::Plane(_) | Draft::ResultPlane(_), _) => {}
        }
        self.marked = nodes;
        self.picking = None;
    }

    fn action(&mut self, model: &Model, action: PickerAction) {
        match action {
            PickerAction::Undo | PickerAction::Clear => self.marked.clear(),
            PickerAction::Ids(Operation::Add, ids) => {
                let nodes = (ids.into_iter())
                    .filter(|&id| model.mesh.node_index(id).is_some())
                    .collect();
                self.take(model, nodes);
            }
            _ => {}
        }
    }

    fn title(&self) -> String {
        let (kind, name) = match &self.draft {
            Draft::ReferencePoint(r) => ("Reference Point", &r.name),
            Draft::CoordinateSystem(c) => ("Coordinate System", &c.name),
            Draft::Path(p) => ("Pfad", &p.name),
            Draft::Plane(p) => ("Plane", &p.name),
            Draft::ResultPlane(p) => ("Ergebnisse in der Ebene", &p.name),
        };
        let action = if self.index.is_some() {
            "bearbeiten"
        } else {
            "erstellen"
        };
        format!("{kind} {action}: {name}")
    }

    /// `cut` holds the values on the plane of an edited plane result, on the undeformed mesh.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        model: &Model,
        cut: Option<&SectionValues>,
    ) -> FeatureResult {
        let mut result = FeatureResult::Open;
        let mut open = true;
        let is_path = matches!(self.draft, Draft::Path(_));
        let is_plane_result = matches!(self.draft, Draft::ResultPlane(_));
        let window = egui::Window::new(self.title())
            .id(egui::Id::new("feature dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(is_path)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                egui::Grid::new("feature form")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| self.form(ui, &model.fe));
                if is_path {
                    ui.separator();
                    self.path_results(ui, model);
                }
                if is_plane_result {
                    ui.separator();
                    self.plane_results(ui, model, cut);
                }
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::from_rgb(200, 0, 0), error);
                }
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        result = FeatureResult::Cancel;
                    }
                    if ui.button("OK").clicked() {
                        self.error = self.validate(&model.fe).err();
                        if self.error.is_none() {
                            result = FeatureResult::Ok;
                        }
                    }
                });
            });
        if let Some(window) = window
            && self.picking.is_some()
        {
            let can_undo = !self.marked.is_empty();
            if let Some(picked) =
                self.picker
                    .window(ctx, window.response.rect, Target::Nodes, can_undo)
            {
                self.action(model, picked);
            }
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            result = FeatureResult::Cancel;
        }
        result
    }

    fn form(&mut self, ui: &mut Ui, fe: &FeModel) {
        let speed = self.speed;
        let picking = &mut self.picking;
        match &mut self.draft {
            Draft::ReferencePoint(point) => {
                name_row(ui, &mut point.name);
                point_rows(
                    ui,
                    "Punkt",
                    Slot::Position,
                    &mut point.position,
                    picking,
                    speed,
                );
            }
            Draft::CoordinateSystem(system) => {
                name_row(ui, &mut system.name);
                ui.label("Typ");
                ui.horizontal(|ui| {
                    for kind in CoordinateSystemKind::ALL {
                        ui.radio_value(&mut system.kind, kind, kind.label());
                    }
                });
                ui.end_row();
                point_rows(
                    ui,
                    "Ursprung",
                    Slot::Origin,
                    &mut system.origin,
                    picking,
                    speed,
                );
                point_rows(
                    ui,
                    "Punkt auf x-Achse",
                    Slot::PointX,
                    &mut system.point_x,
                    picking,
                    speed,
                );
                point_rows(
                    ui,
                    "Punkt in xy-Ebene",
                    Slot::PointXy,
                    &mut system.point_xy,
                    picking,
                    speed,
                );
                if system.kind == CoordinateSystemKind::Cylindrical {
                    ui.label("");
                    ui.weak("Die z-Achse ist die Zylinderachse; lokale Richtungen r, theta, z.");
                    ui.end_row();
                }
            }
            Draft::Path(path) => {
                name_row(ui, &mut path.name);
                point_ref_rows(
                    ui,
                    "Anfang",
                    Slot::Start,
                    &mut path.start,
                    fe,
                    picking,
                    speed,
                );
                point_ref_rows(ui, "Ende", Slot::End, &mut path.end, fe, picking, speed);
                ui.label("Punkte");
                ui.add(numeric::drag_value(&mut path.points).range(2..=10_000));
                ui.end_row();
            }
            Draft::Plane(plane) => {
                name_row(ui, &mut plane.name);
                plane_rows(ui, plane, fe, picking, speed);
            }
            Draft::ResultPlane(result) => {
                name_row(ui, &mut result.name);
                ui.label("Plane");
                if fe.planes.is_empty() {
                    ui.colored_label(
                        Color32::from_rgb(200, 0, 0),
                        "Zuerst unter Features eine Plane anlegen.",
                    );
                } else {
                    egui::ComboBox::from_id_salt("result plane")
                        .selected_text(result.plane.as_str())
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for plane in &fe.planes {
                                ui.selectable_value(
                                    &mut result.plane,
                                    plane.name.clone(),
                                    &plane.name,
                                );
                            }
                        });
                }
                ui.end_row();
            }
        }
    }

    /// The values of the shown result where the plane cuts the visible parts.
    fn plane_results(&mut self, ui: &mut Ui, model: &Model, cut: Option<&SectionValues>) {
        let Draft::ResultPlane(result) = &self.draft else {
            return;
        };
        let Some(view) = &model.results else {
            ui.weak("Keine Ergebnisse geladen.");
            return;
        };
        let Some((field, component)) = view.current() else {
            ui.weak("Im Results-Baum eine Komponente wählen.");
            return;
        };
        let plane = model.fe.plane(&result.plane);
        if let Some(Err(error)) = plane.map(|p| p.resolve(&model.fe)) {
            ui.colored_label(Color32::from_rgb(200, 0, 0), error);
            return;
        }
        let header = format!("{} {}", field.name, component.name);
        ui.horizontal(|ui| {
            ui.strong(&header);
            ui.label(format!(
                "Inkrement {}",
                crate::results::ResultsView::increment_label(
                    view.current_increment().unwrap_or(&view.increments[0])
                )
            ));
        });
        let Some((cut, [min, max])) = cut.and_then(|c| Some((c, c.extremes?))) else {
            ui.colored_label(
                Color32::from_rgb(200, 0, 0),
                "Die Ebene schneidet keinen sichtbaren Part.",
            );
            return;
        };
        let at = |p: glam::DVec3| {
            format!(
                "bei ({}, {}, {})",
                format_value(p.x as f32),
                format_value(p.y as f32),
                format_value(p.z as f32)
            )
        };
        egui::Grid::new("plane values")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("Min");
                ui.strong(format_value(min.1));
                ui.weak(at(min.0));
                ui.end_row();
                ui.label("Max");
                ui.strong(format_value(max.1));
                ui.weak(at(max.0));
                ui.end_row();
                if let Some(mean) = cut.mean() {
                    let (label, measure, name) = if cut.area > 0.0 {
                        ("Mittelwert", cut.area, "Schnittfläche")
                    } else {
                        ("Mittelwert", cut.length, "Schnittlänge")
                    };
                    ui.label(label);
                    ui.strong(format_value(mean as f32));
                    ui.weak(if cut.area > 0.0 {
                        "flächengewichtet"
                    } else {
                        "längengewichtet"
                    });
                    ui.end_row();
                    ui.label(name);
                    ui.label(format_value(measure as f32));
                    ui.end_row();
                }
                ui.label("Schnittpunkte");
                ui.label(cut.points.len().to_string());
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if ui.button("In Zwischenablage kopieren").clicked() {
                ui.ctx().copy_text(cut_csv(cut, &header, '\t'));
            }
            if ui.button("Als CSV speichern …").clicked() {
                let name = format!("{}.csv", result.name);
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name(name)
                    .save_file()
                {
                    let csv = cut_csv(cut, &header, ';');
                    self.error = std::fs::write(&file, csv)
                        .err()
                        .map(|e| format!("{}: {e}", file.display()));
                }
            }
        });
        ui.weak(
            "Werte am unverformten Netz. Der Haken im Results-Baum zeigt nur die \
             Schnittfläche im 3D-Fenster.",
        );
    }

    /// The plot and table of the shown result along the path.
    fn path_results(&mut self, ui: &mut Ui, model: &Model) {
        let Draft::Path(path) = &self.draft else {
            return;
        };
        let Some(view) = &model.results else {
            ui.weak("Keine Ergebnisse geladen.");
            return;
        };
        if self.located.as_ref().is_none_or(|l| l.path != *path) {
            let points = path
                .samples(&model.fe)
                .map(|samples| path::path_points(&model.mesh, &samples));
            self.located = Some(Located {
                path: path.clone(),
                points,
            });
        }
        let Some(located) = &self.located else {
            return;
        };
        let points = match &located.points {
            Ok(points) => points,
            Err(error) => {
                ui.colored_label(Color32::from_rgb(200, 0, 0), error);
                return;
            }
        };
        let Some((field, component)) = view.current() else {
            ui.weak("Im Results-Baum eine Komponente wählen.");
            return;
        };
        let values = path::interpolate(points, &component.values);
        let header = format!("{} {}", field.name, component.name);
        let inside = values.iter().filter(|v| v.is_finite()).count();
        ui.horizontal(|ui| {
            ui.strong(&header);
            ui.label(format!(
                "Inkrement {}",
                crate::results::ResultsView::increment_label(
                    view.current_increment().unwrap_or(&view.increments[0])
                )
            ));
        });
        if inside == 0 {
            ui.colored_label(
                Color32::from_rgb(200, 0, 0),
                "Der Pfad verläuft nirgends durch das Netz.",
            );
            return;
        }
        let (min, max) = values
            .iter()
            .filter(|v| v.is_finite())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
                (a.min(v), b.max(v))
            });
        ui.label(format!(
            "Min {}   Max {}   ({inside} von {} Punkten im Netz)",
            format_value(min as f32),
            format_value(max as f32),
            values.len()
        ));
        let distances: Vec<f64> = points.iter().map(|p| p.distance).collect();
        plot(ui, &distances, &values);
        ui.horizontal(|ui| {
            if ui.button("In Zwischenablage kopieren").clicked() {
                ui.ctx()
                    .copy_text(path::to_csv(points, &values, &header, '\t'));
            }
            if ui.button("Als CSV speichern …").clicked() {
                let name = format!("{}.csv", located.path.name);
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name(name)
                    .save_file()
                {
                    let csv = path::to_csv(points, &values, &header, ';');
                    self.error = std::fs::write(&file, csv)
                        .err()
                        .map(|e| format!("{}: {e}", file.display()));
                }
            }
        });
        egui::CollapsingHeader::new("Tabelle")
            .id_salt("path table")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        egui::Grid::new("path table grid")
                            .striped(true)
                            .spacing([16.0, 2.0])
                            .show(ui, |ui| {
                                for title in ["Abstand", "X", "Y", "Z", component.name.as_str()] {
                                    ui.strong(title);
                                }
                                ui.end_row();
                                for (point, value) in points.iter().zip(&values) {
                                    ui.label(format_value(point.distance as f32));
                                    for c in point.position {
                                        ui.label(format_value(c as f32));
                                    }
                                    ui.label(if value.is_finite() {
                                        format_value(*value as f32)
                                    } else {
                                        "–".into()
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            });
    }

    fn validate(&self, fe: &FeModel) -> Result<(), String> {
        let (name, taken): (&str, Vec<&str>) = match &self.draft {
            Draft::ReferencePoint(r) => (&r.name, names(&fe.reference_points, |r| &r.name)),
            Draft::CoordinateSystem(c) => (&c.name, names(&fe.coordinate_systems, |c| &c.name)),
            Draft::Path(p) => (&p.name, names(&fe.result_paths, |p| &p.name)),
            Draft::Plane(p) => (&p.name, names(&fe.planes, |p| &p.name)),
            Draft::ResultPlane(p) => (&p.name, names(&fe.result_planes, |p| &p.name)),
        };
        if name.trim().is_empty() {
            return Err("Bitte einen Namen eingeben.".into());
        }
        let duplicate = (taken.iter().enumerate())
            .any(|(i, other)| Some(i) != self.index && other.eq_ignore_ascii_case(name));
        if duplicate {
            return Err(format!("Der Name {name} ist schon vergeben."));
        }
        match &self.draft {
            Draft::CoordinateSystem(c) if c.name.eq_ignore_ascii_case("Global") => {
                Err("Der Name Global steht für das globale Koordinatensystem.".into())
            }
            Draft::CoordinateSystem(c) => c.axes().map(|_| ()),
            Draft::Path(p) => p.samples(fe).map(|_| ()),
            Draft::Plane(p) => p.resolve(fe).map(|_| ()),
            Draft::ResultPlane(p) => match fe.plane(&p.plane) {
                Some(plane) => plane.resolve(fe).map(|_| ()),
                None => Err("Bitte eine Plane wählen.".into()),
            },
            Draft::ReferencePoint(_) => Ok(()),
        }
    }

    /// Copies the draft into the model; references follow a renamed item.
    pub fn apply(self, fe: &mut FeModel) {
        fn put<T>(items: &mut Vec<T>, index: Option<usize>, item: T) {
            match index.and_then(|i| items.get_mut(i)) {
                Some(slot) => *slot = item,
                None => items.push(item),
            }
        }
        match self.draft {
            Draft::ReferencePoint(point) => {
                if let Some(old) = self.index.and_then(|i| fe.reference_points.get(i)) {
                    let old = old.name.clone();
                    fe.rename_reference_point(&old, &point.name);
                }
                put(&mut fe.reference_points, self.index, point);
            }
            Draft::CoordinateSystem(system) => {
                if let Some(old) = self.index.and_then(|i| fe.coordinate_systems.get(i)) {
                    let old = old.name.clone();
                    fe.rename_coordinate_system(&old, &system.name);
                }
                put(&mut fe.coordinate_systems, self.index, system);
            }
            Draft::Plane(plane) => {
                if let Some(old) = self.index.and_then(|i| fe.planes.get(i)) {
                    let old = old.name.clone();
                    fe.rename_plane(&old, &plane.name);
                }
                put(&mut fe.planes, self.index, plane);
            }
            Draft::Path(path) => put(&mut fe.result_paths, self.index, path),
            Draft::ResultPlane(result) => put(&mut fe.result_planes, self.index, result),
        }
    }

    /// The feature as it is being edited, to draw it in place of the stored one.
    pub fn mark(&self, model: &Model) -> Option<FeatureMark> {
        match &self.draft {
            Draft::ReferencePoint(r) => Some(point_mark(model, r, true)),
            Draft::CoordinateSystem(c) => system_mark(model, c, true),
            Draft::Plane(p) => plane_mark(model, p, true),
            Draft::Path(_) | Draft::ResultPlane(_) => None,
        }
    }

    /// The path as it is being edited, in render coordinates.
    pub fn path_line(&self, model: &Model) -> Option<([Vec3; 2], String)> {
        let Draft::Path(path) = &self.draft else {
            return None;
        };
        path_line(model, path)
    }
}

fn names<T>(items: &[T], name: impl Fn(&T) -> &String) -> Vec<&str> {
    items.iter().map(|i| name(i).as_str()).collect()
}

/// Removes a feature; references to it become invalid.
pub fn delete(fe: &mut FeModel, item: FeatureItem) -> bool {
    fn remove<T>(items: &mut Vec<T>, index: usize) -> bool {
        (index < items.len()).then(|| items.remove(index)).is_some()
    }
    match item.kind {
        FeatureKind::ReferencePoint => remove(&mut fe.reference_points, item.index),
        FeatureKind::CoordinateSystem => remove(&mut fe.coordinate_systems, item.index),
        FeatureKind::ResultPath => remove(&mut fe.result_paths, item.index),
        FeatureKind::Plane => remove(&mut fe.planes, item.index),
        FeatureKind::ResultPlane => remove(&mut fe.result_planes, item.index),
    }
}

/// The reference points, coordinate systems and planes of a model as drawn in the 3D view,
/// with `selected` marked and `replaced` left out for the dialog to draw.
pub fn marks(
    model: &Model,
    selected: Option<FeatureItem>,
    replaced: Option<FeatureItem>,
) -> Vec<FeatureMark> {
    let is = |kind, index| Some(FeatureItem { kind, index });
    let points = (model.fe.reference_points.iter().enumerate())
        .filter(|(i, _)| replaced != is(FeatureKind::ReferencePoint, *i))
        .map(|(i, r)| point_mark(model, r, selected == is(FeatureKind::ReferencePoint, i)));
    let systems = (model.fe.coordinate_systems.iter().enumerate())
        .filter(|(i, _)| replaced != is(FeatureKind::CoordinateSystem, *i))
        .filter_map(|(i, c)| {
            system_mark(model, c, selected == is(FeatureKind::CoordinateSystem, i))
        });
    let planes = (model.fe.planes.iter().enumerate())
        .filter(|(i, _)| replaced != is(FeatureKind::Plane, *i))
        .filter_map(|(i, p)| plane_mark(model, p, selected == is(FeatureKind::Plane, i)));
    points.chain(systems).chain(planes).collect()
}

/// A plane as the rectangle that covers the model's bounding box seen along the normal, a
/// little larger, with its sides along global axes where the plane allows.
fn plane_mark(model: &Model, plane: &Plane, selected: bool) -> Option<FeatureMark> {
    use glam::DVec3;
    let (point, normal) = plane.resolve(&model.fe).ok()?;
    let (min, max) = model.mesh.bounds()?;
    let (point, normal) = (DVec3::from(point), DVec3::from(normal));
    let (min, max) = (DVec3::from(min), DVec3::from(max));
    let center = (min + max) * 0.5;
    let on_plane = center - normal * normal.dot(center - point);
    // The global axis most in the plane gives the first side.
    let axis = [DVec3::X, DVec3::Y, DVec3::Z]
        .into_iter()
        .min_by(|a, b| a.dot(normal).abs().total_cmp(&b.dot(normal).abs()))
        .unwrap_or(DVec3::X);
    let u = (axis - normal * axis.dot(normal)).normalize_or(normal.any_orthonormal_vector());
    let v = normal.cross(u);
    let (mut low, mut high) = (glam::DVec2::splat(f64::INFINITY), glam::DVec2::NEG_INFINITY);
    for k in 0..8 {
        let corner = DVec3::new(
            if k & 1 == 0 { min.x } else { max.x },
            if k & 2 == 0 { min.y } else { max.y },
            if k & 4 == 0 { min.z } else { max.z },
        );
        let d = corner - on_plane;
        let local = glam::DVec2::new(d.dot(u), d.dot(v));
        (low, high) = (low.min(local), high.max(local));
    }
    let margin = (high - low).max_element().max(1e-9) * 0.08;
    let (low, high) = (low - margin, high + margin);
    let at = |a: f64, b: f64| model.to_render((on_plane + u * a + v * b).to_array());
    let middle = (low + high) * 0.5;
    Some(FeatureMark::Plane {
        corners: [
            at(high.x, high.y),
            at(low.x, high.y),
            at(low.x, low.y),
            at(high.x, low.y),
        ],
        center: at(middle.x, middle.y),
        normal: normal.as_vec3(),
        name: plane.name.clone(),
        selected,
    })
}

/// The points where a plane cuts the model with their values, as a table.
fn cut_csv(cut: &SectionValues, header: &str, separator: char) -> String {
    let s = separator;
    let mut csv = format!("X{s}Y{s}Z{s}{header}\n");
    for (position, value) in &cut.points {
        csv.push_str(&format!(
            "{}{s}{}{s}{}{s}{}\n",
            position.x, position.y, position.z, value
        ));
    }
    csv
}

/// The definition of a plane: by a point and normal, three points, or a plane of a
/// coordinate system.
fn plane_rows(
    ui: &mut Ui,
    plane: &mut Plane,
    fe: &FeModel,
    picking: &mut Option<Slot>,
    speed: f64,
) {
    let kind = match plane.source {
        PlaneSource::CoordinateSystem { .. } => 0,
        PlaneSource::ThreePoints { .. } => 1,
        PlaneSource::PointNormal { .. } => 2,
    };
    ui.label("Typ");
    let mut chosen = kind;
    ui.horizontal(|ui| {
        ui.radio_value(&mut chosen, 0, "Koordinatensystem");
        ui.radio_value(&mut chosen, 1, "Drei Punkte");
        ui.radio_value(&mut chosen, 2, "Punkt und Normale");
    });
    ui.end_row();
    if chosen != kind {
        // The new definition starts from the plane as it is now.
        let (point, normal) = plane.resolve(fe).unwrap_or(([0.0; 3], [0.0, 0.0, 1.0]));
        let at = PointRef::Coordinates;
        plane.source = match chosen {
            0 => PlaneSource::CoordinateSystem {
                system: GLOBAL.into(),
                plane: CoordinatePlane::Xy,
                offset: point[2],
            },
            1 => {
                let n = glam::DVec3::from(normal);
                let (u, v) = n.any_orthonormal_pair();
                let p = glam::DVec3::from(point);
                PlaneSource::ThreePoints {
                    points: [at(point), at((p + u).to_array()), at((p + v).to_array())],
                }
            }
            _ => PlaneSource::PointNormal {
                point: at(point),
                normal,
            },
        };
        *picking = None;
    }
    match &mut plane.source {
        PlaneSource::CoordinateSystem {
            system,
            plane,
            offset,
        } => {
            ui.label("Koordinatensystem");
            egui::ComboBox::from_id_salt("plane system")
                .selected_text(system.as_str())
                .width(200.0)
                .show_ui(ui, |ui| {
                    let names = std::iter::once(GLOBAL)
                        .chain(fe.coordinate_systems.iter().map(|c| c.name.as_str()));
                    for name in names {
                        ui.selectable_value(system, name.to_string(), name);
                    }
                });
            ui.end_row();
            ui.label("Ebene");
            ui.horizontal(|ui| {
                for candidate in CoordinatePlane::ALL {
                    let button = egui::Button::selectable(*plane == candidate, candidate.label())
                        .frame_when_inactive(true);
                    if ui.add(button).clicked() {
                        *plane = candidate;
                    }
                }
            });
            ui.end_row();
            ui.label("Abstand");
            ui.add(numeric::drag_value(offset).speed(speed).max_decimals(6));
            ui.end_row();
        }
        PlaneSource::ThreePoints { points } => {
            for (k, point) in points.iter_mut().enumerate() {
                let label = ["Punkt 1", "Punkt 2", "Punkt 3"][k];
                point_ref_rows(ui, label, Slot::PlanePoint(k), point, fe, picking, speed);
            }
            ui.label("");
            ui.weak("Die Normale folgt der Rechte-Hand-Regel von 1 über 2 nach 3.");
            ui.end_row();
        }
        PlaneSource::PointNormal { point, normal } => {
            point_ref_rows(ui, "Punkt", Slot::PlanePoint(0), point, fe, picking, speed);
            ui.label(RichText::new("Normale").strong());
            ui.horizontal(|ui| {
                for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                    if ui
                        .button(label)
                        .on_hover_text("Normale entlang der Achse")
                        .clicked()
                    {
                        *normal = [0.0; 3];
                        normal[axis] = 1.0;
                    }
                }
            });
            ui.end_row();
            coordinate_rows(ui, normal, 0.01);
        }
    }
}

fn point_mark(model: &Model, point: &ReferencePoint, selected: bool) -> FeatureMark {
    FeatureMark::Point {
        position: model.to_render(point.position),
        name: point.name.clone(),
        selected,
    }
}

fn system_mark(model: &Model, system: &CoordinateSystem, selected: bool) -> Option<FeatureMark> {
    let axes = system.axes().ok()?;
    Some(FeatureMark::System {
        origin: model.to_render(system.origin),
        axes: axes.map(|a| glam::DVec3::from(a).as_vec3()),
        name: system.name.clone(),
        selected,
    })
}

/// A path's ends in render coordinates with its name.
pub fn path_line(model: &Model, path: &ResultPath) -> Option<([Vec3; 2], String)> {
    let (start, end) = path.ends(&model.fe).ok()?;
    Some((
        [model.to_render(start), model.to_render(end)],
        path.name.clone(),
    ))
}

fn name_row(ui: &mut Ui, name: &mut String) {
    ui.label("Name");
    ui.add(egui::TextEdit::singleline(name).desired_width(200.0));
    ui.end_row();
}

/// A point's coordinates with a button that picks it in the 3D view.
fn point_rows(
    ui: &mut Ui,
    label: &str,
    slot: Slot,
    point: &mut [f64; 3],
    picking: &mut Option<Slot>,
    speed: f64,
) {
    ui.label(RichText::new(label).strong());
    pick_button(ui, slot, picking);
    ui.end_row();
    coordinate_rows(ui, point, speed);
}

fn pick_button(ui: &mut Ui, slot: Slot, picking: &mut Option<Slot>) {
    let active = *picking == Some(slot);
    let text = if active {
        "Im 3D-Fenster wählen …"
    } else {
        "Punkt wählen"
    };
    if ui.selectable_label(active, text).clicked() {
        *picking = if active { None } else { Some(slot) };
    }
}

fn coordinate_rows(ui: &mut Ui, point: &mut [f64; 3], speed: f64) {
    for (k, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        ui.label(format!("    {axis}"));
        ui.add(
            numeric::drag_value(&mut point[k])
                .speed(speed)
                .max_decimals(6),
        );
        ui.end_row();
    }
}

/// A point given by coordinates or as a reference point of the model.
fn point_ref_rows(
    ui: &mut Ui,
    label: &str,
    slot: Slot,
    point: &mut PointRef,
    fe: &FeModel,
    picking: &mut Option<Slot>,
    speed: f64,
) {
    ui.label(RichText::new(label).strong());
    let shown = match point {
        PointRef::Coordinates(_) => "Koordinaten".to_string(),
        PointRef::ReferencePoint(name) => name.clone(),
    };
    egui::ComboBox::from_id_salt(("point source", label))
        .selected_text(shown)
        .width(200.0)
        .show_ui(ui, |ui| {
            let coordinates = matches!(point, PointRef::Coordinates(_));
            if ui.selectable_label(coordinates, "Koordinaten").clicked() && !coordinates {
                let at = point.resolve(fe).unwrap_or([0.0; 3]);
                *point = PointRef::Coordinates(at);
            }
            for reference in &fe.reference_points {
                let chosen = matches!(point, PointRef::ReferencePoint(n) if *n == reference.name);
                if ui.selectable_label(chosen, &reference.name).clicked() {
                    *point = PointRef::ReferencePoint(reference.name.clone());
                }
            }
        });
    ui.end_row();
    match point {
        PointRef::Coordinates(p) => {
            ui.label("");
            pick_button(ui, slot, picking);
            ui.end_row();
            coordinate_rows(ui, p, speed);
        }
        PointRef::ReferencePoint(name) => {
            ui.label("");
            match fe.reference_point(name) {
                Some(r) => ui.weak(format!(
                    "({}, {}, {})",
                    format_value(r.position[0] as f32),
                    format_value(r.position[1] as f32),
                    format_value(r.position[2] as f32)
                )),
                None => ui.colored_label(Color32::from_rgb(200, 0, 0), "existiert nicht"),
            };
            ui.end_row();
        }
    }
}

/// Values over the distance along the path; gaps where the path leaves the mesh. Hovering
/// shows the value at the nearest point.
fn plot(ui: &mut Ui, distances: &[f64], values: &[f64]) {
    let width = ui.available_width().max(360.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 220.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let area = Rect::from_min_max(rect.min + vec2(64.0, 10.0), rect.max - vec2(12.0, 28.0));
    let axis_stroke = Stroke::new(1.0, Color32::from_gray(80));
    let grid = Stroke::new(1.0, Color32::from_gray(225));
    let (x0, x1) = (
        distances.first().copied().unwrap_or(0.0),
        distances.last().copied().unwrap_or(1.0),
    );
    let (mut y0, mut y1) = values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    if !(y0.is_finite() && y1.is_finite()) {
        return;
    }
    if y1 - y0 <= 1e-12 * y0.abs().max(y1.abs()).max(1e-30) {
        let pad = y0.abs().max(1.0) * 0.05;
        (y0, y1) = (y0 - pad, y1 + pad);
    }
    let to_screen = |x: f64, y: f64| {
        pos2(
            area.left() + ((x - x0) / (x1 - x0).max(1e-300)) as f32 * area.width(),
            area.bottom() - ((y - y0) / (y1 - y0)) as f32 * area.height(),
        )
    };
    let font = egui::FontId::proportional(12.0);
    let text = Color32::from_gray(40);
    for k in 0..=4 {
        let t = k as f64 / 4.0;
        let y = y0 + (y1 - y0) * t;
        let p = to_screen(x0, y);
        painter.hline(area.x_range(), p.y, grid);
        painter.text(
            pos2(area.left() - 6.0, p.y),
            egui::Align2::RIGHT_CENTER,
            format_value(y as f32),
            font.clone(),
            text,
        );
        let x = x0 + (x1 - x0) * t;
        let p = to_screen(x, y0);
        painter.vline(p.x, area.y_range(), grid);
        painter.text(
            pos2(p.x, area.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            format_value(x as f32),
            font.clone(),
            text,
        );
    }
    painter.rect_stroke(area, 0.0, axis_stroke, egui::StrokeKind::Inside);
    let color = Color32::from_rgb(200, 0, 160);
    let mut segment: Vec<Pos2> = Vec::new();
    let flush = |segment: &mut Vec<Pos2>| {
        match segment.len() {
            0 => {}
            1 => {
                painter.circle_filled(segment[0], 2.0, color);
            }
            _ => {
                painter.add(egui::Shape::line(
                    std::mem::take(segment),
                    Stroke::new(2.0, color),
                ));
            }
        }
        segment.clear();
    };
    for (&x, &y) in distances.iter().zip(values) {
        if y.is_finite() {
            segment.push(to_screen(x, y));
        } else {
            flush(&mut segment);
        }
    }
    flush(&mut segment);
    if let Some(hover) = response.hover_pos().filter(|p| area.contains(*p)) {
        let nearest = (distances.iter().zip(values))
            .filter(|(_, y)| y.is_finite())
            .min_by(|a, b| {
                let d = |x: f64| (to_screen(x, y0).x - hover.x).abs();
                d(*a.0).total_cmp(&d(*b.0))
            });
        if let Some((&x, &y)) = nearest {
            let p = to_screen(x, y);
            painter.circle_stroke(p, 4.0, Stroke::new(1.5, Color32::BLACK));
            painter.text(
                p + vec2(6.0, -6.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{}: {}", format_value(x as f32), format_value(y as f32)),
                font,
                Color32::BLACK,
            );
        }
    }
    painter.text(
        pos2(rect.left() + 4.0, rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        "Abstand",
        egui::FontId::proportional(12.0),
        text,
    );
}
