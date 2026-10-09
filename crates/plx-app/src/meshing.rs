//! Meshing the CAD geometry with Gmsh: PrePoMax's mesh setup with its items, the default
//! meshing parameters and the mesh generation part by part on a worker thread.

use std::collections::BTreeSet;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use plx_mesh::{ElementShape, NodeId};
use plx_mesher::{CadEntity, GeneratedMesh};
use plx_model::{
    Algorithm2d, Algorithm3d, Geometry, MeshSetupItem, MeshSetupKind, MeshingParameters, next_name,
};

use crate::model::{Highlight, Hit, Model};
use crate::numeric;
use crate::selection::{History, Operation};
use crate::viewport::Preview;

const ERROR: egui::Color32 = egui::Color32::from_rgb(200, 0, 0);

/// The rows of the meshing parameters in a two-column grid.
fn parameters_ui(ui: &mut egui::Ui, d: &mut MeshingParameters) {
    ui.label("Max. Elementgröße");
    ui.add(numeric::drag_value(&mut d.max_size).range(0.0..=f64::MAX));
    ui.end_row();
    ui.label("Min. Elementgröße");
    ui.add(numeric::drag_value(&mut d.min_size).range(0.0..=f64::MAX));
    ui.end_row();
    ui.label("Elemente pro Krümmungsradius")
        .on_hover_text("0 schaltet die Verfeinerung an gekrümmten Flächen ab.");
    ui.add(numeric::drag_value(&mut d.elements_per_curvature).range(0.0..=100.0));
    ui.end_row();
    ui.label("Netztyp");
    ui.vertical(|ui| {
        ui.checkbox(&mut d.second_order, "Zweite Ordnung (C3D10, CPS6/CPS8)");
        ui.add_enabled(
            d.second_order,
            egui::Checkbox::new(
                &mut d.midside_nodes_on_geometry,
                "Mittelknoten auf der Geometrie",
            ),
        );
        ui.checkbox(&mut d.optimize, "Netz optimieren (Netgen)");
        ui.checkbox(&mut d.quad_dominated, "Vierecke bevorzugen (2D)")
            .on_hover_text("Flächen von 2D-Modellen überwiegend mit Vierecken vernetzen.");
    });
    ui.end_row();
}

/// The open window of the default meshing parameters, which apply to every part that no
/// Meshing Parameters item covers.
pub struct MeshSetupWindow {
    draft: MeshingParameters,
}

pub enum MeshSetupResult {
    Open,
    /// Take over the parameters and close.
    Ok(MeshingParameters),
    /// Take over the parameters, close and mesh all parts.
    Mesh(MeshingParameters),
    Cancel,
}

impl MeshSetupWindow {
    pub fn new(setup: &MeshingParameters) -> Self {
        Self { draft: *setup }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> MeshSetupResult {
        let mut open = true;
        let mut result = MeshSetupResult::Open;
        egui::Window::new("Standard-Netzparameter")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                let d = &mut self.draft;
                ui.weak("Gelten für alle Parts ohne eigene Meshing Parameters.");
                egui::Grid::new("mesh size")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| parameters_ui(ui, d));
                if d.max_size <= 0.0 {
                    ui.colored_label(ERROR, "Die maximale Elementgröße muss größer als 0 sein.");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    let valid = d.max_size > 0.0;
                    if ui
                        .add_enabled(valid, egui::Button::new("Alle Parts vernetzen"))
                        .clicked()
                    {
                        result = MeshSetupResult::Mesh(*d);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Abbrechen").clicked() {
                            result = MeshSetupResult::Cancel;
                        }
                        if ui.add_enabled(valid, egui::Button::new("OK")).clicked() {
                            result = MeshSetupResult::Ok(*d);
                        }
                    });
                });
            });
        if !open {
            result = MeshSetupResult::Cancel;
        }
        result
    }
}

/// The item types of PrePoMax's "Create Mesh Setup Item" that prepolix supports.
const TYPES: [&str; 3] = ["Meshing Parameters", "Local Mesh Size", "Tetrahedral Gmsh"];

fn type_index(kind: &MeshSetupKind) -> usize {
    match kind {
        MeshSetupKind::MeshingParameters { .. } => 0,
        MeshSetupKind::LocalMeshSize { .. } => 1,
        MeshSetupKind::TetrahedralGmsh { .. } => 2,
    }
}

