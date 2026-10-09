//! The model tree in PrePoMax's three views Geometry, FE Model and Results, with PrePoMax's
//! node names. Nodes for features prepolix does not support yet are shown as empty
//! placeholders, so that the structure is already the familiar one.

use std::collections::{HashMap, HashSet};

use egui::collapsing_header::CollapsingState;
use egui::epaint::Mesh;
use egui::{Color32, Pos2, Rect, Response, Shape, Ui, Vec2, WidgetText, pos2, vec2};
use plx_job::JobStatus;
use plx_model::ModelItem;

use crate::model::{Model, PartInfo};
use crate::setup::NewItem;
use crate::tree_icons::{self, TreeIcon};

/// Which of the three trees is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TreeView {
    Geometry,
    FeModel,
    Results,
}

impl TreeView {
    pub fn title(self) -> &'static str {
        match self {
            TreeView::Geometry => "Geometry",
            TreeView::FeModel => "FE Model",
            TreeView::Results => "Results",
        }
    }
}

/// A node of a tree; placeholders and group nodes are identified by their name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TreeItem {
    Group(&'static str),
    Model,
    Mesh,
    Part(usize),
    NodeSet(String),
    ElementSet(String),
    Surface(String),
    Material(usize),
    Section(usize),
    Step(usize),
    /// A container inside a step, such as "BCs".
    StepGroup(usize, &'static str),
    BoundaryCondition(usize, usize),
    Load(usize, usize),
    FieldOutput(usize, usize),
    Analysis,
    HotSpot(usize),
    FieldOutputs,
    /// Field of the current increment, by index.
    Field(usize),
    /// Field of the current increment computed from a derived field output, by index.
    ResultFieldOutput(usize),
    /// A computed history output, by index of its data.
    HistorySet(usize),
    HistoryField(usize, usize),
    HistoryComponent(usize, usize, usize),
    Component(usize, usize),
}

/// Selection shared by the three trees; an item is selected in one view only.
#[derive(Default)]
pub struct TreeState {
    pub selected: Option<(TreeView, TreeItem)>,
    /// Expand (true) or collapse an item with all its descendants in the next frame.
    expand: Option<(TreeView, TreeItem, bool)>,
    /// The selection was made in the 3D view: open its branches and scroll it into view.
    pub reveal: bool,
}

/// What the user did in the tree this frame.
#[derive(Default)]
pub struct TreeResponse {
    pub visibility: Vec<(usize, bool)>,
    /// A result component was picked (field, component).
    pub component: Option<(usize, usize)>,
    /// An item was double-clicked: show its properties.
    pub open: Option<TreeItem>,
    /// Create a new item from a container's context menu or by double-clicking it.
    pub create: Option<NewItem>,
    pub delete: Option<TreeItem>,
    /// Run the analysis.
    pub run: bool,
    /// Open the material library.
    pub material_library: bool,
    /// Open the meshing parameters of the geometry.
    pub mesh_setup: bool,
    /// Mesh the geometry.
    pub generate_mesh: bool,
    /// Evaluate the hot spots with the current results.
    pub evaluate_hot_spots: bool,
}

/// Tree label with a fixed size: highlight and hover frame are painted over the same area, so
/// that hovering never moves the rows below (egui's selectable label grows by its frame).
fn row_label(
    ui: &mut Ui,
    selected: bool,
    color: Option<Color32>,
    text: impl Into<WidgetText>,
) -> Response {
    let padding = egui::vec2(3.0, 1.0);
    let mut text = text.into();
    if let Some(color) = color.filter(|_| !selected) {
        text = text.color(color);
    }
    let galley = text.into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let (rect, response) =
        ui.allocate_exact_size(galley.size() + 2.0 * padding, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let text_color = if selected {
            visuals.selection.stroke.color
        } else {
            visuals.text_color()
        };
        if selected {
            ui.painter()
                .rect_filled(rect, 0.0, visuals.selection.bg_fill);
        } else if response.hovered() {
            ui.painter().rect(
                rect,
                0.0,
                crate::style::HOVER_FILL,
                egui::Stroke::new(1.0, crate::style::HIGHLIGHT),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().galley(rect.min + padding, galley, text_color);
    }
    response
}

/// Text colour of invalid items and of the containers holding them, Windows' red as in
/// PrePoMax.
const INVALID: Color32 = Color32::from_rgb(255, 0, 0);

/// Warning sign next to an invalid item, PrePoMax's warning icon.
fn warning_sign(ui: &mut Ui) -> Response {
    ui.add_space(3.0);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(tree_icons::SIZE), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        tree_icons::paint(ui.painter(), rect.min, TreeIcon::Warning);
    }
    response
}

/// Tree item of an item of the FE model and the containers it is shown in, innermost first.
fn tree_items(item: ModelItem) -> (TreeItem, Vec<TreeItem>) {
    let step = |s: usize, group: &'static str| {
        vec![
            TreeItem::StepGroup(s, group),
            TreeItem::Step(s),
            TreeItem::Group("Steps"),
            TreeItem::Model,
        ]
    };
    match item {
        ModelItem::Section(i) => (
            TreeItem::Section(i),
            vec![TreeItem::Group("Sections"), TreeItem::Model],
        ),
        ModelItem::BoundaryCondition(s, i) => (TreeItem::BoundaryCondition(s, i), step(s, "BCs")),
        ModelItem::Load(s, i) => (TreeItem::Load(s, i), step(s, "Loads")),
    }
}

/// What double-clicking a container creates.
fn creates(item: &TreeItem) -> Option<NewItem> {
    match *item {
        TreeItem::Group("Materials") => Some(NewItem::Material),
        TreeItem::Group("Sections") => Some(NewItem::Section),
        TreeItem::Group("Steps") => Some(NewItem::Step),
        TreeItem::StepGroup(step, "BCs") => Some(NewItem::BoundaryCondition(step)),
        TreeItem::StepGroup(step, "Loads") => Some(NewItem::Load(step)),
        TreeItem::Group(HOT_SPOTS) => Some(NewItem::HotSpot),
        TreeItem::FieldOutputs => Some(NewItem::ResultFieldOutput),
        TreeItem::Group("History Outputs") => Some(NewItem::ResultHistoryOutput),
        _ => None,
    }
}

/// Whether double-clicking opens a dialog. As in PrePoMax, containers such as "Mesh" or
/// "Parts" have none: they create their kind of item or open and close.
fn has_properties(item: &TreeItem) -> bool {
    !matches!(
        item,
        TreeItem::Group(_)
            | TreeItem::Mesh
            | TreeItem::StepGroup(..)
            | TreeItem::FieldOutputs
            | TreeItem::HistoryField(..)
    )
}

/// Items of the FE model that have an edit dialog.
fn is_fe_item(item: &TreeItem) -> bool {
    matches!(
        item,
        TreeItem::Material(_)
            | TreeItem::Section(_)
            | TreeItem::Step(_)
            | TreeItem::BoundaryCondition(..)
            | TreeItem::Load(..)
            | TreeItem::FieldOutput(..)
            | TreeItem::HotSpot(_)
    )
}

/// "Name (n)" when there are entries, as PrePoMax labels its containers.
fn counted(name: &str, count: usize) -> String {
    if count > 0 {
        format!("{name} ({count})")
    } else {
        name.to_string()
    }
}

/// Colour of the dotted lines that connect the nodes, as in the Windows tree view.
const LINE: Color32 = Color32::from_rgb(160, 160, 160);

/// Height of the connector line below the top of a row's icon: PrePoMax's dotted images
/// have their line in pixel row 9.
const LINE_OFFSET: f32 = 9.0;

/// Dotted line between two points on a horizontal or vertical, one screen pixel wide with a
/// dot on every second pixel, so that crossing lines share their dots. Drawn as a plain mesh,
/// as egui would blur rectangles of a single pixel.
fn dotted(mesh: &mut Mesh, ppp: f32, a: Pos2, b: Pos2) {
    let px = |v: f32| (v * ppp).floor() as i32;
    let mut dot = |x: i32, y: i32| {
        let min = pos2(x as f32 / ppp, y as f32 / ppp);
        mesh.add_colored_rect(Rect::from_min_size(min, Vec2::splat(1.0 / ppp)), LINE);
    };
    let (x0, y0, x1, y1) = (px(a.x), px(a.y), px(b.x), px(b.y));
    if x0 == x1 {
        let first = y0.min(y1);
        for y in (first + first.rem_euclid(2)..=y0.max(y1)).step_by(2) {
            dot(x0, y);
        }
    } else {
        let first = x0.min(x1);
        for x in (first + first.rem_euclid(2)..=x0.max(x1)).step_by(2) {
            dot(x, y0);
        }
    }
}

/// The plus and minus box of the classic Windows tree view, on whole pixels.
fn expander(ui: &mut Ui, openness: f32, response: &Response) {
    let ppp = ui.pixels_per_point();
    let snap = |v: f32| (v * ppp).floor() / ppp;
    let center = response.rect.center();
    // Centred on the connector line of the row.
    let min = pos2(
        snap(center.x) - 4.0,
        snap(center.y - 8.0 + LINE_OFFSET) - 4.0,
    );
    let mut mesh = Mesh::default();
    let mut fill = |x: f32, y: f32, w: f32, h: f32, color: Color32| {
        let rect = Rect::from_min_size(min + vec2(x, y), vec2(w, h));
        mesh.add_colored_rect(rect, color);
    };
    fill(0.0, 0.0, 9.0, 9.0, Color32::from_rgb(145, 145, 145));
    fill(1.0, 1.0, 7.0, 7.0, Color32::WHITE);
    fill(2.0, 4.0, 5.0, 1.0, Color32::BLACK);
    if openness < 0.5 {
        fill(4.0, 2.0, 1.0, 5.0, Color32::BLACK);
    }
    ui.painter().add(Shape::mesh(mesh));
}

/// Icon of a part after its element types, as PrePoMax tells solid, shell and beam parts apart.
fn part_icon(part: &PartInfo) -> TreeIcon {
    if !part.visible {
        return TreeIcon::Hidden;
    }
    let is = |prefixes: &[&str]| {
        (part.element_types.iter())
            .any(|(name, _)| prefixes.iter().any(|p| name.to_uppercase().starts_with(p)))
    };
    if is(&["C3D", "F3D"]) {
        TreeIcon::Solid
    } else if is(&["S", "M3D", "CPS", "CPE", "CAX"]) {
        TreeIcon::Shell
    } else if is(&["B", "T"]) {
        TreeIcon::Wire
    } else {
        TreeIcon::Solid
    }
}

/// Context menu of a part, the same in the tree and in the 3D view.
pub fn part_menu(ui: &mut Ui, index: usize, visible: bool, response: &mut TreeResponse) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    if ui.button("Eigenschaften …").clicked() {
        response.open = Some(TreeItem::Part(index));
    }
    let label = if visible { "Ausblenden" } else { "Einblenden" };
    if ui.button(label).clicked() {
        response.visibility.push((index, !visible));
    }
}

