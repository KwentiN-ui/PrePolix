//! The model tree in PrePoMax's three views Geometry, FE Model and Results, with PrePoMax's
//! node names. Nodes for features prepolix does not support yet are shown as empty
//! placeholders, so that the structure is already the familiar one.

use egui::collapsing_header::CollapsingState;
use egui::{Response, Ui, WidgetText};

use crate::model::Model;
use crate::setup::NewItem;

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
    FieldOutputs,
    /// Field of the current increment, by index.
    Field(usize),
    Component(usize, usize),
}

/// Selection shared by the three trees; an item is selected in one view only.
#[derive(Default)]
pub struct TreeState {
    pub selected: Option<(TreeView, TreeItem)>,
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
}

/// Tree label with a fixed size: highlight and hover frame are painted over the same area, so
/// that hovering never moves the rows below (egui's selectable label grows by its frame).
fn row_label(ui: &mut Ui, selected: bool, text: impl Into<WidgetText>) -> Response {
    let padding = egui::vec2(3.0, 1.0);
    let galley = text.into().into_galley(
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

/// What double-clicking a container creates.
fn creates(item: &TreeItem) -> Option<NewItem> {
    match *item {
        TreeItem::Group("Materials") => Some(NewItem::Material),
        TreeItem::Group("Sections") => Some(NewItem::Section),
        TreeItem::Group("Steps") => Some(NewItem::Step),
        TreeItem::StepGroup(step, "BCs") => Some(NewItem::BoundaryCondition(step)),
        TreeItem::StepGroup(step, "Loads") => Some(NewItem::Load(step)),
        _ => None,
    }
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

struct Tree<'a> {
    view: TreeView,
    state: &'a mut TreeState,
    response: TreeResponse,
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
        let response = row_label(ui, self.is_selected(&item), text);
        if response.clicked() || response.double_clicked() {
            self.state.selected = Some((self.view, item.clone()));
        }
        let creates = creates(&item);
        if response.double_clicked() {
            match creates {
                // PrePoMax creates an item when its container is double-clicked.
                Some(kind) => self.response.create = Some(kind),
                None => self.response.open = Some(item.clone()),
            }
        }
        let editable = is_fe_item(&item);
        if creates.is_some() || editable || item == TreeItem::Analysis {
            response.context_menu(|ui| {
                if let Some(kind) = creates
                    && ui.button("Erstellen …").clicked()
                {
                    self.response.create = Some(kind);
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

    /// Node without children, aligned with the labels of sibling branches.
    fn leaf(&mut self, ui: &mut Ui, item: TreeItem, text: impl Into<WidgetText>) -> Response {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
            self.label(ui, item, text)
        })
        .inner
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
        CollapsingState::load_with_default_open(ui.ctx(), id, default_open)
            .show_header(ui, |ui| self.label(ui, item, text))
            .body(|ui| body(self, ui));
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
            let parts = counted("Parts", model.parts.len());
            tree.branch(ui, TreeItem::Group("Parts"), parts, true, |tree, ui| {
                for (index, part) in model.parts.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
                        if ui.checkbox(&mut part.visible, "").changed() {
                            tree.response.visibility.push((index, part.visible));
                        }
                        let (swatch, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        let [r, g, b] = part.color.map(|c| (c * 255.0).round() as u8);
                        ui.painter()
                            .rect_filled(swatch, 0.0, egui::Color32::from_rgb(r, g, b));
                        ui.painter().rect_stroke(
                            swatch,
                            0.0,
                            egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                            egui::StrokeKind::Inside,
                        );
                        tree.label(ui, TreeItem::Part(index), &part.name);
                    });
                }
            });
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
    state: &mut TreeState,
) -> TreeResponse {
    let mut tree = Tree {
        view,
        state,
        response: TreeResponse::default(),
    };
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| match view {
            TreeView::Geometry => {
                tree.leaf(ui, TreeItem::Group("Parts"), "Parts");
                tree.leaf(ui, TreeItem::Group("Mesh Setup"), "Mesh Setup");
            }
            TreeView::FeModel => fe_model(&mut tree, ui, model),
            TreeView::Results => results(&mut tree, ui, model),
        });
    tree.response
}

fn fe_model(tree: &mut Tree, ui: &mut Ui, model: Option<&mut Model>) {
    // A results file has no FE model, as in PrePoMax; its mesh lives in the Results tree.
    let model = model.filter(|m| !m.is_results());
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
        let id = ui.make_persistent_id((tree.view, "Contacts"));
        CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                tree.label(ui, TreeItem::Group("Contacts"), "Contacts")
            })
            .body(|ui| {
                for name in ["Surface Interactions", "Contact Pairs"] {
                    tree.leaf(ui, TreeItem::Group(name), name);
                }
            });
        for name in ["Distributions", "Amplitudes", "Initial Conditions"] {
            tree.leaf(ui, TreeItem::Group(name), name);
        }
        let steps = TreeItem::Group("Steps");
        if fe.steps.is_empty() {
            tree.leaf(ui, steps, "Steps");
            return;
        }
        let text = counted("Steps", fe.steps.len());
        tree.branch(ui, steps, text, true, |tree, ui| {
            for (s, step) in fe.steps.iter().enumerate() {
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

/// Name of the analysis job, PrePoMax's first default.
pub const ANALYSIS_NAME: &str = "Analysis-1";

fn results(tree: &mut Tree, ui: &mut Ui, model: Option<&mut Model>) {
    let model = model.filter(|m| m.results.is_some());
    let mut fields = Vec::new();
    let mut active = None;
    if let Some(view) = model.as_ref().and_then(|m| m.results.as_ref()) {
        if let Some(increment) = view.current_increment() {
            fields = increment
                .fields
                .iter()
                .map(|f| {
                    let components: Vec<String> =
                        f.components.iter().map(|c| c.name.clone()).collect();
                    (f.name.clone(), components)
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
                    for (f, (name, components)) in fields.into_iter().enumerate() {
                        // PrePoMax opens the first two fields.
                        tree.branch(ui, TreeItem::Field(f), name, f < 2, |tree, ui| {
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
            tree.leaf(ui, TreeItem::Group("History Outputs"), "History Outputs");
        },
    );
}
