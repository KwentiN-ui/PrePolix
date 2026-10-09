//! Creating and editing the FE model: PrePoMax's item dialogs for materials, sections, steps,
//! boundary conditions, loads and field outputs, and prepolix's hot spot definitions.
//!
//! Regions are picked in the 3D view while a dialog is open. As in PrePoMax the user never
//! defines node or element sets for this; the input file writer derives them.

use std::collections::BTreeSet;

use egui::Ui;
use plx_mesh::{ElementId, FeMesh, NodeId};
use plx_model::{
    BoundaryCondition, BoundaryKind, Constraint, ContactPair, Elastic, EquationSolver,
    Extrapolation, FeModel, FieldOutput, FrequencyStep, HotSpot, HotSpotComponent, Incrementation,
    Load, LoadKind, Material, ModelSpace, OutputKind, Region, Section, StaticStep, Step, StepKind,
    SurfaceInteraction, extrapolation_weights, next_name,
};

use crate::constraint_dialog::ConstraintDraft;
use crate::contacts::{self, MasterSlave};
use crate::model::{Highlight, Hit, Model};
use crate::numeric;
use crate::selection::{History, Items, Operation, Picker, PickerAction, Target};
use crate::tree::TreeItem;
use crate::viewport::{BoxSelect, Preview};

/// Kinds of items the tree can create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewItem {
    Material,
    Section,
    Step,
    BoundaryCondition(usize),
    Load(usize),
    HotSpot,
    /// A spring, support or tie, chosen in the dialog.
    Constraint,
    SurfaceInteraction,
    ContactPair,
    /// A field output derived from results, created in the Results tree.
    ResultFieldOutput,
    /// A history output derived from results, created in the Results tree.
    ResultHistoryOutput,
    /// An item of the geometry's mesh setup, created in the Geometry tree.
    MeshSetupItem,
}

/// How a region is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// Picked in the 3D view.
    Selection,
    Parts,
    NodeSet,
    ElementSet,
    Surface,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Selection => "Auswahl im 3D-Fenster",
            Source::Parts => "Parts",
            Source::NodeSet => "Node Set",
            Source::ElementSet => "Element Set",
            Source::Surface => "Surface",
        }
    }
}

/// A region while it is edited in a dialog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegionDraft {
    sources: &'static [Source],
    pub(crate) source: Source,
    /// Whether picks select nodes or element faces.
    pub(crate) target: Target,
    nodes: History<NodeId>,
    faces: History<(ElementId, u8)>,
    parts: BTreeSet<String>,
    set: String,
}

pub(crate) const NODE_SOURCES: &[Source] = &[Source::Selection, Source::NodeSet, Source::Surface];
pub(crate) const FACE_SOURCES: &[Source] = &[Source::Selection, Source::Surface];
/// Solid elements: whole parts, element sets or the elements of picked faces.
pub(crate) const SOLID_SOURCES: &[Source] = &[Source::Parts, Source::ElementSet, Source::Selection];
pub(crate) const ELEMENT_SOURCES: &[Source] = &[Source::Parts, Source::ElementSet];

impl RegionDraft {
    pub(crate) fn new(sources: &'static [Source], target: Target) -> Self {
        Self {
            sources,
            source: sources[0],
            target,
            nodes: History::default(),
            faces: History::default(),
            parts: BTreeSet::new(),
            set: String::new(),
        }
    }

    pub(crate) fn from_region(
        region: &Region,
        sources: &'static [Source],
        target: Target,
        mesh: &FeMesh,
    ) -> Self {
        let mut draft = Self::new(sources, target);
        match region {
            Region::Parts(parts) => {
                draft.source = Source::Parts;
                draft.parts = parts.iter().cloned().collect();
            }
            Region::Nodes(nodes) => draft.nodes = History::from_items(nodes.iter().copied()),
            Region::Faces(faces) if target != Target::Nodes => {
                draft.faces = History::from_items(faces.iter().copied());
            }
            Region::Faces(_) => draft.nodes = History::from_items(region.nodes(mesh)),
            Region::NodeSet(set) | Region::ElementSet(set) | Region::Surface(set) => {
                draft.source = match region {
                    Region::NodeSet(_) => Source::NodeSet,
                    Region::ElementSet(_) => Source::ElementSet,
                    _ => Source::Surface,
                };
                draft.set = set.clone();
            }
        }
        draft
    }

    pub(crate) fn region(&self) -> Region {
        match self.source {
            Source::Selection => match self.target {
                Target::Nodes => Region::Nodes(self.nodes.items().into_iter().collect()),
                Target::Faces | Target::Edges => {
                    Region::Faces(self.faces.items().into_iter().collect())
                }
            },
            Source::Parts => Region::Parts(self.parts.iter().cloned().collect()),
            Source::NodeSet => Region::NodeSet(self.set.clone()),
            Source::ElementSet => Region::ElementSet(self.set.clone()),
            Source::Surface => Region::Surface(self.set.clone()),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self.source {
            Source::Selection => match self.target {
                Target::Nodes => self.nodes.items().is_empty(),
                Target::Faces | Target::Edges => self.faces.items().is_empty(),
            },
            Source::Parts => self.parts.is_empty(),
            _ => self.set.is_empty(),
        }
    }

    pub(crate) fn count(&self) -> usize {
        match self.target {
            Target::Nodes => self.nodes.items().len(),
            Target::Faces | Target::Edges => self.faces.items().len(),
        }
    }

    pub(crate) fn click(
        &mut self,
        model: &Model,
        picker: &Picker,
        pick: Option<(&Hit, f32)>,
        operation: Operation,
    ) {
        match self.source {
            Source::Parts => {
                if let Some((hit, _)) = pick {
                    let name = model.parts[hit.part].name.clone();
                    if operation == Operation::Subtract {
                        self.parts.remove(&name);
                    } else {
                        self.parts.insert(name);
                    }
                }
            }
            Source::Selection => match pick {
                Some((hit, precision)) => {
                    self.take(picker.pick(model, hit, self.target, precision), operation);
                }
                // PrePoMax clears the selection on a plain click into empty space.
                None if operation == Operation::Replace => self.clear(),
                None => {}
            },
            _ => {}
        }
    }

    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
        self.faces.clear();
    }

