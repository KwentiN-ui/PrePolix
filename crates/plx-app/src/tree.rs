//! The model tree in PrePoMax's three views Geometry, FE Model and Results, with PrePoMax's
//! node names. Nodes for features prepolix does not support yet are shown as empty
//! placeholders, so that the structure is already the familiar one.

use egui::collapsing_header::CollapsingState;
use egui::{Response, Ui, WidgetText};

use crate::model::Model;

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
        let response = ui.selectable_label(self.is_selected(&item), text);
        if response.clicked() || response.double_clicked() {
            self.state.selected = Some((self.view, item.clone()));
        }
        if response.double_clicked() {
            self.response.open = Some(item);
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
    let model = model.filter(|m| m.results.is_none());
    tree.branch(ui, TreeItem::Model, "Model", true, |tree, ui| {
        tree.mesh(ui, model);
        tree.features(ui);
        for name in ["Materials", "Sections", "Constraints"] {
            tree.leaf(ui, TreeItem::Group(name), name);
        }
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
        for name in ["Distributions", "Amplitudes", "Initial Conditions", "Steps"] {
            tree.leaf(ui, TreeItem::Group(name), name);
        }
    });
    tree.leaf(ui, TreeItem::Group("Analyses"), "Analyses");
}

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
