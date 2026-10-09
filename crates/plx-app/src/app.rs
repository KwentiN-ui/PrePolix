use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use plx_render::StandardView;

use crate::analysis::{Analysis, MonitorEvent};
use crate::animation::{AnimationKind, ColorLimits, Playback};
use crate::icons::{self, Icon};
use crate::keywords::KeywordEditor;
use crate::material_library::{LibraryResult, MaterialLibraryEditor};
use crate::model::{self, LoadedModel, Model};
use crate::numeric;
use crate::overlay::{Marker, Overlay};
use crate::properties;
use crate::results::{Deformation, ResultsView, format_legend_value};
use crate::screenshot::{self, Screenshot};
use crate::selection::Operation;
use crate::settings::{self, Settings, SettingsWindow, WindowResult};
use crate::setup::{Editor, EditorResult, NewItem};
use crate::tree::{self, TreeItem, TreeState, TreeView};
use crate::viewport::{Axis, BoxSelect, Click, ViewCommand, Viewport};
use plx_render::RenderMesh;

enum LoadEvent {
    Started(PathBuf),
    Finished(PathBuf, Result<Box<LoadedModel>, String>),
}

struct Workbench {
    settings: Settings,
    /// Open settings window with its unsaved draft.
    settings_window: Option<SettingsWindow>,
    viewport: Viewport,
    /// The FE model workspace: mesh and analysis set up from an input or project file.
    model: Option<Model>,
    /// The results workspace: every results file opened in this session, PrePoMax's results
    /// collection. One of them is shown on the Results tab.
    results: Vec<Model>,
    current_result: usize,
    /// Camera of the workspace not shown, so switching tabs keeps each view.
    parked_camera: Option<plx_render::Camera>,
    tree: TreeState,
    /// Item whose properties window is open.
    dialog: Option<TreeItem>,
    /// Which of the three trees is shown.
    tree_view: TreeView,
    output: Vec<String>,
    view_command: Option<ViewCommand>,
    /// The result selection or deformation changed; the scene must be rebuilt.
    results_changed: bool,
    /// Only the animation frame changed; frames already built are reused.
    frame_changed: bool,
    /// Scene of each animation frame shown so far, cleared when anything else changes.
    frame_cache: std::collections::HashMap<usize, Vec<RenderMesh>>,
    /// Open dialog creating or editing an item of the FE model.
    editor: Option<Editor>,
    /// Open CalculiX keyword editor.
    keyword_editor: Option<KeywordEditor>,
    /// Open material library editor.
    material_library: Option<MaterialLibraryEditor>,
    /// The tree selection whose region is highlighted.
    highlighted: Option<(TreeView, TreeItem)>,
    analysis: Option<Analysis>,
    /// Results file the user asked to open; read by the app on a worker thread.
    open_results: Option<PathBuf>,
    screenshot: Screenshot,
}

