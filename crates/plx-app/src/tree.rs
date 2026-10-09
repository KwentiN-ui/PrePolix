//! The model tree in PrePoMax's three views Geometry, FE Model and Results, with PrePoMax's
//! node names. Nodes for features prepolix does not support yet are shown as empty
//! placeholders, so that the structure is already the familiar one.

use std::collections::{BTreeSet, HashMap, HashSet};

use egui::collapsing_header::CollapsingState;
use egui::epaint::Mesh;
use egui::{Color32, Pos2, Rect, Response, Shape, Ui, Vec2, WidgetText, pos2, vec2};
use plx_job::JobStatus;
use plx_model::{FeModel, Finding, ModelItem, Severity};

use crate::features::{FeatureItem, FeatureKind};
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
    /// An item of the geometry's mesh setup.
    MeshItem(usize),
    NodeSet(String),
    ElementSet(String),
    Surface(String),
    Material(usize),
    Section(usize),
    Constraint(usize),
    SurfaceInteraction(usize),
    ContactPair(usize),
    /// A node tie, listed with the contact pairs.
    NodeTie(usize),
    Amplitude(usize),
    InitialCondition(usize),
    Step(usize),
    /// A container inside a step, such as "BCs".
    StepGroup(usize, &'static str),
    BoundaryCondition(usize, usize),
    Load(usize, usize),
    FieldOutput(usize, usize),
    Analysis,
    /// Features of the FE model or of the shown results, by index.
    ReferencePoint(usize),
    CoordinateSystem(usize),
    Plane(usize),
    /// A result path of the shown results, by index.
    ResultPath(usize),
    /// Results on a plane of the shown results, by index.
    ResultPlane(usize),
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
    /// A hot spot definition of the current results, by index.
    HotSpot(usize),
}

/// Selection shared by the three trees; an item is selected in one view only.
#[derive(Default)]
pub struct TreeState {
    pub selected: Option<(TreeView, TreeItem)>,
    /// Expand (true) or collapse an item with all its descendants in the next frame.
    expand: Option<(TreeView, TreeItem, bool)>,
    /// The selection was made in the 3D view: open its branches and scroll it into view.
    pub reveal: bool,
    /// Parts selected along with the selected part, by Ctrl or Shift click, as in PrePoMax;
    /// they count only while a part of the same view is selected.
    more_parts: BTreeSet<usize>,
    /// Where the range of a Shift click starts: the part last clicked without Shift.
    anchor: Option<usize>,
}

impl TreeState {
    /// The parts selected in `view`, in ascending order; none unless a part is selected
    /// there.
    pub fn selected_parts(&self, view: TreeView) -> BTreeSet<usize> {
        match &self.selected {
            Some((selected, TreeItem::Part(index))) if *selected == view => {
                let mut parts = self.more_parts.clone();
                parts.insert(*index);
                parts
            }
            _ => BTreeSet::new(),
        }
    }

    /// Selects one part alone, by a plain click.
    pub fn select_part(&mut self, view: TreeView, index: usize) {
        self.select_parts(view, BTreeSet::from([index]), index);
        self.anchor = Some(index);
    }

    /// Ctrl click: adds a part to the selection, or takes it out when it is selected.
    pub fn toggle_part(&mut self, view: TreeView, index: usize) {
        let mut parts = self.selected_parts(view);
        if !parts.remove(&index) {
            parts.insert(index);
        }
        self.select_parts(view, parts, index);
        self.anchor = Some(index);
    }

    /// Adds parts to the selection; the last becomes the selected one.
    pub fn add_parts(&mut self, view: TreeView, added: impl IntoIterator<Item = usize>) {
        let mut parts = self.selected_parts(view);
        let mut lead = None;
        for index in added {
            parts.insert(index);
            lead = Some(index);
        }
        if let Some(lead) = lead {
            self.select_parts(view, parts, lead);
        }
    }

    /// Shift click in the tree: the parts from the anchor to `index`, added to the selection
    /// with Ctrl.
    pub fn select_range(&mut self, view: TreeView, index: usize, add: bool) {
        let anchor = (self.anchor)
            .filter(|_| !self.selected_parts(view).is_empty())
            .unwrap_or(index);
        let range = anchor.min(index)..=anchor.max(index);
        let mut parts = if add {
            self.selected_parts(view)
        } else {
            BTreeSet::new()
        };
        parts.extend(range);
        self.select_parts(view, parts, index);
        self.anchor = Some(anchor);
    }