/// A new item of a type with the values of the geometry's defaults.
fn new_kind(index: usize, geometry: &Geometry) -> MeshSetupKind {
    match index {
        0 => MeshSetupKind::MeshingParameters {
            parts: Vec::new(),
            parameters: geometry.meshing,
        },
        1 => MeshSetupKind::LocalMeshSize {
            faces: Vec::new(),
            edges: Vec::new(),
            size: geometry.meshing.max_size / 4.0,
        },
        _ => MeshSetupKind::TetrahedralGmsh {
            parts: Vec::new(),
            algorithm_2d: Algorithm2d::default(),
            algorithm_3d: Algorithm3d::default(),
        },
    }
}

const ALGORITHMS_2D: [(Algorithm2d, &str); 4] = [
    (Algorithm2d::FrontalDelaunay, "Frontal-Delaunay"),
    (Algorithm2d::Delaunay, "Delaunay"),
    (Algorithm2d::MeshAdapt, "MeshAdapt"),
    (Algorithm2d::Automatic, "Automatisch"),
];

const ALGORITHMS_3D: [(Algorithm3d, &str); 3] = [
    (Algorithm3d::Delaunay, "Delaunay"),
    (Algorithm3d::Frontal, "Frontal"),
    (Algorithm3d::Hxt, "HXT"),
];

/// PrePoMax's dialog to create or edit a mesh setup item. Parts are ticked in the dialog or
/// clicked in the 3D view, faces and edges of a local mesh size are clicked on the geometry.
pub struct MeshItemEditor {
    draft: MeshSetupItem,
    /// Index of the edited item; `None` creates a new one.
    index: Option<usize>,
    /// Names of the other items.
    others: Vec<String>,
    /// Faces and edges of a local mesh size.
    picks: History<CadEntity>,
    error: Option<String>,
}

pub enum MeshItemResult {
    Open,
    /// The item to store at the index, or to add.
    Ok(Option<usize>, MeshSetupItem),
    Cancel,
}

impl MeshItemEditor {
    pub fn create(geometry: &Geometry) -> Self {
        let others: Vec<String> = geometry.mesh_items.iter().map(|i| i.name.clone()).collect();
        let kind = new_kind(0, geometry);
        let name = next_name(kind.type_name(), others.iter().map(String::as_str));
        Self {
            draft: MeshSetupItem { name, kind },
            index: None,
            others,
            picks: History::default(),
            error: None,
        }
    }

    pub fn edit(geometry: &Geometry, index: usize) -> Option<Self> {
        let draft = geometry.mesh_items.get(index)?.clone();
        let others = (geometry.mesh_items.iter().enumerate())
            .filter(|&(i, _)| i != index)
            .map(|(_, item)| item.name.clone())
            .collect();
        let picks = match &draft.kind {
            MeshSetupKind::LocalMeshSize { faces, edges, .. } => History::from_items(
                (faces.iter().map(|&f| CadEntity::Face(f)))
                    .chain(edges.iter().map(|&e| CadEntity::Edge(e))),
            ),
            _ => History::default(),
        };
        Some(Self {
            draft,
            index: Some(index),
            others,
            picks,
            error: None,
        })
    }

    fn title(&self) -> String {
        match self.index {
            Some(_) => format!("Mesh-Setup-Eintrag bearbeiten: {}", self.draft.name),
            None => "Mesh-Setup-Eintrag erstellen".into(),
        }
    }

