//! PrePoMax's selection dialog ("Set Selection"): while an item dialog picks a region in the
//! 3D view, this window chooses what a click selects, and keeps the history of picks so that
//! Undo works.
//!
//! As in PrePoMax a click without modifier replaces the selection, Shift adds, Ctrl removes
//! and Shift+Ctrl keeps the intersection. A click into empty space clears the selection.

use std::collections::{BTreeSet, HashMap, HashSet};

use glam::{DVec3, Vec3};
use plx_mesh::{CadEntity, ElementId, NodeId, SkinFace, face_normal};

use crate::model::{Hit, Model};
use crate::numeric;
use crate::viewport::{BoxSelect, Preview};

/// How a pick combines with the selection so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl Operation {
    pub fn from_modifiers(shift: bool, ctrl: bool) -> Self {
        match (shift, ctrl) {
            (true, true) => Operation::Intersect,
            (true, false) => Operation::Add,
            (false, true) => Operation::Subtract,
            (false, false) => Operation::Replace,
        }
    }
}

/// The picks of a selection in order; the selected items are the picks replayed.
#[derive(Clone, Debug, PartialEq)]
pub struct History<T> {
    steps: Vec<(Operation, BTreeSet<T>)>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self { steps: Vec::new() }
    }
}

impl<T: Ord + Clone> History<T> {
    /// A history holding a selection made earlier, e.g. of an item being edited.
    pub fn from_items(items: impl IntoIterator<Item = T>) -> Self {
        let items: BTreeSet<T> = items.into_iter().collect();
        let mut history = Self::default();
        if !items.is_empty() {
            history.push(Operation::Replace, items);
        }
        history
    }

    pub fn push(&mut self, operation: Operation, items: BTreeSet<T>) {
        if operation == Operation::Replace {
            self.steps.clear();
        }
        self.steps.push((operation, items));
    }

    pub fn undo(&mut self) {
        self.steps.pop();
    }

    pub fn clear(&mut self) {
        self.steps.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.steps.is_empty()
    }

    pub fn items(&self) -> BTreeSet<T> {
        let mut items = BTreeSet::new();
        for (operation, picked) in &self.steps {
            match operation {
                Operation::Replace => items = picked.clone(),
                Operation::Add => items.extend(picked.iter().cloned()),
                Operation::Subtract => items.retain(|i| !picked.contains(i)),
                Operation::Intersect => items.retain(|i| picked.contains(i)),
            }
        }
        items
    }
}

/// What the region of the item consists of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Boundary conditions and concentrated forces act on nodes.
    Nodes,
    /// Pressure and surface traction act on element faces.
    Faces,
    /// In 2D models they act on the outline: the edges of the 2D elements, stored as faces.
    Edges,
}

/// What a click selects, PrePoMax's radio buttons in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectBy {
    /// The surface patch under the mouse, or the edge or corner point near the click.
    Geometry,
    GeometryPart,
    GeometryEdgeAngle,
    GeometrySurfaceAngle,
    Node,
    Element,
    Edge,
    Surface,
    Part,
    EdgeAngle,
    FaceAngle,
    Id,
}

const GEOMETRY_MODES: [SelectBy; 4] = [
    SelectBy::Geometry,
    SelectBy::GeometryPart,
    SelectBy::GeometryEdgeAngle,
    SelectBy::GeometrySurfaceAngle,
];

const MESH_MODES: [SelectBy; 8] = [
    SelectBy::Node,
    SelectBy::Element,
    SelectBy::Edge,
    SelectBy::Surface,
    SelectBy::Part,
    SelectBy::EdgeAngle,
    SelectBy::FaceAngle,
    SelectBy::Id,
];

impl SelectBy {
    fn label(self, target: Target) -> &'static str {
        match self {
            SelectBy::Geometry if target == Target::Faces => "Flächen",
            SelectBy::Geometry if target == Target::Edges => "Kanten",
            SelectBy::Geometry => "Flächen, Kanten und Punkte",
            SelectBy::GeometryPart | SelectBy::Part => "Part",
            SelectBy::GeometryEdgeAngle | SelectBy::EdgeAngle => "Kantenwinkel",
            SelectBy::GeometrySurfaceAngle => "Flächenwinkel",
            SelectBy::Node => "Knoten",
            SelectBy::Element => "Element",
            SelectBy::Edge => "Kante",
            SelectBy::Surface => "Fläche",
            SelectBy::FaceAngle => "Flächenwinkel",
            SelectBy::Id => "ID",
        }
    }

    fn is_geometry(self) -> bool {
        GEOMETRY_MODES.contains(&self)
    }

    /// Edges and points only make sense for regions of nodes, faces not for edges.
    fn allowed(self, target: Target) -> bool {
        match target {
            Target::Nodes => true,
            Target::Faces => !matches!(
                self,
                SelectBy::Node | SelectBy::Edge | SelectBy::EdgeAngle | SelectBy::GeometryEdgeAngle
            ),
            Target::Edges => !matches!(
                self,
                SelectBy::Node
                    | SelectBy::Surface
                    | SelectBy::FaceAngle
                    | SelectBy::GeometrySurfaceAngle
            ),
        }
    }

    fn angle_mode(self) -> bool {
        matches!(
            self,
            SelectBy::GeometryEdgeAngle
                | SelectBy::GeometrySurfaceAngle
                | SelectBy::EdgeAngle
                | SelectBy::FaceAngle
        )
    }
}