/// A row of a branch: the height of its connector line and the left edge of the row.
#[derive(Clone, Copy)]
struct Row {
    y: f32,
    left: f32,
}

struct Tree<'a> {
    view: TreeView,
    state: &'a mut TreeState,
    response: TreeResponse,
    /// Status of the analysis job, shown as the icon of the analysis.
    job: Option<JobStatus>,
    /// Rows of the open branches, innermost last, for the connector lines.
    levels: Vec<Vec<Row>>,
    /// Inside an item being expanded or collapsed: the state all branches take.
    forced_open: Option<bool>,
    /// Items whose references are gone, with the reason, shown red with a warning sign.
    invalid: HashMap<TreeItem, String>,
    /// Containers holding invalid items, shown red so that they are found when collapsed.
    holds_invalid: HashSet<TreeItem>,
    /// Containers that cannot take items, with the reason, such as the loads of a frequency
    /// step.
    closed: HashMap<TreeItem, &'static str>,
}

impl Tree<'_> {
    fn is_selected(&self, item: &TreeItem) -> bool {
        self.state
            .selected
            .as_ref()
            .is_some_and(|(view, selected)| *view == self.view && selected == item)
    }

    /// Selectable label of an item: a click selects it, a double click opens its properties.
    fn label(&mut self, ui: &mut Ui, item: TreeItem, text: impl Into<WidgetText>) -> Response {
        let reason = self.invalid.get(&item).cloned();
        let red = reason.is_some() || self.holds_invalid.contains(&item);
        let mut response = row_label(ui, self.is_selected(&item), red.then_some(INVALID), text);
        if let Some(reason) = reason {
            warning_sign(ui).on_hover_text(&reason);
            response = response.on_hover_text(reason);
        }
        // Like PrePoMax, a right click selects the item its context menu belongs to.
        if response.clicked() || response.double_clicked() || response.secondary_clicked() {
            self.state.selected = Some((self.view, item.clone()));
        }
        if self.state.reveal && self.is_selected(&item) {
            response.scroll_to_me(None);
        }
        let closed = self.closed.get(&item).copied();
        if let Some(reason) = closed {
            response = response.on_hover_text(reason);
        }
        let creates = creates(&item).filter(|_| closed.is_none());
        let meshing = self.view == TreeView::Geometry
            && matches!(
                item,
                TreeItem::Group("Mesh Setup") | TreeItem::Group("Parts")
            );
        if response.double_clicked() && item == TreeItem::Group("Mesh Setup") {
            self.response.mesh_setup = true;
        }
        if meshing {
            response.context_menu(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if ui.button("Netzparameter …").clicked() {
                    self.response.mesh_setup = true;
                }
                if ui.button("Netz erzeugen").clicked() {
                    self.response.generate_mesh = true;
                }
            });
        }
        if response.double_clicked() {
            match creates {
                // PrePoMax creates an item when its container is double-clicked.
                Some(kind) => self.response.create = Some(kind),
                None if has_properties(&item) => self.response.open = Some(item.clone()),
                // Other containers only open or close, see `branch`.
                None => {}
            }
        }
        let editable = is_fe_item(&item)
            || matches!(
                item,
                TreeItem::ResultFieldOutput(_) | TreeItem::HistorySet(_)
            );
        if creates.is_some() || editable || item == TreeItem::Analysis {
            response.context_menu(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if let Some(kind) = creates
                    && ui.button("Erstellen …").clicked()
                {
                    self.response.create = Some(kind);
                }
                if item == TreeItem::Group("Materials") {
                    ui.separator();
                    if ui.button("Materialbibliothek …").clicked() {
                        self.response.material_library = true;
                    }
                }
                if item == TreeItem::Group(HOT_SPOTS) {
                    ui.separator();
                    if ui.button("Mit aktuellen Ergebnissen auswerten").clicked() {
                        self.response.evaluate_hot_spots = true;
                    }
                }
                if creates.is_some() {
                    ui.separator();
                    if ui.button("Alle aufklappen").clicked() {
                        self.state.expand = Some((self.view, item.clone(), true));
                    }
                    if ui.button("Alle zuklappen").clicked() {
                        self.state.expand = Some((self.view, item.clone(), false));
                    }
                }
                if editable {
                    if ui.button("Bearbeiten …").clicked() {
                        self.response.open = Some(item.clone());
                    }
                    let deletable = !matches!(item, TreeItem::FieldOutput(..));
                    if deletable && ui.button("Löschen").clicked() {
                        self.response.delete = Some(item.clone());
                    }
                }
                if item == TreeItem::Analysis && ui.button("Starten").clicked() {
                    self.response.run = true;
                }
            });
        }
        response
    }

    /// PrePoMax's image of a node; containers without an own image get the dotted line.
    fn icon(&self, item: &TreeItem, open: bool) -> TreeIcon {
        let dots = if open {
            TreeIcon::DotsOpen
        } else {
            TreeIcon::Dots
        };
        match item {
            TreeItem::Mesh => TreeIcon::Mesh,
            TreeItem::Group("Parts") if self.view == TreeView::Geometry => TreeIcon::Geometry,
            TreeItem::Group("Parts") => TreeIcon::Part,
            TreeItem::Group("Mesh Setup") => TreeIcon::MeshSetup,
            TreeItem::Group("Node Sets") => TreeIcon::NodeSet,
            TreeItem::Group("Element Sets") => TreeIcon::ElementSet,
            TreeItem::Group("Surfaces") => TreeIcon::Surface,
            TreeItem::Group("Features") => TreeIcon::Features,
            TreeItem::Group("Reference Points") => TreeIcon::ReferencePoint,
            TreeItem::Group("Coordinate Systems") => TreeIcon::CoordinateSystem,
            TreeItem::Group("Materials") => TreeIcon::Material,
            TreeItem::Group("Sections") => TreeIcon::Section,
            TreeItem::Group("Constraints") => TreeIcon::Constraints,
            TreeItem::Group("Contacts") => TreeIcon::Contacts,
            TreeItem::Group("Surface Interactions") => TreeIcon::SurfaceInteraction,
            TreeItem::Group("Contact Pairs") => TreeIcon::ContactPair,
            TreeItem::Group("Distributions") => TreeIcon::Distribution,
            TreeItem::Group("Amplitudes") => TreeIcon::Amplitude,
            TreeItem::Group("Initial Conditions") => TreeIcon::InitialConditions,
            TreeItem::Group("Steps") => TreeIcon::Steps,
            TreeItem::Step(_) => TreeIcon::Step,
            TreeItem::StepGroup(_, "Field Outputs") | TreeItem::FieldOutputs => {
                TreeIcon::FieldOutput
            }
            TreeItem::StepGroup(_, "History Outputs") | TreeItem::Group("History Outputs") => {
                TreeIcon::HistoryOutput
            }
            TreeItem::StepGroup(_, "BCs") => TreeIcon::BoundaryCondition,
            TreeItem::StepGroup(_, "Loads") => TreeIcon::Load,
            TreeItem::StepGroup(_, "Defined Fields") => TreeIcon::DefinedField,
            TreeItem::Group("Analyses") => TreeIcon::Analysis,
            TreeItem::Group(HOT_SPOTS) => TreeIcon::HotSpot,
            TreeItem::Analysis => match self.job {
                Some(JobStatus::Running) => TreeIcon::Running,
                Some(JobStatus::Completed) => TreeIcon::Finished,
                Some(JobStatus::FailedWithResults) => TreeIcon::Warning,
                _ => TreeIcon::NoResult,
            },
            _ => dots,
        }
    }

    /// A row: plus and minus box (or its space), icon and label. `check` adds a check box in
    /// front of the icon. Records the row for the connector lines of its branch.
    fn row(
        &mut self,
        ui: &mut Ui,
        state: Option<&mut CollapsingState>,
        check: Option<&mut bool>,
        icon: TreeIcon,
        item: TreeItem,
        text: impl Into<WidgetText>,
    ) -> (Row, Response, bool) {
        ui.horizontal(|ui| {
            let left = ui.cursor().left();
            ui.spacing_mut().item_spacing.x = 0.0;
            match state {
                Some(state) => {
                    state.show_toggle_button(ui, expander);
                }
                None => ui.add_space(ui.spacing().indent),
            }
            let changed = check.is_some_and(|checked| {
                let changed = ui.checkbox(checked, "").changed();
                ui.add_space(2.0);
                changed
            });
            let (rect, _) =
                ui.allocate_exact_size(Vec2::splat(tree_icons::SIZE), egui::Sense::hover());
            if ui.is_rect_visible(rect) {
                tree_icons::paint(ui.painter(), rect.min, icon);
            }
            ui.add_space(3.0);
            let response = self.label(ui, item, text);
            let row = Row {
                y: rect.top() + LINE_OFFSET,
                left,
            };
            if let Some(level) = self.levels.last_mut() {
                level.push(row);
            }
            (row, response, changed)
        })
        .inner
    }

    /// Node without children, aligned with the labels of sibling branches.
    fn leaf(&mut self, ui: &mut Ui, item: TreeItem, text: impl Into<WidgetText>) -> Response {
        let icon = self.icon(&item, false);
        self.row(ui, None, None, icon, item, text).1
    }

    fn branch(
        &mut self,
        ui: &mut Ui,
        item: TreeItem,
        text: impl Into<WidgetText>,
        default_open: bool,
        body: impl FnOnce(&mut Self, &mut Ui),
    ) {
        let id = ui.make_persistent_id((self.view, &item));
        let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
        let outer = self.forced_open;
        if let Some((view, target, open)) = &self.state.expand
            && *view == self.view
            && *target == item
        {
            self.forced_open = Some(*open);
        }
        let revealing_part = self.state.reveal
            && matches!(&self.state.selected, Some((view, TreeItem::Part(_))) if *view == self.view);
        if revealing_part
            && matches!(
                item,
                TreeItem::Model | TreeItem::Mesh | TreeItem::Group("Parts")
            )
        {
            state.set_open(true);
        }
        if let Some(open) = self.forced_open {
            // While a branch closes, egui still draws its body for the animation, so the
            // descendants are collapsed as well.
            state.set_open(open);
        }
        let toggles = creates(&item).is_none() && !has_properties(&item);
        let icon = self.icon(&item, state.is_open());
        let (row, header, _) = self.row(ui, Some(&mut state), None, icon, item, text);
        // Lines go below the rows of the body, so that the boxes of sub-branches cover them.
        let lines = ui.painter().add(Shape::Noop);
        self.levels.push(Vec::new());
        let body = state.show_body_indented(&header, ui, |ui| body(self, ui));
        let rows = self.levels.pop().unwrap_or_default();
        if let Some(body) = body {
            let x = row.left + ui.spacing().indent / 2.0;
            let shapes = self.connectors(ui, x, row.y + 5.0, &rows, body.response.rect.bottom());
            ui.painter().set(lines, shapes);
        }
        self.forced_open = outer;
        if toggles && header.double_clicked() {
            let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
            state.toggle(ui);
            state.store(ui.ctx());
        }
    }

    /// Dotted lines from a vertical at `x`, starting at `top`, to the icons of `rows`; rows
    /// below `bottom` are hidden by the opening animation.
    fn connectors(&self, ui: &Ui, x: f32, top: f32, rows: &[Row], bottom: f32) -> Shape {
        let ppp = ui.pixels_per_point();
        let indent = ui.spacing().indent;
        let mut mesh = Mesh::default();
        let rows: Vec<Row> = rows.iter().copied().filter(|r| r.y <= bottom).collect();
        if let Some(last) = rows.last() {
            dotted(&mut mesh, ppp, pos2(x, top), pos2(x, last.y));
        }
        for row in rows {
            dotted(
                &mut mesh,
                ppp,
                pos2(x, row.y),
                pos2(row.left + indent - 1.0, row.y),
            );
        }
        Shape::mesh(mesh)
    }

    /// Group node that may be empty: a placeholder leaf without children.
    fn group(&mut self, ui: &mut Ui, name: &'static str, children: &[&'static str]) {
        if children.is_empty() {
            self.leaf(ui, TreeItem::Group(name), name);
        } else {
            self.branch(ui, TreeItem::Group(name), name, true, |tree, ui| {
                for &child in children {
                    tree.leaf(ui, TreeItem::Group(child), child);
                }
            });
        }
    }

    /// Parts with visibility and colour; of the mesh, or of the geometry.
    fn parts(&mut self, ui: &mut Ui, model: &mut Model) {
        let parts = counted("Parts", model.parts.len());
        self.branch(ui, TreeItem::Group("Parts"), parts, true, |tree, ui| {
            for (index, part) in model.parts.iter_mut().enumerate() {
                let icon = part_icon(part);
                let item = TreeItem::Part(index);
                let (_, response, changed) =
                    tree.row(ui, None, Some(&mut part.visible), icon, item, &part.name);
                if changed {
                    tree.response.visibility.push((index, part.visible));
                }
                response.context_menu(|ui| {
                    part_menu(ui, index, part.visible, &mut tree.response);
                });
            }
        });
    }

    /// Mesh with parts and sets; shared by the FE Model and Results trees.
    fn mesh(&mut self, ui: &mut Ui, model: Option<&mut Model>) {
        let Some(model) = model else {
            self.branch(ui, TreeItem::Mesh, "Mesh", true, |tree, ui| {
                for name in ["Parts", "Node Sets", "Element Sets", "Surfaces"] {
                    tree.leaf(ui, TreeItem::Group(name), name);
                }
            });
            return;
        };
        self.branch(ui, TreeItem::Mesh, "Mesh", true, |tree, ui| {
            tree.parts(ui, model);
            let mesh = &model.mesh;
            let sets: [(&'static str, Vec<TreeItem>); 3] = [
                (
                    "Node Sets",
                    mesh.node_sets
                        .keys()
                        .map(|n| TreeItem::NodeSet(n.clone()))
                        .collect(),
                ),
                (
                    "Element Sets",
                    mesh.element_sets
                        .keys()
                        .map(|n| TreeItem::ElementSet(n.clone()))
                        .collect(),
                ),
                (
                    "Surfaces",
                    mesh.surfaces
                        .keys()
                        .map(|n| TreeItem::Surface(n.clone()))
                        .collect(),
                ),
            ];
            for (name, items) in sets {
                if items.is_empty() {
                    tree.leaf(ui, TreeItem::Group(name), name);
                    continue;
                }
                let text = counted(name, items.len());
                tree.branch(ui, TreeItem::Group(name), text, false, |tree, ui| {
                    for item in items {
                        let label = match &item {
                            TreeItem::NodeSet(n)
                            | TreeItem::ElementSet(n)
                            | TreeItem::Surface(n) => n.clone(),
                            _ => String::new(),
                        };
                        tree.leaf(ui, item, label);
                    }
                });
            }
        });
    }

    /// Container of model items, e.g. "Materials (2)".
    fn container(&mut self, ui: &mut Ui, name: &'static str, items: Vec<(TreeItem, &str)>) {
        if items.is_empty() {
            self.leaf(ui, TreeItem::Group(name), name);
            return;
        }
        let text = counted(name, items.len());
        self.branch(ui, TreeItem::Group(name), text, true, |tree, ui| {
            for (item, label) in items {
                tree.leaf(ui, item, label);
            }
        });
    }

    fn step_container(
        &mut self,
        ui: &mut Ui,
        step: usize,
        name: &'static str,
        items: Vec<(TreeItem, &str)>,
    ) {
        let group = TreeItem::StepGroup(step, name);
        if items.is_empty() {
            self.leaf(ui, group, name);
            return;
        }
        let text = counted(name, items.len());
        self.branch(ui, group, text, true, |tree, ui| {
            for (item, label) in items {
                tree.leaf(ui, item, label);
            }
        });
    }

    fn features(&mut self, ui: &mut Ui) {
        self.group(ui, "Features", &["Reference Points", "Coordinate Systems"]);
    }
}