    pub(crate) fn take(&mut self, items: Items, operation: Operation) {
        match items {
            Items::Nodes(nodes) => self.nodes.push(operation, nodes),
            Items::Faces(faces) => self.faces.push(operation, faces),
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        match self.target {
            Target::Nodes => self.nodes.can_undo(),
            Target::Faces | Target::Edges => self.faces.can_undo(),
        }
    }

    /// Applies a button of the selection window.
    pub(crate) fn action(&mut self, model: &Model, action: PickerAction) {
        match (action, self.target) {
            (PickerAction::Undo, Target::Nodes) => self.nodes.undo(),
            (PickerAction::Undo, Target::Faces | Target::Edges) => self.faces.undo(),
            (PickerAction::Clear, _) => self.clear(),
            (PickerAction::All, Target::Nodes) => {
                self.nodes.push(Operation::Replace, model.visible_nodes());
            }
            (PickerAction::All, Target::Faces) => {
                self.faces.push(Operation::Replace, model.visible_faces());
            }
            (PickerAction::Invert, Target::Nodes) => {
                let selected = self.nodes.items();
                let mut all = model.visible_nodes();
                all.retain(|n| !selected.contains(n));
                self.nodes.push(Operation::Replace, all);
            }
            (PickerAction::All, Target::Edges) => {
                self.faces.push(Operation::Replace, visible_edges(model));
            }
            (PickerAction::Invert, Target::Edges) => {
                let selected = self.faces.items();
                let mut all = visible_edges(model);
                all.retain(|f| !selected.contains(f));
                self.faces.push(Operation::Replace, all);
            }
            (PickerAction::Ids(operation, ids), Target::Edges) => {
                // Ids of elements: their edges on the outline.
                let elements: BTreeSet<ElementId> = ids.into_iter().collect();
                let mut edges = visible_edges(model);
                edges.retain(|(element, _)| elements.contains(element));
                self.faces.push(operation, edges);
            }
            (PickerAction::Invert, Target::Faces) => {
                let selected = self.faces.items();
                let mut all = model.visible_faces();
                all.retain(|f| !selected.contains(f));
                self.faces.push(Operation::Replace, all);
            }
            (PickerAction::Ids(operation, ids), Target::Nodes) => {
                let ids = ids
                    .into_iter()
                    .filter(|&id| model.mesh.node_index(id).is_some())
                    .collect();
                self.nodes.push(operation, ids);
            }
            (PickerAction::Ids(operation, ids), Target::Faces) => {
                // Ids of elements: their faces on the surface.
                let elements: BTreeSet<ElementId> = ids.into_iter().collect();
                let faces = model
                    .visible_faces()
                    .into_iter()
                    .filter(|(element, _)| elements.contains(element))
                    .collect();
                self.faces.push(operation, faces);
            }
        }
    }

    pub(crate) fn ui(&mut self, ui: &mut Ui, model: &Model) {
        self.ui_labeled(ui, model, "Region", "region");
    }

    /// The region's rows under its own label; `id` keeps its widgets apart from those of
    /// another region in the same dialog.
    pub(crate) fn ui_labeled(&mut self, ui: &mut Ui, model: &Model, label: &str, id: &str) {
        ui.label(label);
        ui.push_id(id, |ui| self.ui_body(ui, model));
        ui.end_row();
    }

    fn ui_body(&mut self, ui: &mut Ui, model: &Model) {
        ui.vertical(|ui| {
            egui::ComboBox::from_id_salt("region source")
                .selected_text(self.source.label())
                .width(200.0)
                .show_ui(ui, |ui| {
                    for &source in self.sources {
                        ui.selectable_value(&mut self.source, source, source.label());
                    }
                });
            match self.source {
                Source::Selection => {
                    ui.horizontal(|ui| {
                        let count = self.count();
                        let what = match self.target {
                            Target::Nodes => "Knoten",
                            Target::Faces => "Elementflächen",
                            Target::Edges => "Elementkanten",
                        };
                        if count == 0 {
                            ui.label("Leer");
                        } else {
                            ui.label(format!("{count} {what}"));
                        }
                        if ui.button("Auswahl löschen").clicked() {
                            self.clear();
                        }
                    });
                    ui.weak("Im Fenster \"Auswahl\" wählen, was ein Klick auswählt.");
                }
                Source::Parts => {
                    for part in &model.parts {
                        let mut checked = self.parts.contains(&part.name);
                        if ui.checkbox(&mut checked, &part.name).changed() {
                            if checked {
                                self.parts.insert(part.name.clone());
                            } else {
                                self.parts.remove(&part.name);
                            }
                        }
                    }
                    ui.weak("Ein Klick auf ein Part im 3D-Fenster wählt es ebenfalls.");
                }
                Source::NodeSet | Source::ElementSet | Source::Surface => {
                    let names: Vec<&String> = match self.source {
                        Source::NodeSet => model.mesh.node_sets.keys().collect(),
                        Source::ElementSet => model.mesh.element_sets.keys().collect(),
                        _ => model.mesh.surfaces.keys().collect(),
                    };
                    if names.is_empty() {
                        ui.weak("Das Netz enthält keine solchen Sets.");
                    }
                    egui::ComboBox::from_id_salt("region set")
                        .selected_text(self.set.as_str())
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for name in names {
                                ui.selectable_value(&mut self.set, name.clone(), name);
                            }
                        });
                }
            }
        });
    }

    pub(crate) fn highlight(&self, model: &Model) -> Highlight {
        region_highlight(model, &self.region())
    }

    /// A selection box dragged in the 3D view.
    pub(crate) fn box_select(
        &mut self,
        model: &Model,
        picker: &Picker,
        area: &BoxSelect,
        operation: Operation,
    ) {
        if self.source == Source::Selection {
            self.take(picker.pick_box(model, area, self.target), operation);
        }
    }

    /// What a click at the hit would select, for the hover preview.
    pub(crate) fn preview(
        &self,
        model: &Model,
        picker: &Picker,
        hit: &Hit,
        precision: f32,
    ) -> Preview {
        if self.source != Source::Selection {
            return Preview::default();
        }
        crate::selection::preview(model, &picker.pick(model, hit, self.target, precision))
    }
}

/// Edges of the outline of the visible parts, the faces of a 2D model.
fn visible_edges(model: &Model) -> BTreeSet<(ElementId, u8)> {
    (model.outline_edges().into_iter())
        .filter(|(part, _, _)| model.parts[*part].visible)
        .map(|(_, face, _)| face)
        .collect()
}

/// Pressure and surface traction act on element faces, in 2D models on their edges.
fn face_target(fe: &FeModel) -> Target {
    if fe.properties.space.is_2d() {
        Target::Edges
    } else {
        Target::Faces
    }
}

/// The components of a force; 2D models have none along z.
fn force_rows(ui: &mut Ui, force: &mut [f64; 3], two_d: bool) {
    let count = if two_d { 2 } else { 3 };
    for (value, label) in force.iter_mut().zip(["F1", "F2", "F3"]).take(count) {
        ui.label(label);
        ui.add(numeric::drag_value(value).speed(1.0));
        ui.end_row();
    }
}

/// Forces of axisymmetric models act on the whole revolution, as in CalculiX.
fn revolution_hint(ui: &mut Ui, axisymmetric: bool) {
    if axisymmetric {
        ui.label("");
        ui.weak("Rotationssymmetrisch: Kraft auf den ganzen Umfang (360°).");
        ui.end_row();
    }
}

/// How a region is shown selected in the 3D view.
pub fn region_highlight(model: &Model, region: &Region) -> Highlight {
    let mut highlight = Highlight::default();
    match region {
        Region::Parts(names) => {
            highlight.parts = (model.parts.iter().enumerate())
                .filter(|(_, p)| names.contains(&p.name))
                .map(|(i, _)| i)
                .collect();
        }
        Region::Faces(_) | Region::Surface(_) if model.is_plane() => {
            // The faces of 2D elements are edges, drawn as lines.
            let faces = region.faces(&model.mesh).into_iter().collect();
            highlight.lines = crate::selection::edge_lines(model, &faces);
            if highlight.lines.is_empty() {
                highlight.nodes = region.nodes(&model.mesh);
            }
        }
        Region::Faces(_) | Region::Surface(_) => {
            highlight.faces = region.faces(&model.mesh).into_iter().collect();
            if highlight.faces.is_empty() {
                highlight.nodes = region.nodes(&model.mesh);
            }
        }
        Region::ElementSet(_) => {
            let elements: BTreeSet<ElementId> = region.elements(&model.mesh).into_iter().collect();
            highlight.faces = model
                .skin_faces()
                .filter(|(element, _)| elements.contains(element))
                .collect();
        }
        Region::Nodes(_) | Region::NodeSet(_) => {
            highlight.nodes = region.nodes(&model.mesh);
            // Faces whose corners are all selected show as faces, like PrePoMax does for
            // picked surfaces.
            let selected: std::collections::HashSet<NodeId> =
                highlight.nodes.iter().copied().collect();
            highlight.faces = model
                .skin_faces_with_corners()
                .filter(|(_, corners)| corners.iter().all(|n| selected.contains(n)))
                .map(|(face, _)| face)
                .collect();
        }
    }
    highlight
}

/// The item a dialog edits, as a draft that OK copies into the model.
enum Draft {
    Material(Material),
    Section(Section, RegionDraft),
    Step(Step),
    BoundaryCondition(usize, BoundaryCondition, RegionDraft),
    Load(usize, Load, RegionDraft),
    FieldOutput(usize, FieldOutput),
    HotSpot(HotSpot, RegionDraft, HotSpotText),
    Constraint(ConstraintDraft),
    /// The interaction with the index of the model whose properties are shown.
    SurfaceInteraction(SurfaceInteraction, usize),
    ContactPair(ContactPair, MasterSlave),
}

/// The region clicks in the 3D view pick for, if the dialog has one.
fn draft_region(draft: &Draft) -> Option<&RegionDraft> {
    match draft {
        Draft::Section(_, r)
        | Draft::BoundaryCondition(_, _, r)
        | Draft::Load(_, _, r)
        | Draft::HotSpot(_, r, _) => Some(r),
        Draft::ContactPair(_, regions) => Some(regions.current()),
        Draft::Constraint(c) => Some(c.region()),
        _ => None,
    }
}

fn draft_region_mut(draft: &mut Draft) -> Option<&mut RegionDraft> {
    match draft {
        Draft::Section(_, r)
        | Draft::BoundaryCondition(_, _, r)
        | Draft::Load(_, _, r)
        | Draft::HotSpot(_, r, _) => Some(r),
        Draft::ContactPair(_, regions) => Some(regions.current_mut()),
        Draft::Constraint(c) => Some(c.region_mut()),
        _ => None,
    }
}