pub struct PrepolixApp {
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
        crate::style::apply(&cc.egui_ctx);
        let adapter = render_state.adapter.get_info();
        let output = vec![format!(
            "prepolix {} gestartet, Grafik: {} ({:?})",
            env!("CARGO_PKG_VERSION"),
            adapter.name,
            adapter.backend
        )];
        let app = Self {
            workbench: Workbench {
                settings: cc
                    .storage
                    .and_then(|s| eframe::get_value(s, settings::STORAGE_KEY))
                    .unwrap_or_default(),
                settings_window: None,
                viewport: Viewport::new(render_state),
                model: None,
                results: Vec::new(),
                current_result: 0,
                parked_camera: None,
                tree: TreeState::default(),
                dialog: None,
                tree_view: TreeView::FeModel,
                output,
                view_command: None,
                results_changed: false,
                frame_changed: false,
                frame_cache: Default::default(),
                editor: None,
                keyword_editor: None,
                material_library: None,
                highlighted: None,
                analysis: None,
                open_results: None,
                screenshot: Screenshot::default(),
            },
            load_events: channel(),
            loading: None,
        };
        // Every file on the command line is opened in turn, e.g. a model and its results.
        let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if !paths.is_empty() {
            let (sender, ctx) = (app.load_events.0.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || {
                for path in paths {
                    load_in_background(path, sender.clone(), ctx.clone());
                }
            });
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
                .add_filter(
                    "Projekt, CalculiX-Modell oder -Ergebnisse (*.plx, *.inp, *.frd)",
                    &["plx", "PLX", "inp", "INP", "frd", "FRD"],
                )
                .add_filter("prepolix-Projekt (*.plx)", &["plx", "PLX"])
                .add_filter("Eingabedatei (*.inp)", &["inp", "INP"])
                .add_filter("Ergebnisdatei (*.frd)", &["frd", "FRD"])
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
                let new = egui::Button::new("Neu").shortcut_text("Strg+N");
                if ui.add_enabled(self.workbench.has_anything(), new).clicked() {
                    self.workbench.close_model();
                }
                let open = egui::Button::new("Öffnen …").shortcut_text("Strg+O");
                if ui.add_enabled(self.loading.is_none(), open).clicked() {
                    self.open_dialog(ui.ctx());
                }
                let setup = self.workbench.setup_model().is_some();
                let save = egui::Button::new("Speichern").shortcut_text("Strg+S");
                if ui.add_enabled(setup, save).clicked() {
                    self.workbench.save_project(false);
                }
                let save_as =
                    egui::Button::new("Speichern unter …").shortcut_text("Strg+Umschalt+S");
                if ui.add_enabled(setup, save_as).clicked() {
                    self.workbench.save_project(true);
                }
                ui.separator();
                let export = egui::Button::new("CalculiX-Eingabedatei exportieren …");
                if ui.add_enabled(setup, export).clicked() {
                    self.workbench.export_inp();
                }
                ui.separator();
                if ui.button("Beenden").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Bearbeiten", not_implemented);
            ui.menu_button("Ansicht", |ui| {
                if ui.button("Einpassen").clicked() {
                    self.workbench.view_command = Some(ViewCommand::Fit);
                }
                for (view, label) in STANDARD_VIEWS {
                    if ui.button(label).clicked() {
                        self.workbench.view_command = Some(ViewCommand::View(view));
                    }
                }
                if ui.button("Vertikal").clicked() {
                    self.workbench.view_command = Some(ViewCommand::Vertical);
                }
                ui.menu_button("Achse senkrecht", |ui| {
                    for axis in Axis::ALL {
                        if ui.button(axis.label()).clicked() {
                            self.workbench.view_command = Some(ViewCommand::VerticalAxis(axis));
                        }
                    }
                });
                ui.menu_button("Isometrisch, Achse oben", |ui| {
                    for axis in Axis::ALL {
                        if ui.button(axis.label()).clicked() {
                            self.workbench.view_command = Some(ViewCommand::IsometricAxis(axis));
                        }
                    }
                });
                ui.separator();
                ui.checkbox(
                    &mut self.workbench.viewport.options.mesh_edges,
                    "Netzkanten",
                );
            });
            for menu in ["Geometrie", "Netz"] {
                ui.menu_button(menu, not_implemented);
            }
            ui.menu_button("Modell", |ui| self.workbench.model_menu(ui));
            ui.menu_button("Analyse", |ui| self.workbench.analysis_menu(ui));
            ui.menu_button("Ergebnisse", |ui| self.workbench.results_menu(ui));
            ui.menu_button("Werkzeuge", |ui| {
                if ui.button("Einstellungen …").clicked() {
                    self.workbench.settings_window =
                        Some(SettingsWindow::new(&self.workbench.settings));
                }
            });
            ui.menu_button("Hilfe", not_implemented);
        });
    }

    /// PrePoMax's main tool bar: file commands, then views and display options.
    fn tool_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let has_model = self.workbench.has_anything();
            if icons::button(ui, Icon::New, "Neu (Strg+N)", has_model, false).clicked() {
                self.workbench.close_model();
            }
            let can_open = self.loading.is_none();
            if icons::button(ui, Icon::Open, "Öffnen (Strg+O)", can_open, false).clicked() {
                self.open_dialog(ui.ctx());
            }
            let can_save = self.workbench.setup_model().is_some();
            if icons::button(ui, Icon::Save, "Speichern (Strg+S)", can_save, false).clicked() {
                self.workbench.save_project(false);
            }
            ui.separator();
            if icons::button(ui, Icon::Fit, "Einpassen", true, false).clicked() {
                self.workbench.view_command = Some(ViewCommand::Fit);
            }
            for (view, label) in STANDARD_VIEWS {
                if icons::button(ui, Icon::View(view), label, true, false).clicked() {
                    self.workbench.view_command = Some(ViewCommand::View(view));
                }
            }
            if icons::button(ui, Icon::Vertical, "Vertikal", true, false).clicked() {
                self.workbench.view_command = Some(ViewCommand::Vertical);
            }
            let camera = icons::button(ui, Icon::Screenshot, "Screenshot", true, false);
            egui::Popup::menu(&camera).show(|ui| {
                if ui.button("In Zwischenablage kopieren").clicked() {
                    self.workbench
                        .screenshot
                        .request(screenshot::Target::Clipboard);
                }
                if ui.button("Speichern unter …").clicked() {
                    self.workbench.screenshot.request(screenshot::Target::File);
                }
            });
            ui.separator();
            let options = &mut self.workbench.viewport.options;
            let mesh = options.mesh_edges;
            if icons::button(ui, Icon::FeatureEdges, "Nur Kanten", true, !mesh).clicked() {
                options.mesh_edges = false;
            }
            if icons::button(ui, Icon::MeshEdges, "Netzkanten", true, mesh).clicked() {
                options.mesh_edges = true;
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match (&self.loading, self.workbench.shown()) {
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
                ui.weak(if self.workbench.tree_view == TreeView::Results {
                    "Keine Ergebnisse geladen"
                } else {
                    "Kein Modell geladen"
                });
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

const STANDARD_VIEWS: [(StandardView, &str); 7] = [
    (StandardView::Front, "Vorne"),
    (StandardView::Back, "Hinten"),
    (StandardView::Top, "Oben"),
    (StandardView::Bottom, "Unten"),
    (StandardView::Left, "Links"),
    (StandardView::Right, "Rechts"),
    (StandardView::Isometric, "Isometrisch"),
];

impl eframe::App for PrepolixApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, settings::STORAGE_KEY, &self.workbench.settings);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_load_events();
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.open_dialog(&ctx);
        }
        let shift_command = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        if ctx.input_mut(|i| i.consume_key(shift_command, egui::Key::S)) {
            self.workbench.save_project(true);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.workbench.save_project(false);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.workbench.close_model();
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
        egui::Panel::top("tools").show(ui, |ui| {
            self.tool_bar(ui);
            // Like PrePoMax, the results row stays in place and is greyed out outside the
            // Results tab, so the 3D view does not jump when switching tabs.
            ui.separator();
            self.workbench.results_tool_bar(ui);
        });
        self.workbench.animate(&ctx);
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        // PrePoMax's fixed layout: tree on the left over the full height, 3D view with the
        // output below it; only the separators move.
        let pane = egui::Frame::new()
            .fill(crate::style::WINDOW)
            .stroke(egui::Stroke::new(1.0, crate::style::BORDER));
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(280.0)
            .size_range(160.0..=700.0)
            .frame(egui::Frame::new().fill(crate::style::CONTROL))
            .show(ui, |ui| {
                self.workbench.tree_tabs(ui);
                pane.inner_margin(4).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let view = self.workbench.tree_view;
                    self.workbench.model_tree(ui, view);
                });
            });
        egui::Panel::bottom("output")
            .resizable(true)
            .default_size(140.0)
            .size_range(40.0..=600.0)
            .frame(pane.inner_margin(4))
            .show(ui, |ui| self.workbench.output(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.workbench.viewport.selecting = self.workbench.picking();
            let response = self.workbench.viewport.ui(ui);
            if let Some(command) = response.command {
                self.workbench.view_command = Some(command);
            }
            if let Some(click) = response.click {
                self.workbench.click(click);
            }
            if let Some(area) = response.box_select {
                self.workbench.box_select(&area);
            }
            if let Some(hover) = response.hover {
                self.workbench.hover(hover);
                ui.ctx().request_repaint();
            }
        });
        let view = self.workbench.viewport.rect;
        let workbench = &mut self.workbench;
        workbench
            .screenshot
            .update(&ctx, view, &mut workbench.output);
        self.workbench.properties_window(&ctx);
        self.workbench.editor_window(&ctx);
        self.workbench.keyword_editor_window(&ctx);
        self.workbench.material_library_window(&ctx);
        self.workbench.run_analysis(&ctx);
        if let Some(path) = self.workbench.open_results.take() {
            self.open_path(path, &ctx);
        }
        self.workbench.update_highlight();
        self.workbench.settings_window(&ctx);
        self.workbench.rebuild_if_results_changed();

        if let Some(command) = self.workbench.view_command.take() {
            let bounds = self.workbench.shown().and_then(Model::visible_bounds);
            self.workbench.viewport.apply(command, bounds);
        }
    }
}