pub fn show(
    ui: &mut Ui,
    view: TreeView,
    model: Option<&mut Model>,
    job: Option<JobStatus>,
    state: &mut TreeState,
) -> TreeResponse {
    let mut tree = Tree {
        view,
        state,
        response: TreeResponse::default(),
        job,
        levels: vec![Vec::new()],
        forced_open: None,
        invalid: HashMap::new(),
        holds_invalid: HashSet::new(),
        closed: HashMap::new(),
    };
    let expanding = tree.state.expand.clone();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The dotted connectors replace egui's indent line.
            ui.visuals_mut().indent_has_left_vline = false;
            // The root nodes are connected too, as with PrePoMax's root lines.
            let lines = ui.painter().add(Shape::Noop);
            match view {
                TreeView::Geometry => {
                    match model {
                        Some(model) => tree.parts(ui, model),
                        None => {
                            tree.leaf(ui, TreeItem::Group("Parts"), "Parts");
                        }
                    }
                    tree.leaf(ui, TreeItem::Group("Mesh Setup"), "Mesh Setup");
                }
                TreeView::FeModel => fe_model(&mut tree, ui, model),
                TreeView::Results => results(&mut tree, ui, model),
            }
            let roots = tree.levels.pop().unwrap_or_default();
            if let Some(first) = roots.first() {
                let x = first.left + ui.spacing().indent / 2.0;
                let shapes = tree.connectors(ui, x, first.y, &roots, f32::INFINITY);
                ui.painter().set(lines, shapes);
            }
        });
    // Applied for one frame; a request made in this frame's context menu waits for the next.
    if tree.state.expand == expanding {
        tree.state.expand = None;
    }
    tree.state.reveal = false;
    tree.response
}