/// Text fields of the hot spot dialog that are parsed on every change.
struct HotSpotText {
    /// Own read-out distances, e.g. "2, 6".
    distances: String,
}

/// An open item dialog.
pub struct Editor {
    draft: Draft,
    /// Index of the edited item; `None` creates a new one.
    index: Option<usize>,
    error: Option<String>,
    /// The selection window shown while the region is picked in the 3D view.
    picker: Picker,
}

pub enum EditorResult {
    Open,
    Ok,
    Cancel,
}

fn names<'a, T: 'a>(items: &'a [T], name: impl Fn(&T) -> &str + 'a) -> Vec<&'a str> {
    items.iter().map(name).collect()
}

const FIXED: &str = "Fixed";
const DISPLACEMENT: &str = "Displacement_Rotation";
const FORCE: &str = "Concentrated_Force";
const PRESSURE: &str = "Pressure";
const TRACTION: &str = "Surface_Traction";

/// The load kinds of the dialog: label, default name and the kind with zero values.
fn load_kinds() -> [(&'static str, &'static str, LoadKind); 3] {
    [
        ("Einzelkraft", FORCE, LoadKind::ConcentratedForce([0.0; 3])),
        ("Druck", PRESSURE, LoadKind::Pressure(0.0)),
        ("Flächenlast", TRACTION, LoadKind::SurfaceTraction([0.0; 3])),
    ]
}

fn load_kind_name(kind: &LoadKind) -> &'static str {
    match kind {
        LoadKind::ConcentratedForce(_) => FORCE,
        LoadKind::Pressure(_) => PRESSURE,
        LoadKind::SurfaceTraction(_) => TRACTION,
    }
}

impl Editor {
    pub fn create(kind: NewItem, fe: &FeModel) -> Option<Self> {
        let draft = match kind {
            NewItem::Material => Draft::Material(Material {
                name: next_name("Material", names(&fe.materials, |m| &m.name)),
                density: None,
                elastic: Some(Elastic {
                    young: 0.0,
                    poisson: 0.0,
                }),
            }),
            NewItem::Section => Draft::Section(
                Section {
                    name: next_name("Solid_Section", names(&fe.sections, |s| &s.name)),
                    material: fe
                        .materials
                        .first()
                        .map(|m| m.name.clone())
                        .unwrap_or_default(),
                    region: Region::Parts(Vec::new()),
                    thickness: 1.0,
                },
                RegionDraft::new(ELEMENT_SOURCES, Target::Faces),
            ),
            NewItem::Step => {
                let mut step = Step::new_static(next_name("Step", names(&fe.steps, |s| &s.name)));
                step.kind = StepKind::Static(previous_static(fe));
                Draft::Step(step)
            }
            NewItem::BoundaryCondition(step) => {
                let existing = names(&fe.steps.get(step)?.boundary_conditions, |b| &b.name);
                Draft::BoundaryCondition(
                    step,
                    BoundaryCondition {
                        name: next_name(FIXED, existing),
                        active: true,
                        region: Region::Nodes(Vec::new()),
                        kind: BoundaryKind::Fixed,
                    },
                    RegionDraft::new(NODE_SOURCES, Target::Nodes),
                )
            }
            NewItem::Load(step) => {
                // A frequency step takes no loads, as in PrePoMax.
                let target = fe.steps.get(step).filter(|s| s.kind.supports_loads())?;
                let existing = names(&target.loads, |l| &l.name);
                let region = RegionDraft::new(NODE_SOURCES, Target::Nodes);
                Draft::Load(
                    step,
                    Load {
                        name: next_name(FORCE, existing),
                        active: true,
                        region: Region::Nodes(Vec::new()),
                        kind: LoadKind::ConcentratedForce([0.0; 3]),
                    },
                    region,
                )
            }
            NewItem::HotSpot => {
                let name = next_name("Hot_Spot", names(&fe.hot_spots, |h| &h.name));
                Draft::HotSpot(
                    HotSpot::new(name),
                    RegionDraft::new(NODE_SOURCES, Target::Nodes),
                    HotSpotText {
                        distances: String::new(),
                    },
                )
            }
            NewItem::Constraint => Draft::Constraint(ConstraintDraft::new(fe)),
            NewItem::SurfaceInteraction => {
                let existing = names(&fe.surface_interactions, |s| &s.name);
                Draft::SurfaceInteraction(
                    SurfaceInteraction {
                        name: next_name("Surface_Interaction", existing),
                        properties: Vec::new(),
                    },
                    0,
                )
            }
            NewItem::ContactPair => {
                let existing = names(&fe.contact_pairs, |c| &c.name);
                let interaction = (fe.surface_interactions.first())
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                Draft::ContactPair(
                    ContactPair::new(next_name("Contact_Pair", existing), interaction),
                    MasterSlave::new(),
                )
            }
            NewItem::ResultFieldOutput | NewItem::ResultHistoryOutput | NewItem::MeshSetupItem => {
                return None;
            }
        };
        Some(Self {
            draft,
            index: None,
            error: None,
            picker: Picker::default(),
        })
    }

    pub fn edit(item: &TreeItem, fe: &FeModel, mesh: &FeMesh) -> Option<Self> {
        let (draft, index) = match *item {
            TreeItem::Material(i) => (Draft::Material(fe.materials.get(i)?.clone()), i),
            TreeItem::Section(i) => {
                let section = fe.sections.get(i)?.clone();
                let region =
                    RegionDraft::from_region(&section.region, ELEMENT_SOURCES, Target::Faces, mesh);
                (Draft::Section(section, region), i)
            }
            TreeItem::Step(i) => (Draft::Step(fe.steps.get(i)?.clone()), i),
            TreeItem::BoundaryCondition(s, i) => {
                let bc = fe.steps.get(s)?.boundary_conditions.get(i)?.clone();
                let region =
                    RegionDraft::from_region(&bc.region, NODE_SOURCES, Target::Nodes, mesh);
                (Draft::BoundaryCondition(s, bc, region), i)
            }
            TreeItem::Load(s, i) => {
                let load = fe.steps.get(s)?.loads.get(i)?.clone();
                let region = match load.kind {
                    LoadKind::ConcentratedForce(_) => {
                        RegionDraft::from_region(&load.region, NODE_SOURCES, Target::Nodes, mesh)
                    }
                    LoadKind::Pressure(_) | LoadKind::SurfaceTraction(_) => {
                        RegionDraft::from_region(&load.region, FACE_SOURCES, face_target(fe), mesh)
                    }
                };
                (Draft::Load(s, load, region), i)
            }
            TreeItem::FieldOutput(s, i) => {
                let output = fe.steps.get(s)?.field_outputs.get(i)?.clone();
                (Draft::FieldOutput(s, output), i)
            }
            TreeItem::Constraint(i) => (
                Draft::Constraint(ConstraintDraft::edit(fe.constraints.get(i)?, mesh)),
                i,
            ),
            TreeItem::SurfaceInteraction(i) => (
                Draft::SurfaceInteraction(fe.surface_interactions.get(i)?.clone(), 0),
                i,
            ),
            TreeItem::ContactPair(i) => {
                let pair = fe.contact_pairs.get(i)?.clone();
                let regions = MasterSlave::from_regions(&pair.master, &pair.slave, mesh);
                (Draft::ContactPair(pair, regions), i)
            }
            TreeItem::HotSpot(i) => {
                let hot_spot = fe.hot_spots.get(i)?.clone();
                let region =
                    RegionDraft::from_region(&hot_spot.toe, NODE_SOURCES, Target::Nodes, mesh);
                let distances = match &hot_spot.extrapolation {
                    Extrapolation::Custom(d) => format_distances(d),
                    _ => String::new(),
                };
                (
                    Draft::HotSpot(hot_spot, region, HotSpotText { distances }),
                    i,
                )
            }
            _ => return None,
        };
        Some(Self {
            draft,
            index: Some(index),
            error: None,
            picker: Picker::default(),
        })
    }

    pub fn title(&self) -> String {
        let (kind, name): (&str, &str) = match &self.draft {
            Draft::Material(m) => ("Material", &m.name),
            Draft::Section(s, _) => ("Section", &s.name),
            Draft::Step(s) => ("Step", &s.name),
            Draft::BoundaryCondition(_, b, _) => ("Randbedingung", &b.name),
            Draft::Load(_, l, _) => ("Last", &l.name),
            Draft::FieldOutput(_, f) => ("Field Output", &f.name),
            Draft::HotSpot(h, ..) => ("Hot Spot", &h.name),
            Draft::Constraint(c) => ("Constraint", c.name()),
            Draft::SurfaceInteraction(s, _) => ("Surface Interaction", &s.name),
            Draft::ContactPair(c, _) => ("Contact Pair", &c.name),
        };
        let action = if self.index.is_some() {
            "bearbeiten"
        } else {
            "erstellen"
        };
        format!("{kind} {action}: {name}")
    }