impl Workbench {
    fn model_loaded(&mut self, path: PathBuf, result: Result<Box<LoadedModel>, String>) {
        match result {
            Ok(loaded) => {
                let LoadedModel {
                    mut model,
                    render_meshes,
                } = *loaded;
                // The scene built on the worker thread is reused unless the settings change it.
                let mut rebuild = false;
                if let Some(view) = &mut model.results {
                    let post = &self.settings.post;
                    if (view.levels, view.show_undeformed) != (post.levels, post.undeformed_outline)
                    {
                        view.levels = post.levels;
                        view.show_undeformed = post.undeformed_outline;
                        rebuild = true;
                    }
                }
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
                if let Some(view) = &model.results {
                    self.output.push(format!(
                        "{} Ergebnis-Inkrement(e) gelesen",
                        view.increments.len()
                    ));
                }
                self.dialog = None;
                self.viewport.labels = Default::default();
                if let Some(view) = &model.results {
                    // A results file joins the results collection and leaves the FE model
                    // alone; opening the same file again replaces it.
                    self.tree.selected = Some((
                        TreeView::Results,
                        TreeItem::Component(view.field, view.component),
                    ));
                    self.set_tree_view(TreeView::Results);
                    match self.results.iter().position(|r| r.path == model.path) {
                        Some(index) => {
                            self.results[index] = model;
                            self.current_result = index;
                        }
                        None => {
                            self.results.push(model);
                            self.current_result = self.results.len() - 1;
                        }
                    }
                } else {
                    self.tree.selected = None;
                    self.editor = None;
                    self.highlighted = None;
                    self.set_tree_view(TreeView::FeModel);
                    self.model = Some(model);
                }
                self.frame_cache.clear();
                self.viewport.set_parts(&render_meshes);
                self.results_changed = rebuild;
                self.update_contour();
                self.view_command = Some(ViewCommand::Fit);
            }
            Err(error) => self.output.push(format!("Fehler beim Laden: {error}")),
        }
    }