fn fe_model(tree: &mut Tree, ui: &mut Ui, model: Option<&mut Model>) {
    // A results file has no FE model, as in PrePoMax; its mesh lives in the Results tree.
    let model = model.filter(|m| !m.is_results());
    if let Some(model) = &model {
        for invalid in model.fe.invalid_items(&model.mesh) {
            let (item, containers) = tree_items(invalid.item);
            tree.invalid.insert(item, invalid.reason);
            tree.holds_invalid.extend(containers);
        }
    }
    let fe = model.as_ref().map(|m| m.fe.clone()).unwrap_or_default();
    let has_model = model.is_some();
    tree.branch(ui, TreeItem::Model, "Model", true, |tree, ui| {
        tree.mesh(ui, model);
        tree.features(ui);
        let materials: Vec<(TreeItem, &str)> = (fe.materials.iter().enumerate())
            .map(|(i, m)| (TreeItem::Material(i), m.name.as_str()))
            .collect();
        tree.container(ui, "Materials", materials);
        let sections: Vec<(TreeItem, &str)> = (fe.sections.iter().enumerate())
            .map(|(i, s)| (TreeItem::Section(i), s.name.as_str()))
            .collect();
        tree.container(ui, "Sections", sections);
        tree.leaf(ui, TreeItem::Group("Constraints"), "Constraints");
        let contacts = TreeItem::Group("Contacts");
        tree.branch(ui, contacts, "Contacts", false, |tree, ui| {
            for name in ["Surface Interactions", "Contact Pairs"] {
                tree.leaf(ui, TreeItem::Group(name), name);
            }
        });
        for name in ["Distributions", "Amplitudes", "Initial Conditions"] {
            tree.leaf(ui, TreeItem::Group(name), name);
        }
        // Not in PrePoMax: hot spot stresses, evaluated on the results of the analysis.
        let hot_spots = (fe.hot_spots.iter().enumerate())
            .map(|(i, h)| (TreeItem::HotSpot(i), h.name.as_str()))
            .collect();
        tree.container(ui, HOT_SPOTS, hot_spots);
        let steps = TreeItem::Group("Steps");
        if fe.steps.is_empty() {
            tree.leaf(ui, steps, "Steps");
            return;
        }
        let text = counted("Steps", fe.steps.len());
        tree.branch(ui, steps, text, true, |tree, ui| {
            for (s, step) in fe.steps.iter().enumerate() {
                if !step.kind.supports_loads() {
                    tree.closed.insert(
                        TreeItem::StepGroup(s, "Loads"),
                        "Ein Frequency Step hat keine Lasten.",
                    );
                }
                tree.branch(ui, TreeItem::Step(s), &step.name, true, |tree, ui| {
                    let outputs = (step.field_outputs.iter().enumerate())
                        .map(|(i, f)| (TreeItem::FieldOutput(s, i), f.name.as_str()))
                        .collect();
                    tree.step_container(ui, s, "Field Outputs", outputs);
                    tree.step_container(ui, s, "History Outputs", Vec::new());
                    let bcs = (step.boundary_conditions.iter().enumerate())
                        .map(|(i, b)| (TreeItem::BoundaryCondition(s, i), b.name.as_str()))
                        .collect();
                    tree.step_container(ui, s, "BCs", bcs);
                    let loads = (step.loads.iter().enumerate())
                        .map(|(i, l)| (TreeItem::Load(s, i), l.name.as_str()))
                        .collect();
                    tree.step_container(ui, s, "Loads", loads);
                    tree.step_container(ui, s, "Defined Fields", Vec::new());
                });
            }
        });
    });
    if has_model {
        tree.branch(
            ui,
            TreeItem::Group("Analyses"),
            "Analyses (1)",
            true,
            |tree, ui| {
                tree.leaf(ui, TreeItem::Analysis, ANALYSIS_NAME);
            },
        );
    } else {
        tree.leaf(ui, TreeItem::Group("Analyses"), "Analyses");
    }
}