    /// Selects `parts` with `lead` as the selected item, or the last part when `lead` is not
    /// among them; nothing when no part is left.
    fn select_parts(&mut self, view: TreeView, mut parts: BTreeSet<usize>, lead: usize) {
        let lead = if parts.contains(&lead) {
            Some(lead)
        } else {
            parts.last().copied()
        };
        match lead {
            Some(lead) => {
                parts.remove(&lead);
                self.selected = Some((view, TreeItem::Part(lead)));
                self.more_parts = parts;
            }
            None => {
                self.selected = None;
                self.more_parts.clear();
            }
        }
    }
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
    /// Delete an item, by its context menu or the Delete key; the app asks first.
    pub delete: Option<TreeItem>,
    /// Activate a deactivated step, boundary condition or load, or deactivate an active one.
    pub toggle_active: Option<TreeItem>,
    /// Swap master and slave of a tie, spring connection or contact pair.
    pub swap_master_slave: Option<TreeItem>,
    /// An entry of the analysis' context menu, or the monitor by double click.
    pub analysis: Option<AnalysisAction>,
    /// Open the material library.
    pub material_library: bool,
    /// Open the default meshing parameters of the geometry.
    pub mesh_defaults: bool,
    /// Mesh all parts of the geometry.
    pub generate_mesh: bool,
    /// Mesh one part of the geometry, by index.
    pub mesh_part: Option<usize>,
    /// Show the table of the hot spot values.
    pub hot_spot_table: bool,
    /// Open PrePoMax's Search Contact Pairs.
    pub search_contacts: bool,
    /// The warning sign of an item was clicked: explain its findings.
    pub findings: Option<Vec<Finding>>,
    /// Show the results on a plane, by index, or none.
    pub plane_result: Option<Option<usize>>,
}

/// What the user asked of the analysis, PrePoMax's analysis context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisAction {
    /// Edit the job: executable, work directory and threads, which are settings here.
    Edit,
    Run,
    CheckModel,
    Monitor,
    Results,
    Kill,
}

/// What the tree shows of the analysis job.
#[derive(Clone, Copy, Debug)]
pub struct JobState {
    pub status: JobStatus,
    /// A results file is there to be opened.
    pub results: bool,
}

/// Tree label with a fixed size: highlight and hover frame are painted over the same area, so
/// that hovering never moves the rows below (egui's selectable label grows by its frame).
pub(crate) fn row_label(
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

/// Text colour of deactivated items, PrePoMax's gray.
const INACTIVE: Color32 = Color32::from_rgb(128, 128, 128);

/// Items that can be deactivated, as far as prepolix has them; PrePoMax also deactivates
/// materials, surface interactions and outputs.
fn can_deactivate(item: &TreeItem) -> bool {
    matches!(
        item,
        TreeItem::Step(_)
            | TreeItem::BoundaryCondition(..)
            | TreeItem::Load(..)
            | TreeItem::Constraint(_)
            | TreeItem::ContactPair(_)
            | TreeItem::NodeTie(_)
            | TreeItem::InitialCondition(_)
    )
}

/// Warning sign next to an item with findings, PrePoMax's warning icon; a click explains them.
fn warning_sign(ui: &mut Ui) -> Response {
    ui.add_space(3.0);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(tree_icons::SIZE), egui::Sense::click());
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
        ModelItem::Constraint(i) => (
            TreeItem::Constraint(i),
            vec![TreeItem::Group("Constraints"), TreeItem::Model],
        ),
        ModelItem::ContactPair(i) => (
            TreeItem::ContactPair(i),
            vec![
                TreeItem::Group("Contact Pairs"),
                TreeItem::Group("Contacts"),
                TreeItem::Model,
            ],
        ),
        ModelItem::NodeTie(i) => (
            TreeItem::NodeTie(i),
            vec![
                TreeItem::Group("Contact Pairs"),
                TreeItem::Group("Contacts"),
                TreeItem::Model,
            ],
        ),
        ModelItem::BoundaryCondition(s, i) => (TreeItem::BoundaryCondition(s, i), step(s, "BCs")),
        ModelItem::Load(s, i) => (TreeItem::Load(s, i), step(s, "Loads")),
        ModelItem::Amplitude(i) => (
            TreeItem::Amplitude(i),
            vec![TreeItem::Group("Amplitudes"), TreeItem::Model],
        ),
        ModelItem::Material(i) => (
            TreeItem::Material(i),
            vec![TreeItem::Group("Materials"), TreeItem::Model],
        ),
        ModelItem::Part(i) => (
            TreeItem::Part(i),
            vec![TreeItem::Group("Parts"), TreeItem::Mesh, TreeItem::Model],
        ),
        ModelItem::Step(s) => (
            TreeItem::Step(s),
            vec![TreeItem::Group("Steps"), TreeItem::Model],
        ),
        ModelItem::BoundaryConditions(s) => {
            let mut containers = step(s, "BCs");
            containers.remove(0);
            (TreeItem::StepGroup(s, "BCs"), containers)
        }
        ModelItem::Analysis => (TreeItem::Analysis, vec![TreeItem::Group("Analyses")]),
        ModelItem::InitialCondition(i) => (
            TreeItem::InitialCondition(i),
            vec![TreeItem::Group("Initial Conditions"), TreeItem::Model],
        ),
    }
}