    /// Tab strip above the tree, like PrePoMax's Windows tab control.
    fn tree_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(2.0);
            for view in [TreeView::Geometry, TreeView::FeModel, TreeView::Results] {
                let selected = self.tree_view == view;
                let galley = ui.painter().layout_no_wrap(
                    view.title().to_string(),
                    egui::TextStyle::Body.resolve(ui.style()),
                    egui::Color32::BLACK,
                );
                let size = galley.size() + egui::vec2(16.0, 8.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                let fill = if selected {
                    crate::style::WINDOW
                } else if response.hovered() {
                    crate::style::HOVER_FILL
                } else {
                    crate::style::CONTROL
                };
                let rect = if selected {
                    rect
                } else {
                    rect.shrink2(egui::vec2(0.0, 1.0))
                        .translate(egui::vec2(0.0, 1.0))
                };
                ui.painter().rect(
                    rect,
                    0.0,
                    fill,
                    egui::Stroke::new(1.0, crate::style::BORDER),
                    egui::StrokeKind::Inside,
                );
                ui.painter().galley(
                    rect.center() - galley.size() * 0.5,
                    galley,
                    egui::Color32::BLACK,
                );
                if response.clicked() {
                    self.set_tree_view(view);
                }
            }
        });
    }

    fn output(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.output {
                    ui.monospace(line);
                }
            });
    }

    fn model_tree(&mut self, ui: &mut egui::Ui, view: TreeView) {
        let empty = match view {
            TreeView::Geometry => None,
            TreeView::FeModel => self.model.is_none().then_some("Kein Modell geladen"),
            TreeView::Results => self
                .results
                .is_empty()
                .then_some("Keine Ergebnisse geladen"),
        };
        if let Some(text) = empty {
            ui.weak(format!("{text}.\nDatei > Öffnen (Strg+O) oder eine .plx-, .inp- oder .frd-Datei ins Fenster ziehen."));
            ui.separator();
        }
        let shown = match view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => self.model.as_mut(),
        };
        let job = self.analysis.as_ref().map(Analysis::status);
        let response = tree::show(ui, view, shown, job, &mut self.tree);
        for (index, visible) in response.visibility {
            self.viewport.set_part_visible(index, visible);
        }
        if let (Some((field, component)), Some(results)) =
            (response.component, self.shown_results_mut())
        {
            results.field = field;
            results.component = component;
            self.results_changed = true;
        }
        if let Some(item) = response.open {
            let model = self.model.as_ref().filter(|_| view != TreeView::Results);
            match model.and_then(|m| Editor::edit(&item, &m.fe, &m.mesh)) {
                Some(editor) => self.editor = Some(editor),
                None => self.dialog = Some(item),
            }
        }
        if let Some(kind) = response.create {
            self.create(kind);
        }
        if let (Some(item), Some(model)) = (response.delete, self.model.as_mut())
            && crate::setup::delete(&mut model.fe, &item)
        {
            self.tree.selected = None;
            self.editor = None;
        }
        if response.run {
            self.start_analysis();
        }
        if response.material_library {
            self.open_material_library();
        }
    }

    /// The FE model, which can be set up.
    fn setup_model(&self) -> Option<&Model> {
        self.model.as_ref()
    }

    fn has_anything(&self) -> bool {
        self.model.is_some() || !self.results.is_empty()
    }

    /// What the 3D view shows: the current results on the Results tab, else the FE model.
    fn shown(&self) -> Option<&Model> {
        match self.tree_view {
            TreeView::Results => self.results.get(self.current_result),
            _ => self.model.as_ref(),
        }
    }

    fn shown_mut(&mut self) -> Option<&mut Model> {
        match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => self.model.as_mut(),
        }
    }

    fn shown_results_mut(&mut self) -> Option<&mut ResultsView> {
        self.shown_mut()?.results.as_mut()
    }

    /// Whether clicks in the 3D view pick for an open dialog; only on the FE model.
    fn picking(&self) -> bool {
        self.tree_view != TreeView::Results && self.editor.as_ref().is_some_and(Editor::picks)
    }

    /// Switches the tree tab. The Results tab is a workspace of its own, as in PrePoMax: the
    /// 3D view swaps between the FE model and the current results, each with its own camera.
    fn set_tree_view(&mut self, view: TreeView) {
        if self.tree_view == view {
            return;
        }
        let was_results = self.tree_view == TreeView::Results;
        if was_results == (view == TreeView::Results) {
            self.tree_view = view;
            return;
        }
        // A fit still pending, e.g. right after loading, belongs to the workspace left.
        if let Some(command) = self.view_command.take() {
            let bounds = self.shown().and_then(Model::visible_bounds);
            self.viewport.apply(command, bounds);
        }
        self.tree_view = view;
        self.dialog = None;
        self.viewport.labels = Default::default();
        match &mut self.parked_camera {
            Some(camera) => self.viewport.swap_camera(camera),
            None => {
                let mut camera = plx_render::Camera::default();
                self.viewport.swap_camera(&mut camera);
                self.parked_camera = Some(camera);
                self.view_command = Some(ViewCommand::Fit);
            }
        }
        self.results_changed = true;
    }

    /// Shows another results file of the collection, like PrePoMax's Result box.
    fn select_result(&mut self, index: usize) {
        if index != self.current_result && index < self.results.len() {
            self.current_result = index;
            if let Some(view) = &self.results[index].results {
                self.tree.selected = Some((
                    TreeView::Results,
                    TreeItem::Component(view.field, view.component),
                ));
            }
            self.dialog = None;
            self.viewport.labels = Default::default();
            self.results_changed = true;
        }
    }

    /// Closes the current results file, or all of them.
    fn close_results(&mut self, all: bool) {
        let closed: Vec<Model> = if all {
            std::mem::take(&mut self.results)
        } else if self.current_result < self.results.len() {
            vec![self.results.remove(self.current_result)]
        } else {
            Vec::new()
        };
        for model in closed {
            self.output
                .push(format!("{} geschlossen", model.file_name()));
        }
        self.current_result = self
            .current_result
            .min(self.results.len().saturating_sub(1));
        if self.tree_view == TreeView::Results {
            self.dialog = None;
            self.tree.selected = None;
            self.results_changed = true;
        }
    }

    /// PrePoMax's Results menu.
    fn results_menu(&mut self, ui: &mut egui::Ui) {
        let any = !self.results.is_empty();
        if ui
            .add_enabled(any, egui::Button::new("Aktuelle Ergebnisse schließen"))
            .clicked()
        {
            self.close_results(false);
        }
        if ui
            .add_enabled(any, egui::Button::new("Alle Ergebnisse schließen"))
            .clicked()
        {
            self.close_results(true);
        }
    }

    /// The results row of the tool bar: PrePoMax's Result box with all opened results
    /// files, then the controls of the shown result.
    fn results_tool_bar(&mut self, ui: &mut egui::Ui) {
        let enabled = self.tree_view == TreeView::Results && !self.results.is_empty();
        ui.add_enabled_ui(enabled, |ui| self.results_tool_bar_row(ui, enabled));
    }

    fn results_tool_bar_row(&mut self, ui: &mut egui::Ui, enabled: bool) {
        ui.horizontal(|ui| {
            ui.label("Ergebnis");
            let mut selected = self.current_result;
            let current = self
                .results
                .get(selected)
                .map(|m| m.path.display().to_string());
            egui::ComboBox::from_id_salt("result file")
                .selected_text(current.clone().unwrap_or_default())
                .width(320.0)
                .truncate()
                .show_ui(ui, |ui| {
                    for (index, model) in self.results.iter().enumerate() {
                        ui.selectable_value(&mut selected, index, model.path.display().to_string());
                    }
                })
                .response
                .on_hover_text(current.unwrap_or_default());
            self.select_result(selected);
            ui.separator();
            // Greyed out, the row shows the current results file, or empty controls.
            let mut placeholder = None;
            let view = match self.results.get_mut(self.current_result) {
                Some(model) => model.results.as_mut(),
                None => None,
            }
            .unwrap_or_else(|| placeholder.insert(ResultsView::new(Vec::new(), None)));
            if results_tool_bar(ui, view) && enabled {
                self.results_changed = true;
            }
        });
    }

    fn create(&mut self, kind: NewItem) {
        if let Some(model) = self.setup_model() {
            self.editor = Editor::create(kind, &model.fe);
            self.set_tree_view(TreeView::FeModel);
        }
    }

    /// PrePoMax's Model menu: create items of the FE model.
    fn model_menu(&mut self, ui: &mut egui::Ui) {
        if self.setup_model().is_none() {
            ui.label("Zuerst eine .inp-Datei öffnen");
            return;
        }
        if ui.button("CalculiX-Keywords bearbeiten …").clicked() {
            self.open_keyword_editor();
        }
        ui.separator();
        let Some(model) = self.setup_model() else {
            return;
        };
        let last_step = model.fe.steps.len().checked_sub(1);
        let mut kind = None;
        for (item, label, enabled) in [
            (NewItem::Material, "Material erstellen …", true),
            (NewItem::Section, "Section erstellen …", true),
            (NewItem::Step, "Step erstellen …", true),
            (
                NewItem::BoundaryCondition(last_step.unwrap_or(0)),
                "Randbedingung erstellen …",
                last_step.is_some(),
            ),
            (
                NewItem::Load(last_step.unwrap_or(0)),
                "Last erstellen …",
                last_step.is_some(),
            ),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                kind = Some(item);
            }
        }
        if let Some(kind) = kind {
            self.create(kind);
        }
    }

    /// PrePoMax's Model > Edit CalculiX Keywords.
    fn open_keyword_editor(&mut self) {
        let Some(model) = self.setup_model() else {
            return;
        };
        let heading = format!("prepolix: {}", model.file_name());
        match KeywordEditor::new(&model.mesh, &model.fe, &heading) {
            Ok(editor) => self.keyword_editor = Some(editor),
            Err(error) => self
                .output
                .push(format!("Keyword-Editor nicht möglich: {error}")),
        }
    }

    /// PrePoMax's Material Library Editor, from the context menu of Materials.
    fn open_material_library(&mut self) {
        if let Some(model) = self.setup_model() {
            self.material_library = Some(MaterialLibraryEditor::new(&model.fe.materials));
        }
    }

    fn material_library_window(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.material_library else {
            return;
        };
        match editor.show(ctx) {
            LibraryResult::Open => {}
            LibraryResult::Ok(materials) => {
                self.material_library = None;
                if let Some(materials) = materials
                    && let Some(model) = self.model.as_mut()
                {
                    model.fe.materials = materials;
                    // Material indices may have changed.
                    self.tree.selected = None;
                    self.editor = None;
                }
            }
            LibraryResult::Cancel => self.material_library = None,
        }
    }

    fn keyword_editor_window(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.keyword_editor else {
            return;
        };
        match editor.show(ctx) {
            crate::keywords::EditorResult::Open => {}
            crate::keywords::EditorResult::Ok(keywords) => {
                self.keyword_editor = None;
                if let Some(model) = self.model.as_mut() {
                    model.fe.user_keywords = keywords;
                }
            }
            crate::keywords::EditorResult::Cancel => self.keyword_editor = None,
        }
    }

    fn analysis_menu(&mut self, ui: &mut egui::Ui) {
        let running = self.analysis.as_ref().is_some_and(Analysis::is_running);
        let can_start = self.setup_model().is_some() && !running;
        if ui
            .add_enabled(
                can_start,
                egui::Button::new("Analyse starten").shortcut_text("F5"),
            )
            .clicked()
        {
            self.start_analysis();
        }
        if ui
            .add_enabled(running, egui::Button::new("Analyse abbrechen"))
            .clicked()
            && let Some(analysis) = &mut self.analysis
        {
            analysis.kill();
        }
        if ui
            .add_enabled(self.analysis.is_some(), egui::Button::new("Monitor"))
            .clicked()
            && let Some(analysis) = &mut self.analysis
        {
            analysis.monitor = true;
        }
        let results = self.analysis.as_ref().and_then(Analysis::results);
        if ui
            .add_enabled(
                !running && results.is_some(),
                egui::Button::new("Ergebnisse öffnen"),
            )
            .clicked()
        {
            self.open_results = results;
        }
    }

    fn start_analysis(&mut self) {
        if self.analysis.as_ref().is_some_and(Analysis::is_running) {
            return;
        }
        if self.setup_model().is_none() {
            return;
        }
        let default_solver = self.settings.solver.default_solver();
        let Some(model) = self.setup_model() else {
            return;
        };
        match Analysis::start(&self.settings.solver, model, default_solver) {
            Ok(analysis) => {
                self.output.push(format!(
                    "Analyse gestartet: {}",
                    self.settings.solver.work_dir().display()
                ));
                self.analysis = Some(analysis);
            }
            Err(error) => self.output.push(error),
        }
    }

    /// Polls the running analysis and shows its monitor.
    fn run_analysis(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.start_analysis();
        }
        let Some(analysis) = &mut self.analysis else {
            return;
        };
        if let Some(status) = analysis.poll() {
            self.output
                .push(format!("Analyse {}", Analysis::status_text(status)));
        }
        if analysis.monitor
            && let MonitorEvent::OpenResults(path) = analysis.window(ctx)
        {
            self.open_results = Some(path);
        }
    }

    /// Saves mesh and FE model as a project, to the project file it came from, or to a file
    /// the user picks for a model opened from an input file or with `save_as`.
    fn save_project(&mut self, save_as: bool) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let path = if model.is_project() && !save_as {
            model.path.clone()
        } else {
            let stem = model
                .path
                .file_stem()
                .map_or_else(|| "Projekt".into(), |s| s.to_string_lossy().into_owned());
            let mut dialog = rfd::FileDialog::new()
                .set_title("Projekt speichern")
                .add_filter("prepolix-Projekt (*.plx)", &[model::PROJECT_EXTENSION])
                .set_file_name(format!("{stem}.{}", model::PROJECT_EXTENSION));
            if let Some(dir) = model.path.parent() {
                dialog = dialog.set_directory(dir);
            }
            let Some(mut path) = dialog.save_file() else {
                return;
            };
            if path.extension().is_none() {
                path.set_extension(model::PROJECT_EXTENSION);
            }
            path
        };
        match plx_io::project::save_project(&path, &model.mesh, &model.fe) {
            Ok(()) => {
                self.output.push(format!("{} gespeichert", path.display()));
                model.path = path;
            }
            Err(error) => self.output.push(format!("Nicht gespeichert: {error}")),
        }
    }

    /// Writes the input file of the set-up model to a file the user picks.
    fn export_inp(&mut self) {
        if self.setup_model().is_none() {
            return;
        }
        let default_solver = self.settings.solver.default_solver();
        let Some(model) = self.setup_model() else {
            return;
        };
        let heading = format!("prepolix: {}", model.file_name());
        let mut fe = model.fe.clone();
        fe.resolve_default_solver(default_solver);
        let text = match plx_io::inp::write_inp(&model.mesh, &fe, &heading) {
            Ok(text) => text,
            Err(error) => {
                self.output.push(format!("Export nicht möglich: {error}"));
                return;
            }
        };
        let picked = rfd::FileDialog::new()
            .set_title("CalculiX-Eingabedatei exportieren")
            .add_filter("Eingabedatei (*.inp)", &["inp"])
            .set_file_name(format!("{}.inp", crate::tree::ANALYSIS_NAME))
            .save_file();
        if let Some(path) = picked {
            match std::fs::write(&path, text) {
                Ok(()) => self.output.push(format!("{} geschrieben", path.display())),
                Err(error) => self.output.push(format!("{}: {error}", path.display())),
            }
        }
    }

    /// A click in the 3D view picks for the open dialog.
    fn click(&mut self, click: Click) {
        if !self.picking() {
            return;
        }
        let (Some(editor), Some(model)) = (&mut self.editor, &self.model) else {
            return;
        };
        let hit = model.pick(click.origin, click.direction);
        let pick = hit.as_ref().map(|hit| (hit, click.precision_at(hit.point)));
        editor.click(
            model,
            pick,
            Operation::from_modifiers(click.shift, click.ctrl),
        );
    }

    fn box_select(&mut self, area: &BoxSelect) {
        if !self.picking() {
            return;
        }
        if let (Some(editor), Some(model)) = (&mut self.editor, &self.model) {
            editor.box_select(
                model,
                area,
                Operation::from_modifiers(area.shift, area.ctrl),
            );
        }
    }

    /// Shows what a click would select where the mouse rests.
    fn hover(&mut self, hover: Option<Click>) {
        let hover = hover.filter(|_| self.picking());
        let preview = match (hover, &self.editor, &self.model) {
            (Some(click), Some(editor), Some(model)) => model
                .pick(click.origin, click.direction)
                .map(|hit| editor.preview(model, &hit, click.precision_at(hit.point)))
                .unwrap_or_default(),
            _ => Default::default(),
        };
        self.viewport.preview = preview;
    }

    fn editor_window(&mut self, ctx: &egui::Context) {
        let (Some(editor), Some(model)) = (&mut self.editor, &mut self.model) else {
            return;
        };
        match editor.show(ctx, model) {
            EditorResult::Open => {}
            EditorResult::Ok => {
                if let Some(editor) = self.editor.take() {
                    editor.apply(&mut model.fe);
                }
                self.highlighted = None;
            }
            EditorResult::Cancel => {
                self.editor = None;
                self.highlighted = None;
            }
        }
    }

    /// Highlights the region of the open dialog, or of the item selected in the tree.
    fn update_highlight(&mut self) {
        let Some(model) = &mut self.model else {
            return;
        };
        let highlight = if let Some(editor) = &self.editor {
            editor.highlight(model)
        } else {
            if self.highlighted == self.tree.selected {
                return;
            }
            self.highlighted = self.tree.selected.clone();
            match &self.tree.selected {
                Some((TreeView::FeModel, item)) => crate::setup::item_region(&model.fe, item)
                    .map(|region| crate::setup::region_highlight(model, region))
                    .unwrap_or_default(),
                _ => Default::default(),
            }
        };
        if highlight != model.highlight {
            model.highlight = highlight;
            self.results_changed = true;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(window) = &mut self.settings_window else {
            return;
        };
        let new = match window.show(ctx) {
            WindowResult::Open => return,
            WindowResult::Apply(settings) => settings,
            WindowResult::Ok(settings) => {
                self.settings_window = None;
                settings
            }
            WindowResult::Cancel => {
                self.settings_window = None;
                return;
            }
        };
        if new != self.settings {
            self.settings = new;
            self.update_contour();
        }
    }

    /// PrePoMax-style properties dialog of the double-clicked tree item.
    fn properties_window(&mut self, ctx: &egui::Context) {
        let Some(item) = self.dialog.clone() else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new(properties::title(self.shown(), &item))
            .id(egui::Id::new("properties window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                properties::show(ui, self.shown(), &item);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close = ui.button("Schließen").clicked();
                });
            });
        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dialog = None;
        }
    }

    /// Removes the model and all results, like PrePoMax's File > New.
    fn close_model(&mut self) {
        if let Some(model) = self.model.take() {
            self.output
                .push(format!("{} geschlossen", model.file_name()));
        }
        self.close_results(true);
        self.tree.selected = None;
        self.dialog = None;
        self.editor = None;
        self.highlighted = None;
        self.parked_camera = None;
        self.results_changed = true;
    }

    /// Plays the animation and shows its window; marks the scene for rebuilding.
    fn animate(&mut self, ctx: &egui::Context) {
        let model = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => self.model.as_mut(),
        };
        let Some(view) = model.and_then(|m| m.results.as_mut()) else {
            return;
        };
        let Some(animation) = &mut view.animation else {
            return;
        };
        if animation.playing {
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            if animation.tick(dt) {
                view.show_animation_frame();
                self.frame_changed = true;
            }
            ctx.request_repaint();
        }
        match animation_window(ctx, view) {
            WindowEvent::None => {}
            WindowEvent::Frame => {
                view.show_animation_frame();
                self.frame_changed = true;
            }
            WindowEvent::Settings => self.results_changed = true,
            WindowEvent::Close => {
                view.stop_animation();
                self.results_changed = true;
            }
        }
    }

    /// Rebuilds the scene after the result selection or deformation changed.
    fn rebuild_if_results_changed(&mut self) {
        let frame_only = !self.results_changed && self.frame_changed;
        if !(std::mem::take(&mut self.results_changed) | std::mem::take(&mut self.frame_changed)) {
            return;
        }
        let model = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => self.model.as_mut(),
        };
        let Some(model) = model else {
            self.frame_cache.clear();
            self.viewport.set_parts(&[]);
            self.update_contour();
            return;
        };
        let frame = model
            .results
            .as_ref()
            .and_then(|v| v.animation.as_ref())
            .map(|a| a.frame);
        if !frame_only {
            self.frame_cache.clear();
        }
        let fresh;
        let meshes: &Vec<RenderMesh> = match frame {
            Some(frame) => self
                .frame_cache
                .entry(frame)
                .or_insert_with(|| model.render_meshes()),
            None => {
                fresh = model.render_meshes();
                &fresh
            }
        };
        self.viewport.set_parts(meshes);
        for (index, (part, mesh)) in model.parts.iter_mut().zip(meshes).enumerate() {
            part.bounds = mesh.bounds();
            self.viewport.set_part_visible(index, part.visible);
        }
        self.update_contour();
    }

    /// Contour settings and annotations of the 3D view for the current result.
    fn update_contour(&mut self) {
        let model = match self.tree_view {
            TreeView::Results => self.results.get(self.current_result),
            _ => self.model.as_ref(),
        };
        let Some(model) = model else {
            self.viewport.options.contour_levels = None;
            self.viewport.overlay = Overlay {
                show_view_triad: self.settings.graphics.view_triad,
                ..Overlay::default()
            };
            return;
        };
        let view = model.results.as_ref();
        self.viewport.options.contour_levels =
            view.filter(|v| v.current().is_some()).map(|v| v.levels);
        let (graphics, post) = (&self.settings.graphics, &self.settings.post);
        let marker = |label: &str, extreme: Option<(usize, f32)>| {
            let (index, value) = extreme?;
            let value = value * view.map_or(1.0, ResultsView::amplitude);
            Some(Marker {
                position: model.node_position(index)?,
                text: format!(
                    "{label}: {}\nNode id: {}",
                    format_legend_value(value),
                    model.mesh.node_ids()[index]
                ),
            })
        };
        self.viewport.overlay = Overlay {
            legend: view.and_then(ResultsView::legend),
            status: view
                .filter(|_| post.status_block)
                .map_or_else(Vec::new, |v| v.status_lines(&model.file_name())),
            maximum: view
                .filter(|_| post.max_label)
                .and_then(|v| marker("Max", v.maximum())),
            minimum: view
                .filter(|_| post.min_label)
                .and_then(|v| marker("Min", v.minimum())),
            global_origin: graphics.global_axes.then(|| model.global_origin()),
            show_scale_bar: graphics.scale_bar,
            show_view_triad: graphics.view_triad,
            nodes: (model.highlight.nodes.iter())
                .filter_map(|&id| model.node_position(model.mesh.node_index(id)?))
                .collect(),
        };
    }
}