/// Picked items in the id space of the target.
#[derive(Clone, Debug, PartialEq)]
pub enum Items {
    Nodes(BTreeSet<NodeId>),
    Faces(BTreeSet<(ElementId, u8)>),
    /// Faces, edges and vertices of the CAD geometry the mesh was generated from.
    Geometry(BTreeSet<CadEntity>),
}

impl Items {
    /// CAD entities as the nodes or element faces of the target they stand for.
    pub fn resolved(self, mesh: &plx_mesh::FeMesh, target: Target) -> Items {
        match self {
            Items::Geometry(entities) => {
                let entities: Vec<CadEntity> = entities.into_iter().collect();
                match target {
                    Target::Nodes => Items::Nodes(mesh.cad_nodes(&entities).into_iter().collect()),
                    Target::Faces | Target::Edges => {
                        Items::Faces(mesh.cad_faces(&entities).into_iter().collect())
                    }
                }
            }
            items => items,
        }
    }
}

/// The dialog's settings, kept while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub select_by: SelectBy,
    /// Angles in degrees for the angle modes, PrePoMax's default 30°.
    angle: f64,
    ids: String,
    /// Shows the mesh based modes next to the geometry based ones.
    expanded: bool,
    error: Option<String>,
}

/// A button of the dialog the region has to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum PickerAction {
    Undo,
    Clear,
    All,
    Invert,
    Ids(Operation, Vec<u32>),
}

impl Default for Picker {
    fn default() -> Self {
        Self {
            select_by: SelectBy::Geometry,
            angle: 30.0,
            ids: String::new(),
            expanded: false,
            error: None,
        }
    }
}

