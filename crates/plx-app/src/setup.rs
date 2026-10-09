//! Creating and editing the FE model: PrePoMax's item dialogs for materials, sections, steps,
//! boundary conditions, loads and field outputs, and prepolix's hot spot definitions.
//!
//! Regions are picked in the 3D view while a dialog is open. As in PrePoMax the user never
//! defines node or element sets for this; the input file writer derives them.

use std::collections::BTreeSet;

use egui::Ui;
use plx_mesh::{CadEntity, ElementId, FeMesh, NodeId};
use plx_model::{
    Amplitude, BeamOrientation, BeamProfile, BeamSection, BoundaryCondition, BoundaryKind,
    Constraint, ContactPair, Elastic, EquationSolver, FeModel, FieldOutput, FrequencyStep,
    Hardening, HeatTransferStep, HistoryKind, HistoryOutput, Incrementation, InitialCondition,
    InitialConditionKind, Load, LoadKind, Material, ModelSpace, NodeTie, OutputKind, PlasticPoint,
    Quantity, Region, Section, SectionKind, StaticStep, Step, StepKind, SurfaceInteraction,
    UnitSystem, next_name,
};

use crate::amplitude_dialog::{self, AmplitudeView, amplitude_row};
use crate::constraint_dialog::ConstraintDraft;
use crate::contacts::{self, MasterSlave};
use crate::model::{Highlight, Hit, Model};
use crate::numeric;
use crate::selection::{History, Items, Operation, PartPicks, Picker, PickerAction, Target};
use crate::tree::TreeItem;
use crate::viewport::{BoxSelect, Preview};

/// Kinds of items the tree can create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewItem {
    Material,
    Section,
    InitialCondition,
    Step,
    BoundaryCondition(usize),
    Load(usize),
    /// A history output of a step, printed into the `.dat` file.
    HistoryOutput(usize),
    /// A spring, support or tie, chosen in the dialog.
    Constraint,
    SurfaceInteraction,
    ContactPair,
    /// Nodes tied to each other, the ends of beams; listed with the contact pairs.
    NodeTie,
    /// A time curve for boundary conditions and loads.
    Amplitude,
    /// A field output derived from results, created in the Results tree.
    ResultFieldOutput,
    /// A history output derived from results, created in the Results tree.
    ResultHistoryOutput,
    /// A hot spot definition of results, created in the Results tree.
    ResultHotSpot,
    /// An item of the geometry's mesh setup, created in the Geometry tree.
    MeshSetupItem,
    /// A reference point, coordinate system or result path, edited in its own dialog.
    Feature(crate::features::FeatureKind),
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
    /// Picks of CAD entities on a mesh generated from geometry. Mixed with picks of nodes or
    /// faces they turn into those, so only one of the histories is in use at a time.
    geometry: History<CadEntity>,
    parts: PartPicks,
    set: String,
}

pub(crate) const NODE_SOURCES: &[Source] = &[Source::Selection, Source::NodeSet, Source::Surface];
pub(crate) const FACE_SOURCES: &[Source] = &[Source::Selection, Source::Surface];
/// Solid elements: whole parts, element sets or the elements of picked faces.
pub(crate) const SOLID_SOURCES: &[Source] = &[Source::Parts, Source::ElementSet, Source::Selection];
pub(crate) const ELEMENT_SOURCES: &[Source] = &[Source::Parts, Source::ElementSet];
/// Nodes, also those of whole parts, e.g. for the initial temperature of the model.
pub(crate) const NODE_PART_SOURCES: &[Source] = &[
    Source::Selection,
    Source::Parts,
    Source::NodeSet,
    Source::Surface,
];