    /// The boundary condition or load being edited as it would be applied: its step, its
    /// index (`None` for a new one) and the symbol item, for the 3D view.
    pub fn step_item(&self) -> Option<(usize, Option<usize>, crate::symbols::Item)> {
        let (step, kind, region) = match &self.draft {
            Draft::BoundaryCondition(s, bc, r) => (*s, crate::symbols::Kind::Boundary(bc.kind), r),
            Draft::Load(s, load, r) => (*s, crate::symbols::Kind::Load(load.kind), r),
            _ => return None,
        };
        let item = crate::symbols::Item {
            kind,
            region: region.region(),
            selected: true,
        };
        Some((step, self.index, item))
    }

    /// Whether clicks in the 3D view pick for this dialog.
    pub fn picks(&self) -> bool {
        self.region()
            .is_some_and(|r| matches!(r.source, Source::Selection | Source::Parts))
    }

    fn region(&self) -> Option<&RegionDraft> {
        draft_region(&self.draft)
    }

    /// A click in the 3D view: the hit with the pick tolerance there, or `None` for empty
    /// space.
    pub fn click(&mut self, model: &Model, pick: Option<(&Hit, f32)>, operation: Operation) {
        if let Some(r) = draft_region_mut(&mut self.draft) {
            r.click(model, &self.picker, pick, operation);
        }
    }

    /// A selection box dragged in the 3D view.
    pub fn box_select(&mut self, model: &Model, area: &BoxSelect, operation: Operation) {
        if let Some(r) = draft_region_mut(&mut self.draft)
            && r.source == Source::Selection
        {
            r.take(self.picker.pick_box(model, area, r.target), operation);
        }
    }

    /// What a click at the hit would select, for the hover preview.
    pub fn preview(&self, model: &Model, hit: &Hit, precision: f32) -> Preview {
        match self.region() {
            Some(region) if region.source == Source::Selection => {
                let items = self.picker.pick(model, hit, region.target, precision);
                crate::selection::preview(model, &items)
            }
            _ => Preview::default(),
        }
    }

    fn region_mut(&mut self) -> Option<&mut RegionDraft> {
        draft_region_mut(&mut self.draft)
    }