impl Picker {
    /// The selection window next to the item dialog `anchor`.
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        anchor: egui::Rect,
        target: Target,
        can_undo: bool,
    ) -> Option<PickerAction> {
        if !self.select_by.allowed(target) {
            self.select_by = SelectBy::Geometry;
        }
        let mut action = None;
        // egui's own constraint uses a default size before the first layout and pins the
        // window there; keep it on screen with its real size instead.
        let id = egui::Id::new("selection window");
        let width = egui::AreaState::load(ctx, id)
            .and_then(|s| s.size)
            .map_or(350.0, |s| s.x);
        let screen = ctx.content_rect();
        let pos = egui::pos2(
            (anchor.right() + 8.0)
                .min(screen.right() - width)
                .max(screen.left()),
            anchor.top(),
        );
        egui::Window::new("Auswahl")
            .id(id)
            .constrain(false)
            .collapsible(false)
            .resizable(false)
            .title_bar(true)
            .current_pos(pos)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.strong("Geometriebasiert");
                            for mode in GEOMETRY_MODES {
                                self.mode_row(ui, mode, target);
                            }
                        });
                    });
                    if self.expanded {
                        ui.group(|ui| {
                            ui.vertical(|ui| {
                                ui.strong("Netzbasiert");
                                for mode in MESH_MODES {
                                    self.mode_row(ui, mode, target);
                                }
                                ui.add_enabled_ui(self.select_by == SelectBy::Id, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.ids)
                                            .hint_text("z. B. 1, 5, 10-20")
                                            .desired_width(150.0),
                                    );
                                    ui.horizontal(|ui| {
                                        for (label, operation) in [
                                            ("Hinzufügen", Operation::Add),
                                            ("Entfernen", Operation::Subtract),
                                        ] {
                                            if ui.button(label).clicked() {
                                                match parse_ids(&self.ids) {
                                                    Ok(ids) => {
                                                        self.error = None;
                                                        action =
                                                            Some(PickerAction::Ids(operation, ids));
                                                    }
                                                    Err(error) => self.error = Some(error),
                                                }
                                            }
                                        }
                                    });
                                    if target != Target::Nodes {
                                        ui.weak("IDs von Elementen");
                                    }
                                });
                            });
                        });
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 0, 0), error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_undo, egui::Button::new("Rückgängig"))
                        .clicked()
                    {
                        action = Some(PickerAction::Undo);
                    }
                    if ui.button("Löschen").clicked() {
                        action = Some(PickerAction::Clear);
                    }
                    let mesh = !self.select_by.is_geometry();
                    if ui.add_enabled(mesh, egui::Button::new("Alle")).clicked() {
                        action = Some(PickerAction::All);
                    }
                    if ui
                        .add_enabled(mesh, egui::Button::new("Invertieren"))
                        .clicked()
                    {
                        action = Some(PickerAction::Invert);
                    }
                    let more = if self.expanded { "Weniger" } else { "Mehr" };
                    if ui.button(more).clicked() {
                        self.expanded = !self.expanded;
                        if !self.expanded && !self.select_by.is_geometry() {
                            self.select_by = SelectBy::Geometry;
                        }
                    }
                });
                ui.weak("Umschalt: hinzufügen, Strg: entfernen");
                ui.weak("Ziehen: Rahmen (nach links: auch angeschnittene)");
                ui.weak("Mittlere Maustaste: drehen, mit Umschalt verschieben");
            });
        action
    }

    fn mode_row(&mut self, ui: &mut egui::Ui, mode: SelectBy, target: Target) {
        ui.add_enabled_ui(mode.allowed(target), |ui| {
            ui.horizontal(|ui| {
                if ui
                    .radio(self.select_by == mode, mode.label(target))
                    .clicked()
                {
                    self.select_by = mode;
                }
                if mode.angle_mode() {
                    ui.add_enabled(
                        self.select_by == mode,
                        numeric::drag_value(&mut self.angle)
                            .range(0.0..=180.0)
                            .speed(1.0)
                            .suffix(" °"),
                    );
                }
            });
        });
    }

    /// What a click on `hit` selects. `precision` is the pick tolerance in render units at
    /// the hit, for edges and points.
    pub fn pick(&self, model: &Model, hit: &Hit, target: Target, precision: f32) -> Items {
        // On a mesh generated from geometry the geometry mode picks CAD entities, which stay
        // valid when the part is remeshed.
        if self.select_by == SelectBy::Geometry
            && model.has_cad()
            && let Some(entity) = crate::cad_selection::pick(model, hit, target, precision)
        {
            return Items::Geometry(BTreeSet::from([entity]));
        }
        let picker = MeshPicker::new(model, hit.part);
        if target == Target::Edges {
            // Edges of the outline whose nodes the same click would select.
            let nodes = match self.select_by {
                SelectBy::Geometry => match picker.nearest_edge(hit.point) {
                    Some(edge) => picker.edge_chain(edge, Some(OUTLINE_ANGLE.to_radians())),
                    None => BTreeSet::new(),
                },
                _ => match self.pick(model, hit, Target::Nodes, precision) {
                    Items::Nodes(nodes) => nodes,
                    Items::Faces(_) | Items::Geometry(_) => BTreeSet::new(),
                },
            };
            return Items::Faces(outline_edges_within(model, Some(hit.part), &nodes));
        }
        let face = hit.face;
        let angle = self.angle.to_radians();
        let faces: Vec<usize> = match self.select_by {
            SelectBy::Geometry => {
                if target == Target::Nodes {
                    if let Some(node) = picker.corner_near(hit.point, precision) {
                        return Items::Nodes(picker.node_ids([node]));
                    }
                    if let Some(edge) = picker.edge_near(face, hit.point, precision) {
                        // The outline of a 2D model is one loop; its edges end where it bends.
                        let angle = model.is_plane().then(|| OUTLINE_ANGLE.to_radians());
                        return Items::Nodes(picker.edge_chain(edge, angle));
                    }
                }
                picker.patch(face)
            }
            SelectBy::GeometryPart | SelectBy::Part => (0..picker.faces().len()).collect(),
            SelectBy::GeometryEdgeAngle | SelectBy::EdgeAngle | SelectBy::Edge => {
                let limit = (self.select_by != SelectBy::Edge).then_some(angle);
                return match picker.nearest_edge(hit.point) {
                    Some(edge) => Items::Nodes(picker.edge_chain(edge, limit)),
                    None => Items::Nodes(BTreeSet::new()),
                };
            }
            SelectBy::GeometrySurfaceAngle | SelectBy::FaceAngle => {
                picker.faces_by_angle(face, angle)
            }
            SelectBy::Node => return Items::Nodes(BTreeSet::from([model.hit_node(hit)])),
            SelectBy::Element => {
                let element = picker.faces()[face].element;
                if target == Target::Nodes {
                    let element = &model.mesh.elements()[element];
                    return Items::Nodes(element.nodes.iter().copied().collect());
                }
                (0..picker.faces().len())
                    .filter(|&f| picker.faces()[f].element == element)
                    .collect()
            }
            SelectBy::Surface => vec![face],
            SelectBy::Id => return picker.empty(target),
        };
        match target {
            Target::Faces | Target::Edges => {
                Items::Faces(faces.iter().map(|&f| picker.face_id(f)).collect())
            }
            Target::Nodes => Items::Nodes(
                picker.node_ids(
                    faces
                        .iter()
                        .flat_map(|&f| picker.face_nodes(f))
                        .collect::<Vec<_>>(),
                ),
            ),
        }
    }
}

