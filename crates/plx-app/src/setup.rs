//! Creating and editing the FE model: PrePoMax's item dialogs for materials, sections, steps,
//! boundary conditions, loads and field outputs.
//!
//! Regions are picked in the 3D view while a dialog is open. As in PrePoMax the user never
//! defines node or element sets for this; the input file writer derives them.

use std::collections::BTreeSet;

use egui::Ui;
use plx_mesh::{ElementId, NodeId};
use plx_model::{
    BoundaryCondition, BoundaryKind, Elastic, FeModel, FieldOutput, Incrementation, Load, LoadKind,
    Material, OutputKind, Region, Section, Step, StepKind, next_name,
};

use crate::model::{Highlight, Hit, Model};
use crate::tree::TreeItem;

/// Kinds of items the tree can create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewItem {
    Material,
    Section,
    Step,
    BoundaryCondition(usize),
    Load(usize),
}

/// How a region is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
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

/// What a click in the 3D view selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickMode {
    Node,
    ElementFace,
    /// The smooth surface patch around the clicked face, bounded by feature edges; the
    /// closest the mesh has to PrePoMax's selection of a geometry face.
    Surface,
}

impl PickMode {
    fn label(self) -> &'static str {
        match self {
            PickMode::Node => "Knoten",
            PickMode::ElementFace => "Elementfläche",
            PickMode::Surface => "Fläche bis Kante",
        }
    }
}

/// A region while it is edited in a dialog.
#[derive(Clone, Debug, PartialEq)]
struct RegionDraft {
    sources: &'static [Source],
    modes: &'static [PickMode],
    source: Source,
    mode: PickMode,
    nodes: BTreeSet<NodeId>,
    faces: BTreeSet<(ElementId, u8)>,
    parts: BTreeSet<String>,
    set: String,
}

const NODE_SOURCES: &[Source] = &[Source::Selection, Source::NodeSet, Source::Surface];
const ALL_MODES: &[PickMode] = &[PickMode::Surface, PickMode::ElementFace, PickMode::Node];
const FACE_SOURCES: &[Source] = &[Source::Selection, Source::Surface];
const FACE_MODES: &[PickMode] = &[PickMode::Surface, PickMode::ElementFace];
const ELEMENT_SOURCES: &[Source] = &[Source::Parts, Source::ElementSet];

impl RegionDraft {
    fn new(sources: &'static [Source], modes: &'static [PickMode]) -> Self {
        Self {
            sources,
            modes,
            source: sources[0],
            mode: modes.first().copied().unwrap_or(PickMode::Surface),
            nodes: BTreeSet::new(),
            faces: BTreeSet::new(),
            parts: BTreeSet::new(),
            set: String::new(),
        }
    }