    /// The region being edited, for the 3D view.
    pub fn highlight(&self, model: &Model) -> Highlight {
        match &self.draft {
            Draft::ContactPair(_, regions) => regions.highlight(model),
            Draft::Constraint(c) => c.highlight(model),
            _ => self
                .region()
                .map_or_else(Highlight::default, |r| r.highlight(model)),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, model: &Model) -> EditorResult {
        let mut result = EditorResult::Open;
        let mut open = true;
        let window = egui::Window::new(self.title())
            .id(egui::Id::new("item editor"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                egui::Grid::new("item form")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| self.form(ui, model));
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 0, 0), error);
                }
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Abbrechen").clicked() {
                        result = EditorResult::Cancel;
                    }
                    if ui.button("OK").clicked() {
                        self.error = self.validate(&model.fe).err();
                        if self.error.is_none() {
                            result = EditorResult::Ok;
                        }
                    }
                });
            });
        if let Some(window) = window
            && let Some(region) = self.region()
            && region.source == Source::Selection
        {
            let (target, can_undo) = (region.target, region.can_undo());
            let action = self
                .picker
                .window(ctx, window.response.rect, target, can_undo);
            if let (Some(action), Some(region)) = (action, self.region_mut()) {
                region.action(model, action);
            }
        }
        if !open {
            result = EditorResult::Cancel;
        }
        result
    }

    fn form(&mut self, ui: &mut Ui, model: &Model) {
        let taken = self.taken(&model.fe);
        let space = model.fe.properties.space;
        let (two_d, axisymmetric) = (space.is_2d(), space == ModelSpace::Axisymmetric);
        match &mut self.draft {
            Draft::Material(material) => material_form(ui, material),
            Draft::Section(section, region) => {
                name_row(ui, &mut section.name);
                ui.label("Material");
                egui::ComboBox::from_id_salt("section material")
                    .selected_text(section.material.as_str())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for material in &model.fe.materials {
                            let name = material.name.clone();
                            ui.selectable_value(&mut section.material, name, &material.name);
                        }
                    });
                ui.end_row();
                // Plane stress and plane strain sections have a thickness, as in PrePoMax.
                if model.fe.properties.space.has_thickness() {
                    ui.label("Dicke");
                    ui.add(numeric::drag_value(&mut section.thickness).range(0.0..=f64::MAX));
                    ui.end_row();
                }
                region.ui(ui, model);
            }
            Draft::Step(step) => step_form(ui, step, self.index.is_none(), &model.fe),
            Draft::BoundaryCondition(_, bc, region) => {
                name_row(ui, &mut bc.name);
                ui.label("Art");
                ui.horizontal(|ui| {
                    let fixed = matches!(bc.kind, BoundaryKind::Fixed);
                    if ui.radio(fixed, "Fest eingespannt").clicked() && !fixed {
                        bc.kind = BoundaryKind::Fixed;
                        rename_default(&mut bc.name, DISPLACEMENT, FIXED, &taken);
                    }
                    if ui.radio(!fixed, "Verschiebung/Rotation").clicked() && fixed {
                        bc.kind =
                            BoundaryKind::Displacement([Some(0.0), None, None, None, None, None]);
                        rename_default(&mut bc.name, FIXED, DISPLACEMENT, &taken);
                    }
                });
                ui.end_row();
                if let BoundaryKind::Displacement(values) = &mut bc.kind {
                    // Nodes of 2D models only move in the x-y plane.
                    let dofs = if two_d { 2 } else { 6 };
                    for (value, label) in values
                        .iter_mut()
                        .zip(["U1", "U2", "U3", "UR1", "UR2", "UR3"])
                        .take(dofs)
                    {
                        let mut set = value.is_some();
                        ui.checkbox(&mut set, label);
                        let mut number = value.unwrap_or(0.0);
                        ui.add_enabled(set, numeric::drag_value(&mut number).speed(0.01));
                        *value = set.then_some(number);
                        ui.end_row();
                    }
                }
                region.ui(ui, model);
            }
            Draft::Load(_, load, region) => {
                name_row(ui, &mut load.name);
                ui.label("Art");
                ui.horizontal(|ui| {
                    let current = load_kind_name(&load.kind);
                    for (label, name, kind) in load_kinds() {
                        if ui.radio(current == name, label).clicked() && current != name {
                            let was_on_nodes = matches!(load.kind, LoadKind::ConcentratedForce(_));
                            let on_nodes = matches!(kind, LoadKind::ConcentratedForce(_));
                            load.kind = kind;
                            rename_default(&mut load.name, current, name, &taken);
                            if on_nodes {
                                *region = RegionDraft::new(NODE_SOURCES, Target::Nodes);
                            } else if was_on_nodes {
                                *region = RegionDraft::new(FACE_SOURCES, face_target(&model.fe));
                            }
                        }
                    }
                });
                ui.end_row();
                match &mut load.kind {
                    LoadKind::ConcentratedForce(force) => {
                        force_rows(ui, force, two_d);
                        ui.label("");
                        ui.weak("Die Kraft wirkt an jedem Knoten der Region.");
                        ui.end_row();
                        revolution_hint(ui, axisymmetric);
                    }
                    LoadKind::Pressure(pressure) => {
                        ui.label("Druck");
                        ui.add(numeric::drag_value(pressure).speed(0.1));
                        ui.end_row();
                    }
                    LoadKind::SurfaceTraction(force) => {
                        force_rows(ui, force, two_d);
                        ui.label("");
                        ui.weak(
                            "Gesamtkraft, beim Export flächengewichtet auf die Knoten verteilt.",
                        );
                        ui.end_row();
                        revolution_hint(ui, axisymmetric);
                    }
                }
                region.ui(ui, model);
            }
            Draft::FieldOutput(_, output) => {
                name_row(ui, &mut output.name);
                let choices: &[&str] = match output.kind {
                    OutputKind::Node => &["RF", "U"],
                    OutputKind::Element => &["S", "E", "ME", "PEEQ", "ENER"],
                };
                ui.label("Variablen");
                ui.horizontal(|ui| {
                    for &variable in choices {
                        let mut on = output.variables.iter().any(|v| v == variable);
                        if ui.checkbox(&mut on, variable).changed() {
                            output.variables.retain(|v| v != variable);
                            if on {
                                // Keep PrePoMax's order of the variables.
                                output.variables.push(variable.to_string());
                                output.variables.sort_by_key(|v| {
                                    choices.iter().position(|c| c == v).unwrap_or(usize::MAX)
                                });
                            }
                        }
                    }
                });
                ui.end_row();
            }
            Draft::HotSpot(hot_spot, region, text) => {
                hot_spot_form(ui, model, hot_spot, region, text)
            }
            Draft::Constraint(c) => c.form(ui, model, &taken, self.index.is_none()),
            Draft::SurfaceInteraction(interaction, selected) => {
                name_row(ui, &mut interaction.name);
                contacts::interaction_form(ui, interaction, selected);
            }
            Draft::ContactPair(pair, regions) => {
                name_row(ui, &mut pair.name);
                contacts::contact_pair_form(ui, model, pair, regions);
            }
        }
    }

    /// The hot spot as currently entered, for showing its paths while the dialog is open.
    pub fn hot_spot(&self) -> Option<HotSpot> {
        match &self.draft {
            Draft::HotSpot(hot_spot, region, _) => Some(HotSpot {
                toe: region.region(),
                ..hot_spot.clone()
            }),
            _ => None,
        }
    }

    /// Names of the other items of the same kind, which the draft's name must not repeat.
    fn taken<'a>(&self, fe: &'a FeModel) -> Vec<&'a str> {
        let mut siblings = match &self.draft {
            Draft::Material(_) => names(&fe.materials, |m| &m.name),
            Draft::Section(..) => names(&fe.sections, |s| &s.name),
            Draft::Step(_) => names(&fe.steps, |s| &s.name),
            Draft::BoundaryCondition(step, ..) => {
                names(&fe.steps[*step].boundary_conditions, |b| &b.name)
            }
            Draft::Load(step, ..) => names(&fe.steps[*step].loads, |l| &l.name),
            Draft::FieldOutput(step, _) => names(&fe.steps[*step].field_outputs, |f| &f.name),
            Draft::HotSpot(..) => names(&fe.hot_spots, |h| &h.name),
            Draft::Constraint(_) => fe.constraints.iter().map(Constraint::name).collect(),
            Draft::SurfaceInteraction(..) => names(&fe.surface_interactions, |s| &s.name),
            Draft::ContactPair(..) => names(&fe.contact_pairs, |c| &c.name),
        };
        if let Some(index) = self.index.filter(|&i| i < siblings.len()) {
            siblings.remove(index);
        }
        siblings
    }

    fn validate(&self, fe: &FeModel) -> Result<(), String> {
        let name = match &self.draft {
            Draft::Material(m) => &m.name,
            Draft::Section(s, _) => &s.name,
            Draft::Step(s) => &s.name,
            Draft::BoundaryCondition(_, b, _) => &b.name,
            Draft::Load(_, l, _) => &l.name,
            Draft::FieldOutput(_, f) => &f.name,
            Draft::HotSpot(h, ..) => &h.name,
            Draft::Constraint(c) => c.name(),
            Draft::SurfaceInteraction(s, _) => &s.name,
            Draft::ContactPair(c, _) => &c.name,
        };
        if name.trim().is_empty() {
            return Err("Bitte einen Namen eingeben.".into());
        }
        let duplicate = self
            .taken(fe)
            .iter()
            .any(|other| other.eq_ignore_ascii_case(name));
        if duplicate {
            return Err(format!("Der Name {name} ist schon vergeben."));
        }
        if let Draft::Section(section, _) = &self.draft
            && !fe.materials.iter().any(|m| m.name == section.material)
        {
            return Err("Bitte ein Material wählen; zuerst unter Materials anlegen.".into());
        }
        match &self.draft {
            Draft::Constraint(c) => c.validate()?,
            Draft::ContactPair(pair, regions) => {
                contacts::validate_contact_pair(pair, fe)?;
                regions.validate()?;
            }
            Draft::SurfaceInteraction(interaction, _) => {
                contacts::validate_interaction(interaction)?;
            }
            _ => {
                if self.region().is_some_and(RegionDraft::is_empty) {
                    return Err("Die Region ist leer.".into());
                }
            }
        }
        if let Draft::Step(Step {
            kind: StepKind::Frequency(settings),
            ..
        }) = &self.draft
        {
            validate_frequency_step(settings)?;
        }
        if let Some(hot_spot) = self.hot_spot() {
            validate_hot_spot(&hot_spot)?;
        }
        Ok(())
    }

    /// Copies the draft into the model.
    pub fn apply(self, fe: &mut FeModel) {
        fn put<T>(items: &mut Vec<T>, index: Option<usize>, item: T) {
            match index.and_then(|i| items.get_mut(i)) {
                Some(slot) => *slot = item,
                None => items.push(item),
            }
        }
        let index = self.index;
        match self.draft {
            Draft::Material(material) => {
                // Sections follow a renamed material.
                if let Some(old) = index.and_then(|i| fe.materials.get(i)) {
                    let old = old.name.clone();
                    for section in fe.sections.iter_mut().filter(|s| s.material == old) {
                        section.material = material.name.clone();
                    }
                }
                put(&mut fe.materials, index, material);
            }
            Draft::Section(mut section, region) => {
                section.region = region.region();
                put(&mut fe.sections, index, section);
            }
            Draft::Step(step) => match index.and_then(|i| fe.steps.get_mut(i)) {
                // The dialog edits the settings; the step's items stay.
                Some(existing) => {
                    existing.name = step.name;
                    existing.kind = step.kind;
                }
                None => {
                    let mut step = step;
                    copy_items_of_last_step(fe, &mut step);
                    fe.steps.push(step);
                }
            },
            // Switching an item on or off while its dialog is open is kept.
            Draft::BoundaryCondition(s, mut bc, region) => {
                bc.region = region.region();
                let list = &mut fe.steps[s].boundary_conditions;
                if let Some(existing) = index.and_then(|i| list.get(i)) {
                    bc.active = existing.active;
                }
                put(list, index, bc);
            }
            Draft::Load(s, mut load, region) => {
                load.region = region.region();
                let list = &mut fe.steps[s].loads;
                if let Some(existing) = index.and_then(|i| list.get(i)) {
                    load.active = existing.active;
                }
                put(list, index, load);
            }
            Draft::FieldOutput(s, output) => put(&mut fe.steps[s].field_outputs, index, output),
            Draft::HotSpot(mut hot_spot, region, _) => {
                hot_spot.toe = region.region();
                put(&mut fe.hot_spots, index, hot_spot);
            }
            Draft::Constraint(draft) => {
                let mut constraint = draft.finish();
                if let Some(existing) = index.and_then(|i| fe.constraints.get(i)) {
                    *constraint.active_mut() = existing.active();
                }
                put(&mut fe.constraints, index, constraint);
            }
            Draft::SurfaceInteraction(interaction, _) => {
                // Contact pairs follow a renamed interaction.
                if let Some(old) = index.and_then(|i| fe.surface_interactions.get(i)) {
                    let old = old.name.clone();
                    for pair in fe.contact_pairs.iter_mut().filter(|c| c.interaction == old) {
                        pair.interaction = interaction.name.clone();
                    }
                }
                put(&mut fe.surface_interactions, index, interaction);
            }
            Draft::ContactPair(mut pair, regions) => {
                (pair.master, pair.slave) = regions.regions();
                if let Some(existing) = index.and_then(|i| fe.contact_pairs.get(i)) {
                    pair.active = existing.active;
                }
                put(&mut fe.contact_pairs, index, pair);
            }
        }
    }
}

/// Removes an item; returns false for items that cannot be deleted.
pub fn delete(fe: &mut FeModel, item: &TreeItem) -> bool {
    fn remove<T>(items: &mut Vec<T>, index: usize) -> bool {
        (index < items.len()).then(|| items.remove(index)).is_some()
    }
    match *item {
        TreeItem::Material(i) => remove(&mut fe.materials, i),
        TreeItem::Section(i) => remove(&mut fe.sections, i),
        TreeItem::Step(i) => remove(&mut fe.steps, i),
        TreeItem::BoundaryCondition(s, i) => fe
            .steps
            .get_mut(s)
            .is_some_and(|st| remove(&mut st.boundary_conditions, i)),
        TreeItem::Load(s, i) => fe
            .steps
            .get_mut(s)
            .is_some_and(|st| remove(&mut st.loads, i)),
        TreeItem::FieldOutput(s, i) => fe
            .steps
            .get_mut(s)
            .is_some_and(|st| remove(&mut st.field_outputs, i)),
        TreeItem::HotSpot(i) => remove(&mut fe.hot_spots, i),
        TreeItem::Constraint(i) => remove(&mut fe.constraints, i),
        TreeItem::SurfaceInteraction(i) => remove(&mut fe.surface_interactions, i),
        TreeItem::ContactPair(i) => remove(&mut fe.contact_pairs, i),
        _ => false,
    }
}