/// Container of the hot spot definitions.
pub const HOT_SPOTS: &str = "Hot Spot Stresses";

/// Name of the analysis job, PrePoMax's first default.
pub const ANALYSIS_NAME: &str = "Analysis-1";

/// A name with the names of its children, e.g. a field with its components.
type NamedList = (String, Vec<String>);

fn results(tree: &mut Tree, ui: &mut Ui, model: Option<&mut Model>) {
    let model = model.filter(|m| m.results.is_some());
    let mut fields = Vec::new();
    let mut active = None;
    let mut history: Vec<(String, Vec<NamedList>)> = Vec::new();
    if let Some(view) = model.as_ref().and_then(|m| m.results.as_ref()) {
        history = (view.history.iter())
            .map(|set| {
                let fields = (set.fields.iter())
                    .map(|f| {
                        let components = f.components.iter().map(|c| c.name.clone()).collect();
                        (f.name.clone(), components)
                    })
                    .collect();
                (set.name.clone(), fields)
            })
            .collect();
        if let Some(increment) = view.current_increment() {
            fields = increment
                .fields
                .iter()
                .map(|f| {
                    let components: Vec<String> =
                        f.components.iter().map(|c| c.name.clone()).collect();
                    let derived = view.field_outputs.iter().any(|o| o.name == f.name);
                    (f.name.clone(), components, derived)
                })
                .collect();
        }
        active = Some((view.field, view.component));
    }
    tree.branch(ui, TreeItem::Model, "Model", true, |tree, ui| {
        tree.mesh(ui, model);
        tree.features(ui);
    });
    tree.branch(
        ui,
        TreeItem::Group("Results"),
        "Results",
        true,
        |tree, ui| {
            let text = counted("Field Outputs", fields.len());
            if fields.is_empty() {
                tree.leaf(ui, TreeItem::FieldOutputs, text);
            } else {
                tree.branch(ui, TreeItem::FieldOutputs, text, true, |tree, ui| {
                    for (f, (name, components, derived)) in fields.into_iter().enumerate() {
                        let item = if derived {
                            TreeItem::ResultFieldOutput(f)
                        } else {
                            TreeItem::Field(f)
                        };
                        // PrePoMax opens the first two fields.
                        tree.branch(ui, item, name, f < 2, |tree, ui| {
                            for (c, component) in components.into_iter().enumerate() {
                                let item = TreeItem::Component(f, c);
                                if tree.leaf(ui, item, component).clicked()
                                    && active != Some((f, c))
                                {
                                    tree.response.component = Some((f, c));
                                }
                            }
                        });
                    }
                });
            }
            let group = TreeItem::Group("History Outputs");
            if history.is_empty() {
                tree.leaf(ui, group, "History Outputs");
                return;
            }
            let text = counted("History Outputs", history.len());
            tree.branch(ui, group, text, true, |tree, ui| {
                for (s, (name, fields)) in history.into_iter().enumerate() {
                    tree.branch(ui, TreeItem::HistorySet(s), name, true, |tree, ui| {
                        for (f, (name, components)) in fields.into_iter().enumerate() {
                            let item = TreeItem::HistoryField(s, f);
                            tree.branch(ui, item, name, true, |tree, ui| {
                                for (c, component) in components.into_iter().enumerate() {
                                    tree.leaf(ui, TreeItem::HistoryComponent(s, f, c), component);
                                }
                            });
                        }
                    });
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_have_no_properties_dialog() {
        for item in [
            TreeItem::Mesh,
            TreeItem::Group("Parts"),
            TreeItem::StepGroup(0, "BCs"),
            TreeItem::FieldOutputs,
        ] {
            assert!(!has_properties(&item), "{item:?}");
        }
        for item in [TreeItem::Model, TreeItem::Part(0), TreeItem::Material(0)] {
            assert!(has_properties(&item), "{item:?}");
        }
    }
}
