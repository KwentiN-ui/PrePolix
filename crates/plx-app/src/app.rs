use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};

use crate::model::{self, LoadedModel, Model};
use crate::viewport::{ViewCommand, Viewport};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tab {
    ModelTree,
    Properties,
    Viewport,
    Output,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::ModelTree => "Modell",
            Tab::Properties => "Eigenschaften",
            Tab::Viewport => "3D-Ansicht",
            Tab::Output => "Ausgabe",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Part(usize),
    NodeSet(String),
    ElementSet(String),
    Surface(String),
}

enum LoadEvent {
    Started(PathBuf),
    Finished(PathBuf, Result<Box<LoadedModel>, String>),
}

struct Workbench {
    viewport: Viewport,
    model: Option<Model>,
    selection: Option<Selection>,
    output: Vec<String>,
    view_command: Option<ViewCommand>,
}

pub struct PrepolixApp {
    dock: DockState<Tab>,
    workbench: Workbench,
    load_events: (Sender<LoadEvent>, Receiver<LoadEvent>),
    loading: Option<PathBuf>,
}

impl PrepolixApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let render_state = cc
            .wgpu_render_state
            .clone()
            .ok_or("prepolix benötigt das wgpu-Backend von eframe")?;
        let adapter = render_state.adapter.get_info();
        let output = vec![format!(
            "prepolix {} gestartet, Grafik: {} ({:?})",
            env!("CARGO_PKG_VERSION"),
            adapter.name,
            adapter.backend
        )];
        let mut app = Self {
            dock: default_layout(),
            workbench: Workbench {
                viewport: Viewport::new(render_state),
                model: None,
                selection: None,
                output,
                view_command: None,
            },
            load_events: channel(),
            loading: None,
        };
        if let Some(path) = std::env::args_os().nth(1) {
            app.open_path(PathBuf::from(path), &cc.egui_ctx);
        }
        Ok(app)
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() {
            return;
        }
        let sender = self.load_events.0.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Modell öffnen")
                .add_filter("CalculiX / Abaqus (*.inp)", &["inp", "INP"])
                .pick_file();
            if let Some(path) = picked {
                load_in_background(path, sender, ctx);
            }
        });
    }

    fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.loading.is_none() {
            let (sender, ctx) = (self.load_events.0.clone(), ctx.clone());
            std::thread::spawn(move || load_in_background(path, sender, ctx));
        }
    }

    fn handle_load_events(&mut self) {
        while let Ok(event) = self.load_events.1.try_recv() {
            match event {
                LoadEvent::Started(path) => self.loading = Some(path),
                LoadEvent::Finished(path, result) => {
                    self.loading = None;
                    self.workbench.model_loaded(path, result);
                }
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Datei", |ui| {
                let open = egui::Button::new("Öffnen …").shortcut_text("Strg+O");
                if ui.add_enabled(self.loading.is_none(), open).clicked() {
                    self.open_dialog(ui.ctx());
                }
                ui.separator();
                if ui.button("Beenden").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Bearbeiten", not_implemented);
            ui.menu_button("Ansicht", |ui| {
                for (label, command) in [
                    ("Einpassen", ViewCommand::Fit),
                    ("Isometrisch", ViewCommand::Isometric),
                    ("Vorne", ViewCommand::Front),
                ] {
                    if ui.button(label).clicked() {
                        self.workbench.view_command = Some(command);
                    }
                }
                ui.separator();
                ui.checkbox(
                    &mut self.workbench.viewport.options.mesh_edges,
                    "Netzkanten",
                );
            });
            for menu in [
                "Geometrie",
                "Netz",
                "Modell",
                "Analyse",
                "Ergebnisse",
                "Werkzeuge",
                "Hilfe",
            ] {
                ui.menu_button(menu, not_implemented);
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match (&self.loading, &self.workbench.model) {
            (Some(path), _) => {
                ui.spinner();
                ui.label(format!("Lade {} …", path.display()));
            }
            (None, Some(model)) => {
                ui.label(format!(
                    "{}: {} Knoten, {} Elemente, {} Parts",
                    model.file_name(),
                    model.mesh.node_count(),
                    model.mesh.element_count(),
                    model.parts.len()
                ));
            }
            (None, None) => {
                ui.weak("Kein Modell geladen");
            }
        });
    }
}

fn not_implemented(ui: &mut egui::Ui) {
    ui.label("Noch nicht implementiert");
}

fn load_in_background(path: PathBuf, sender: Sender<LoadEvent>, ctx: egui::Context) {
    let _ = sender.send(LoadEvent::Started(path.clone()));
    ctx.request_repaint();
    let result = model::load(&path).map(Box::new);
    let _ = sender.send(LoadEvent::Finished(path, result));
    ctx.request_repaint();
}

fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Viewport]);
    let surface = dock.main_surface_mut();
    let [viewport, model] = surface.split_left(NodeIndex::root(), 0.22, vec![Tab::ModelTree]);
    surface.split_below(viewport, 0.8, vec![Tab::Output]);
    surface.split_below(model, 0.55, vec![Tab::Properties]);
    dock
}