/// PrePoMax's results tool bar: deformation, colour bands and the increment with its
/// navigation buttons. Returns true when anything that affects the scene changed.
fn results_tool_bar(ui: &mut egui::Ui, view: &mut ResultsView) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Verformung");
        let before = view.deformation;
        egui::ComboBox::from_id_salt("deformation")
            .selected_text(view.deformation.label())
            .width(150.0)
            .show_ui(ui, |ui| {
                for choice in Deformation::CHOICES {
                    ui.selectable_value(&mut view.deformation, choice, choice.label());
                }
            });
        changed |= view.deformation != before;
        ui.label("Faktor");
        let user = view.deformation == Deformation::UserDefined;
        let mut factor = if user { view.user_scale } else { view.scale() };
        let response = ui.add_enabled(
            user,
            numeric::drag_value(&mut factor).speed(0.1).max_decimals(4),
        );
        if user && response.changed() {
            view.user_scale = factor;
            changed = true;
        }
        changed |= ui
            .checkbox(&mut view.show_undeformed, "Unverformt zeigen")
            .changed();
        ui.separator();
        ui.label("Farbstufen");
        changed |= ui
            .add(numeric::drag_value(&mut view.levels).range(2..=plx_render::contour::MAX_LEVELS))
            .changed();
        ui.separator();

        ui.label("Schritt, Inkrement");
        let mut increment = view.increment;
        let selected = view
            .current_increment()
            .map(ResultsView::increment_label)
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("increment")
            .selected_text(selected)
            .width(70.0)
            .show_ui(ui, |ui| {
                for (index, inc) in view.increments.iter().enumerate() {
                    ui.selectable_value(&mut increment, index, ResultsView::increment_label(inc));
                }
            });
        ui.spacing_mut().item_spacing.x = 1.0;
        let last = view.increments.len().saturating_sub(1);
        let current = view.increment;
        for (icon, tooltip, target) in [
            (Icon::First, "Erstes Inkrement", 0),
            (
                Icon::Previous,
                "Vorheriges Inkrement",
                current.saturating_sub(1),
            ),
            (Icon::Next, "Nächstes Inkrement", (current + 1).min(last)),
            (Icon::Last, "Letztes Inkrement", last),
        ] {
            let enabled = target != current && view.animation.is_none();
            if icons::button(ui, icon, tooltip, enabled, false).clicked() {
                increment = target;
            }
        }
        if increment != view.increment && view.animation.is_none() {
            view.select_increment(increment);
            changed = true;
        }
        ui.add_space(4.0);
        let animating = view.animation.is_some();
        if icons::button(ui, Icon::Animate, "Animation", true, animating).clicked() {
            if animating {
                view.stop_animation();
            } else {
                view.start_animation(AnimationKind::ScaleFactor);
            }
            changed = true;
        }
    });
    changed
}