/// What double-clicking a container creates.
fn creates(item: &TreeItem) -> Option<NewItem> {
    match *item {
        TreeItem::Group("Materials") => Some(NewItem::Material),
        TreeItem::Group("Sections") => Some(NewItem::Section),
        TreeItem::Group("Steps") => Some(NewItem::Step),
        TreeItem::Group("Constraints") => Some(NewItem::Constraint),
        TreeItem::Group("Surface Interactions") => Some(NewItem::SurfaceInteraction),
        TreeItem::Group("Contact Pairs") => Some(NewItem::ContactPair),
        TreeItem::Group("Amplitudes") => Some(NewItem::Amplitude),
        TreeItem::Group("Initial Conditions") => Some(NewItem::InitialCondition),
        TreeItem::StepGroup(step, "BCs") => Some(NewItem::BoundaryCondition(step)),
        TreeItem::StepGroup(step, "Loads") => Some(NewItem::Load(step)),
        TreeItem::Group(HOT_SPOTS) => Some(NewItem::ResultHotSpot),
        TreeItem::Group(REFERENCE_POINTS) => Some(NewItem::Feature(FeatureKind::ReferencePoint)),
        TreeItem::Group(COORDINATE_SYSTEMS) => {
            Some(NewItem::Feature(FeatureKind::CoordinateSystem))
        }
        TreeItem::Group(PLANES) => Some(NewItem::Feature(FeatureKind::Plane)),
        TreeItem::Group(PATHS) => Some(NewItem::Feature(FeatureKind::ResultPath)),
        TreeItem::Group(PLANE_RESULTS) => Some(NewItem::Feature(FeatureKind::ResultPlane)),
        TreeItem::Group("Mesh Setup") => Some(NewItem::MeshSetupItem),
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
/// Items the context menu and the Delete key remove; a step's field outputs are not, nor
/// the parts of results.
fn deletable(view: TreeView, item: &TreeItem) -> bool {
    (is_fe_item(item) && !matches!(item, TreeItem::FieldOutput(..)))
        || (matches!(item, TreeItem::Part(_)) && view != TreeView::Results)
        || feature(item).is_some()
        || matches!(
            item,
            TreeItem::ResultFieldOutput(_) | TreeItem::HistorySet(_) | TreeItem::MeshItem(_)
        )
}

fn is_fe_item(item: &TreeItem) -> bool {
    matches!(
        item,
        TreeItem::Material(_)
            | TreeItem::Section(_)
            | TreeItem::Constraint(_)
            | TreeItem::SurfaceInteraction(_)
            | TreeItem::ContactPair(_)
            | TreeItem::NodeTie(_)
            | TreeItem::Amplitude(_)
            | TreeItem::InitialCondition(_)
            | TreeItem::Step(_)
            | TreeItem::BoundaryCondition(..)
            | TreeItem::Load(..)
            | TreeItem::FieldOutput(..)
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
/// Whether `item` lies in the branch `branch`, which opens to reveal it.
fn contains(branch: &TreeItem, item: &TreeItem) -> bool {
    use TreeItem::*;
    match branch {
        Model => !matches!(item, Model),
        Mesh | Group("Parts") => matches!(item, Part(_)),
        Group("Constraints") => matches!(item, Constraint(_)),
        Group("Amplitudes") => matches!(item, Amplitude(_)),
        Group("Contacts") => matches!(
            item,
            SurfaceInteraction(_)
                | ContactPair(_)
                | NodeTie(_)
                | Group("Surface Interactions")
                | Group("Contact Pairs")
        ),
        Group("Surface Interactions") => matches!(item, SurfaceInteraction(_)),
        Group("Contact Pairs") => matches!(item, ContactPair(_) | NodeTie(_)),
        _ => false,
    }
}

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

/// Context menu of a part, the same in the tree and in the 3D view. A part of the geometry
/// is meshed from here, as in PrePoMax. On one of several selected parts, hiding and
/// deleting take them all.
pub fn part_menu(
    ui: &mut Ui,
    index: usize,
    visible: bool,
    view: TreeView,
    selected: &BTreeSet<usize>,
    response: &mut TreeResponse,
) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    if view == TreeView::Geometry {
        if ui.button("Netz erzeugen").clicked() {
            response.mesh_part = Some(index);
        }
        ui.separator();
    }
    if ui.button("Eigenschaften …").clicked() {
        response.open = Some(TreeItem::Part(index));
    }
    let label = if visible { "Ausblenden" } else { "Einblenden" };
    if ui.button(label).clicked() {
        if selected.contains(&index) {
            (response.visibility).extend(selected.iter().map(|&i| (i, !visible)));
        } else {
            response.visibility.push((index, !visible));
        }
    }
    if deletable(view, &TreeItem::Part(index)) {
        ui.separator();
        if ui.button("Löschen").clicked() {
            response.delete = Some(TreeItem::Part(index));
        }
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
    /// The analysis job: its status is the icon of the analysis.
    job: Option<JobState>,
    /// Rows of the open branches, innermost last, for the connector lines.
    levels: Vec<Vec<Row>>,
    /// Inside an item being expanded or collapsed: the state all branches take.
    forced_open: Option<bool>,
    /// Problems found by the model checks, by item: shown with a warning sign, and red when
    /// CalculiX would abort.
    findings: HashMap<TreeItem, Vec<Finding>>,
    /// Containers holding items with errors, shown red so that they are found when collapsed.
    holds_invalid: HashSet<TreeItem>,
    /// Containers that cannot take items, with the reason, such as the loads of a frequency
    /// step.
    closed: HashMap<TreeItem, &'static str>,
    /// Deactivated items, shown gray with PrePoMax's no-entry sign.
    inactive: HashSet<TreeItem>,
    /// Items with a master and a slave, whose menu offers to swap them.
    master_slave: HashSet<TreeItem>,
}

impl Tree<'_> {
    fn is_selected(&self, item: &TreeItem) -> bool {
        match item {
            TreeItem::Part(index) => self.state.selected_parts(self.view).contains(index),
            _ => (self.state.selected.as_ref())
                .is_some_and(|(view, selected)| *view == self.view && selected == item),
        }
    }

    /// Selectable label of an item: a click selects it, a double click opens its properties.
    fn label(&mut self, ui: &mut Ui, item: TreeItem, text: impl Into<WidgetText>) -> Response {
        let findings = self.findings.get(&item).cloned();
        let error = (findings.iter().flatten()).any(|f| f.severity() == Severity::Error);
        let red = error || self.holds_invalid.contains(&item);
        let inactive = self.inactive.contains(&item);
        let color = if red {
            Some(INVALID)
        } else {
            inactive.then_some(INACTIVE)
        };
        let mut response = row_label(ui, self.is_selected(&item), color, text);
        if let Some(findings) = findings {
            let mut hover: Vec<String> = (findings.iter())
                .map(|f| format!("{}: {}", f.problem.title(), f.detail))
                .collect();
            hover.push("Klick auf das Warnsymbol erklärt das Problem.".into());
            let hover = hover.join("\n");
            if warning_sign(ui).on_hover_text(&hover).clicked() {
                self.response.findings = Some(findings);
            }
            response = response.on_hover_text(hover);
        }
        // Like PrePoMax, a right click selects the item its context menu belongs to; on a
        // part of several selected ones it keeps them, so that the menu deletes them all.
        if response.clicked() || response.double_clicked() || response.secondary_clicked() {
            let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            match item {
                TreeItem::Part(_) if response.secondary_clicked() && self.is_selected(&item) => {}
                // Several parts are selected with Ctrl and Shift, as in a Windows tree.
                TreeItem::Part(index) if response.clicked() && shift => {
                    self.state.select_range(self.view, index, ctrl);
                }
                TreeItem::Part(index) if response.clicked() && ctrl => {
                    self.state.toggle_part(self.view, index);
                }
                TreeItem::Part(index) => self.state.select_part(self.view, index),
                _ => self.state.selected = Some((self.view, item.clone())),
            }
        }
        let lead =
            (self.state.selected.as_ref()).is_some_and(|(v, s)| *v == self.view && *s == item);
        if self.state.reveal && lead {
            response.scroll_to_me(None);
        }
        let closed = self.closed.get(&item).copied();
        if let Some(reason) = closed {
            response = response.on_hover_text(reason);
        }
        let creates = creates(&item).filter(|_| closed.is_none());
        if self.view == TreeView::Geometry && item == TreeItem::Group("Parts") {
            response.context_menu(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if ui.button("Alle Parts vernetzen").clicked() {
                    self.response.generate_mesh = true;
                }
            });
        }
        if response.double_clicked() && item == TreeItem::Analysis {
            // Unlike PrePoMax, which edits the job, a double click reopens the monitor: the
            // job settings are global settings here.
            self.response.analysis = Some(AnalysisAction::Monitor);
        } else if response.double_clicked() {
            match creates {
                // PrePoMax creates an item when its container is double-clicked.
                Some(kind) => self.response.create = Some(kind),
                None if has_properties(&item) => self.response.open = Some(item.clone()),
                // Other containers only open or close, see `branch`.
                None => {}
            }
        }
        let editable = is_fe_item(&item)
            || feature(&item).is_some()
            || matches!(
                item,
                TreeItem::ResultFieldOutput(_)
                    | TreeItem::HistorySet(_)
                    | TreeItem::HotSpot(_)
                    | TreeItem::MeshItem(_)
            );
        if item == TreeItem::Analysis {
            response.context_menu(|ui| self.analysis_menu(ui));
        }
        if creates.is_some() || editable {
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
                if item == TreeItem::Group("Mesh Setup") {
                    ui.separator();
                    if ui.button("Standard-Netzparameter …").clicked() {
                        self.response.mesh_defaults = true;
                    }
                    if ui.button("Alle Parts vernetzen").clicked() {
                        self.response.generate_mesh = true;
                    }
                }
                // Node ties live with the contact pairs; the search creates most of them.
                if item == TreeItem::Group("Contact Pairs")
                    && ui.button("Node Tie erstellen …").clicked()
                {
                    self.response.create = Some(NewItem::NodeTie);
                }
                // PrePoMax offers the search on constraints and contact pairs.
                if matches!(item, TreeItem::Group("Constraints" | "Contact Pairs")) {
                    ui.separator();
                    if ui.button("Kontaktpaare suchen …").clicked() {
                        self.response.search_contacts = true;
                    }
                }
                if item == TreeItem::Group(HOT_SPOTS) {
                    ui.separator();
                    if ui.button("Tabelle anzeigen").clicked() {
                        self.response.hot_spot_table = true;
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
                    if can_deactivate(&item) {
                        ui.separator();
                        let label = if inactive {
                            "Aktivieren"
                        } else {
                            "Deaktivieren"
                        };
                        if ui.button(label).clicked() {
                            self.response.toggle_active = Some(item.clone());
                        }
                        ui.separator();
                    }
                    if self.master_slave.contains(&item) {
                        if ui.button("Master und Slave tauschen").clicked() {
                            self.response.swap_master_slave = Some(item.clone());
                        }
                        ui.separator();
                    }
                    if deletable(self.view, &item) && ui.button("Löschen").clicked() {
                        self.response.delete = Some(item.clone());
                    }
                }
            });
        }
        response
    }

    /// PrePoMax's context menu of an analysis. Duplicating and deleting are shown but
    /// disabled, as there is one analysis only.
    fn analysis_menu(&mut self, ui: &mut Ui) {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let running = self.job.is_some_and(|j| j.status == JobStatus::Running);
        let results = self.job.is_some_and(|j| j.results) && !running;
        let single = "Es gibt nur eine Analyse.";
        let mut action = |ui: &mut Ui, enabled: bool, text: &str, action: AnalysisAction| {
            if ui.add_enabled(enabled, egui::Button::new(text)).clicked() {
                self.response.analysis = Some(action);
            }
        };
        action(ui, true, "Bearbeiten …", AnalysisAction::Edit);
        ui.add_enabled(false, egui::Button::new("Duplizieren"))
            .on_disabled_hover_text(single);
        ui.separator();
        action(ui, !running, "Starten", AnalysisAction::Run);
        action(ui, !running, "Modell prüfen", AnalysisAction::CheckModel);
        action(ui, self.job.is_some(), "Monitor", AnalysisAction::Monitor);
        action(ui, results, "Ergebnisse", AnalysisAction::Results);
        action(ui, running, "Abbrechen", AnalysisAction::Kill);
        ui.separator();
        // An analysis has no children, as in PrePoMax the entries are there all the same.
        ui.add_enabled(false, egui::Button::new("Alle aufklappen"));
        ui.add_enabled(false, egui::Button::new("Alle zuklappen"));
        ui.separator();
        ui.add_enabled(false, egui::Button::new("Löschen"))
            .on_disabled_hover_text(single);
    }

    /// PrePoMax's image of a node; containers without an own image get the dotted line.
    fn icon(&self, item: &TreeItem, open: bool) -> TreeIcon {
        if self.inactive.contains(item) {
            return TreeIcon::Inactive;
        }
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
            TreeItem::Analysis => match self.job.map(|j| j.status) {
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
            // As in PrePoMax, clicking the sign of a deactivated item activates it.
            let inactive = self.inactive.contains(&item);
            let sense = if inactive {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            };
            let (rect, icon_response) =
                ui.allocate_exact_size(Vec2::splat(tree_icons::SIZE), sense);
            if ui.is_rect_visible(rect) {
                tree_icons::paint(ui.painter(), rect.min, icon);
            }
            if inactive && icon_response.on_hover_text("Aktivieren").clicked() {
                self.response.toggle_active = Some(item.clone());
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
        let revealing = (self.state.reveal)
            .then_some(self.state.selected.as_ref())
            .flatten()
            .filter(|(view, _)| *view == self.view);
        if let Some((_, selected)) = revealing
            && contains(&item, selected)
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
                let view = tree.view;
                let selected = tree.state.selected_parts(view);
                response.context_menu(|ui| {
                    part_menu(ui, index, part.visible, view, &selected, &mut tree.response);
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

    /// PrePoMax's features: reference points and coordinate systems, and planes.
    fn features(&mut self, ui: &mut Ui, fe: &FeModel) {
        let open = !fe.reference_points.is_empty()
            || !fe.coordinate_systems.is_empty()
            || !fe.planes.is_empty();
        self.branch(
            ui,
            TreeItem::Group("Features"),
            "Features",
            open,
            |tree, ui| {
                let points = (fe.reference_points.iter().enumerate())
                    .map(|(i, r)| (TreeItem::ReferencePoint(i), r.name.as_str()))
                    .collect();
                tree.container(ui, REFERENCE_POINTS, points);
                let systems = (fe.coordinate_systems.iter().enumerate())
                    .map(|(i, c)| (TreeItem::CoordinateSystem(i), c.name.as_str()))
                    .collect();
                tree.container(ui, COORDINATE_SYSTEMS, systems);
                let planes = (fe.planes.iter().enumerate())
                    .map(|(i, p)| (TreeItem::Plane(i), p.name.as_str()))
                    .collect();
                tree.container(ui, PLANES, planes);
            },
        );
    }
}

/// `mesh_items` names the items of the geometry's mesh setup, for the Geometry tree;
/// `solver_findings` are the problems CalculiX reported in the last run.
pub fn show(
    ui: &mut Ui,
    view: TreeView,
    model: Option<&mut Model>,
    mesh_items: &[String],
    job: Option<JobState>,
    solver_findings: &[Finding],
    state: &mut TreeState,
) -> TreeResponse {
    let mut tree = Tree {
        view,
        state,
        response: TreeResponse::default(),
        job,
        levels: vec![Vec::new()],
        forced_open: None,
        findings: HashMap::new(),
        holds_invalid: HashSet::new(),
        closed: HashMap::new(),
        inactive: HashSet::new(),
        master_slave: HashSet::new(),
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
                    let items = (mesh_items.iter().enumerate())
                        .map(|(i, name)| (TreeItem::MeshItem(i), name.as_str()))
                        .collect();
                    tree.container(ui, "Mesh Setup", items);
                }
                TreeView::FeModel => fe_model(&mut tree, ui, model, solver_findings),
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
    // Like PrePoMax, the space bar switches the selected item on or off; here only while the
    // pointer is over the tree, so that typing elsewhere never does.
    if let Some((selected_view, item)) = &tree.state.selected
        && *selected_view == view
        && can_deactivate(item)
        && ui.rect_contains_pointer(ui.max_rect())
        && !ui.ctx().text_edit_focused()
        && ui.input(|i| i.key_pressed(egui::Key::Space))
    {
        tree.response.toggle_active = Some(item.clone());
    }
    // Like PrePoMax, the Delete key deletes the selected item, after the same question as the
    // context menu. The selection may come from the 3D view, so the pointer can be anywhere.
    if let Some((selected_view, item)) = &tree.state.selected
        && *selected_view == view
        && deletable(view, item)
        && !ui.ctx().text_edit_focused()
        && ui.input(|i| i.key_pressed(egui::Key::Delete))
    {
        tree.response.delete = Some(item.clone());
    }
    tree.response
}

fn fe_model(tree: &mut Tree, ui: &mut Ui, model: Option<&mut Model>, solver: &[Finding]) {
    // A results file has no FE model, as in PrePoMax; its mesh lives in the Results tree.
    let model = model.filter(|m| !m.is_results());
    if let Some(model) = &model {
        for finding in model.findings().into_iter().chain(solver.iter().cloned()) {
            let (item, containers) = tree_items(finding.item);
            if finding.severity() == Severity::Error {
                tree.holds_invalid.extend(containers);
            }
            tree.findings.entry(item).or_default().push(finding);
        }
    }
    let fe = model.as_ref().map(|m| m.fe.clone()).unwrap_or_default();
    let has_model = model.is_some();
    tree.branch(ui, TreeItem::Model, "Model", true, |tree, ui| {
        tree.mesh(ui, model);
        tree.features(ui, &fe);
        let materials: Vec<(TreeItem, &str)> = (fe.materials.iter().enumerate())
            .map(|(i, m)| (TreeItem::Material(i), m.name.as_str()))
            .collect();
        tree.container(ui, "Materials", materials);
        let sections: Vec<(TreeItem, &str)> = (fe.sections.iter().enumerate())
            .map(|(i, s)| (TreeItem::Section(i), s.name.as_str()))
            .collect();
        tree.container(ui, "Sections", sections);
        for (i, constraint) in fe.constraints.iter().enumerate() {
            if !constraint.active() {
                tree.inactive.insert(TreeItem::Constraint(i));
            }
            if constraint.master_slave().is_some() {
                tree.master_slave.insert(TreeItem::Constraint(i));
            }
        }
        for (i, pair) in fe.contact_pairs.iter().enumerate() {
            if !pair.active {
                tree.inactive.insert(TreeItem::ContactPair(i));
            }
            tree.master_slave.insert(TreeItem::ContactPair(i));
        }
        for (i, _) in (fe.node_ties.iter().enumerate()).filter(|(_, t)| !t.active) {
            tree.inactive.insert(TreeItem::NodeTie(i));
        }
        let constraints = (fe.constraints.iter().enumerate())
            .map(|(i, c)| (TreeItem::Constraint(i), c.name()))
            .collect();
        tree.container(ui, "Constraints", constraints);
        let contacts = TreeItem::Group("Contacts");
        let open = !fe.surface_interactions.is_empty()
            || !fe.contact_pairs.is_empty()
            || !fe.node_ties.is_empty();
        tree.branch(ui, contacts, "Contacts", open, |tree, ui| {
            let interactions = (fe.surface_interactions.iter().enumerate())
                .map(|(i, s)| (TreeItem::SurfaceInteraction(i), s.name.as_str()))
                .collect();
            tree.container(ui, "Surface Interactions", interactions);
            let pairs = (fe.contact_pairs.iter().enumerate())
                .map(|(i, c)| (TreeItem::ContactPair(i), c.name.as_str()))
                .chain(
                    (fe.node_ties.iter().enumerate())
                        .map(|(i, t)| (TreeItem::NodeTie(i), t.name.as_str())),
                )
                .collect();
            tree.container(ui, "Contact Pairs", pairs);
        });
        tree.leaf(ui, TreeItem::Group("Distributions"), "Distributions");
        let amplitudes = (fe.amplitudes.iter().enumerate())
            .map(|(i, a)| (TreeItem::Amplitude(i), a.name.as_str()))
            .collect();
        tree.container(ui, "Amplitudes", amplitudes);
        for (i, _) in (fe.initial_conditions.iter().enumerate()).filter(|(_, c)| !c.active) {
            tree.inactive.insert(TreeItem::InitialCondition(i));
        }
        let initial = (fe.initial_conditions.iter().enumerate())
            .map(|(i, c)| (TreeItem::InitialCondition(i), c.name.as_str()))
            .collect();
        tree.container(ui, "Initial Conditions", initial);
        let steps = TreeItem::Group("Steps");
        if fe.steps.is_empty() {
            tree.leaf(ui, steps, "Steps");
            return;
        }
        let text = counted("Steps", fe.steps.len());
        tree.branch(ui, steps, text, true, |tree, ui| {
            for (s, step) in fe.steps.iter().enumerate() {
                if !step.active {
                    tree.inactive.insert(TreeItem::Step(s));
                }
                for (i, _) in
                    (step.boundary_conditions.iter().enumerate()).filter(|(_, b)| !b.active)
                {
                    tree.inactive.insert(TreeItem::BoundaryCondition(s, i));
                }
                for (i, _) in (step.loads.iter().enumerate()).filter(|(_, l)| !l.active) {
                    tree.inactive.insert(TreeItem::Load(s, i));
                }
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
pub const REFERENCE_POINTS: &str = "Reference Points";
pub const COORDINATE_SYSTEMS: &str = "Coordinate Systems";
pub const PLANES: &str = "Planes";
/// Container of the result paths in the Results tree.
pub const PATHS: &str = "Paths";
/// Container of the results on planes in the Results tree.
pub const PLANE_RESULTS: &str = "Plane Results";

/// The feature a tree item stands for.
pub fn feature(item: &TreeItem) -> Option<FeatureItem> {
    let (kind, index) = match *item {
        TreeItem::ReferencePoint(i) => (FeatureKind::ReferencePoint, i),
        TreeItem::CoordinateSystem(i) => (FeatureKind::CoordinateSystem, i),
        TreeItem::Plane(i) => (FeatureKind::Plane, i),
        TreeItem::ResultPath(i) => (FeatureKind::ResultPath, i),
        TreeItem::ResultPlane(i) => (FeatureKind::ResultPlane, i),
        _ => return None,
    };
    Some(FeatureItem { kind, index })
}

/// Name of the analysis job, PrePoMax's first default.
pub const ANALYSIS_NAME: &str = "Analysis-1";

/// A name with the names of its children, e.g. a field with its components.
type NamedList = (String, Vec<String>);

fn results(tree: &mut Tree, ui: &mut Ui, model: Option<&mut Model>) {
    let model = model.filter(|m| m.results.is_some());
    let mut fields = Vec::new();
    let mut active = None;
    let mut shown_plane = None;
    let mut history: Vec<(String, Vec<NamedList>)> = Vec::new();
    let hot_spots: Vec<String> = (model.iter())
        .flat_map(|m| &m.hot_spots.definitions)
        .map(|h| h.name.clone())
        .collect();
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
        shown_plane = view.plane_result;
    }
    let fe = model.as_ref().map(|m| m.fe.clone()).unwrap_or_default();
    tree.branch(ui, TreeItem::Model, "Model", true, |tree, ui| {
        tree.mesh(ui, model);
        tree.features(ui, &fe);
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
            } else {
                let text = counted("History Outputs", history.len());
                tree.branch(ui, group, text, true, |tree, ui| {
                    for (s, (name, fields)) in history.into_iter().enumerate() {
                        tree.branch(ui, TreeItem::HistorySet(s), name, true, |tree, ui| {
                            for (f, (name, components)) in fields.into_iter().enumerate() {
                                let item = TreeItem::HistoryField(s, f);
                                tree.branch(ui, item, name, true, |tree, ui| {
                                    for (c, component) in components.into_iter().enumerate() {
                                        let item = TreeItem::HistoryComponent(s, f, c);
                                        tree.leaf(ui, item, component);
                                    }
                                });
                            }
                        });
                    }
                });
            }
            // Not in PrePoMax: hot spot stresses, defined on the results.
            let hot_spots = hot_spots.iter().enumerate();
            let items = hot_spots.map(|(i, name)| (TreeItem::HotSpot(i), name.as_str()));
            tree.container(ui, HOT_SPOTS, items.collect());
            // Not in PrePoMax: results along straight lines through the model.
            let paths = (fe.result_paths.iter().enumerate())
                .map(|(i, p)| (TreeItem::ResultPath(i), p.name.as_str()))
                .collect();
            tree.container(ui, PATHS, paths);
            // Not in PrePoMax either: the values where a plane cuts the model, shown alone
            // while checked.
            let group = TreeItem::Group(PLANE_RESULTS);
            if fe.result_planes.is_empty() {
                tree.leaf(ui, group, PLANE_RESULTS);
            } else {
                let text = counted(PLANE_RESULTS, fe.result_planes.len());
                tree.branch(ui, group, text, true, |tree, ui| {
                    for (i, result) in fe.result_planes.iter().enumerate() {
                        let item = TreeItem::ResultPlane(i);
                        let icon = tree.icon(&item, false);
                        let mut shown = shown_plane == Some(i);
                        let (_, response, changed) =
                            tree.row(ui, None, Some(&mut shown), icon, item, &result.name);
                        let _ = response.on_hover_text(
                            "Angehakt zeigt das 3D-Fenster nur die Schnittfläche mit der \
                             Legende der Werte darauf.",
                        );
                        if changed {
                            tree.response.plane_result = Some(shown.then_some(i));
                        }
                    }
                });
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(state: &TreeState, view: TreeView) -> Vec<usize> {
        state.selected_parts(view).into_iter().collect()
    }

    #[test]
    fn ctrl_and_shift_select_several_parts() {
        let view = TreeView::Geometry;
        let mut state = TreeState::default();
        state.select_part(view, 1);
        state.toggle_part(view, 4);
        assert_eq!(parts(&state, view), [1, 4]);
        assert_eq!(state.selected, Some((view, TreeItem::Part(4))));
        // Shift selects from the part last clicked, Ctrl+Shift adds the range.
        state.select_range(view, 2, false);
        assert_eq!(parts(&state, view), [2, 3, 4]);
        state.select_part(view, 0);
        state.select_range(view, 1, true);
        assert_eq!(parts(&state, view), [0, 1]);
        state.toggle_part(view, 5);
        state.select_range(view, 7, true);
        assert_eq!(parts(&state, view), [0, 1, 5, 6, 7]);
        // Ctrl takes a part out again; the last one leaves nothing selected.
        state.toggle_part(view, 7);
        assert_eq!(parts(&state, view), [0, 1, 5, 6]);
        assert!(matches!(state.selected, Some((_, TreeItem::Part(_)))));
        state.select_part(view, 3);
        state.toggle_part(view, 3);
        assert_eq!(state.selected, None);
        assert!(parts(&state, view).is_empty());
    }

    #[test]
    fn other_selections_drop_the_parts() {
        let mut state = TreeState::default();
        state.select_part(TreeView::FeModel, 0);
        state.toggle_part(TreeView::FeModel, 2);
        assert!(parts(&state, TreeView::Geometry).is_empty());
        state.selected = Some((TreeView::FeModel, TreeItem::Material(0)));
        assert!(parts(&state, TreeView::FeModel).is_empty());
        // A part selected anew comes alone.
        state.toggle_part(TreeView::FeModel, 1);
        assert_eq!(parts(&state, TreeView::FeModel), [1]);
        // Shift without a part selected starts at the clicked one.
        state.selected = None;
        state.select_range(TreeView::FeModel, 3, false);
        assert_eq!(parts(&state, TreeView::FeModel), [3]);
    }

    #[test]
    fn revealed_items_open_their_branches() {
        let pair = TreeItem::ContactPair(0);
        for branch in [
            TreeItem::Model,
            TreeItem::Group("Contacts"),
            TreeItem::Group("Contact Pairs"),
        ] {
            assert!(contains(&branch, &pair), "{branch:?}");
        }
        assert!(!contains(&TreeItem::Group("Constraints"), &pair));
        assert!(contains(
            &TreeItem::Group("Constraints"),
            &TreeItem::Constraint(1)
        ));
        assert!(contains(&TreeItem::Group("Parts"), &TreeItem::Part(2)));
    }

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