    /// Whether clicks in the 3D view pick for this dialog.
    pub fn picks(&self) -> bool {
        true
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        geometry: &Geometry,
        view: Option<&Model>,
    ) -> MeshItemResult {
        let mut result = MeshItemResult::Open;
        let mut open = true;
        egui::Window::new(self.title())
            .id(egui::Id::new("mesh item editor"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.strong("Typ");
                let current = type_index(&self.draft.kind);
                // PrePoMax's list of item types; an edited item keeps its type.
                egui::Frame::group(ui.style())
                    .fill(crate::style::WINDOW)
                    .show(ui, |ui| {
                        ui.set_min_width(360.0);
                        ui.add_enabled_ui(self.index.is_none(), |ui| {
                            for (index, label) in TYPES.into_iter().enumerate() {
                                if ui.selectable_label(index == current, label).clicked()
                                    && index != current
                                {
                                    self.draft.kind = new_kind(index, geometry);
                                    self.draft.name = next_name(
                                        self.draft.kind.type_name(),
                                        self.others.iter().map(String::as_str),
                                    );
                                    self.picks.clear();
                                }
                            }
                        });
                    });
                ui.separator();
                ui.strong("Eigenschaften");
                egui::Grid::new("mesh item form")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| self.form(ui, view));
                if let Some(error) = &self.error {
                    ui.colored_label(ERROR, error);
                }
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        result = MeshItemResult::Cancel;
                    }
                    if ui.button("OK").clicked() {
                        match self.finish() {
                            Ok(item) => result = MeshItemResult::Ok(self.index, item),
                            Err(error) => self.error = Some(error),
                        }
                    }
                });
            });
        if !open {
            result = MeshItemResult::Cancel;
        }
        result
    }

    fn form(&mut self, ui: &mut egui::Ui, view: Option<&Model>) {
        ui.label("Name");
        ui.text_edit_singleline(&mut self.draft.name);
        ui.end_row();
        let part_names: Vec<&str> = view
            .map(|v| v.parts.iter().map(|p| p.name.as_str()).collect())
            .unwrap_or_default();
        match &mut self.draft.kind {
            MeshSetupKind::MeshingParameters { parts, parameters } => {
                parts_ui(ui, parts, &part_names);
                parameters_ui(ui, parameters);
            }
            MeshSetupKind::LocalMeshSize { size, .. } => {
                ui.label("Region");
                ui.vertical(|ui| {
                    let picked = self.picks.items();
                    let faces = picked
                        .iter()
                        .filter(|e| matches!(e, CadEntity::Face(_)))
                        .count();
                    let edges = picked.len() - faces;
                    ui.horizontal(|ui| {
                        if picked.is_empty() {
                            ui.label("Leer");
                        } else {
                            ui.label(format!("{faces} Flächen, {edges} Kanten"));
                        }
                        if ui.button("Auswahl löschen").clicked() {
                            self.picks.clear();
                        }
                        if ui
                            .add_enabled(self.picks.can_undo(), egui::Button::new("Rückgängig"))
                            .clicked()
                        {
                            self.picks.undo();
                        }
                    });
                    ui.weak("Flächen und Kanten im 3D-Fenster anklicken.");
                    ui.weak("Umschalt: hinzufügen, Strg: entfernen");
                });
                ui.end_row();
                ui.label("Elementgröße");
                ui.add(numeric::drag_value(size).range(0.0..=f64::MAX));
                ui.end_row();
            }
            MeshSetupKind::TetrahedralGmsh {
                parts,
                algorithm_2d,
                algorithm_3d,
            } => {
                parts_ui(ui, parts, &part_names);
                ui.label("Algorithmus Flächen");
                algorithm_combo(ui, "algorithm 2d", algorithm_2d, &ALGORITHMS_2D);
                ui.end_row();
                ui.label("Algorithmus Volumen");
                algorithm_combo(ui, "algorithm 3d", algorithm_3d, &ALGORITHMS_3D);
                ui.end_row();
            }
        }
    }

    /// The item as entered, or what is missing.
    fn finish(&self) -> Result<MeshSetupItem, String> {
        let mut item = self.draft.clone();
        item.name = item.name.trim().to_string();
        if item.name.is_empty() {
            return Err("Der Name fehlt.".into());
        }
        if self
            .others
            .iter()
            .any(|o| o.eq_ignore_ascii_case(&item.name))
        {
            return Err(format!("Der Name {} ist schon vergeben.", item.name));
        }
        match &mut item.kind {
            MeshSetupKind::MeshingParameters { parts, parameters } => {
                if parts.is_empty() {
                    return Err("Kein Part gewählt.".into());
                }
                if parameters.max_size <= 0.0 {
                    return Err("Die maximale Elementgröße muss größer als 0 sein.".into());
                }
            }
            MeshSetupKind::TetrahedralGmsh { parts, .. } => {
                if parts.is_empty() {
                    return Err("Kein Part gewählt.".into());
                }
            }
            MeshSetupKind::LocalMeshSize { faces, edges, size } => {
                let picked = self.picks.items();
                *faces = (picked.iter())
                    .filter_map(|e| match e {
                        CadEntity::Face(tag) => Some(*tag),
                        CadEntity::Edge(_) => None,
                    })
                    .collect();
                *edges = (picked.iter())
                    .filter_map(|e| match e {
                        CadEntity::Edge(tag) => Some(*tag),
                        CadEntity::Face(_) => None,
                    })
                    .collect();
                if picked.is_empty() {
                    return Err("Keine Fläche oder Kante gewählt.".into());
                }
                if *size <= 0.0 {
                    return Err("Die Elementgröße muss größer als 0 sein.".into());
                }
            }
        }
        Ok(item)
    }

    /// A click on the geometry: toggles the clicked part, or picks the face or edge for a
    /// local mesh size.
    pub fn click(&mut self, view: &Model, pick: Option<(&Hit, f32)>, operation: Operation) {
        match &mut self.draft.kind {
            MeshSetupKind::MeshingParameters { parts, .. }
            | MeshSetupKind::TetrahedralGmsh { parts, .. } => {
                let Some((hit, _)) = pick else { return };
                let name = &view.parts[hit.part].name;
                let index = parts.iter().position(|p| p == name);
                match (index, operation) {
                    (Some(index), Operation::Subtract | Operation::Replace) => {
                        parts.remove(index);
                    }
                    (None, Operation::Subtract) | (Some(_), _) => {}
                    (None, _) => parts.push(name.clone()),
                }
            }
            MeshSetupKind::LocalMeshSize { .. } => match pick {
                Some((hit, precision)) => {
                    if let Some(entity) = cad_pick(view, hit, precision) {
                        self.picks.push(operation, BTreeSet::from([entity]));
                    }
                }
                // As in the selection of the FE model, a plain click into empty space
                // clears the selection.
                None if operation == Operation::Replace => self.picks.clear(),
                None => {}
            },
        }
    }

    /// What a click at the hit would select, for the hover preview.
    pub fn preview(&self, view: &Model, hit: &Hit, precision: f32) -> Preview {
        match self.draft.kind {
            MeshSetupKind::LocalMeshSize { .. } => cad_pick(view, hit, precision)
                .map(|entity| entity_preview(view, entity))
                .unwrap_or_default(),
            _ => Preview::default(),
        }
    }

    /// The parts, faces and edges of the item, shown red on the geometry.
    pub fn highlight(&self, view: &Model) -> Highlight {
        match &self.draft.kind {
            MeshSetupKind::LocalMeshSize { .. } => entities_highlight(view, &self.picks.items()),
            kind => item_highlight(view, kind),
        }
    }
}