enum WindowEvent {
    None,
    /// Another frame is shown.
    Frame,
    /// Settings changed that affect every frame.
    Settings,
    Close,
}

/// PrePoMax's animation dialog: kind, frames, speed, playback mode, colour limits and the
/// player controls.
fn animation_window(ctx: &egui::Context, view: &mut ResultsView) -> WindowEvent {
    let mut event = WindowEvent::None;
    let mut open = true;
    let mut restart = None;
    egui::Window::new("Animation")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(ctx.content_rect().right_bottom() + egui::vec2(-130.0, -230.0))
        .show(ctx, |ui| {
            let Some(animation) = &mut view.animation else {
                return;
            };
            egui::Grid::new("animation settings")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Art");
                    ui.horizontal(|ui| {
                        for (kind, label) in [
                            (AnimationKind::ScaleFactor, "Skalierung"),
                            (AnimationKind::Increments, "Inkremente des Steps"),
                        ] {
                            if ui.radio(animation.kind == kind, label).clicked()
                                && animation.kind != kind
                            {
                                restart = Some(kind);
                            }
                        }
                    });
                    ui.end_row();
                    if animation.kind == AnimationKind::ScaleFactor {
                        ui.label("Bilder");
                        let frames = numeric::drag_value(&mut animation.frames).range(2..=200);
                        if ui.add(frames).changed() {
                            animation.go_to(animation.frame);
                            event = WindowEvent::Settings;
                        }
                        ui.end_row();
                    }
                    ui.label("Bilder pro Sekunde");
                    ui.add(numeric::drag_value(&mut animation.fps).range(1.0..=60.0));
                    ui.end_row();
                    ui.label("Ablauf");
                    ui.horizontal(|ui| {
                        for (playback, label) in [
                            (Playback::Once, "Einmal"),
                            (Playback::Loop, "Schleife"),
                            (Playback::Swing, "Hin und her"),
                        ] {
                            ui.radio_value(&mut animation.playback, playback, label);
                        }
                    });
                    ui.end_row();
                    ui.label("Farbskala");
                    ui.horizontal(|ui| {
                        for (limits, label) in [
                            (ColorLimits::CurrentFrame, "Aktuelles Bild"),
                            (ColorLimits::AllFrames, "Alle Bilder"),
                        ] {
                            if ui
                                .radio_value(&mut animation.limits, limits, label)
                                .changed()
                            {
                                event = WindowEvent::Settings;
                            }
                        }
                    });
                    ui.end_row();
                });
            ui.separator();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                let last = animation.frame_count() - 1;
                let frame = animation.frame;
                let mut target = None;
                if icons::button(ui, Icon::First, "Erstes Bild", frame > 0, false).clicked() {
                    target = Some(0);
                }
                if icons::button(ui, Icon::Previous, "Vorheriges Bild", frame > 0, false).clicked()
                {
                    target = Some(frame - 1);
                }
                let (icon, tip) = if animation.playing {
                    (Icon::Pause, "Anhalten")
                } else {
                    (Icon::Animate, "Abspielen")
                };
                if icons::button(ui, icon, tip, true, false).clicked() {
                    if animation.playing {
                        animation.playing = false;
                    } else {
                        animation.play();
                    }
                }
                if icons::button(ui, Icon::Next, "Nächstes Bild", frame < last, false).clicked() {
                    target = Some(frame + 1);
                }
                if icons::button(ui, Icon::Last, "Letztes Bild", frame < last, false).clicked() {
                    target = Some(last);
                }
                ui.add_space(8.0);
                let mut slider = frame;
                let response = ui.add(egui::Slider::new(&mut slider, 0..=last).show_value(false));
                if response.changed() {
                    target = Some(slider);
                }
                ui.add_space(8.0);
                ui.label(format!("Bild {} von {}", frame + 1, last + 1));
                if let Some(target) = target {
                    animation.playing = false;
                    if animation.go_to(target) && matches!(event, WindowEvent::None) {
                        event = WindowEvent::Frame;
                    }
                }
            });
        });
    if let Some(kind) = restart {
        view.start_animation(kind);
        event = WindowEvent::Settings;
    }
    if !open {
        event = WindowEvent::Close;
    }
    event
}