impl Picker {
    /// What a selection box selects: nodes in the node modes, otherwise element faces (whole
    /// elements in the element mode) inside the box, or crossing it when dragged from right
    /// to left. Only visible parts count.
    pub fn pick_box(&self, model: &Model, area: &BoxSelect, target: Target) -> Items {
        let mesh = &model.mesh;
        if target == Target::Edges {
            let edges = (model.outline_edges().into_iter())
                .filter(|(part, _, _)| model.parts[*part].visible)
                .filter(|(_, _, nodes)| {
                    let mut inside = (nodes.iter())
                        .filter_map(|&id| mesh.node_index(id))
                        .map(|n| area.contains(model.render_position(n)));
                    if area.crossing {
                        inside.any(|i| i)
                    } else {
                        inside.all(|i| i)
                    }
                })
                .map(|(_, face, _)| face)
                .collect();
            return Items::Faces(edges);
        }
        let inside: Vec<bool> = (0..mesh.node_count())
            .map(|n| area.contains(model.render_position(n)))
            .collect();
        let taken = |nodes: &mut dyn Iterator<Item = usize>| {
            let mut nodes = nodes.map(|n| inside[n]);
            if area.crossing {
                nodes.any(|inside| inside)
            } else {
                nodes.all(|inside| inside)
            }
        };
        let node_modes = matches!(
            self.select_by,
            SelectBy::Node
                | SelectBy::Edge
                | SelectBy::EdgeAngle
                | SelectBy::GeometryEdgeAngle
                | SelectBy::Id
        );
        if node_modes && target == Target::Nodes {
            let visible = model.visible_nodes();
            return Items::Nodes(
                visible
                    .into_iter()
                    .filter(|&id| mesh.node_index(id).is_some_and(|n| inside[n]))
                    .collect(),
            );
        }
        let mut faces = Vec::new();
        for part in (0..model.parts.len()).filter(|&p| model.parts[p].visible) {
            let picker = MeshPicker::new(model, part);
            if self.select_by == SelectBy::Element {
                let mut elements: Vec<usize> = picker.faces().iter().map(|f| f.element).collect();
                elements.sort_unstable();
                elements.dedup();
                for element in elements {
                    let nodes = &mesh.elements()[element].nodes;
                    let mut indices = nodes.iter().filter_map(|&id| mesh.node_index(id));
                    if taken(&mut indices) {
                        faces.extend(
                            (0..picker.faces().len())
                                .filter(|&f| picker.faces()[f].element == element)
                                .map(|f| (part, f)),
                        );
                    }
                }
            } else {
                for f in 0..picker.faces().len() {
                    if taken(&mut picker.face_nodes(f)) {
                        faces.push((part, f));
                    }
                }
            }
        }
        match target {
            Target::Faces | Target::Edges => Items::Faces(
                faces
                    .iter()
                    .map(|&(part, f)| MeshPicker::new(model, part).face_id(f))
                    .collect(),
            ),
            Target::Nodes => {
                let ids = mesh.node_ids();
                Items::Nodes(
                    faces
                        .iter()
                        .flat_map(|&(part, f)| MeshPicker::new(model, part).face_nodes(f))
                        .map(|n| ids[n])
                        .collect(),
                )
            }
        }
    }
}

/// The hover preview of picked items: outlines of faces and points of nodes.
pub fn preview(model: &Model, items: &Items) -> Preview {
    let mut preview = Preview::default();
    match items {
        Items::Geometry(entities) => return crate::cad_selection::preview(model, entities),
        Items::Nodes(nodes) => {
            preview.points = nodes
                .iter()
                .filter_map(|&id| model.mesh.node_index(id))
                .map(|n| model.render_position(n))
                .collect();
        }
        Items::Faces(faces) if model.is_plane() => {
            preview.lines = edge_lines(model, faces)
                .into_iter()
                .filter_map(|[a, b]| {
                    let position = |id| Some(model.render_position(model.mesh.node_index(id)?));
                    Some([position(a)?, position(b)?])
                })
                .collect();
        }
        Items::Faces(faces) => {
            // An edge between two picked faces is drawn once.
            let mut edges = HashSet::new();
            for (part, skin) in (0..model.parts.len()).map(|p| (p, model.skin(p))) {
                let picker = MeshPicker::new(model, part);
                for (index, face) in skin.faces.iter().enumerate() {
                    if faces.contains(&picker.face_id(index)) {
                        edges
                            .extend(corner_edges(&face.corners).map(|(a, b)| (a.min(b), a.max(b))));
                    }
                }
            }
            preview.lines = (edges.into_iter())
                .map(|(a, b)| [model.render_position(a), model.render_position(b)])
                .collect();
        }
    }
    preview
}