fn parts_ui(ui: &mut egui::Ui, parts: &mut Vec<String>, names: &[&str]) {
    ui.label("Parts");
    ui.vertical(|ui| {
        for &name in names {
            let mut checked = parts.iter().any(|p| p == name);
            if ui.checkbox(&mut checked, name).changed() {
                if checked {
                    parts.push(name.to_string());
                } else {
                    parts.retain(|p| p != name);
                }
            }
        }
        ui.weak("Auch per Klick im 3D-Fenster");
    });
    ui.end_row();
}

fn algorithm_combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    choices: &[(T, &str)],
) {
    let label = (choices.iter())
        .find(|(v, _)| v == value)
        .map_or("", |(_, l)| *l);
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width(160.0)
        .show_ui(ui, |ui| {
            for &(choice, label) in choices {
                ui.selectable_value(value, choice, label);
            }
        });
}

/// The CAD edge near the hit point, else the CAD face under it.
pub fn cad_pick(view: &Model, hit: &Hit, precision: f32) -> Option<CadEntity> {
    let mesh = &view.mesh;
    if hit.line {
        let element = view.skin(hit.part).line_elements[hit.face];
        return view.cad_entity(mesh.elements()[element].id);
    }
    let position = |id: NodeId| mesh.node_index(id).map(|n| view.render_position(n));
    let mut nearest: Option<(f32, CadEntity)> = None;
    for &id in &mesh.parts.get(hit.part)?.elements {
        let Some(element) = mesh.element(id) else {
            continue;
        };
        if element.shape != ElementShape::Line2 {
            continue;
        }
        let (Some(a), Some(b)) = (position(element.nodes[0]), position(element.nodes[1])) else {
            continue;
        };
        let ab = b - a;
        let t =
            ((hit.point - a).dot(ab) / ab.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
        let distance = (a + ab * t).distance(hit.point);
        if distance <= precision
            && nearest.is_none_or(|(d, _)| distance < d)
            && let Some(entity) = view.cad_entity(id)
        {
            nearest = Some((distance, entity));
        }
    }
    if let Some((_, entity)) = nearest {
        return Some(entity);
    }
    let face = view.skin(hit.part).faces.get(hit.face)?;
    view.cad_entity(mesh.elements()[face.element].id)
}

/// Element faces of the display that show a CAD face.
fn face_items(view: &Model, tags: &BTreeSet<i32>) -> std::collections::HashSet<(u32, u8)> {
    let mut faces = std::collections::HashSet::new();
    for part in 0..view.parts.len() {
        for face in &view.skin(part).faces {
            let id = view.mesh.elements()[face.element].id;
            if let Some(CadEntity::Face(tag)) = view.cad_entity(id)
                && tags.contains(&tag)
            {
                faces.insert((id, face.face as u8 + 1));
            }
        }
    }
    faces
}

/// Segments of the display that show a CAD edge, as node pairs.
fn edge_segments(view: &Model, tags: &BTreeSet<i32>) -> Vec<[NodeId; 2]> {
    (view.mesh.elements().iter())
        .filter(|e| e.shape == ElementShape::Line2)
        .filter(
            |e| matches!(view.cad_entity(e.id), Some(CadEntity::Edge(tag)) if tags.contains(&tag)),
        )
        .map(|e| [e.nodes[0], e.nodes[1]])
        .collect()
}

fn entity_preview(view: &Model, entity: CadEntity) -> Preview {
    match entity {
        CadEntity::Face(tag) => {
            let faces = face_items(view, &BTreeSet::from([tag]));
            crate::selection::preview(
                view,
                &crate::selection::Items::Faces(faces.into_iter().collect()),
            )
        }
        CadEntity::Edge(tag) => {
            let position = |id| view.mesh.node_index(id).map(|n| view.render_position(n));
            Preview {
                lines: edge_segments(view, &BTreeSet::from([tag]))
                    .into_iter()
                    .filter_map(|[a, b]| Some([position(a)?, position(b)?]))
                    .collect(),
                points: Vec::new(),
            }
        }
    }
}

fn entities_highlight(view: &Model, entities: &BTreeSet<CadEntity>) -> Highlight {
    let (mut faces, mut edges) = (BTreeSet::new(), BTreeSet::new());
    for entity in entities {
        match *entity {
            CadEntity::Face(tag) => faces.insert(tag),
            CadEntity::Edge(tag) => edges.insert(tag),
        };
    }
    Highlight {
        faces: face_items(view, &faces),
        lines: edge_segments(view, &edges),
        ..Highlight::default()
    }
}

/// How a stored mesh setup item shows on the geometry.
pub fn item_highlight(view: &Model, kind: &MeshSetupKind) -> Highlight {
    match kind {
        MeshSetupKind::MeshingParameters { parts, .. }
        | MeshSetupKind::TetrahedralGmsh { parts, .. } => Highlight {
            parts: (view.parts.iter().enumerate())
                .filter(|(_, p)| parts.contains(&p.name))
                .map(|(i, _)| i)
                .collect(),
            ..Highlight::default()
        },
        MeshSetupKind::LocalMeshSize { faces, edges, .. } => {
            let entities = (faces.iter().map(|&f| CadEntity::Face(f)))
                .chain(edges.iter().map(|&e| CadEntity::Edge(e)))
                .collect();
            entities_highlight(view, &entities)
        }
    }
}

/// The meshes of some parts, generated one after the other.
pub struct PartMeshes {
    pub meshes: Vec<GeneratedMesh>,
}

/// Mesh generation running on a worker thread.
pub struct MeshingJob {
    pub started: Instant,
    receiver: Receiver<Result<PartMeshes, String>>,
}

impl MeshingJob {
    /// Meshes the named parts, or all parts with `None`.
    pub fn start(geometry: Geometry, parts: Option<Vec<String>>, ctx: &egui::Context) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let parts = match parts {
                    Some(parts) => parts,
                    None => plx_mesher::part_names(&geometry).map_err(|e| e.to_string())?,
                };
                let mut meshes = Vec::with_capacity(parts.len());
                for part in parts {
                    let mesh = plx_mesher::generate_part_mesh(&geometry, &part)
                        .map_err(|e| e.to_string())?;
                    meshes.push(mesh);
                }
                Ok(PartMeshes { meshes })
            })();
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        Self {
            started: Instant::now(),
            receiver,
        }
    }

    /// The result once the worker is done.
    pub fn poll(&self) -> Option<Result<PartMeshes, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("Vernetzung abgebrochen".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    fn geometry_view() -> Option<(Geometry, Model)> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/platte_mit_loch.step");
        match crate::model::load(&path) {
            Ok(loaded) => Some((loaded.model.geometry?, loaded.geometry_view?)),
            Err(error) if std::env::var_os("PREPOLIX_REQUIRE_GMSH").is_none() => {
                eprintln!("Gmsh nicht verfügbar, Test übersprungen: {error}");
                None
            }
            Err(error) => panic!("Gmsh wird verlangt: {error}"),
        }
    }

    /// The hit of a ray straight down onto the plate (100 x 40 x 10, centred in render
    /// coordinates) at x, y.
    fn hit(view: &Model, x: f32, y: f32) -> Hit {
        view.pick(Vec3::new(x, y, 50.0), Vec3::NEG_Z).unwrap()
    }

    #[test]
    fn clicks_pick_cad_faces_and_edges_for_a_local_mesh_size() {
        let Some((geometry, view)) = geometry_view() else {
            return;
        };
        let top = cad_pick(&view, &hit(&view, -30.0, 0.0), 0.5);
        assert!(matches!(top, Some(CadEntity::Face(_))), "{top:?}");
        // Close to the long edge at y = -20 the edge wins.
        let edge = cad_pick(&view, &hit(&view, -30.0, -19.8), 0.5);
        assert!(matches!(edge, Some(CadEntity::Edge(_))), "{edge:?}");
        assert_eq!(cad_pick(&view, &hit(&view, 30.0, 0.0), 0.5), top);

        let mut editor = MeshItemEditor::create(&geometry);
        editor.draft.kind = new_kind(1, &geometry);
        let (face_hit, edge_hit) = (hit(&view, -30.0, 0.0), hit(&view, -30.0, -19.8));
        editor.click(&view, Some((&face_hit, 0.5)), Operation::Replace);
        editor.click(&view, Some((&edge_hit, 0.5)), Operation::Add);
        let highlight = editor.highlight(&view);
        assert!(!highlight.faces.is_empty() && !highlight.lines.is_empty());
        let preview = editor.preview(&view, &edge_hit, 0.5);
        assert!(!preview.lines.is_empty());
        let item = editor.finish().unwrap();
        let MeshSetupKind::LocalMeshSize { faces, edges, size } = item.kind else {
            panic!("{item:?}");
        };
        assert_eq!((faces.len(), edges.len()), (1, 1));
        assert_eq!(size, geometry.meshing.max_size / 4.0);
        editor.click(&view, None, Operation::Replace);
        assert!(editor.finish().is_err(), "a click into empty space clears");
    }

    #[test]
    fn part_items_take_clicked_parts_and_need_one() {
        let Some((geometry, view)) = geometry_view() else {
            return;
        };
        let mut editor = MeshItemEditor::create(&geometry);
        assert_eq!(editor.draft.name, "Meshing_Parameters-1");
        assert!(editor.finish().is_err(), "no part chosen");
        let hit = hit(&view, -30.0, 0.0);
        editor.click(&view, Some((&hit, 0.5)), Operation::Replace);
        let item = editor.finish().unwrap();
        assert!(matches!(
            &item.kind,
            MeshSetupKind::MeshingParameters { parts, .. } if parts == &["SOLID-1"]
        ));
        assert_eq!(item_highlight(&view, &item.kind).parts.len(), 1);
        // A second plain click on the part takes it out again.
        editor.click(&view, Some((&hit, 0.5)), Operation::Replace);
        assert!(editor.finish().is_err());

        let mut with_item = geometry.clone();
        with_item.mesh_items.push(item);
        let edited = MeshItemEditor::edit(&with_item, 0).unwrap();
        assert!(edited.finish().is_ok(), "its own name is no clash");
        let mut other = MeshItemEditor::create(&with_item);
        assert_eq!(other.draft.name, "Meshing_Parameters-2");
        other.draft.name = "meshing_parameters-1".into();
        other.click(&view, Some((&hit, 0.5)), Operation::Replace);
        assert!(other.finish().is_err(), "names are unique");
    }
}