impl eframe::App for PrepolixApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_load_events();
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.open_dialog(&ctx);
        }
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| !p.as_os_str().is_empty())
        });
        if let Some(path) = dropped {
            self.open_path(path, &ctx);
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        DockArea::new(&mut self.dock)
            .show_close_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(ui, &mut self.workbench);

        if let Some(command) = self.workbench.view_command.take() {
            let bounds = self
                .workbench
                .model
                .as_ref()
                .and_then(Model::visible_bounds);
            self.workbench.viewport.apply(command, bounds);
        }
    }
}

impl Workbench {
    fn model_loaded(&mut self, path: PathBuf, result: Result<Box<LoadedModel>, String>) {
        match result {
            Ok(loaded) => {
                let LoadedModel {
                    model,
                    render_meshes,
                } = *loaded;
                self.output.push(format!(
                    "{} geladen: {} Knoten, {} Elemente, {} Parts ({} ms)",
                    path.display(),
                    model.mesh.node_count(),
                    model.mesh.element_count(),
                    model.parts.len(),
                    model.load_time.as_millis()
                ));
                if model.included_files > 0 {
                    self.output.push(format!(
                        "{} eingebundene Datei(en) gelesen",
                        model.included_files
                    ));
                }
                for warning in &model.warnings {
                    self.output.push(format!("Warnung: {warning}"));
                }
                if !model.skipped_keywords.is_empty() {
                    let keywords: Vec<String> = model
                        .skipped_keywords
                        .keys()
                        .map(|k| format!("*{k}"))
                        .collect();
                    self.output
                        .push(format!("Noch nicht ausgewertet: {}", keywords.join(", ")));
                }
                self.viewport.set_parts(&render_meshes);
                self.viewport
                    .apply(ViewCommand::Fit, model.visible_bounds());
                self.model = Some(model);
                self.selection = None;
            }
            Err(error) => self.output.push(format!("Fehler beim Laden: {error}")),
        }
    }