    fn from_region(
        region: &Region,
        sources: &'static [Source],
        modes: &'static [PickMode],
    ) -> Self {
        let mut draft = Self::new(sources, modes);
        match region {
            Region::Parts(parts) => {
                draft.source = Source::Parts;
                draft.parts = parts.iter().cloned().collect();
            }
            Region::Nodes(nodes) => {
                draft.mode = PickMode::Node;
                draft.nodes = nodes.iter().copied().collect();
            }
            Region::Faces(faces) => {
                draft.faces = faces.iter().copied().collect();
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

    fn region(&self) -> Region {
        match self.source {
            Source::Selection if self.mode == PickMode::Node => {
                Region::Nodes(self.nodes.iter().copied().collect())
            }
            Source::Selection => Region::Faces(self.faces.iter().copied().collect()),
            Source::Parts => Region::Parts(self.parts.iter().cloned().collect()),
            Source::NodeSet => Region::NodeSet(self.set.clone()),
            Source::ElementSet => Region::ElementSet(self.set.clone()),
            Source::Surface => Region::Surface(self.set.clone()),
        }
    }

    fn is_empty(&self) -> bool {
        match self.source {
            Source::Selection if self.mode == PickMode::Node => self.nodes.is_empty(),
            Source::Selection => self.faces.is_empty(),
            Source::Parts => self.parts.is_empty(),
            _ => self.set.is_empty(),
        }
    }

    fn click(&mut self, model: &Model, hit: &Hit, remove: bool) {
        fn toggle<T: Ord>(set: &mut BTreeSet<T>, items: impl IntoIterator<Item = T>, remove: bool) {
            for item in items {
                if remove {
                    set.remove(&item);
                } else {
                    set.insert(item);
                }
            }
        }
        match (self.source, self.mode) {
            (Source::Parts, _) => {
                let name = model.parts[hit.part].name.clone();
                toggle(&mut self.parts, [name], remove);
            }
            (Source::Selection, PickMode::Node) => {
                toggle(&mut self.nodes, [model.hit_node(hit)], remove);
            }
            (Source::Selection, PickMode::ElementFace) => {
                toggle(&mut self.faces, [model.hit_face(hit)], remove);
            }
            (Source::Selection, PickMode::Surface) => {
                toggle(&mut self.faces, model.hit_patch(hit), remove);
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut Ui, model: &Model) {
        ui.label("Region");
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
                        for &mode in self.modes {
                            if ui.radio(self.mode == mode, mode.label()).clicked() {
                                if (mode == PickMode::Node) != (self.mode == PickMode::Node) {
                                    self.nodes.clear();
                                    self.faces.clear();
                                }
                                self.mode = mode;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let count = if self.mode == PickMode::Node {
                            format!("{} Knoten", self.nodes.len())
                        } else {
                            format!("{} Elementflächen", self.faces.len())
                        };
                        ui.label(count);
                        if ui.button("Auswahl löschen").clicked() {
                            self.nodes.clear();
                            self.faces.clear();
                        }
                    });
                    ui.weak("Klick wählt aus, Strg+Klick entfernt.");
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
        ui.end_row();
    }

    fn highlight(&self, model: &Model) -> Highlight {
        let mut highlight = region_highlight(model, &self.region());
        if self.source == Source::Selection && self.mode == PickMode::Node {
            highlight.faces.clear();
        }
        highlight
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
        Region::Nodes(_) | Region::NodeSet(_) => highlight.nodes = region.nodes(&model.mesh),
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
}

/// An open item dialog.
pub struct Editor {
    draft: Draft,
    /// Index of the edited item; `None` creates a new one.
    index: Option<usize>,
    error: Option<String>,
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
                },
                RegionDraft::new(ELEMENT_SOURCES, &[]),
            ),
            NewItem::Step => {
                let mut step = Step::new_static(next_name("Step", names(&fe.steps, |s| &s.name)));
                // PrePoMax carries the solution settings of the previous step over.
                if let (Some(previous), StepKind::Static(settings)) =
                    (fe.steps.last(), &mut step.kind)
                {
                    let StepKind::Static(previous) = &previous.kind;
                    *settings = previous.clone();
                }
                Draft::Step(step)
            }
            NewItem::BoundaryCondition(step) => {
                let existing = names(&fe.steps.get(step)?.boundary_conditions, |b| &b.name);
                Draft::BoundaryCondition(
                    step,
                    BoundaryCondition {
                        name: next_name(FIXED, existing),
                        region: Region::Nodes(Vec::new()),
                        kind: BoundaryKind::Fixed,
                    },
                    RegionDraft::new(NODE_SOURCES, ALL_MODES),
                )
            }
            NewItem::Load(step) => {
                let existing = names(&fe.steps.get(step)?.loads, |l| &l.name);
                let mut region = RegionDraft::new(NODE_SOURCES, ALL_MODES);
                region.mode = PickMode::Node;
                Draft::Load(
                    step,
                    Load {
                        name: next_name(FORCE, existing),
                        region: Region::Nodes(Vec::new()),
                        kind: LoadKind::ConcentratedForce([0.0; 3]),
                    },
                    region,
                )
            }
        };
        Some(Self {
            draft,
            index: None,
            error: None,
        })
    }

    pub fn edit(item: &TreeItem, fe: &FeModel) -> Option<Self> {
        let (draft, index) = match *item {
            TreeItem::Material(i) => (Draft::Material(fe.materials.get(i)?.clone()), i),
            TreeItem::Section(i) => {
                let section = fe.sections.get(i)?.clone();
                let region = RegionDraft::from_region(&section.region, ELEMENT_SOURCES, &[]);
                (Draft::Section(section, region), i)
            }
            TreeItem::Step(i) => (Draft::Step(fe.steps.get(i)?.clone()), i),
            TreeItem::BoundaryCondition(s, i) => {
                let bc = fe.steps.get(s)?.boundary_conditions.get(i)?.clone();
                let region = RegionDraft::from_region(&bc.region, NODE_SOURCES, ALL_MODES);
                (Draft::BoundaryCondition(s, bc, region), i)
            }
            TreeItem::Load(s, i) => {
                let load = fe.steps.get(s)?.loads.get(i)?.clone();
                let region = match load.kind {
                    LoadKind::ConcentratedForce(_) => {
                        RegionDraft::from_region(&load.region, NODE_SOURCES, ALL_MODES)
                    }
                    LoadKind::Pressure(_) => {
                        RegionDraft::from_region(&load.region, FACE_SOURCES, FACE_MODES)
                    }
                };
                (Draft::Load(s, load, region), i)
            }
            TreeItem::FieldOutput(s, i) => {
                let output = fe.steps.get(s)?.field_outputs.get(i)?.clone();
                (Draft::FieldOutput(s, output), i)
            }
            _ => return None,
        };
        Some(Self {
            draft,
            index: Some(index),
            error: None,
        })
    }

    pub fn title(&self) -> String {
        let (kind, name) = match &self.draft {
            Draft::Material(m) => ("Material", &m.name),
            Draft::Section(s, _) => ("Section", &s.name),
            Draft::Step(s) => ("Step", &s.name),
            Draft::BoundaryCondition(_, b, _) => ("Randbedingung", &b.name),
            Draft::Load(_, l, _) => ("Last", &l.name),
            Draft::FieldOutput(_, f) => ("Field Output", &f.name),
        };
        let action = if self.index.is_some() {
            "bearbeiten"
        } else {
            "erstellen"
        };
        format!("{kind} {action}: {name}")
    }

    /// Whether clicks in the 3D view pick for this dialog.
    pub fn picks(&self) -> bool {
        self.region()
            .is_some_and(|r| matches!(r.source, Source::Selection | Source::Parts))
    }

    fn region(&self) -> Option<&RegionDraft> {
        match &self.draft {
            Draft::Section(_, r) | Draft::BoundaryCondition(_, _, r) | Draft::Load(_, _, r) => {
                Some(r)
            }
            _ => None,
        }
    }

    pub fn click(&mut self, model: &Model, hit: &Hit, remove: bool) {
        if let Draft::Section(_, r) | Draft::BoundaryCondition(_, _, r) | Draft::Load(_, _, r) =
            &mut self.draft
        {
            r.click(model, hit, remove);
        }
    }

    /// The region being edited, for the 3D view.
    pub fn highlight(&self, model: &Model) -> Highlight {
        self.region()
            .map_or_else(Highlight::default, |r| r.highlight(model))
    }

    pub fn show(&mut self, ctx: &egui::Context, model: &Model) -> EditorResult {
        let mut result = EditorResult::Open;
        let mut open = true;
        egui::Window::new(self.title())
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
        if !open {
            result = EditorResult::Cancel;
        }
        result
    }

    fn form(&mut self, ui: &mut Ui, model: &Model) {
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
                region.ui(ui, model);
            }
            Draft::Step(step) => step_form(ui, step),
            Draft::BoundaryCondition(_, bc, region) => {
                name_row(ui, &mut bc.name);
                ui.label("Art");
                ui.horizontal(|ui| {
                    let fixed = matches!(bc.kind, BoundaryKind::Fixed);
                    if ui.radio(fixed, "Fest eingespannt").clicked() && !fixed {
                        bc.kind = BoundaryKind::Fixed;
                        rename_default(&mut bc.name, DISPLACEMENT, FIXED);
                    }
                    if ui.radio(!fixed, "Verschiebung/Rotation").clicked() && fixed {
                        bc.kind =
                            BoundaryKind::Displacement([Some(0.0), None, None, None, None, None]);
                        rename_default(&mut bc.name, FIXED, DISPLACEMENT);
                    }
                });
                ui.end_row();
                if let BoundaryKind::Displacement(values) = &mut bc.kind {
                    for (value, label) in values
                        .iter_mut()
                        .zip(["U1", "U2", "U3", "UR1", "UR2", "UR3"])
                    {
                        let mut set = value.is_some();
                        ui.checkbox(&mut set, label);
                        let mut number = value.unwrap_or(0.0);
                        ui.add_enabled(set, egui::DragValue::new(&mut number).speed(0.01));
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
                    let force = matches!(load.kind, LoadKind::ConcentratedForce(_));
                    if ui.radio(force, "Einzelkraft").clicked() && !force {
                        load.kind = LoadKind::ConcentratedForce([0.0; 3]);
                        rename_default(&mut load.name, PRESSURE, FORCE);
                        *region = RegionDraft::new(NODE_SOURCES, ALL_MODES);
                        region.mode = PickMode::Node;
                    }
                    if ui.radio(!force, "Druck").clicked() && force {
                        load.kind = LoadKind::Pressure(0.0);
                        rename_default(&mut load.name, FORCE, PRESSURE);
                        *region = RegionDraft::new(FACE_SOURCES, FACE_MODES);
                    }
                });
                ui.end_row();
                match &mut load.kind {
                    LoadKind::ConcentratedForce(force) => {
                        for (value, label) in force.iter_mut().zip(["F1", "F2", "F3"]) {
                            ui.label(label);
                            ui.add(egui::DragValue::new(value).speed(1.0));
                            ui.end_row();
                        }
                        ui.label("");
                        ui.weak("Die Kraft wirkt an jedem Knoten der Region.");
                        ui.end_row();
                    }
                    LoadKind::Pressure(pressure) => {
                        ui.label("Druck");
                        ui.add(egui::DragValue::new(pressure).speed(0.1));
                        ui.end_row();
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
        }
    }

    fn validate(&self, fe: &FeModel) -> Result<(), String> {
        let (name, siblings): (&str, Vec<&str>) = match &self.draft {
            Draft::Material(m) => (&m.name, names(&fe.materials, |m| &m.name)),
            Draft::Section(s, _) => (&s.name, names(&fe.sections, |s| &s.name)),
            Draft::Step(s) => (&s.name, names(&fe.steps, |s| &s.name)),
            Draft::BoundaryCondition(step, b, _) => (
                &b.name,
                names(&fe.steps[*step].boundary_conditions, |b| &b.name),
            ),
            Draft::Load(step, l, _) => (&l.name, names(&fe.steps[*step].loads, |l| &l.name)),
            Draft::FieldOutput(step, f) => {
                (&f.name, names(&fe.steps[*step].field_outputs, |f| &f.name))
            }
        };
        if name.trim().is_empty() {
            return Err("Bitte einen Namen eingeben.".into());
        }
        let duplicate = siblings
            .iter()
            .enumerate()
            .any(|(i, other)| Some(i) != self.index && other.eq_ignore_ascii_case(name));
        if duplicate {
            return Err(format!("Der Name {name} ist schon vergeben."));
        }
        if let Draft::Section(section, _) = &self.draft
            && !fe.materials.iter().any(|m| m.name == section.material)
        {
            return Err("Bitte ein Material wählen; zuerst unter Materials anlegen.".into());
        }
        if self.region().is_some_and(RegionDraft::is_empty) {
            return Err("Die Region ist leer.".into());
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
                None => fe.steps.push(step),
            },
            Draft::BoundaryCondition(s, mut bc, region) => {
                bc.region = region.region();
                put(&mut fe.steps[s].boundary_conditions, index, bc);
            }
            Draft::Load(s, mut load, region) => {
                load.region = region.region();
                put(&mut fe.steps[s].loads, index, load);
            }
            Draft::FieldOutput(s, output) => put(&mut fe.steps[s].field_outputs, index, output),
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
        _ => false,
    }
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
        _ => None,
    }
}

/// Keeps a default name in step with the item kind, e.g. Fixed-1 becomes
/// Displacement_Rotation-1, but leaves names the user chose.
fn rename_default(name: &mut String, from: &str, to: &str) {
    if let Some(number) = name.strip_prefix(from).and_then(|n| n.strip_prefix('-'))
        && number.parse::<u32>().is_ok()
    {
        *name = format!("{to}-{number}");
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
        egui::DragValue::new(&mut values.poisson)
            .range(0.0..=0.5)
            .speed(0.01)
            .max_decimals(4),
    );
    ui.end_row();
    material.elastic = elastic.then_some(values);
}

fn step_form(ui: &mut Ui, step: &mut Step) {
    name_row(ui, &mut step.name);
    let StepKind::Static(settings) = &mut step.kind;
    ui.label("Art");
    ui.label("Statisch");
    ui.end_row();
    ui.label("");
    ui.checkbox(&mut settings.nlgeom, "Geometrisch nichtlinear (Nlgeom)");
    ui.end_row();
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
        egui::DragValue::new(&mut settings.max_increments).range(1..=1_000_000),
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

fn incrementation_label(incrementation: Incrementation) -> &'static str {
    match incrementation {
        Incrementation::Default => "Standard",
        Incrementation::Automatic => "Automatisch",
        Incrementation::Direct => "Fest (Direct)",
    }
}

/// Field for physical values that may be very small or large, such as a density of 7.85e-9.
fn number(value: &mut f64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .speed(0.0)
        .custom_formatter(|v, _| {
            if v != 0.0 && !(1e-3..1e7).contains(&v.abs()) {
                format!("{v:e}")
            } else {
                format!("{v}")
            }
        })
        .custom_parser(|text| text.trim().replace(',', ".").parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_follow_the_kind() {
        let mut name = "Fixed-2".to_string();
        rename_default(&mut name, FIXED, DISPLACEMENT);
        assert_eq!(name, "Displacement_Rotation-2");
        let mut name = "Einspannung".to_string();
        rename_default(&mut name, FIXED, DISPLACEMENT);
        assert_eq!(name, "Einspannung");
    }

    #[test]
    fn regions_survive_the_dialog() {
        for region in [
            Region::Nodes(vec![1, 5]),
            Region::Faces(vec![(3, 2), (4, 6)]),
            Region::NodeSet("FIX".into()),
            Region::Surface("TIP".into()),
        ] {
            let draft = RegionDraft::from_region(&region, NODE_SOURCES, ALL_MODES);
            assert_eq!(draft.region(), region);
        }
        let parts = Region::Parts(vec!["A".into(), "B".into()]);
        let draft = RegionDraft::from_region(&parts, ELEMENT_SOURCES, &[]);
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
            region.faces.insert((1, 4));
        }
        assert_eq!(editor.validate(&fe), Ok(()));
        editor.apply(&mut fe);
        let bc = &fe.steps[0].boundary_conditions[0];
        assert_eq!(
            (bc.name.as_str(), &bc.region),
            ("Fixed-1", &Region::Faces(vec![(1, 4)]))
        );
        assert!(Editor::create(NewItem::Load(3), &fe).is_none());
    }
}