/// How far the outline may bend at a node and still count as one edge when the geometry
/// based mode picks edges of a 2D model, the feature angle of the display.
const OUTLINE_ANGLE: f64 = 30.0;

/// Edges of the outline, of one part or of all, whose nodes are all among `nodes`.
pub fn outline_edges_within(
    model: &Model,
    part: Option<usize>,
    nodes: &BTreeSet<NodeId>,
) -> BTreeSet<(ElementId, u8)> {
    (model.outline_edges().into_iter())
        .filter(|(p, _, _)| part.is_none_or(|part| part == *p))
        .filter(|(_, _, edge)| edge.iter().all(|n| nodes.contains(n)))
        .map(|(_, face, _)| face)
        .collect()
}

/// The edges of 2D elements as lines between nodes, split at the midside node.
pub fn edge_lines(model: &Model, faces: &BTreeSet<(ElementId, u8)>) -> Vec<[NodeId; 2]> {
    let mut lines = Vec::new();
    for &(element, face) in faces {
        let Some(element) = model.mesh.element(element) else {
            continue;
        };
        let Some(edge) = (usize::from(face).checked_sub(1)).and_then(|f| element.faces().get(f))
        else {
            continue;
        };
        let [a, b] = [edge.corners[0], edge.corners[1]].map(|l| element.nodes[l]);
        match edge.mids.first().filter(|_| element.shape.is_quadratic()) {
            Some(&mid) => {
                let m = element.nodes[mid];
                lines.extend([[a, m], [m, b]]);
            }
            None => lines.push([a, b]),
        }
    }
    lines
}

/// `1, 5, 10-20` as a list of ids.
fn parse_ids(text: &str) -> Result<Vec<u32>, String> {
    let mut ids = Vec::new();
    for token in text.split([',', ' ', ';']).filter(|t| !t.trim().is_empty()) {
        let invalid = || format!("Keine gültige ID: {token}");
        match token.split_once('-') {
            Some((from, to)) => {
                let from: u32 = from.trim().parse().map_err(|_| invalid())?;
                let to: u32 = to.trim().parse().map_err(|_| invalid())?;
                if to < from || to - from > 10_000_000 {
                    return Err(invalid());
                }
                ids.extend(from..=to);
            }
            None => ids.push(token.trim().parse().map_err(|_| invalid())?),
        }
    }
    Ok(ids)
}

/// Geometric queries on the skin of one part.
struct MeshPicker<'a> {
    model: &'a Model,
    part: usize,
}

impl<'a> MeshPicker<'a> {
    fn new(model: &'a Model, part: usize) -> Self {
        Self { model, part }
    }