/// Activates a deactivated step, boundary condition or load, or deactivates an active one,
/// like PrePoMax's Activate and Deactivate; returns false for other items.
pub fn toggle_active(fe: &mut FeModel, item: &TreeItem) -> bool {
    let active = match *item {
        TreeItem::Step(s) => fe.steps.get_mut(s).map(|st| &mut st.active),
        TreeItem::BoundaryCondition(s, i) => (fe.steps.get_mut(s))
            .and_then(|st| st.boundary_conditions.get_mut(i))
            .map(|b| &mut b.active),
        TreeItem::Load(s, i) => (fe.steps.get_mut(s))
            .and_then(|st| st.loads.get_mut(i))
            .map(|l| &mut l.active),
        TreeItem::Constraint(i) => fe.constraints.get_mut(i).map(Constraint::active_mut),
        TreeItem::ContactPair(i) => fe.contact_pairs.get_mut(i).map(|c| &mut c.active),
        _ => None,
    };
    active.map(|a| *a = !*a).is_some()
}

/// How an item selected in the tree shows in the 3D view: its region, or master and slave.
pub fn item_highlight(model: &Model, item: &TreeItem) -> Highlight {
    let fe = &model.fe;
    let master_slave = match *item {
        TreeItem::Constraint(i) => fe.constraints.get(i).and_then(Constraint::master_slave),
        TreeItem::ContactPair(i) => fe.contact_pairs.get(i).map(|c| [&c.master, &c.slave]),
        _ => None,
    };
    if let Some([master, slave]) = master_slave {
        return contacts::master_slave_highlight(model, master, slave);
    }
    item_region(fe, item)
        .map(|region| region_highlight(model, region))
        .unwrap_or_default()
}

/// Region of an item, for highlighting it when it is selected in the tree.
pub fn item_region<'a>(fe: &'a FeModel, item: &TreeItem) -> Option<&'a Region> {
    match *item {
        TreeItem::Section(i) => fe.sections.get(i).map(|s| &s.region),
        TreeItem::BoundaryCondition(s, i) => fe
            .steps
            .get(s)?
            .boundary_conditions
            .get(i)
            .map(|b| &b.region),
        TreeItem::Load(s, i) => fe.steps.get(s)?.loads.get(i).map(|l| &l.region),
        TreeItem::HotSpot(i) => fe.hot_spots.get(i).map(|h| &h.toe),
        TreeItem::Constraint(i) => fe.constraints.get(i)?.regions().first().copied(),
        _ => None,
    }
}

/// Keeps a default name in step with the item kind, e.g. Fixed-1 becomes the next free
/// Displacement_Rotation-n, but leaves names the user chose.
fn rename_default(name: &mut String, from: &str, to: &str, taken: &[&str]) {
    if let Some(number) = name.strip_prefix(from).and_then(|n| n.strip_prefix('-'))
        && number.parse::<u32>().is_ok()
    {
        *name = next_name(to, taken.iter().copied());
    }
}

fn name_row(ui: &mut Ui, name: &mut String) {
    ui.label("Name");
    ui.add(egui::TextEdit::singleline(name).desired_width(200.0));
    ui.end_row();
}

fn material_form(ui: &mut Ui, material: &mut Material) {
    name_row(ui, &mut material.name);
    let mut has_density = material.density.is_some();
    ui.checkbox(&mut has_density, "Dichte");
    let mut density = material.density.unwrap_or(0.0);
    ui.add_enabled(has_density, number(&mut density));
    material.density = has_density.then_some(density);
    ui.end_row();
    let mut elastic = material.elastic.is_some();
    ui.checkbox(&mut elastic, "Elastizität");
    ui.end_row();
    let mut values = material.elastic.unwrap_or(Elastic {
        young: 0.0,
        poisson: 0.0,
    });
    ui.label("    E-Modul");
    ui.add_enabled(elastic, number(&mut values.young));
    ui.end_row();
    ui.label("    Querkontraktionszahl");
    ui.add_enabled(
        elastic,
        numeric::drag_value(&mut values.poisson)
            .range(0.0..=0.5)
            .speed(0.01)
            .max_decimals(4),
    );
    ui.end_row();
    material.elastic = elastic.then_some(values);
}