impl RegionDraft {
    pub(crate) fn new(sources: &'static [Source], target: Target) -> Self {
        Self {
            sources,
            source: sources[0],
            target,
            nodes: History::default(),
            faces: History::default(),
            geometry: History::default(),
            parts: PartPicks::default(),
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
                draft.parts = PartPicks::from_names(parts.iter().cloned());
            }
            Region::Nodes(nodes) => draft.nodes = History::from_items(nodes.iter().copied()),
            Region::Faces(faces) if target != Target::Nodes => {
                draft.faces = History::from_items(faces.iter().copied());
            }
            Region::Faces(_) => draft.nodes = History::from_items(region.nodes(mesh)),
            Region::Geometry(entities) => {
                draft.geometry = History::from_items(entities.iter().copied());
            }
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
            Source::Selection if self.geometry.can_undo() => {
                Region::Geometry(self.geometry.items().into_iter().collect())
            }
            Source::Selection => match self.target {
                Target::Nodes => Region::Nodes(self.nodes.items().into_iter().collect()),
                Target::Faces | Target::Edges => {
                    Region::Faces(self.faces.items().into_iter().collect())
                }
            },
            Source::Parts => Region::Parts(self.parts.names().into_iter().collect()),
            Source::NodeSet => Region::NodeSet(self.set.clone()),
            Source::ElementSet => Region::ElementSet(self.set.clone()),
            Source::Surface => Region::Surface(self.set.clone()),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self.source {
            Source::Selection if self.geometry.can_undo() => self.geometry.items().is_empty(),
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
            Source::Parts => self.parts.click(model, pick.map(|(hit, _)| hit), operation),
            Source::Selection => match pick {
                Some((hit, precision)) => {
                    let items = picker.pick(model, hit, self.target, precision);
                    self.take(&model.mesh, items, operation);
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
        self.geometry.clear();
    }

    /// Adds picked items to the selection. CAD entities stay CAD entities unless nodes or
    /// faces are picked too; then they become the nodes or faces they stand for.
    pub(crate) fn take(&mut self, mesh: &FeMesh, items: Items, operation: Operation) {
        let replace = operation == Operation::Replace;
        match items {
            Items::Geometry(entities) if replace || !self.ids_can_undo() => {
                self.nodes.clear();
                self.faces.clear();
                self.geometry.push(operation, entities);
            }
            Items::Geometry(entities) => {
                let items = Items::Geometry(entities).resolved(mesh, self.target);
                self.take(mesh, items, operation);
            }
            items => {
                if replace {
                    self.geometry.clear();
                } else {
                    self.geometry_to_ids(mesh);
                }
                match items {
                    Items::Nodes(nodes) => self.nodes.push(operation, nodes),
                    Items::Faces(faces) => self.faces.push(operation, faces),
                    Items::Geometry(_) => unreachable!("handled above"),
                }
            }
        }
    }

    /// Turns picked CAD entities into the nodes or faces they stand for.
    fn geometry_to_ids(&mut self, mesh: &FeMesh) {
        if !self.geometry.can_undo() {
            return;
        }
        let entities: Vec<CadEntity> = self.geometry.items().into_iter().collect();
        self.geometry.clear();
        match self.target {
            Target::Nodes => self.nodes = History::from_items(mesh.cad_nodes(&entities)),
            Target::Faces | Target::Edges => {
                self.faces = History::from_items(mesh.cad_faces(&entities));
            }
        }
    }

    fn ids_can_undo(&self) -> bool {
        match self.target {
            Target::Nodes => self.nodes.can_undo(),
            Target::Faces | Target::Edges => self.faces.can_undo(),
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        match self.source {
            Source::Parts => self.parts.can_undo(),
            _ => self.geometry.can_undo() || self.ids_can_undo(),
        }
    }

    /// Whether clicks in the 3D view pick for this region, with the selection window open.
    pub(crate) fn picks(&self) -> bool {
        matches!(self.source, Source::Selection | Source::Parts)
    }

    /// The selection window next to the dialog `anchor` while this region is picked;
    /// regions of parts only pick whole parts.
    pub(crate) fn picker_window(
        &mut self,
        ctx: &egui::Context,
        anchor: egui::Rect,
        picker: &mut Picker,
        model: &Model,
    ) {
        let can_undo = self.can_undo();
        let action = match self.source {
            Source::Selection => picker.window(ctx, anchor, self.target, can_undo),
            Source::Parts => picker.parts_window(ctx, anchor, can_undo),
            _ => None,
        };
        if let Some(action) = action {
            self.action(model, action);
        }
    }

    /// Applies a button of the selection window.
    pub(crate) fn action(&mut self, model: &Model, action: PickerAction) {
        if self.source == Source::Parts {
            return self.parts.action(model, action);
        }
        match action {
            PickerAction::Undo if self.geometry.can_undo() => return self.geometry.undo(),
            PickerAction::Undo | PickerAction::Clear => {}
            // The buttons work on nodes and faces.
            _ => self.geometry_to_ids(&model.mesh),
        }
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

    /// The rows of the only region of a dialog, which clicks in the 3D view always fill.
    pub(crate) fn ui(&mut self, ui: &mut Ui, model: &Model) {
        self.ui_labeled(ui, model, "Region", "region", true);
    }

    /// The region's rows under its own label; `id` keeps its widgets apart from those of
    /// another region in the same dialog. `active` tells whether clicks in the 3D view fill
    /// this region; returns whether the user wants them to, by its "..." button or by
    /// changing where the region comes from.
    pub(crate) fn ui_labeled(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        label: &str,
        id: &str,
        active: bool,
    ) -> bool {
        ui.label(label);
        let wanted = ui.push_id(id, |ui| self.ui_body(ui, model, active)).inner;
        ui.end_row();
        wanted
    }

    fn ui_body(&mut self, ui: &mut Ui, model: &Model, active: bool) -> bool {
        let mut wanted = false;
        ui.vertical(|ui| {
            let source = self.source;
            egui::ComboBox::from_id_salt("region source")
                .selected_text(self.source.label())
                .width(200.0)
                .show_ui(ui, |ui| {
                    for &source in self.sources {
                        ui.selectable_value(&mut self.source, source, source.label());
                    }
                });
            wanted |= self.source != source;
            match self.source {
                Source::Selection => {
                    ui.horizontal(|ui| {
                        wanted |= pick_button(ui, active);
                        let count = self.count();
                        let geometry = self.geometry.items();
                        let what = match self.target {
                            Target::Nodes => "Knoten",
                            Target::Faces => "Elementflächen",
                            Target::Edges => "Elementkanten",
                        };
                        if !geometry.is_empty() {
                            let entities: Vec<CadEntity> = geometry.into_iter().collect();
                            ui.label(plx_model::describe_entities(&entities));
                        } else if count == 0 {
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
                Source::Parts => wanted |= self.parts.ui(ui, active),
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
        wanted
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
        match self.source {
            Source::Selection => {
                let items = picker.pick_box(model, area, self.target);
                self.take(&model.mesh, items, operation);
            }
            Source::Parts => self.parts.box_select(model, area, operation),
            _ => {}
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
        match self.source {
            Source::Selection => {
                crate::selection::preview(model, &picker.pick(model, hit, self.target, precision))
            }
            Source::Parts => PartPicks::preview(model, hit),
            _ => Preview::default(),
        }
    }
}

/// The "..." button of a field filled by clicks in the 3D view, shown pressed while it
/// is the field they fill. Returns whether it was clicked.
pub(crate) fn pick_button(ui: &mut Ui, active: bool) -> bool {
    let hint = if active {
        "Klicks im 3D-Fenster wählen in dieses Feld."
    } else {
        "In dieses Feld im 3D-Fenster wählen"
    };
    let button = egui::Button::new("...").selected(active);
    ui.add(button).on_hover_text(hint).clicked()
}

/// Edges of the outline of the visible parts, the faces of a 2D model.
fn visible_edges(model: &Model) -> BTreeSet<(ElementId, u8)> {
    (model.outline_edges().into_iter())
        .filter(|(part, _, _)| model.parts[*part].visible)
        .map(|(_, face, _)| face)
        .collect()
}

/// Pressure and surface traction act on element faces, in 2D models on their edges.
pub(crate) fn face_target(fe: &FeModel) -> Target {
    if fe.properties.space.is_2d() {
        Target::Edges
    } else {
        Target::Faces
    }
}

/// The components of a force; 2D models have none along z.
fn force_rows(ui: &mut Ui, force: &mut [f64; 3], two_d: bool, units: UnitSystem) {
    let count = if two_d { 2 } else { 3 };
    for (value, label) in force.iter_mut().zip(["F1", "F2", "F3"]).take(count) {
        ui.label(label);
        ui.add(numeric::quantity(value, units, Quantity::Force).speed(1.0));
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
        Region::Geometry(entities) => return crate::cad_selection::highlight(model, entities),
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
    InitialCondition(InitialCondition, RegionDraft),
    FieldOutput(usize, FieldOutput),
    /// The region draft is unused by contact history outputs.
    HistoryOutput(usize, HistoryOutput, RegionDraft),
    Constraint(ConstraintDraft),
    SurfaceInteraction(SurfaceInteraction, contacts::InteractionView),
    ContactPair(ContactPair, MasterSlave),
    NodeTie(NodeTie, RegionDraft),
    Amplitude(Amplitude, AmplitudeView),
}

/// The region clicks in the 3D view pick for, if the dialog has one.
fn draft_region(draft: &Draft) -> Option<&RegionDraft> {
    match draft {
        Draft::Section(_, r)
        | Draft::BoundaryCondition(_, _, r)
        | Draft::Load(_, _, r)
        | Draft::InitialCondition(_, r)
        | Draft::NodeTie(_, r) => Some(r),
        Draft::ContactPair(_, regions) => Some(regions.current()),
        Draft::Constraint(c) => Some(c.region()),
        Draft::HistoryOutput(_, output, r) if output.kind.region().is_some() => Some(r),
        _ => None,
    }
}

fn draft_region_mut(draft: &mut Draft) -> Option<&mut RegionDraft> {
    match draft {
        Draft::Section(_, r)
        | Draft::BoundaryCondition(_, _, r)
        | Draft::Load(_, _, r)
        | Draft::InitialCondition(_, r)
        | Draft::NodeTie(_, r) => Some(r),
        Draft::ContactPair(_, regions) => Some(regions.current_mut()),
        Draft::Constraint(c) => Some(c.region_mut()),
        Draft::HistoryOutput(_, output, r) if output.kind.region().is_some() => Some(r),
        _ => None,
    }
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
const TEMPERATURE: &str = "Temperature";
const FORCE: &str = "Concentrated_Force";
const PRESSURE: &str = "Pressure";
const TRACTION: &str = "Surface_Traction";
const CFLUX: &str = "Concentrated_Flux";
const SURFACE_FLUX: &str = "Surface_Flux";
const BODY_FLUX: &str = "Body_Flux";
const FILM: &str = "Convective_Film";
const RADIATION: &str = "Radiation";

/// The boundary condition kinds of the dialog: label, default name and the kind.
fn boundary_kinds() -> [(&'static str, &'static str, BoundaryKind); 3] {
    [
        ("Fest eingespannt", FIXED, BoundaryKind::Fixed),
        (
            "Verschiebung/Rotation",
            DISPLACEMENT,
            BoundaryKind::Displacement([Some(0.0), None, None, None, None, None]),
        ),
        ("Temperatur", TEMPERATURE, BoundaryKind::Temperature(0.0)),
    ]
}

fn boundary_kind_name(kind: &BoundaryKind) -> &'static str {
    match kind {
        BoundaryKind::Fixed => FIXED,
        BoundaryKind::Displacement(_) => DISPLACEMENT,
        BoundaryKind::Temperature(_) => TEMPERATURE,
    }
}

/// The load kinds of the dialog in PrePoMax's order: label, default name and the kind with
/// zero values.
fn load_kinds() -> [(&'static str, &'static str, LoadKind); 8] {
    [
        ("Einzelkraft", FORCE, LoadKind::ConcentratedForce([0.0; 3])),
        ("Druck", PRESSURE, LoadKind::Pressure(0.0)),
        ("Flächenlast", TRACTION, LoadKind::SurfaceTraction([0.0; 3])),
        (
            "Wärmestrom (Knoten)",
            CFLUX,
            LoadKind::ConcentratedFlux(0.0),
        ),
        (
            "Wärmestromdichte (Fläche)",
            SURFACE_FLUX,
            LoadKind::SurfaceFlux(0.0),
        ),
        ("Wärmequelle (Volumen)", BODY_FLUX, LoadKind::BodyFlux(0.0)),
        (
            "Konvektion (Film)",
            FILM,
            LoadKind::Film {
                sink: 20.0,
                coefficient: 0.0,
            },
        ),
        (
            "Strahlung",
            RADIATION,
            LoadKind::Radiation {
                sink: 20.0,
                emissivity: 0.8,
            },
        ),
    ]
}

fn load_kind_name(kind: &LoadKind) -> &'static str {
    match kind {
        LoadKind::ConcentratedForce(_) => FORCE,
        LoadKind::Pressure(_) => PRESSURE,
        LoadKind::SurfaceTraction(_) => TRACTION,
        LoadKind::ConcentratedFlux(_) => CFLUX,
        LoadKind::SurfaceFlux(_) => SURFACE_FLUX,
        LoadKind::BodyFlux(_) => BODY_FLUX,
        LoadKind::Film { .. } => FILM,
        LoadKind::Radiation { .. } => RADIATION,
    }
}

/// What a load acts on, which decides how its region is picked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadTarget {
    Nodes,
    Faces,
    Elements,
}

fn load_target(kind: &LoadKind) -> LoadTarget {
    match kind {
        LoadKind::ConcentratedForce(_) | LoadKind::ConcentratedFlux(_) => LoadTarget::Nodes,
        LoadKind::BodyFlux(_) => LoadTarget::Elements,
        _ => LoadTarget::Faces,
    }
}

/// An empty region for a load of the kind, or the load's region.
fn load_region(
    kind: &LoadKind,
    region: Option<&Region>,
    fe: &FeModel,
    mesh: &FeMesh,
) -> RegionDraft {
    let (sources, target) = match load_target(kind) {
        LoadTarget::Nodes => (NODE_SOURCES, Target::Nodes),
        LoadTarget::Faces => (FACE_SOURCES, face_target(fe)),
        LoadTarget::Elements => (SOLID_SOURCES, face_target(fe)),
    };
    match region {
        Some(region) => RegionDraft::from_region(region, sources, target, mesh),
        None => RegionDraft::new(sources, target),
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
                ..Material::default()
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
                    kind: SectionKind::Solid,
                },
                RegionDraft::new(ELEMENT_SOURCES, Target::Faces),
            ),
            NewItem::InitialCondition => {
                let existing = names(&fe.initial_conditions, |i| &i.name);
                Draft::InitialCondition(
                    InitialCondition {
                        name: next_name(TEMPERATURE, existing),
                        active: true,
                        region: Region::Nodes(Vec::new()),
                        kind: InitialConditionKind::Temperature(20.0),
                    },
                    RegionDraft::new(NODE_PART_SOURCES, Target::Nodes),
                )
            }
            NewItem::Step => {
                let mut step = Step::new_static(next_name("Step", names(&fe.steps, |s| &s.name)));
                step.kind = StepKind::Static(previous_static(fe));
                Draft::Step(step)
            }
            NewItem::BoundaryCondition(step) => {
                let target = fe.steps.get(step)?;
                let existing = names(&target.boundary_conditions, |b| &b.name);
                // The first kind the step takes: a heat transfer step only temperatures.
                let (_, name, kind) = boundary_kinds()
                    .into_iter()
                    .find(|(_, _, kind)| target.kind.supports_boundary(kind))?;
                Draft::BoundaryCondition(
                    step,
                    BoundaryCondition {
                        name: next_name(name, existing),
                        active: true,
                        region: Region::Nodes(Vec::new()),
                        kind,
                        amplitude: None,
                    },
                    RegionDraft::new(NODE_SOURCES, Target::Nodes),
                )
            }
            NewItem::Load(step) => {
                // A frequency step takes no loads, as in PrePoMax.
                let target = fe.steps.get(step).filter(|s| s.kind.supports_loads())?;
                let existing = names(&target.loads, |l| &l.name);
                let (_, name, kind) = load_kinds()
                    .into_iter()
                    .find(|(_, _, kind)| target.kind.supports_load(kind))?;
                Draft::Load(
                    step,
                    Load {
                        name: next_name(name, existing),
                        active: true,
                        region: Region::Nodes(Vec::new()),
                        kind,
                        amplitude: None,
                        factor_amplitude: None,
                    },
                    load_region(&kind, None, fe, &FeMesh::default()),
                )
            }
            NewItem::HistoryOutput(step) => {
                let target = fe.steps.get(step)?;
                let output = HistoryOutput::node(
                    next_name("NH_Output", names(&target.history_outputs, |h| &h.name)),
                    Region::Nodes(Vec::new()),
                );
                let region = history_region(&output.kind, fe, &FeMesh::default());
                Draft::HistoryOutput(step, output, region)
            }
            NewItem::Constraint => Draft::Constraint(ConstraintDraft::new(fe)),
            NewItem::SurfaceInteraction => {
                let existing = names(&fe.surface_interactions, |s| &s.name);
                let interaction = SurfaceInteraction {
                    name: next_name("Surface_Interaction", existing),
                    properties: Vec::new(),
                };
                let view = contacts::InteractionView::new(&interaction);
                Draft::SurfaceInteraction(interaction, view)
            }
            NewItem::ContactPair => {
                let existing = names(&fe.contact_pairs, |c| &c.name);
                let interaction = (fe.surface_interactions.first())
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                Draft::ContactPair(
                    ContactPair::new(next_name("Contact_Pair", existing), interaction),
                    MasterSlave::new(face_target(fe)),
                )
            }
            NewItem::NodeTie => {
                let name = next_name("Node_Tie", names(&fe.node_ties, |t| &t.name));
                Draft::NodeTie(
                    NodeTie::new(name),
                    RegionDraft::new(NODE_SOURCES, Target::Nodes),
                )
            }
            NewItem::Amplitude => {
                let name = next_name("Amplitude", names(&fe.amplitudes, |a| &a.name));
                Draft::Amplitude(Amplitude::new(name), AmplitudeView::default())
            }
            NewItem::ResultFieldOutput
            | NewItem::ResultHistoryOutput
            | NewItem::ResultHotSpot
            | NewItem::MeshSetupItem
            | NewItem::Feature(_) => {
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
                let region = load_region(&load.kind, Some(&load.region), fe, mesh);
                (Draft::Load(s, load, region), i)
            }
            TreeItem::InitialCondition(i) => {
                let condition = fe.initial_conditions.get(i)?.clone();
                let region = RegionDraft::from_region(
                    &condition.region,
                    NODE_PART_SOURCES,
                    Target::Nodes,
                    mesh,
                );
                (Draft::InitialCondition(condition, region), i)
            }
            TreeItem::FieldOutput(s, i) => {
                let output = fe.steps.get(s)?.field_outputs.get(i)?.clone();
                (Draft::FieldOutput(s, output), i)
            }
            TreeItem::HistoryOutput(s, i) => {
                let output = fe.steps.get(s)?.history_outputs.get(i)?.clone();
                let region = history_region(&output.kind, fe, mesh);
                (Draft::HistoryOutput(s, output, region), i)
            }
            TreeItem::Constraint(i) => (
                Draft::Constraint(ConstraintDraft::edit(
                    fe.constraints.get(i)?,
                    face_target(fe),
                    mesh,
                )),
                i,
            ),
            TreeItem::SurfaceInteraction(i) => {
                let interaction = fe.surface_interactions.get(i)?.clone();
                let view = contacts::InteractionView::new(&interaction);
                (Draft::SurfaceInteraction(interaction, view), i)
            }
            TreeItem::ContactPair(i) => {
                let pair = fe.contact_pairs.get(i)?.clone();
                let regions =
                    MasterSlave::from_regions(&pair.master, &pair.slave, face_target(fe), mesh);
                (Draft::ContactPair(pair, regions), i)
            }
            TreeItem::NodeTie(i) => {
                let tie = fe.node_ties.get(i)?.clone();
                let region =
                    RegionDraft::from_region(&tie.region, NODE_SOURCES, Target::Nodes, mesh);
                (Draft::NodeTie(tie, region), i)
            }
            TreeItem::Amplitude(i) => {
                let amplitude = fe.amplitudes.get(i)?.clone();
                (Draft::Amplitude(amplitude, AmplitudeView::default()), i)
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
            Draft::InitialCondition(c, _) => ("Anfangsbedingung", &c.name),
            Draft::FieldOutput(_, f) => ("Field Output", &f.name),
            Draft::HistoryOutput(_, h, _) => ("History Output", &h.name),
            Draft::Constraint(c) => ("Constraint", c.name()),
            Draft::SurfaceInteraction(s, _) => ("Surface Interaction", &s.name),
            Draft::ContactPair(c, _) => ("Contact Pair", &c.name),
            Draft::NodeTie(t, _) => ("Node Tie", &t.name),
            Draft::Amplitude(a, _) => ("Amplitude", &a.name),
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
        self.region().is_some_and(RegionDraft::picks)
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
        if let Some(r) = draft_region_mut(&mut self.draft) {
            r.box_select(model, &self.picker, area, operation);
        }
    }

    /// What a click at the hit would select, for the hover preview.
    pub fn preview(&self, model: &Model, hit: &Hit, precision: f32) -> Preview {
        self.region()
            .map(|r| r.preview(model, &self.picker, hit, precision))
            .unwrap_or_default()
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
                if let Draft::SurfaceInteraction(interaction, view) = &mut self.draft {
                    contacts::interaction_dialog(ui, interaction, view, model.fe.properties.units);
                } else if let Draft::Amplitude(amplitude, view) = &mut self.draft {
                    let units = model.fe.properties.units;
                    amplitude_dialog::amplitude_form(ui, amplitude, view, units);
                } else {
                    egui::Grid::new("item form")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| self.form(ui, model));
                }
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
            && let Some(region) = draft_region_mut(&mut self.draft)
        {
            region.picker_window(ctx, window.response.rect, &mut self.picker, model);
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
        let units = model.fe.properties.units;
        match &mut self.draft {
            Draft::Material(material) => material_form(ui, material, units),
            Draft::Section(section, region) => {
                name_row(ui, &mut section.name);
                ui.label("Art");
                let before = section.kind.prefix();
                egui::ComboBox::from_id_salt("section kind")
                    .selected_text(section.kind.label())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for kind in SectionKind::ALL {
                            let selected = kind.prefix() == section.kind.prefix();
                            if ui.selectable_label(selected, kind.label()).clicked() && !selected {
                                section.kind = kind;
                            }
                        }
                    });
                ui.end_row();
                // Another kind keeps a name the user chose and renumbers a default one.
                if section.kind.prefix() != before
                    && (section.name.strip_prefix(before))
                        .and_then(|n| n.strip_prefix('-'))
                        .is_some_and(|n| n.parse::<u32>().is_ok())
                {
                    section.name = next_name(section.kind.prefix(), taken.iter().copied());
                }
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
                match &mut section.kind {
                    SectionKind::Solid => {
                        // Plane stress and plane strain sections have a thickness, as in
                        // PrePoMax.
                        if model.fe.properties.space.has_thickness() {
                            ui.label("Dicke");
                            ui.add(
                                numeric::quantity(&mut section.thickness, units, Quantity::Length)
                                    .range(0.0..=f64::MAX),
                            );
                            ui.end_row();
                        }
                    }
                    SectionKind::Truss { area } => {
                        ui.label("Querschnittsfläche");
                        ui.add(
                            numeric::quantity(area, units, Quantity::Area).range(0.0..=f64::MAX),
                        );
                        ui.end_row();
                    }
                    SectionKind::Beam(beam) => beam_form(ui, beam, units),
                }
                region.ui(ui, model);
            }
            Draft::Step(step) => step_form(ui, step, self.index.is_none(), &model.fe),
            Draft::BoundaryCondition(step, bc, region) => {
                name_row(ui, &mut bc.name);
                ui.label("Art");
                let step_kind = model.fe.steps.get(*step).map(|s| &s.kind);
                ui.horizontal(|ui| {
                    let current = boundary_kind_name(&bc.kind);
                    for (label, name, kind) in boundary_kinds() {
                        // Only the kinds the step takes, as PrePoMax lists them.
                        if step_kind.is_some_and(|s| !s.supports_boundary(&kind)) {
                            continue;
                        }
                        if ui.radio(current == name, label).clicked() && current != name {
                            bc.kind = kind;
                            rename_default(&mut bc.name, current, name, &taken);
                        }
                    }
                });
                ui.end_row();
                if let BoundaryKind::Temperature(t) = &mut bc.kind {
                    ui.label("Temperatur");
                    ui.add(numeric::quantity(t, units, Quantity::Temperature).speed(1.0));
                    ui.end_row();
                }
                if let BoundaryKind::Displacement(values) = &mut bc.kind {
                    // Nodes of 2D models only move in the x-y plane.
                    let dofs = if two_d { 2 } else { 6 };
                    for (i, (value, label)) in values
                        .iter_mut()
                        .zip(["U1", "U2", "U3", "UR1", "UR2", "UR3"])
                        .take(dofs)
                        .enumerate()
                    {
                        let mut set = value.is_some();
                        ui.checkbox(&mut set, label);
                        let mut number = value.unwrap_or(0.0);
                        // Displacements are lengths, rotations angles in radian.
                        let quantity = if i < 3 {
                            Quantity::Length
                        } else {
                            Quantity::Angle
                        };
                        let field = numeric::quantity(&mut number, units, quantity).speed(0.01);
                        ui.add_enabled(set, field);
                        *value = set.then_some(number);
                        ui.end_row();
                    }
                }
                if bc.kind.takes_amplitude() {
                    amplitude_row(
                        ui,
                        "Amplitude",
                        "bc amplitude",
                        &mut bc.amplitude,
                        &model.fe,
                    );
                }
                region.ui(ui, model);
            }
            Draft::Load(step, load, region) => {
                name_row(ui, &mut load.name);
                ui.label("Art");
                let step_kind = model.fe.steps.get(*step).map(|s| &s.kind);
                let current = load_kind_name(&load.kind);
                let label = (load_kinds().into_iter())
                    .find(|(_, name, _)| *name == current)
                    .map_or("", |(label, ..)| label);
                egui::ComboBox::from_id_salt("load kind")
                    .selected_text(label)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for (label, name, kind) in load_kinds() {
                            // Only the kinds the step takes, as PrePoMax lists them.
                            if step_kind.is_some_and(|s| !s.supports_load(&kind)) {
                                continue;
                            }
                            if ui.selectable_label(current == name, label).clicked()
                                && current != name
                            {
                                let moved = load_target(&kind) != load_target(&load.kind);
                                load.kind = kind;
                                rename_default(&mut load.name, current, name, &taken);
                                if moved {
                                    *region = load_region(&kind, None, &model.fe, &model.mesh);
                                }
                            }
                        }
                    });
                ui.end_row();
                match &mut load.kind {
                    LoadKind::ConcentratedForce(force) => {
                        force_rows(ui, force, two_d, units);
                        ui.label("");
                        ui.weak("Die Kraft wirkt an jedem Knoten der Region.");
                        ui.end_row();
                        revolution_hint(ui, axisymmetric);
                    }
                    LoadKind::Pressure(pressure) => {
                        ui.label("Druck");
                        ui.add(numeric::quantity(pressure, units, Quantity::Pressure).speed(0.1));
                        ui.end_row();
                    }
                    LoadKind::SurfaceTraction(force) => {
                        force_rows(ui, force, two_d, units);
                        ui.label("");
                        ui.weak(
                            "Gesamtkraft, beim Export flächengewichtet auf die Knoten verteilt.",
                        );
                        ui.end_row();
                        revolution_hint(ui, axisymmetric);
                    }
                    LoadKind::ConcentratedFlux(flux) => {
                        ui.label("Wärmestrom");
                        ui.add(numeric::physical(flux, units, Quantity::Power));
                        ui.end_row();
                        ui.label("");
                        ui.weak("Der Wärmestrom fließt an jedem Knoten der Region zu.");
                        ui.end_row();
                    }
                    LoadKind::SurfaceFlux(flux) => {
                        ui.label("Wärmestromdichte");
                        ui.add(numeric::physical(flux, units, Quantity::HeatFlux));
                        ui.end_row();
                        ui.label("");
                        ui.weak("Positiv: Wärme fließt in das Bauteil.");
                        ui.end_row();
                    }
                    LoadKind::BodyFlux(flux) => {
                        ui.label("Wärmequelle");
                        ui.add(numeric::physical(flux, units, Quantity::PowerPerVolume));
                        ui.end_row();
                    }
                    LoadKind::Film { sink, coefficient } => {
                        ui.label("Umgebungstemperatur");
                        ui.add(numeric::quantity(sink, units, Quantity::Temperature).speed(1.0));
                        ui.end_row();
                        ui.label("Wärmeübergangskoeffizient");
                        ui.add(
                            numeric::physical(
                                coefficient,
                                units,
                                Quantity::HeatTransferCoefficient,
                            )
                            .range(0.0..=f64::MAX),
                        );
                        ui.end_row();
                    }
                    LoadKind::Radiation { sink, emissivity } => {
                        ui.label("Umgebungstemperatur");
                        ui.add(numeric::quantity(sink, units, Quantity::Temperature).speed(1.0));
                        ui.end_row();
                        ui.label("Emissionsgrad");
                        ui.add(
                            numeric::drag_value(emissivity)
                                .range(0.0..=1.0)
                                .speed(0.01)
                                .max_decimals(4),
                        );
                        ui.end_row();
                        let properties = &model.fe.properties;
                        if properties.absolute_zero.is_none()
                            || properties.stefan_boltzmann.is_none()
                        {
                            ui.label("");
                            ui.colored_label(
                                egui::Color32::from_rgb(200, 0, 0),
                                "Strahlung braucht die physikalischen Konstanten\n\
                                 (Modelleigenschaften).",
                            );
                            ui.end_row();
                        }
                    }
                }
                // Of a film or radiation the first amplitude scales the sink temperature.
                let factor = load.kind.factor_amplitude_label();
                let label = if factor.is_some() {
                    "Amplitude Umgebungstemperatur"
                } else {
                    "Amplitude"
                };
                amplitude_row(ui, label, "load amplitude", &mut load.amplitude, &model.fe);
                if let Some(factor) = factor {
                    let label = format!("Amplitude {factor}");
                    let reference = &mut load.factor_amplitude;
                    amplitude_row(ui, &label, "load factor amplitude", reference, &model.fe);
                }
                region.ui(ui, model);
            }
            Draft::InitialCondition(condition, region) => {
                name_row(ui, &mut condition.name);
                match &mut condition.kind {
                    InitialConditionKind::Temperature(t) => {
                        ui.label("Temperatur");
                        ui.add(numeric::quantity(t, units, Quantity::Temperature).speed(1.0));
                        ui.end_row();
                    }
                }
                region.ui(ui, model);
                ui.label("");
                ui.weak("Temperatur vor dem ersten Step, z. B. für Wärmedehnung.");
                ui.end_row();
            }
            Draft::NodeTie(tie, region) => {
                name_row(ui, &mut tie.name);
                region.ui(ui, model);
                ui.label("Rotationen");
                ui.checkbox(&mut tie.rotations, "biegesteif");
                ui.end_row();
                ui.label("");
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Alle Knoten der Region folgen dem ersten. Verbindet die Enden \
                             von Balken oder Stäben verschiedener Parts; die Kontaktsuche \
                             findet sie. Biegesteif koppelt auch die Rotationen, sonst ist \
                             die Verbindung ein Gelenk; Stäbe haben keine Rotationen.",
                        )
                        .weak(),
                    )
                    .wrap(),
                );
                ui.end_row();
            }
            Draft::FieldOutput(_, output) => {
                name_row(ui, &mut output.name);
                let choices: &[&str] = match output.kind {
                    OutputKind::Node => &["RF", "U", "NT", "RFL"],
                    OutputKind::Element => &["S", "E", "ME", "PEEQ", "ENER", "HFL"],
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
            Draft::HistoryOutput(_, output, region) => {
                history_output_form(ui, model, output, region, &taken);
            }
            Draft::Constraint(c) => c.form(ui, model, &taken, self.index.is_none()),
            // Laid out by their own dialogs, see show.
            Draft::SurfaceInteraction(..) | Draft::Amplitude(..) => {}
            Draft::ContactPair(pair, regions) => {
                name_row(ui, &mut pair.name);
                contacts::contact_pair_form(ui, model, pair, regions);
            }
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
            Draft::InitialCondition(..) => names(&fe.initial_conditions, |i| &i.name),
            Draft::FieldOutput(step, _) => names(&fe.steps[*step].field_outputs, |f| &f.name),
            Draft::HistoryOutput(step, ..) => names(&fe.steps[*step].history_outputs, |h| &h.name),
            Draft::Constraint(_) => fe.constraints.iter().map(Constraint::name).collect(),
            Draft::SurfaceInteraction(..) => names(&fe.surface_interactions, |s| &s.name),
            Draft::ContactPair(..) => names(&fe.contact_pairs, |c| &c.name),
            Draft::NodeTie(..) => names(&fe.node_ties, |t| &t.name),
            Draft::Amplitude(..) => names(&fe.amplitudes, |a| &a.name),
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
            Draft::InitialCondition(c, _) => &c.name,
            Draft::FieldOutput(_, f) => &f.name,
            Draft::HistoryOutput(_, h, _) => &h.name,
            Draft::Constraint(c) => c.name(),
            Draft::SurfaceInteraction(s, _) => &s.name,
            Draft::ContactPair(c, _) => &c.name,
            Draft::NodeTie(t, _) => &t.name,
            Draft::Amplitude(a, _) => &a.name,
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
        if let Draft::Section(section, _) = &self.draft {
            if !fe.materials.iter().any(|m| m.name == section.material) {
                return Err("Bitte ein Material wählen; zuerst unter Materials anlegen.".into());
            }
            match &section.kind {
                SectionKind::Solid => {}
                SectionKind::Truss { area } if !(area.is_finite() && *area > 0.0) => {
                    return Err("Die Querschnittsfläche muss größer als 0 sein.".into());
                }
                SectionKind::Truss { .. } => {}
                SectionKind::Beam(beam) => {
                    if !beam.profile.is_valid() {
                        return Err(
                            "Die Profilmaße müssen größer als 0 sein; Wände dünner als das \
                             Profil."
                                .into(),
                        );
                    }
                    if let BeamOrientation::Direction(n) = beam.orientation
                        && n.iter().all(|v| *v == 0.0)
                    {
                        return Err("Die Normale darf nicht der Nullvektor sein.".into());
                    }
                }
            }
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
            Draft::Amplitude(amplitude, _) => amplitude_dialog::validate(amplitude)?,
            Draft::HistoryOutput(_, output, region) => {
                validate_history_output(output, region, fe)?;
            }
            _ => {
                if self.region().is_some_and(RegionDraft::is_empty) {
                    return Err("Die Region ist leer.".into());
                }
            }
        }
        if let Draft::Step(step) = &self.draft {
            match &step.kind {
                StepKind::Frequency(settings) => validate_frequency_step(settings)?,
                StepKind::HeatTransfer(settings) | StepKind::CoupledTempDisp(settings) => {
                    validate_heat_transfer_step(settings)?
                }
                StepKind::Static(_) => {}
            }
        }
        Ok(())
    }

    /// Copies the draft into the model.
    /// The tree item a new constraint, surface interaction or contact pair gets once
    /// applied to `fe`, to show it in the tree.
    pub fn new_interaction_item(&self, fe: &FeModel) -> Option<TreeItem> {
        if self.index.is_some() {
            return None;
        }
        match self.draft {
            Draft::Constraint(_) => Some(TreeItem::Constraint(fe.constraints.len())),
            Draft::SurfaceInteraction(..) => {
                Some(TreeItem::SurfaceInteraction(fe.surface_interactions.len()))
            }
            Draft::ContactPair(..) => Some(TreeItem::ContactPair(fe.contact_pairs.len())),
            Draft::NodeTie(..) => Some(TreeItem::NodeTie(fe.node_ties.len())),
            Draft::Amplitude(..) => Some(TreeItem::Amplitude(fe.amplitudes.len())),
            _ => None,
        }
    }

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
                if !bc.kind.takes_amplitude() {
                    bc.amplitude = None;
                }
                let list = &mut fe.steps[s].boundary_conditions;
                if let Some(existing) = index.and_then(|i| list.get(i)) {
                    bc.active = existing.active;
                }
                put(list, index, bc);
            }
            Draft::Load(s, mut load, region) => {
                load.region = region.region();
                if load.kind.factor_amplitude_label().is_none() {
                    load.factor_amplitude = None;
                }
                let list = &mut fe.steps[s].loads;
                if let Some(existing) = index.and_then(|i| list.get(i)) {
                    load.active = existing.active;
                }
                put(list, index, load);
            }
            Draft::InitialCondition(mut condition, region) => {
                condition.region = region.region();
                if let Some(existing) = index.and_then(|i| fe.initial_conditions.get(i)) {
                    condition.active = existing.active;
                }
                put(&mut fe.initial_conditions, index, condition);
            }
            Draft::FieldOutput(s, output) => put(&mut fe.steps[s].field_outputs, index, output),
            Draft::NodeTie(mut tie, region) => {
                tie.region = region.region();
                if let Some(existing) = index.and_then(|i| fe.node_ties.get(i)) {
                    tie.active = existing.active;
                }
                put(&mut fe.node_ties, index, tie);
            }
            Draft::HistoryOutput(s, mut output, region) => {
                if let Some(target) = output.kind.region_mut() {
                    *target = region.region();
                }
                output.normalize_variables();
                let list = &mut fe.steps[s].history_outputs;
                if let Some(existing) = index.and_then(|i| list.get(i)) {
                    output.active = existing.active;
                }
                put(list, index, output);
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
                    // Contact history outputs follow a renamed pair.
                    let old = existing.name.clone();
                    fe.rename_contact_pair(&old, &pair.name);
                }
                put(&mut fe.contact_pairs, index, pair);
            }
            Draft::Amplitude(amplitude, _) => {
                // Boundary conditions and loads follow a renamed amplitude.
                if let Some(old) = index.and_then(|i| fe.amplitudes.get(i)) {
                    let old = old.name.clone();
                    fe.rename_amplitude(&old, &amplitude.name);
                }
                put(&mut fe.amplitudes, index, amplitude);
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
        TreeItem::HistoryOutput(s, i) => fe
            .steps
            .get_mut(s)
            .is_some_and(|st| remove(&mut st.history_outputs, i)),
        TreeItem::InitialCondition(i) => remove(&mut fe.initial_conditions, i),
        TreeItem::Constraint(i) => remove(&mut fe.constraints, i),
        TreeItem::SurfaceInteraction(i) => remove(&mut fe.surface_interactions, i),
        TreeItem::ContactPair(i) => remove(&mut fe.contact_pairs, i),
        TreeItem::NodeTie(i) => remove(&mut fe.node_ties, i),
        TreeItem::Amplitude(i) => remove(&mut fe.amplitudes, i),
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
        TreeItem::HistoryOutput(s, i) => (fe.steps.get_mut(s))
            .and_then(|st| st.history_outputs.get_mut(i))
            .map(|h| &mut h.active),
        TreeItem::Constraint(i) => fe.constraints.get_mut(i).map(Constraint::active_mut),
        TreeItem::ContactPair(i) => fe.contact_pairs.get_mut(i).map(|c| &mut c.active),
        TreeItem::NodeTie(i) => fe.node_ties.get_mut(i).map(|t| &mut t.active),
        TreeItem::InitialCondition(i) => fe.initial_conditions.get_mut(i).map(|c| &mut c.active),
        _ => None,
    };
    active.map(|a| *a = !*a).is_some()
}

/// Swaps master and slave of a tie, spring connection or contact pair; a swapped name
/// `<slave>_to_<master>` that is taken gets the next free number, as in PrePoMax.
pub fn swap_master_slave(fe: &mut FeModel, item: &TreeItem) -> bool {
    match *item {
        TreeItem::Constraint(i) => {
            let Some(constraint) = fe.constraints.get_mut(i) else {
                return false;
            };
            let old = constraint.name().to_owned();
            if !constraint.swap_master_slave() {
                return false;
            }
            let name = constraint.name().to_owned();
            let others = (fe.constraints.iter().enumerate())
                .filter(|&(j, _)| j != i)
                .map(|(_, c)| c.name());
            if name != old && others.clone().any(|n| n.eq_ignore_ascii_case(&name)) {
                let free = next_name(&name, others);
                *fe.constraints[i].name_mut() = free;
            }
            true
        }
        TreeItem::ContactPair(i) => {
            let Some(pair) = fe.contact_pairs.get_mut(i) else {
                return false;
            };
            let old = pair.name.clone();
            pair.swap_master_slave();
            let name = pair.name.clone();
            let others = (fe.contact_pairs.iter().enumerate())
                .filter(|&(j, _)| j != i)
                .map(|(_, c)| c.name.as_str());
            if name != old && others.clone().any(|n| n.eq_ignore_ascii_case(&name)) {
                let free = next_name(&name, others);
                fe.contact_pairs[i].name = free;
            }
            let new = fe.contact_pairs[i].name.clone();
            fe.rename_contact_pair(&old, &new);
            true
        }
        _ => false,
    }
}

/// How an item selected in the tree shows in the 3D view: its region, or master and slave.
pub fn item_highlight(model: &Model, item: &TreeItem) -> Highlight {
    let fe = &model.fe;
    let master_slave = match *item {
        TreeItem::Constraint(i) => fe.constraints.get(i).and_then(Constraint::master_slave),
        TreeItem::ContactPair(i) => fe.contact_pairs.get(i).map(|c| [&c.master, &c.slave]),
        TreeItem::HistoryOutput(s, i) => {
            let output = fe.steps.get(s).and_then(|st| st.history_outputs.get(i));
            match output.map(|h| &h.kind) {
                Some(HistoryKind::Contact { pair }) => (fe.contact_pairs.iter())
                    .find(|c| c.name == *pair)
                    .map(|c| [&c.master, &c.slave]),
                _ => None,
            }
        }
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
        TreeItem::HistoryOutput(s, i) => fe.steps.get(s)?.history_outputs.get(i)?.kind.region(),
        TreeItem::InitialCondition(i) => fe.initial_conditions.get(i).map(|c| &c.region),
        TreeItem::Constraint(i) => fe.constraints.get(i)?.regions().first().copied(),
        TreeItem::NodeTie(i) => fe.node_ties.get(i).map(|t| &t.region),
        _ => None,
    }
}

/// The region draft of a history output: nodes for nodal values, elements for element
/// values.
fn history_region(kind: &HistoryKind, fe: &FeModel, mesh: &FeMesh) -> RegionDraft {
    let (sources, target) = match kind {
        HistoryKind::Element { .. } => (SOLID_SOURCES, face_target(fe)),
        _ => (NODE_SOURCES, Target::Nodes),
    };
    match kind.region() {
        Some(region) => RegionDraft::from_region(region, sources, target, mesh),
        None => RegionDraft::new(sources, target),
    }
}

/// PrePoMax's history output dialog: the kind, the variables, the totals and the region or
/// contact pair.
fn history_output_form(
    ui: &mut Ui,
    model: &Model,
    output: &mut HistoryOutput,
    region: &mut RegionDraft,
    taken: &[&str],
) {
    name_row(ui, &mut output.name);
    ui.label("Art");
    ui.horizontal(|ui| {
        let current = output.kind.prefix();
        let pair = (model.fe.contact_pairs.first()).map_or(String::new(), |c| c.name.clone());
        let kinds = [
            HistoryKind::Node {
                region: Region::Nodes(Vec::new()),
            },
            HistoryKind::Element {
                region: Region::Parts(Vec::new()),
            },
            HistoryKind::Contact { pair },
        ];
        for kind in kinds {
            let selected = kind.prefix() == current;
            if ui.radio(selected, kind.label()).clicked() && !selected {
                *region = history_region(&kind, &model.fe, &model.mesh);
                let defaults = match &kind {
                    HistoryKind::Node { region } => HistoryOutput::node("", region.clone()),
                    HistoryKind::Element { region } => HistoryOutput::element("", region.clone()),
                    HistoryKind::Contact { pair } => HistoryOutput::contact("", pair.clone()),
                };
                output.variables = defaults.variables;
                rename_default(&mut output.name, current, kind.prefix(), taken);
                output.kind = kind;
            }
        }
    });
    ui.end_row();
    ui.label("Variablen");
    let choices = output.kind.choices();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(320.0);
        for &variable in choices {
            let mut on = output.variables.iter().any(|v| v == variable);
            if ui.checkbox(&mut on, variable).changed() {
                output.variables.retain(|v| v != variable);
                if on {
                    output.variables.push(variable.to_string());
                }
                output.normalize_variables();
            }
        }
    });
    ui.end_row();
    ui.label("Summen");
    egui::ComboBox::from_id_salt("history totals")
        .selected_text(output.totals.label())
        .width(200.0)
        .show_ui(ui, |ui| {
            for totals in plx_model::Totals::ALL {
                ui.selectable_value(&mut output.totals, totals, totals.label());
            }
        });
    ui.end_row();
    match &mut output.kind {
        HistoryKind::Contact { pair } => {
            ui.label("Contact Pair");
            egui::ComboBox::from_id_salt("history contact pair")
                .selected_text(pair.as_str())
                .width(200.0)
                .show_ui(ui, |ui| {
                    for contact in &model.fe.contact_pairs {
                        let name = contact.name.clone();
                        ui.selectable_value(pair, name, &contact.name);
                    }
                });
            ui.end_row();
            ui.label("");
            ui.weak(
                "CalculiX gibt die Werte aller Kontaktelemente aus;
                 das Paar bestimmt die Flächen der Kontaktkräfte CF.",
            );
            ui.end_row();
        }
        _ => region.ui(ui, model),
    }
}

fn validate_history_output(
    output: &HistoryOutput,
    region: &RegionDraft,
    fe: &FeModel,
) -> Result<(), String> {
    if output.variables.is_empty() {
        return Err("Bitte mindestens eine Variable wählen.".into());
    }
    match &output.kind {
        HistoryKind::Contact { pair } => {
            if !fe.contact_pairs.iter().any(|c| c.name == *pair) {
                return Err(
                    "Bitte ein Contact Pair wählen; zuerst unter Contact Pairs anlegen.".into(),
                );
            }
        }
        _ => {
            if region.is_empty() {
                return Err("Die Region ist leer.".into());
            }
        }
    }
    Ok(())
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

/// Profile, normal and offsets of a beam section, with CalculiX's directions: the
/// 1-direction is the normal, the 2-direction the beam axis crossed with it.
fn beam_form(ui: &mut Ui, beam: &mut BeamSection, units: UnitSystem) {
    ui.label("Profil");
    egui::ComboBox::from_id_salt("beam profile")
        .selected_text(beam.profile.label())
        .width(200.0)
        .show_ui(ui, |ui| {
            for profile in BeamProfile::ALL {
                let selected = profile.keyword() == beam.profile.keyword();
                if ui.selectable_label(selected, profile.label()).clicked() && !selected {
                    beam.profile = profile;
                }
            }
        });
    ui.end_row();
    let positive = |ui: &mut Ui, label: &str, value: &mut f64| {
        ui.label(label);
        ui.add(numeric::quantity(value, units, Quantity::Length).range(0.0..=f64::MAX));
        ui.end_row();
    };
    match &mut beam.profile {
        BeamProfile::Rect { a, b } => {
            positive(ui, "Dicke in 1-Richtung (a)", a);
            positive(ui, "Dicke in 2-Richtung (b)", b);
        }
        BeamProfile::Circ { radius } => positive(ui, "Radius", radius),
        BeamProfile::Pipe { radius, thickness } => {
            positive(ui, "Außenradius", radius);
            positive(ui, "Wanddicke", thickness);
        }
        BeamProfile::Box { a, b, t } => {
            positive(ui, "Breite in 1-Richtung (a)", a);
            positive(ui, "Breite in 2-Richtung (b)", b);
            let [t1, t2, t3, t4] = t;
            positive(ui, "Wanddicke bei +1", t1);
            positive(ui, "Wanddicke bei +2", t2);
            positive(ui, "Wanddicke bei -1", t3);
            positive(ui, "Wanddicke bei -2", t4);
        }
    }
    if beam.profile.needs_reduced_integration() {
        ui.label("");
        ui.label(
            egui::RichText::new("Rohr und Kasten brauchen Linien mit 3 Knoten (B32R).")
                .small()
                .weak(),
        );
        ui.end_row();
    }
    ui.label("Normale (1-Richtung)");
    ui.horizontal(|ui| {
        let automatic = matches!(beam.orientation, BeamOrientation::Automatic);
        if ui.radio(automatic, "Automatisch").clicked() && !automatic {
            beam.orientation = BeamOrientation::Automatic;
        }
        if ui.radio(!automatic, "Vektor").clicked() && automatic {
            beam.orientation = BeamOrientation::Direction([0.0, 0.0, 1.0]);
        }
    });
    ui.end_row();
    match &mut beam.orientation {
        BeamOrientation::Automatic => {
            ui.label("");
            ui.label(
                egui::RichText::new("Globale z-Achse, bei Balken entlang z die x-Achse.")
                    .small()
                    .weak(),
            );
            ui.end_row();
        }
        BeamOrientation::Direction(normal) => {
            ui.label("");
            ui.horizontal(|ui| {
                for (axis, value) in ["x", "y", "z"].iter().zip(normal.iter_mut()) {
                    ui.label(*axis);
                    ui.add(numeric::drag_value(value));
                }
            });
            ui.end_row();
        }
    }
    let [offset_1, offset_2] = &mut beam.offset;
    ui.label("Versatz in 1-Richtung");
    ui.add(numeric::drag_value(offset_1));
    ui.end_row();
    ui.label("Versatz in 2-Richtung");
    ui.add(numeric::drag_value(offset_2));
    ui.end_row();
}

fn material_form(ui: &mut Ui, material: &mut Material, units: UnitSystem) {
    name_row(ui, &mut material.name);
    let mut has_density = material.density.is_some();
    ui.checkbox(&mut has_density, "Dichte");
    let mut density = material.density.unwrap_or(0.0);
    ui.add_enabled(
        has_density,
        numeric::physical(&mut density, units, Quantity::Density),
    );
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
    ui.add_enabled(
        elastic,
        numeric::physical(&mut values.young, units, Quantity::Pressure),
    );
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
    plastic_rows(ui, material, units);
    for (label, value, quantity) in [
        (
            "Wärmeleitfähigkeit",
            &mut material.conductivity,
            Quantity::ThermalConductivity,
        ),
        (
            "Spez. Wärmekapazität",
            &mut material.specific_heat,
            Quantity::SpecificHeat,
        ),
    ] {
        let mut set = value.is_some();
        ui.checkbox(&mut set, label);
        let mut number = value.unwrap_or(0.0);
        ui.add_enabled(set, numeric::physical(&mut number, units, quantity));
        *value = set.then_some(number);
        ui.end_row();
    }
    let mut expands = material.expansion.is_some();
    ui.checkbox(&mut expands, "Wärmeausdehnung");
    ui.end_row();
    let mut expansion = material.expansion.unwrap_or_default();
    ui.label("    Ausdehnungskoeffizient");
    ui.add_enabled(
        expands,
        numeric::physical(
            &mut expansion.coefficient,
            units,
            Quantity::ThermalExpansion,
        ),
    );
    ui.end_row();
    ui.label("    Referenztemperatur");
    ui.add_enabled(
        expands,
        numeric::quantity(
            &mut expansion.zero_temperature,
            units,
            Quantity::Temperature,
        ),
    );
    ui.end_row();
    material.expansion = expands.then_some(expansion);
}

/// The plasticity of the material form: the hardening rule and the hardening curve as an
/// editable table like PrePoMax's `Plastic` property, rows of yield stress, plastic strain
/// and temperature.
fn plastic_rows(ui: &mut Ui, material: &mut Material, units: UnitSystem) {
    let mut plastic = material.plastic.is_some();
    ui.checkbox(&mut plastic, "Plasticity");
    ui.end_row();
    let mut values = material.plastic.clone().unwrap_or_default();
    ui.label("    Hardening");
    ui.add_enabled_ui(plastic, |ui| {
        egui::ComboBox::from_id_salt("plastic hardening")
            .selected_text(hardening_label(values.hardening))
            .width(200.0)
            .show_ui(ui, |ui| {
                for hardening in Hardening::ALL {
                    ui.selectable_value(
                        &mut values.hardening,
                        hardening,
                        hardening_label(hardening),
                    );
                }
            });
    });
    ui.end_row();
    ui.label("    Hardening curve");
    ui.add_enabled_ui(plastic, |ui| {
        ui.vertical(|ui| {
            let header = |quantity| {
                let unit = units.unit(quantity);
                if unit.is_empty() {
                    String::new()
                } else {
                    format!(" [{unit}]")
                }
            };
            let mut remove = None;
            egui::Grid::new("plastic points")
                .num_columns(4)
                .striped(true)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.strong(format!("Yield stress{}", header(Quantity::Pressure)));
                    ui.strong("Plastic strain");
                    ui.strong(format!("Temperature{}", header(Quantity::Temperature)));
                    ui.label("");
                    ui.end_row();
                    let removable = values.points.len() > 1;
                    for (i, point) in values.points.iter_mut().enumerate() {
                        ui.add(numeric::without_unit(
                            &mut point.stress,
                            units,
                            Quantity::Pressure,
                        ));
                        ui.add(
                            numeric::drag_value(&mut point.plastic_strain)
                                .speed(0.001)
                                .range(0.0..=f64::MAX),
                        );
                        ui.add(numeric::without_unit(
                            &mut point.temperature,
                            units,
                            Quantity::Temperature,
                        ));
                        if ui
                            .add_enabled(removable, egui::Button::new("Remove").small())
                            .clicked()
                        {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            if let Some(i) = remove {
                values.points.remove(i);
            }
            if ui.button("Add row").clicked() {
                // Continues the curve: the same temperature, a larger plastic strain.
                let next = match values.points.as_slice() {
                    [.., a, b] => PlasticPoint {
                        stress: b.stress,
                        plastic_strain: b.plastic_strain
                            + (b.plastic_strain - a.plastic_strain).max(0.0),
                        temperature: b.temperature,
                    },
                    [b] => PlasticPoint {
                        stress: b.stress,
                        plastic_strain: b.plastic_strain + 0.1,
                        temperature: b.temperature,
                    },
                    [] => PlasticPoint {
                        stress: 0.0,
                        plastic_strain: 0.0,
                        temperature: 0.0,
                    },
                };
                values.points.push(next);
            }
            ui.weak(
                "First row at plastic strain 0 with the yield stress; the strain grows from \
                 row to row. Rows at other temperatures start at 0 again.",
            );
        });
    });
    ui.end_row();
    material.plastic = plastic.then_some(values);
}

fn hardening_label(hardening: Hardening) -> &'static str {
    match hardening {
        Hardening::Isotropic => "Isotropic",
        Hardening::Kinematic => "Kinematic",
        Hardening::Combined => "Combined",
    }
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
/// PrePoMax's `StepCollection.AddStep`: only those the new step takes, so a heat transfer
/// step leaves the supports behind and a static step the temperatures.
fn copy_items_of_last_step(fe: &FeModel, step: &mut Step) {
    let Some(last) = fe.steps.last() else {
        return;
    };
    step.boundary_conditions = (last.boundary_conditions.iter())
        .filter(|bc| step.kind.supports_boundary(&bc.kind))
        .cloned()
        .collect();
    step.loads = (last.loads.iter())
        .filter(|load| step.kind.supports_load(&load.kind))
        .cloned()
        .collect();
}

/// The step kinds of the dialog: label, the kind with its settings and its default field
/// outputs, in PrePoMax's order.
fn step_kinds(fe: &FeModel) -> [(&'static str, StepKind, Vec<FieldOutput>); 4] {
    let heat = HeatTransferStep::default();
    [
        (
            STATIC_LABEL,
            StepKind::Static(previous_static(fe)),
            FieldOutput::defaults(),
        ),
        (
            FREQUENCY_LABEL,
            StepKind::Frequency(FrequencyStep::default()),
            FieldOutput::frequency_defaults(),
        ),
        (
            HEAT_TRANSFER_LABEL,
            StepKind::HeatTransfer(heat.clone()),
            FieldOutput::heat_transfer_defaults(),
        ),
        (
            COUPLED_LABEL,
            StepKind::CoupledTempDisp(heat),
            FieldOutput::coupled_defaults(),
        ),
    ]
}

const STATIC_LABEL: &str = "Statisch (Static)";
const FREQUENCY_LABEL: &str = "Eigenfrequenzen (Frequency)";
const HEAT_TRANSFER_LABEL: &str = "Wärmeübertragung (Heat Transfer)";
const COUPLED_LABEL: &str = "Thermomechanisch gekoppelt (Coupled Temp-Disp)";

fn step_kind_label(kind: &StepKind) -> &'static str {
    match kind {
        StepKind::Static(_) => STATIC_LABEL,
        StepKind::Frequency(_) => FREQUENCY_LABEL,
        StepKind::HeatTransfer(_) => HEAT_TRANSFER_LABEL,
        StepKind::CoupledTempDisp(_) => COUPLED_LABEL,
    }
}

/// The step dialog. The kind is chosen when the step is created, as in PrePoMax's list of
/// step types; switching it starts with that kind's default field outputs.
fn step_form(ui: &mut Ui, step: &mut Step, creating: bool, fe: &FeModel) {
    name_row(ui, &mut step.name);
    ui.label("Art");
    if creating {
        let current = step_kind_label(&step.kind);
        egui::ComboBox::from_id_salt("step kind")
            .selected_text(current)
            .width(280.0)
            .show_ui(ui, |ui| {
                for (label, kind, outputs) in step_kinds(fe) {
                    if ui.selectable_label(current == label, label).clicked() && current != label {
                        step.kind = kind;
                        step.field_outputs = outputs;
                    }
                }
            });
    } else {
        ui.label(step_kind_label(&step.kind));
    }
    ui.end_row();
    let units = fe.properties.units;
    match &mut step.kind {
        StepKind::Static(settings) => static_form(ui, settings, units),
        StepKind::Frequency(settings) => frequency_form(ui, settings, units),
        StepKind::HeatTransfer(settings) => heat_transfer_form(ui, settings, units, false),
        StepKind::CoupledTempDisp(settings) => heat_transfer_form(ui, settings, units, true),
    }
}

/// Settings of a heat transfer or coupled step: steady state or transient, then the
/// increments as in a static step.
fn heat_transfer_form(
    ui: &mut Ui,
    settings: &mut HeatTransferStep,
    units: UnitSystem,
    coupled: bool,
) {
    ui.label("");
    ui.checkbox(&mut settings.steady_state, "Stationär (Steady state)");
    ui.end_row();
    let mut limited = settings.deltmx.is_some();
    ui.add_enabled_ui(!settings.steady_state, |ui| {
        ui.checkbox(&mut limited, "Max. Temperaturänderung");
    });
    let mut deltmx = settings.deltmx.unwrap_or(10.0);
    ui.add_enabled(
        limited && !settings.steady_state,
        numeric::quantity(&mut deltmx, units, Quantity::TemperatureDifference)
            .range(0.0..=f64::MAX)
            .speed(1.0),
    );
    settings.deltmx = limited.then_some(deltmx);
    ui.end_row();
    increments_form(ui, &mut settings.increments, units, coupled);
    if !settings.steady_state {
        ui.label("");
        ui.weak("Instationär: Materialien brauchen Dichte und spezifische Wärmekapazität.");
        ui.end_row();
    }
}

fn validate_heat_transfer_step(settings: &HeatTransferStep) -> Result<(), String> {
    if settings.deltmx.is_some_and(|d| d <= 0.0) && !settings.steady_state {
        return Err("Die maximale Temperaturänderung muss größer als null sein.".into());
    }
    Ok(())
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

fn frequency_form(ui: &mut Ui, settings: &mut FrequencyStep, units: UnitSystem) {
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
            numeric::quantity(&mut value, units, Quantity::Frequency)
                .range(0.0..=f64::MAX)
                .speed(1.0),
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

fn static_form(ui: &mut Ui, settings: &mut StaticStep, units: UnitSystem) {
    increments_form(ui, settings, units, true);
}

/// Nonlinear geometry, solver and increments of a step with a time period; `mechanical`
/// shows the geometric nonlinearity, which only steps with displacements have.
fn increments_form(ui: &mut Ui, settings: &mut StaticStep, units: UnitSystem, mechanical: bool) {
    if mechanical {
        ui.label("");
        ui.checkbox(&mut settings.nlgeom, "Geometrisch nichtlinear (Nlgeom)");
        ui.end_row();
    }
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
        ui.add_enabled(enabled, numeric::physical(value, units, Quantity::Time));
        ui.end_row();
    }
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
            amplitude: None,
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
            amplitude: None,
        });
        fe.steps[0].loads.push(Load {
            name: "Pressure-1".into(),
            active: true,
            region: Region::Surface("TOP".into()),
            kind: LoadKind::Pressure(1.0),
            amplitude: None,
            factor_amplitude: None,
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
    fn a_heat_transfer_step_takes_only_thermal_items() {
        let mut fe = FeModel::default();
        Editor::create(NewItem::Step, &fe).unwrap().apply(&mut fe);
        let mut editor = Editor::create(NewItem::Step, &fe).unwrap();
        if let Draft::Step(step) = &mut editor.draft {
            step.kind = StepKind::CoupledTempDisp(HeatTransferStep::default());
        }
        editor.apply(&mut fe);
        let step = &mut fe.steps[1];
        step.boundary_conditions = vec![
            BoundaryCondition {
                name: "Fixed-1".into(),
                active: true,
                region: Region::NodeSet("FIX".into()),
                kind: BoundaryKind::Fixed,
                amplitude: None,
            },
            BoundaryCondition {
                name: "Temperature-1".into(),
                active: true,
                region: Region::NodeSet("HOT".into()),
                kind: BoundaryKind::Temperature(100.0),
                amplitude: None,
            },
        ];
        step.loads = vec![
            Load {
                name: "Pressure-1".into(),
                active: true,
                region: Region::Surface("TOP".into()),
                kind: LoadKind::Pressure(1.0),
                amplitude: None,
                factor_amplitude: None,
            },
            Load {
                name: "Convective_Film-1".into(),
                active: true,
                region: Region::Surface("TOP".into()),
                kind: LoadKind::Film {
                    sink: 20.0,
                    coefficient: 0.01,
                },
                amplitude: None,
                factor_amplitude: None,
            },
        ];
        // A heat transfer step after it keeps the temperature and the film only.
        let mut editor = Editor::create(NewItem::Step, &fe).unwrap();
        if let Draft::Step(step) = &mut editor.draft {
            step.kind = StepKind::HeatTransfer(HeatTransferStep::default());
        }
        editor.apply(&mut fe);
        let heat = &fe.steps[2];
        let names = |s: &Step| -> Vec<String> {
            (s.boundary_conditions.iter().map(|b| b.name.clone()))
                .chain(s.loads.iter().map(|l| l.name.clone()))
                .collect()
        };
        assert_eq!(names(heat), ["Temperature-1", "Convective_Film-1"]);
        // New items there start as the first thermal kind.
        let Draft::BoundaryCondition(_, bc, _) = Editor::create(NewItem::BoundaryCondition(2), &fe)
            .unwrap()
            .draft
        else {
            unreachable!()
        };
        assert_eq!(
            (bc.name.as_str(), bc.kind),
            ("Temperature-2", BoundaryKind::Temperature(0.0))
        );
        let Draft::Load(_, load, region) = Editor::create(NewItem::Load(2), &fe).unwrap().draft
        else {
            unreachable!()
        };
        assert_eq!(load.kind, LoadKind::ConcentratedFlux(0.0));
        assert_eq!(region.target, Target::Nodes);
        // A static step after it leaves the thermal items behind.
        Editor::create(NewItem::Step, &fe).unwrap().apply(&mut fe);
        assert!(names(&fe.steps[3]).is_empty());
    }

    #[test]
    fn initial_temperatures_are_created_and_switched() {
        let mut fe = FeModel::default();
        let mut editor = Editor::create(NewItem::InitialCondition, &fe).unwrap();
        assert!(editor.validate(&fe).is_err(), "empty region");
        if let Draft::InitialCondition(_, region) = &mut editor.draft {
            region.source = Source::Parts;
            region.parts = PartPicks::from_names(["A".to_string()]);
        }
        assert_eq!(editor.validate(&fe), Ok(()));
        editor.apply(&mut fe);
        let condition = &fe.initial_conditions[0];
        assert_eq!(condition.name, "Temperature-1");
        assert_eq!(condition.region, Region::Parts(vec!["A".into()]));
        assert_eq!(condition.kind, InitialConditionKind::Temperature(20.0));
        let item = TreeItem::InitialCondition(0);
        assert!(toggle_active(&mut fe, &item));
        assert!(!fe.initial_conditions[0].active);
        assert!(delete(&mut fe, &item));
        assert!(fe.initial_conditions.is_empty());
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
            amplitude: None,
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
    fn a_swapped_name_that_is_taken_gets_a_number() {
        let mut fe = FeModel::default();
        fe.contact_pairs.push(ContactPair::new("A_to_B", ""));
        fe.contact_pairs.push(ContactPair::new("B_to_A", ""));
        fe.contact_pairs[0].master = Region::Surface("A".into());
        assert!(swap_master_slave(&mut fe, &TreeItem::ContactPair(0)));
        assert_eq!(fe.contact_pairs[0].name, "B_to_A-1");
        assert_eq!(fe.contact_pairs[0].slave, Region::Surface("A".into()));
        fe.constraints
            .push(Constraint::Tie(plx_model::Tie::new("Tie-1")));
        assert!(swap_master_slave(&mut fe, &TreeItem::Constraint(0)));
        assert!(!swap_master_slave(&mut fe, &TreeItem::Material(0)));
    }

    #[test]
    fn loads_follow_a_renamed_amplitude_and_drop_it_when_they_cannot_use_it() {
        let mut fe = FeModel::default();
        for kind in [NewItem::Amplitude, NewItem::Step] {
            Editor::create(kind, &fe).unwrap().apply(&mut fe);
        }
        assert_eq!(fe.amplitudes[0].name, "Amplitude-1");
        let mut film = Load {
            name: "Film-1".into(),
            active: true,
            region: Region::Nodes(vec![1]),
            kind: LoadKind::Film {
                sink: 20.0,
                coefficient: 1.0,
            },
            amplitude: Some("Amplitude-1".into()),
            factor_amplitude: Some("Amplitude-1".into()),
        };
        fe.steps[0].loads.push(film.clone());
        let mut editor = Editor::edit(&TreeItem::Amplitude(0), &fe, &FeMesh::default()).unwrap();
        if let Draft::Amplitude(amplitude, _) = &mut editor.draft {
            amplitude.name = "Ramp".into();
        }
        editor.apply(&mut fe);
        let load = &fe.steps[0].loads[0];
        assert_eq!(load.amplitude.as_deref(), Some("Ramp"));
        assert_eq!(load.factor_amplitude.as_deref(), Some("Ramp"));
        // A pressure has no film coefficient to scale.
        film.kind = LoadKind::Pressure(1.0);
        film.factor_amplitude = Some("Ramp".into());
        Editor {
            draft: Draft::Load(0, film, RegionDraft::new(FACE_SOURCES, Target::Faces)),
            index: Some(0),
            error: None,
            picker: Picker::default(),
        }
        .apply(&mut fe);
        assert_eq!(fe.steps[0].loads[0].factor_amplitude, None);
        assert!(delete(&mut fe, &TreeItem::Amplitude(0)));
        assert!(fe.amplitudes.is_empty());
    }
}