    fn faces(&self) -> &'a [SkinFace] {
        &self.model.skin(self.part).faces
    }

    fn empty(&self, target: Target) -> Items {
        match target {
            Target::Nodes => Items::Nodes(BTreeSet::new()),
            Target::Faces | Target::Edges => Items::Faces(BTreeSet::new()),
        }
    }

    fn face_id(&self, face: usize) -> (ElementId, u8) {
        let face = &self.faces()[face];
        (
            self.model.mesh.elements()[face.element].id,
            face.face as u8 + 1,
        )
    }

    fn face_nodes(&self, face: usize) -> impl Iterator<Item = usize> + use<'a> {
        let face = &self.faces()[face];
        face.corners.iter().chain(&face.mids).copied()
    }

    fn node_ids(&self, nodes: impl IntoIterator<Item = usize>) -> BTreeSet<NodeId> {
        let ids = self.model.mesh.node_ids();
        nodes.into_iter().map(|n| ids[n]).collect()
    }

    fn position(&self, node: usize) -> Vec3 {
        self.model.render_position(node)
    }

    fn normal(&self, face: usize) -> DVec3 {
        DVec3::from(face_normal(
            self.model.mesh.coords(),
            &self.faces()[face].corners,
        ))
        .normalize_or_zero()
    }

    /// The smooth patch around a face, bounded by feature edges.
    fn patch(&self, face: usize) -> Vec<usize> {
        let region = self.faces()[face].region;
        (0..self.faces().len())
            .filter(|&f| self.faces()[f].region == region)
            .collect()
    }

    /// Faces reached from `start` across edges where neighbouring faces fold less than `angle`.
    fn faces_by_angle(&self, start: usize, angle: f64) -> Vec<usize> {
        let faces = self.faces();
        let mut by_edge: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (index, face) in faces.iter().enumerate() {
            for (a, b) in corner_edges(&face.corners) {
                by_edge.entry((a.min(b), a.max(b))).or_default().push(index);
            }
        }
        let cos_limit = angle.cos();
        let mut seen = vec![false; faces.len()];
        seen[start] = true;
        let mut stack = vec![start];
        while let Some(face) = stack.pop() {
            let normal = self.normal(face);
            for (a, b) in corner_edges(&faces[face].corners) {
                for &next in &by_edge[&(a.min(b), a.max(b))] {
                    if !seen[next] && normal.dot(self.normal(next)) >= cos_limit {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        (0..faces.len()).filter(|&f| seen[f]).collect()
    }

    fn feature_edges(&self) -> impl Iterator<Item = (usize, &'a plx_mesh::SkinEdge)> + 'a {
        self.model
            .skin(self.part)
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.feature)
    }

    /// Number of feature edges at each node they touch.
    fn feature_degree(&self) -> HashMap<usize, usize> {
        let mut degree = HashMap::new();
        for (_, edge) in self.feature_edges() {
            *degree.entry(edge.a).or_insert(0) += 1;
            *degree.entry(edge.b).or_insert(0) += 1;
        }
        degree
    }

    /// A corner of the outline (where other than two feature edges meet, or in a 2D model
    /// where the outline bends) near the point.
    fn corner_near(&self, point: Vec3, precision: f32) -> Option<usize> {
        let plane = self.model.is_plane();
        let bends = if plane {
            self.outline_bends()
        } else {
            HashSet::new()
        };
        self.feature_degree()
            .into_iter()
            .filter(|&(node, degree)| degree != 2 || bends.contains(&node))
            .map(|(node, _)| (node, self.position(node).distance(point)))
            .filter(|&(_, distance)| distance <= precision)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(node, _)| node)
    }

    /// Nodes where two feature edges meet at more than [`OUTLINE_ANGLE`], the corners of a
    /// 2D model's outline.
    fn outline_bends(&self) -> HashSet<usize> {
        let mut at_node: HashMap<usize, Vec<usize>> = HashMap::new();
        for (_, edge) in self.feature_edges() {
            at_node.entry(edge.a).or_default().push(edge.b);
            at_node.entry(edge.b).or_default().push(edge.a);
        }
        let limit = OUTLINE_ANGLE.to_radians().cos() as f32;
        at_node
            .into_iter()
            .filter(|(node, others)| {
                let [a, b] = others[..] else {
                    return false;
                };
                let here = self.position(*node);
                let incoming = (here - self.position(a)).normalize_or_zero();
                let outgoing = (self.position(b) - here).normalize_or_zero();
                incoming.dot(outgoing) < limit
            })
            .map(|(node, _)| node)
            .collect()
    }

    fn edge_distance(&self, edge: &plx_mesh::SkinEdge, point: Vec3) -> f32 {
        let (a, b) = (self.position(edge.a), self.position(edge.b));
        let ab = b - a;
        let t = ((point - a).dot(ab) / ab.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
        (a + ab * t).distance(point)
    }

    /// A feature edge of the hit face near the point.
    fn edge_near(&self, face: usize, point: Vec3, precision: f32) -> Option<usize> {
        let corners = &self.faces()[face].corners;
        self.feature_edges()
            .filter(|(_, e)| corners.contains(&e.a) && corners.contains(&e.b))
            .map(|(i, e)| (i, self.edge_distance(e, point)))
            .filter(|&(_, distance)| distance <= precision)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The feature edge of the part nearest to the point.
    fn nearest_edge(&self, point: Vec3) -> Option<usize> {
        self.feature_edges()
            .map(|(i, e)| (i, self.edge_distance(e, point)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Nodes of the chain of feature edges through `start`. Without an angle the chain runs
    /// to the corners of the outline; with one it continues wherever the next edge bends less
    /// than the angle.
    fn edge_chain(&self, start: usize, angle: Option<f64>) -> BTreeSet<NodeId> {
        let edges = &self.model.skin(self.part).edges;
        let mut at_node: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, edge) in self.feature_edges() {
            at_node.entry(edge.a).or_default().push(index);
            at_node.entry(edge.b).or_default().push(index);
        }
        let direction = |edge: usize, from: usize| {
            let e = &edges[edge];
            let to = if e.a == from { e.b } else { e.a };
            (self.position(to) - self.position(from)).normalize_or_zero()
        };
        let cos_limit = angle.map(|a| a.cos() as f32);
        let mut seen = BTreeSet::from([start]);
        let mut stack = vec![start];
        while let Some(edge) = stack.pop() {
            for node in [edges[edge].a, edges[edge].b] {
                let others = &at_node[&node];
                let incoming = -direction(edge, node);
                for &next in others.iter().filter(|&&n| n != edge) {
                    let follow = match cos_limit {
                        // Continue straight on: the next edge leaves in about the direction the
                        // current one arrived.
                        Some(limit) => incoming.dot(direction(next, node)) >= limit,
                        None => others.len() == 2,
                    };
                    if follow && seen.insert(next) {
                        stack.push(next);
                    }
                }
            }
        }
        let nodes = seen.iter().flat_map(|&e| {
            let edge = &edges[e];
            [Some(edge.a), Some(edge.b), edge.mid].into_iter().flatten()
        });
        self.node_ids(nodes.collect::<Vec<_>>())
    }
}

fn corner_edges(corners: &[usize]) -> impl Iterator<Item = (usize, usize)> + '_ {
    (0..corners.len()).map(|i| (corners[i], corners[(i + 1) % corners.len()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_replays_prepomax_operations() {
        let mut history = History::default();
        history.push(Operation::Replace, BTreeSet::from([1, 2, 3]));
        history.push(Operation::Add, BTreeSet::from([4]));
        history.push(Operation::Subtract, BTreeSet::from([2]));
        assert_eq!(history.items(), BTreeSet::from([1, 3, 4]));
        history.push(Operation::Intersect, BTreeSet::from([3, 4, 5]));
        assert_eq!(history.items(), BTreeSet::from([3, 4]));
        history.undo();
        assert_eq!(history.items(), BTreeSet::from([1, 3, 4]));
        history.push(Operation::Replace, BTreeSet::from([9]));
        assert_eq!(history.items(), BTreeSet::from([9]));
        history.undo();
        assert!(history.items().is_empty(), "replace starts a new history");
    }

    #[test]
    fn modifiers_choose_the_operation() {
        assert_eq!(Operation::from_modifiers(false, false), Operation::Replace);
        assert_eq!(Operation::from_modifiers(true, false), Operation::Add);
        assert_eq!(Operation::from_modifiers(false, true), Operation::Subtract);
        assert_eq!(Operation::from_modifiers(true, true), Operation::Intersect);
    }

    fn testdata(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata")
            .join(name)
    }

    /// The 100 x 10 x 10 cantilever of 10 x 2 x 2 hexahedra, hit from above at x = 95, y = 5,
    /// plus a tolerance of 0.5 mm.
    fn beam_hit(x: f32, y: f32) -> (Model, Hit) {
        let model = crate::model::load(&testdata("kragbalken_c3d8.inp"))
            .unwrap()
            .model;
        // Render coordinates are relative to the model centre.
        let p = model.mesh.coords()[0];
        let first = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);
        let origin = Vec3::new(x, y, 50.0) - first + model.render_position(0);
        let hit = model.pick(origin, Vec3::NEG_Z).unwrap();
        (model, hit)
    }

    fn picker(select_by: SelectBy) -> Picker {
        Picker {
            select_by,
            ..Picker::default()
        }
    }

    fn faces(items: Items) -> BTreeSet<(ElementId, u8)> {
        match items {
            Items::Faces(faces) => faces,
            other => panic!("{other:?} instead of faces"),
        }
    }

    fn nodes(items: Items) -> BTreeSet<NodeId> {
        match items {
            Items::Nodes(nodes) => nodes,
            other => panic!("{other:?} instead of nodes"),
        }
    }

    #[test]
    fn modes_pick_faces_of_the_beam() {
        let (model, hit) = beam_hit(95.0, 5.0);
        let pick = |mode| faces(picker(mode).pick(&model, &hit, Target::Faces, 0.5));
        let single = pick(SelectBy::Surface);
        assert_eq!(single.len(), 1);
        assert_eq!(single.first().unwrap().1, 2, "top face of a hex is S2");
        // The top of the beam is one patch of 10 x 2 element faces.
        assert_eq!(pick(SelectBy::Geometry).len(), 20);
        assert_eq!(pick(SelectBy::FaceAngle).len(), 20);
        assert_eq!(
            pick(SelectBy::Element).len(),
            3,
            "top, end and side face of the element"
        );
        // 4 x 10 x 2 long sides and 2 x 2 x 2 ends.
        assert_eq!(pick(SelectBy::Part).len(), 88);
    }

    #[test]
    fn modes_pick_nodes_of_the_beam() {
        let pick = |x, y, mode| {
            let (model, hit) = beam_hit(x, y);
            let nodes = nodes(picker(mode).pick(&model, &hit, Target::Nodes, 0.5));
            let coords: Vec<[f64; 3]> =
                nodes.iter().map(|&n| model.mesh.node(n).unwrap()).collect();
            coords
        };
        // Inside a face the geometry mode takes the whole top: 11 x 3 nodes.
        assert_eq!(pick(55.0, 5.0, SelectBy::Geometry).len(), 33);
        // Near the long top edge at y = 0 it takes that edge.
        let edge = pick(55.0, 0.3, SelectBy::Geometry);
        assert_eq!(edge.len(), 11);
        assert!(edge.iter().all(|p| p[1] == 0.0 && p[2] == 10.0));
        // Near the corner only the corner.
        assert_eq!(
            pick(99.8, 0.2, SelectBy::Geometry),
            vec![[100.0, 0.0, 10.0]]
        );
        // Edge modes run to the corners; with an angle they stop at the 90° bends too.
        assert_eq!(pick(55.0, 2.0, SelectBy::Edge).len(), 11);
        assert_eq!(pick(55.0, 2.0, SelectBy::EdgeAngle).len(), 11);
        let mut wide = picker(SelectBy::EdgeAngle);
        wide.angle = 95.0;
        let (model, hit) = beam_hit(55.0, 2.0);
        let outline = nodes(wide.pick(&model, &hit, Target::Nodes, 0.5));
        assert!(
            outline.len() > 11,
            "a 95° limit follows the outline around the corners"
        );
    }

    #[test]
    fn boxes_take_items_inside_or_crossing() {
        let (model, _) = beam_hit(55.0, 5.0);
        // Seen from above, the beam spans -50..50 in x around the model centre; the box
        // covers the half with x >= 0.
        let area = |crossing| BoxSelect {
            // Orthographic view from above: x and y scaled into -1..1, depth around 0.5.
            view_proj: glam::Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5))
                * glam::Mat4::from_scale(Vec3::new(1.0 / 60.0, 1.0 / 60.0, -0.005)),
            min: glam::Vec2::new(-0.0001, -1.0),
            max: glam::Vec2::new(1.0, 1.0),
            crossing,
            shift: false,
            ctrl: false,
        };
        let pick = |mode, crossing, target| picker(mode).pick_box(&model, &area(crossing), target);
        // 5 elements along x times 2 on each of the 4 long sides, plus the 4 end faces.
        assert_eq!(
            faces(pick(SelectBy::Geometry, false, Target::Faces)).len(),
            44
        );
        // Crossing also takes the faces of the elements touching x = 0.
        assert_eq!(
            faces(pick(SelectBy::Geometry, true, Target::Faces)).len(),
            52
        );
        // 6 cross sections of 3 x 3 nodes.
        assert_eq!(nodes(pick(SelectBy::Node, false, Target::Nodes)).len(), 54);
    }

    #[test]
    fn ids_and_ranges_are_parsed() {
        assert_eq!(parse_ids("1, 5 10-12"), Ok(vec![1, 5, 10, 11, 12]));
        assert!(parse_ids("1, x").is_err());
        assert!(parse_ids("5-2").is_err());
    }

    /// A 40 x 20 plate of 4 x 2 plane stress elements in the x-y plane, hit from +z at
    /// (x, y).
    fn plate_hit(x: f32, y: f32) -> (Model, Hit) {
        let mut mesh = plx_mesh::FeMesh::default();
        let id = |i: u32, j: u32| j * 5 + i + 1;
        for j in 0..=2 {
            for i in 0..=4 {
                mesh.set_node(id(i, j), [f64::from(i) * 10.0, f64::from(j) * 10.0, 0.0]);
            }
        }
        let mut part = plx_mesh::Part {
            name: "PLATE".into(),
            elements: Vec::new(),
        };
        for j in 0..2 {
            for i in 0..4 {
                let element = j * 4 + i + 1;
                mesh.add_element(plx_mesh::Element {
                    id: element,
                    type_name: "CPS4".into(),
                    shape: plx_mesh::ElementShape::Quad4,
                    nodes: vec![id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1)],
                })
                .unwrap();
                part.elements.push(element);
            }
        }
        mesh.parts.push(part);
        let model = Model::new(std::path::Path::new("platte.inp"), mesh);
        let origin = Vec3::new(x, y, 50.0) + model.global_origin();
        let hit = model.pick(origin, Vec3::NEG_Z).unwrap();
        (model, hit)
    }

    #[test]
    fn two_d_loads_pick_edges_of_the_outline() {
        let (model, hit) = plate_hit(1.0, 8.0);
        let pick = |mode| faces(picker(mode).pick(&model, &hit, Target::Edges, 2.0));
        // The left side, edge 4 of the two elements there.
        assert_eq!(pick(SelectBy::Geometry), BTreeSet::from([(1, 4), (5, 4)]));
        assert_eq!(pick(SelectBy::Part).len(), 12);
        // The corner element has two edges on the outline.
        assert_eq!(pick(SelectBy::Element), BTreeSet::from([(1, 1), (1, 4)]));
        // Boundary conditions take the nodes of the left side only, not the whole outline.
        let left = nodes(picker(SelectBy::Geometry).pick(&model, &hit, Target::Nodes, 2.0));
        assert_eq!(left, BTreeSet::from([1, 6, 11]));
        let (model, corner) = plate_hit(0.5, 0.5);
        let picked = picker(SelectBy::Geometry).pick(&model, &corner, Target::Nodes, 2.0);
        assert_eq!(nodes(picked), BTreeSet::from([1]));
        let preview = preview(&model, &Items::Faces(BTreeSet::from([(1, 4), (5, 4)])));
        assert_eq!(preview.lines.len(), 2);
    }
}