    fn model_tree(&mut self, ui: &mut egui::Ui) {
        let Some(model) = &mut self.model else {
            ui.weak("Kein Modell geladen.\nDatei > Öffnen (Strg+O) oder eine .inp-Datei ins Fenster ziehen.");
            return;
        };
        let selection = &mut self.selection;
        let mut visibility_changes = Vec::new();
        egui::CollapsingHeader::new(format!("Modell: {}", model.file_name()))
            .default_open(true)
            .show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Parts ({})", model.parts.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        for (index, part) in model.parts.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut part.visible, "").changed() {
                                    visibility_changes.push((index, part.visible));
                                }
                                let (swatch, _) = ui.allocate_exact_size(
                                    egui::vec2(12.0, 12.0),
                                    egui::Sense::hover(),
                                );
                                let [r, g, b] = part.color;
                                ui.painter().rect_filled(
                                    swatch,
                                    2.0,
                                    egui::Color32::from(egui::Rgba::from_rgb(r, g, b)),
                                );
                                let item = Selection::Part(index);
                                let label =
                                    format!("{} ({} Elemente)", part.name, part.element_count);
                                if ui
                                    .selectable_label(*selection == Some(item.clone()), label)
                                    .clicked()
                                {
                                    *selection = Some(item);
                                }
                            });
                        }
                    });
                let mesh = &model.mesh;
                set_list(ui, "Knotensets", &mesh.node_sets, selection, |name, ids| {
                    (Selection::NodeSet(name.to_string()), ids.len())
                });
                set_list(
                    ui,
                    "Elementsets",
                    &mesh.element_sets,
                    selection,
                    |name, ids| (Selection::ElementSet(name.to_string()), ids.len()),
                );
                set_list(
                    ui,
                    "Surfaces",
                    &mesh.surfaces,
                    selection,
                    |name, surface| (Selection::Surface(name.to_string()), surface_size(surface)),
                );
            });
        egui::CollapsingHeader::new("Ergebnisse").show(ui, |ui| {
            ui.weak("Keine Ergebnisse geladen");
        });
        for (index, visible) in visibility_changes {
            self.viewport.set_part_visible(index, visible);
        }
    }

    fn properties(&self, ui: &mut egui::Ui) {
        let Some(model) = &self.model else {
            ui.weak("Kein Modell geladen");
            return;
        };
        let mesh = &model.mesh;
        egui::Grid::new("properties")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                let mut row = |key: &str, value: String| {
                    ui.label(key);
                    ui.label(value);
                    ui.end_row();
                };
                match &self.selection {
                    None => {
                        row("Datei", model.path.display().to_string());
                        row("Knoten", mesh.node_count().to_string());
                        row("Elemente", mesh.element_count().to_string());
                        row("Parts", model.parts.len().to_string());
                    }
                    Some(Selection::Part(index)) => {
                        let part = &model.parts[*index];
                        row("Part", part.name.clone());
                        row("Elemente", part.element_count.to_string());
                        row("Knoten", part.node_count.to_string());
                        for (type_name, count) in &part.element_types {
                            row("Elementtyp", format!("{type_name} ({count})"));
                        }
                        row("Sichtbar", if part.visible { "ja" } else { "nein" }.into());
                    }
                    Some(Selection::NodeSet(name)) => {
                        row("Knotenset", name.clone());
                        row(
                            "Knoten",
                            mesh.node_sets.get(name).map_or(0, Vec::len).to_string(),
                        );
                    }
                    Some(Selection::ElementSet(name)) => {
                        row("Elementset", name.clone());
                        row(
                            "Elemente",
                            mesh.element_sets.get(name).map_or(0, Vec::len).to_string(),
                        );
                    }
                    Some(Selection::Surface(name)) => {
                        row("Surface", name.clone());
                        match mesh.surfaces.get(name) {
                            Some(plx_mesh::SurfaceDefinition::ElementFaces(faces)) => {
                                row("Typ", "Elementflächen".into());
                                row("Flächen", faces.len().to_string());
                            }
                            Some(plx_mesh::SurfaceDefinition::Nodes(nodes)) => {
                                row("Typ", "Knoten".into());
                                row("Knoten", nodes.len().to_string());
                            }
                            None => {}
                        }
                    }
                }
            });
    }
}

fn surface_size(surface: &plx_mesh::SurfaceDefinition) -> usize {
    match surface {
        plx_mesh::SurfaceDefinition::ElementFaces(faces) => faces.len(),
        plx_mesh::SurfaceDefinition::Nodes(nodes) => nodes.len(),
    }
}

fn set_list<T>(
    ui: &mut egui::Ui,
    title: &str,
    sets: &std::collections::BTreeMap<String, T>,
    selection: &mut Option<Selection>,
    describe: impl Fn(&str, &T) -> (Selection, usize),
) {
    if sets.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(format!("{title} ({})", sets.len())).show(ui, |ui| {
        for (name, set) in sets {
            let (item, size) = describe(name, set);
            if ui
                .selectable_label(*selection == Some(item.clone()), format!("{name} ({size})"))
                .clicked()
            {
                *selection = Some(item);
            }
        }
    });
}

impl TabViewer for Workbench {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::ModelTree => self.model_tree(ui),
            Tab::Properties => self.properties(ui),
            Tab::Viewport => {
                if let Some(command) = self.viewport.ui(ui) {
                    self.view_command = Some(command);
                }
            }
            Tab::Output => {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.output {
                            ui.monospace(line);
                        }
                    });
            }
        }
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::Viewport => [false, false],
            _ => [true, true],
        }
    }

    fn clear_background(&self, tab: &Tab) -> bool {
        *tab != Tab::Viewport
    }
}