/// Solution settings for a new static step: PrePoMax carries those of the last static step
/// over.
fn previous_static(fe: &FeModel) -> StaticStep {
    (fe.steps.iter().rev())
        .find_map(|s| match &s.kind {
            StepKind::Static(settings) => Some(settings.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// A new step starts with the boundary conditions and loads of the last step, as in
/// PrePoMax; loads only where the new step takes them.
fn copy_items_of_last_step(fe: &FeModel, step: &mut Step) {
    let Some(last) = fe.steps.last() else {
        return;
    };
    step.boundary_conditions = last.boundary_conditions.clone();
    if step.kind.supports_loads() {
        step.loads = last.loads.clone();
    }
}

const STATIC_LABEL: &str = "Statisch (Static)";
const FREQUENCY_LABEL: &str = "Eigenfrequenzen (Frequency)";

fn step_kind_label(kind: &StepKind) -> &'static str {
    match kind {
        StepKind::Static(_) => STATIC_LABEL,
        StepKind::Frequency(_) => FREQUENCY_LABEL,
    }
}

/// The step dialog. The kind is chosen when the step is created, as in PrePoMax's list of
/// step types; switching it starts with that kind's default field outputs.
fn step_form(ui: &mut Ui, step: &mut Step, creating: bool, fe: &FeModel) {
    name_row(ui, &mut step.name);
    ui.label("Art");
    if creating {
        let frequency = matches!(step.kind, StepKind::Frequency(_));
        egui::ComboBox::from_id_salt("step kind")
            .selected_text(step_kind_label(&step.kind))
            .width(200.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(!frequency, STATIC_LABEL).clicked() && frequency {
                    step.kind = StepKind::Static(previous_static(fe));
                    step.field_outputs = FieldOutput::defaults();
                }
                if ui.selectable_label(frequency, FREQUENCY_LABEL).clicked() && !frequency {
                    step.kind = StepKind::Frequency(FrequencyStep::default());
                    step.field_outputs = FieldOutput::frequency_defaults();
                }
            });
    } else {
        ui.label(step_kind_label(&step.kind));
    }
    ui.end_row();
    match &mut step.kind {
        StepKind::Static(settings) => static_form(ui, settings),
        StepKind::Frequency(settings) => frequency_form(ui, settings),
    }
}

fn solver_row(ui: &mut Ui, solver: &mut EquationSolver, eigenvalues: bool) {
    ui.label("Gleichungslöser");
    egui::ComboBox::from_id_salt("equation solver")
        .selected_text(solver_label(*solver))
        .show_ui(ui, |ui| {
            for choice in EquationSolver::ALL {
                if eigenvalues && !choice.solves_eigenvalues() {
                    continue;
                }
                ui.selectable_value(solver, choice, solver_label(choice));
            }
        });
    ui.end_row();
}

fn frequency_form(ui: &mut Ui, settings: &mut FrequencyStep) {
    ui.label("");
    ui.checkbox(
        &mut settings.perturbation,
        "Vorspannung aus vorigem Step (Perturbation)",
    );
    ui.end_row();
    solver_row(ui, &mut settings.solver, true);
    ui.label("Anzahl Eigenfrequenzen");
    ui.add(numeric::drag_value(&mut settings.num_frequencies).range(1..=10_000));
    ui.end_row();
    for (label, bound) in [
        ("Untere Frequenzgrenze", &mut settings.lower_frequency),
        ("Obere Frequenzgrenze", &mut settings.upper_frequency),
    ] {
        let mut set = bound.is_some();
        ui.checkbox(&mut set, label);
        let mut value = bound.unwrap_or(0.0);
        ui.add_enabled(
            set,
            numeric::drag_value(&mut value)
                .range(0.0..=f64::MAX)
                .speed(1.0)
                .suffix(" Hz"),
        );
        *bound = set.then_some(value);
        ui.end_row();
    }
    ui.label("");
    ui.checkbox(
        &mut settings.storage,
        "Matrizen und Eigenformen speichern (Storage, .eig)",
    );
    ui.end_row();
    ui.label("");
    ui.weak("Lasten wirken in einem Frequency Step nicht; nur die Randbedingungen zählen.");
    ui.end_row();
}

fn validate_frequency_step(settings: &FrequencyStep) -> Result<(), String> {
    if !settings.solver.solves_eigenvalues() {
        return Err("Die iterativen Löser können keine Eigenfrequenzen berechnen.".into());
    }
    if let (Some(lower), Some(upper)) = (settings.lower_frequency, settings.upper_frequency)
        && lower >= upper
    {
        return Err("Die untere Frequenzgrenze muss kleiner als die obere sein.".into());
    }
    Ok(())
}

fn static_form(ui: &mut Ui, settings: &mut StaticStep) {
    ui.label("");
    ui.checkbox(&mut settings.nlgeom, "Geometrisch nichtlinear (Nlgeom)");
    ui.end_row();
    solver_row(ui, &mut settings.solver, false);
    ui.label("Inkrementierung");
    egui::ComboBox::from_id_salt("incrementation")
        .selected_text(incrementation_label(settings.incrementation))
        .show_ui(ui, |ui| {
            for choice in [
                Incrementation::Default,
                Incrementation::Automatic,
                Incrementation::Direct,
            ] {
                let label = incrementation_label(choice);
                ui.selectable_value(&mut settings.incrementation, choice, label);
            }
        });
    ui.end_row();
    let custom = settings.incrementation != Incrementation::Default;
    let automatic = settings.incrementation == Incrementation::Automatic;
    ui.label("Max. Inkremente");
    ui.add_enabled(
        custom,
        numeric::drag_value(&mut settings.max_increments).range(1..=1_000_000),
    );
    ui.end_row();
    for (label, value, enabled) in [
        ("Zeitraum", &mut settings.time_period, custom),
        ("Anfangsinkrement", &mut settings.initial_increment, custom),
        ("Min. Inkrement", &mut settings.min_increment, automatic),
        ("Max. Inkrement", &mut settings.max_increment, automatic),
    ] {
        ui.label(label);
        ui.add_enabled(enabled, number(value));
        ui.end_row();
    }
}

fn hot_spot_form(
    ui: &mut Ui,
    model: &Model,
    hot_spot: &mut HotSpot,
    region: &mut RegionDraft,
    text: &mut HotSpotText,
) {
    name_row(ui, &mut hot_spot.name);
    ui.label("Extrapolation");
    let custom = matches!(hot_spot.extrapolation, Extrapolation::Custom(_));
    egui::ComboBox::from_id_salt("hot spot extrapolation")
        .selected_text(hot_spot.extrapolation.label())
        .width(240.0)
        .show_ui(ui, |ui| {
            for method in Extrapolation::IIW {
                let label = method.label();
                ui.selectable_value(&mut hot_spot.extrapolation, method, label);
            }
            if ui.selectable_label(custom, "Eigene Lesepunkte").clicked() && !custom {
                let distances = hot_spot.distances();
                text.distances = format_distances(&distances);
                hot_spot.extrapolation = Extrapolation::Custom(distances);
            }
        });
    ui.end_row();
    if let Extrapolation::Custom(distances) = &mut hot_spot.extrapolation {
        ui.label("Abstände");
        let edit = egui::TextEdit::singleline(&mut text.distances)
            .hint_text("z. B. 2, 6, 10")
            .desired_width(200.0);
        if ui.add(edit).changed() {
            *distances = parse_distances(&text.distances);
        }
        ui.end_row();
    }
    ui.label("Blechdicke t");
    ui.add_enabled(
        hot_spot.extrapolation.uses_thickness(),
        numeric::drag_value(&mut hot_spot.thickness)
            .range(0.0..=f64::MAX)
            .speed(0.1),
    );
    ui.end_row();
    ui.label("Lesepunkte");
    let distances = hot_spot.distances();
    let weights = extrapolation_weights(&distances);
    let mut formula = String::from("S_hs =");
    for (i, (d, w)) in distances.iter().zip(&weights).enumerate() {
        let sign = match (i, *w < 0.0) {
            (0, false) => "",
            (0, true) => " -",
            (_, false) => " +",
            (_, true) => " -",
        };
        formula += &format!("{sign} {:.3} S({})", w.abs(), crate::hot_spots::short(*d));
    }
    ui.vertical(|ui| {
        ui.label(formula);
        if matches!(
            hot_spot.extrapolation,
            Extrapolation::IiwTypeBFine | Extrapolation::IiwTypeBCoarse
        ) {
            ui.weak("Abstände in mm: das Modell muss in mm sein.");
        }
    });
    ui.end_row();
    ui.label("Spannung");
    egui::ComboBox::from_id_salt("hot spot component")
        .selected_text(hot_spot.component.label())
        .width(240.0)
        .show_ui(ui, |ui| {
            for component in HotSpotComponent::ALL {
                ui.selectable_value(&mut hot_spot.component, component, component.label());
            }
        });
    ui.end_row();
    ui.label("Pfadrichtung");
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            for (value, label) in hot_spot.direction.iter_mut().zip(["X", "Y", "Z"]) {
                ui.label(label);
                ui.add(numeric::drag_value(value).speed(0.05));
            }
        });
        ui.weak("Vom Nahtübergang weg; wird quer zur Naht in die Blechoberfläche gedreht.");
    });
    ui.end_row();
    region.ui(ui, model);
    ui.label("");
    ui.weak("Knoten am Nahtübergang, z. B. als Kante.");
    ui.end_row();
}

fn validate_hot_spot(hot_spot: &HotSpot) -> Result<(), String> {
    let distances = hot_spot.distances();
    if hot_spot.extrapolation.uses_thickness()
        && hot_spot.thickness.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
    {
        return Err("Die Blechdicke muss größer als null sein.".into());
    }
    if distances.len() < 2 {
        return Err("Mindestens zwei Lesepunkte angeben.".into());
    }
    let mut sorted = distances.clone();
    sorted.sort_by(f64::total_cmp);
    if sorted[0] <= 0.0 || sorted.windows(2).any(|w| w[0] == w[1]) {
        return Err("Die Abstände müssen positiv und verschieden sein.".into());
    }
    if hot_spot.direction.iter().all(|&v| v == 0.0) {
        return Err("Bitte eine Pfadrichtung angeben.".into());
    }
    Ok(())
}

fn format_distances(distances: &[f64]) -> String {
    let parts: Vec<String> = distances
        .iter()
        .map(|&d| crate::hot_spots::short(d))
        .collect();
    parts.join(", ")
}

/// Distances separated by spaces or semicolons; a comma right after a number separates
/// as well, a comma inside one is a decimal comma ("2, 6" are two, "2,5" is one).
fn parse_distances(text: &str) -> Vec<f64> {
    text.split(|c: char| c == ';' || c.is_whitespace())
        .map(|part| part.trim_end_matches(','))
        .filter(|part| !part.is_empty())
        .filter_map(numeric::parse_number)
        .collect()
}

fn solver_label(solver: EquationSolver) -> &'static str {
    match solver {
        EquationSolver::Default => "Standard (Pardiso, falls vorhanden)",
        EquationSolver::Pardiso => "Pardiso",
        EquationSolver::Spooles => "Spooles",
        EquationSolver::PaStiX => "PaStiX",
        EquationSolver::IterativeScaling => "Iterative scaling",
        EquationSolver::IterativeCholesky => "Iterative Cholesky",
    }
}

fn incrementation_label(incrementation: Incrementation) -> &'static str {
    match incrementation {
        Incrementation::Default => "Standard",
        Incrementation::Automatic => "Automatisch",
        Incrementation::Direct => "Fest (Direct)",
    }
}

/// Field for physical values that may be very small or large, such as a density of 7.85e-9.
pub(crate) fn number(value: &mut f64) -> egui::DragValue<'_> {
    numeric::drag_value(value)
        .speed(0.0)
        .custom_formatter(|v, _| {
            if v != 0.0 && !(1e-3..1e7).contains(&v.abs()) {
                format!("{v:e}")
            } else {
                format!("{v}")
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_follow_the_kind() {
        let mut name = "Fixed-2".to_string();
        rename_default(&mut name, FIXED, DISPLACEMENT, &["Fixed-1"]);
        assert_eq!(name, "Displacement_Rotation-1");
        let mut name = "Einspannung".to_string();
        rename_default(&mut name, FIXED, DISPLACEMENT, &[]);
        assert_eq!(name, "Einspannung");
    }

    #[test]
    fn switching_the_kind_proposes_a_free_name() {
        let mut fe = FeModel::default();
        let mut step = Step::new_static("Step-1");
        step.boundary_conditions.push(BoundaryCondition {
            name: "Displacement_Rotation-1".into(),
            active: true,
            region: Region::Nodes(Vec::new()),
            kind: BoundaryKind::Displacement([Some(0.0), None, None, None, None, None]),
        });
        fe.steps.push(step);
        let mut editor = Editor::create(NewItem::BoundaryCondition(0), &fe).unwrap();
        let taken = editor.taken(&fe);
        let Draft::BoundaryCondition(_, bc, _) = &mut editor.draft else {
            unreachable!()
        };
        assert_eq!(bc.name, "Fixed-1");
        rename_default(&mut bc.name, FIXED, DISPLACEMENT, &taken);
        assert_eq!(bc.name, "Displacement_Rotation-2");
        // An edited item does not block its own name.
        let mut editor = Editor::create(NewItem::BoundaryCondition(0), &fe).unwrap();
        editor.index = Some(0);
        assert!(editor.taken(&fe).is_empty());
    }

    #[test]
    fn regions_survive_the_dialog() {
        let mesh = FeMesh::default();
        for (region, target) in [
            (Region::Nodes(vec![1, 5]), Target::Nodes),
            (Region::Faces(vec![(3, 2), (4, 6)]), Target::Faces),
            (Region::NodeSet("FIX".into()), Target::Nodes),
            (Region::Surface("TIP".into()), Target::Faces),
        ] {
            let draft = RegionDraft::from_region(&region, NODE_SOURCES, target, &mesh);
            assert_eq!(draft.region(), region);
        }
        let parts = Region::Parts(vec!["A".into(), "B".into()]);
        let draft = RegionDraft::from_region(&parts, ELEMENT_SOURCES, Target::Faces, &mesh);
        assert_eq!(draft.region(), parts);
    }

    #[test]
    fn new_items_get_prepomax_names_and_land_in_their_step() {
        let mut fe = FeModel::default();
        for kind in [NewItem::Material, NewItem::Step] {
            Editor::create(kind, &fe).unwrap().apply(&mut fe);
        }
        assert_eq!(fe.materials[0].name, "Material-1");
        assert_eq!(fe.steps[0].name, "Step-1");
        let mut editor = Editor::create(NewItem::BoundaryCondition(0), &fe).unwrap();
        assert!(editor.validate(&fe).is_err(), "empty region");
        if let Draft::BoundaryCondition(_, _, region) = &mut editor.draft {
            region
                .nodes
                .push(Operation::Replace, BTreeSet::from([1, 4]));
        }
        assert_eq!(editor.validate(&fe), Ok(()));
        editor.apply(&mut fe);
        let bc = &fe.steps[0].boundary_conditions[0];
        assert_eq!(
            (bc.name.as_str(), &bc.region),
            ("Fixed-1", &Region::Nodes(vec![1, 4]))
        );
        assert!(Editor::create(NewItem::Load(3), &fe).is_none());
    }

    #[test]
    fn a_frequency_step_keeps_the_bcs_but_takes_no_loads() {
        let mut fe = FeModel::default();
        Editor::create(NewItem::Step, &fe).unwrap().apply(&mut fe);
        fe.steps[0].boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: Region::NodeSet("FIX".into()),
            kind: BoundaryKind::Fixed,
        });
        fe.steps[0].loads.push(Load {
            name: "Pressure-1".into(),
            active: true,
            region: Region::Surface("TOP".into()),
            kind: LoadKind::Pressure(1.0),
        });
        let mut editor = Editor::create(NewItem::Step, &fe).unwrap();
        if let Draft::Step(step) = &mut editor.draft {
            step.kind = StepKind::Frequency(FrequencyStep {
                lower_frequency: Some(100.0),
                upper_frequency: Some(50.0),
                ..FrequencyStep::default()
            });
        }
        assert!(editor.validate(&fe).is_err(), "bounds the wrong way round");
        if let Draft::Step(Step {
            kind: StepKind::Frequency(settings),
            ..
        }) = &mut editor.draft
        {
            settings.upper_frequency = None;
            settings.solver = EquationSolver::IterativeCholesky;
        }
        assert!(editor.validate(&fe).is_err(), "iterative solver");
        if let Draft::Step(Step {
            kind: StepKind::Frequency(settings),
            ..
        }) = &mut editor.draft
        {
            settings.solver = EquationSolver::Default;
        }
        assert_eq!(editor.validate(&fe), Ok(()));
        editor.apply(&mut fe);
        let frequency = &fe.steps[1];
        assert_eq!(frequency.name, "Step-2");
        assert_eq!(
            frequency.boundary_conditions,
            fe.steps[0].boundary_conditions
        );
        assert!(frequency.loads.is_empty());
        assert!(Editor::create(NewItem::Load(1), &fe).is_none());
        // A static step after it starts from the last static step's settings.
        let StepKind::Static(settings) = &mut fe.steps[0].kind else {
            unreachable!()
        };
        settings.nlgeom = true;
        Editor::create(NewItem::Step, &fe).unwrap().apply(&mut fe);
        assert!(matches!(&fe.steps[2].kind, StepKind::Static(s) if s.nlgeom));
    }

    #[test]
    fn steps_bcs_and_loads_are_switched_off_and_on() {
        let mut fe = FeModel::default();
        Editor::create(NewItem::Step, &fe).unwrap().apply(&mut fe);
        fe.steps[0].boundary_conditions.push(BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: Region::NodeSet("FIX".into()),
            kind: BoundaryKind::Fixed,
        });
        let bc = TreeItem::BoundaryCondition(0, 0);
        let editor = Editor::edit(&bc, &fe, &FeMesh::default()).unwrap();
        assert!(toggle_active(&mut fe, &bc));
        assert!(!fe.steps[0].boundary_conditions[0].active);
        // A dialog opened before keeps the switch as it is now.
        editor.apply(&mut fe);
        assert!(!fe.steps[0].boundary_conditions[0].active);
        assert!(toggle_active(&mut fe, &TreeItem::Step(0)));
        let editor = Editor::edit(&TreeItem::Step(0), &fe, &FeMesh::default()).unwrap();
        editor.apply(&mut fe);
        assert!(!fe.steps[0].active);
        // A new step takes the items of the last one as they are, like PrePoMax's copies.
        Editor::create(NewItem::Step, &fe).unwrap().apply(&mut fe);
        assert!(fe.steps[1].active);
        assert!(!fe.steps[1].boundary_conditions[0].active);
        assert!(toggle_active(&mut fe, &bc));
        assert!(fe.steps[0].boundary_conditions[0].active);
        assert!(!toggle_active(&mut fe, &TreeItem::Load(0, 0)));
        assert!(!toggle_active(&mut fe, &TreeItem::Material(0)));
    }

    #[test]
    fn hot_spots_are_created_and_checked() {
        let mut fe = FeModel::default();
        let mut editor = Editor::create(NewItem::HotSpot, &fe).unwrap();
        if let Draft::HotSpot(hot_spot, region, _) = &mut editor.draft {
            region.nodes.push(Operation::Replace, BTreeSet::from([7]));
            hot_spot.extrapolation = Extrapolation::Custom(vec![3.0, 3.0]);
        }
        assert!(editor.validate(&fe).is_err(), "equal distances");
        if let Draft::HotSpot(hot_spot, ..) = &mut editor.draft {
            hot_spot.extrapolation = Extrapolation::IiwTypeBCoarse;
        }
        assert_eq!(editor.validate(&fe), Ok(()));
        editor.apply(&mut fe);
        assert_eq!(fe.hot_spots[0].name, "Hot_Spot-1");
        assert_eq!(fe.hot_spots[0].toe, Region::Nodes(vec![7]));
        assert_eq!(
            item_region(&fe, &TreeItem::HotSpot(0)),
            Some(&Region::Nodes(vec![7]))
        );
    }

    #[test]
    fn distances_accept_lists_and_decimal_commas() {
        assert_eq!(parse_distances("2, 6, 10"), [2.0, 6.0, 10.0]);
        assert_eq!(parse_distances("0,4; 1,5"), [0.4, 1.5]);
        assert_eq!(parse_distances("4 8 x 12"), [4.0, 8.0, 12.0]);
        assert_eq!(format_distances(&[0.4 * 12.0, 12.0]), "4.8, 12");
    }
}
