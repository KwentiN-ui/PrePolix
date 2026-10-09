use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use plx_render::StandardView;

use crate::analysis::{Analysis, MonitorEvent};
use crate::animation::{AnimationKind, ColorLimits, Playback};
use crate::field_output_dialog::{DialogAction, FieldOutputDialog};
use crate::history_output_dialog::HistoryOutputDialog;
use crate::history_table::HistoryTable;
use crate::icons::{self, Icon};
use crate::keywords::KeywordEditor;
use crate::material_library::{LibraryResult, MaterialLibraryEditor};
use crate::meshing::{
    MeshItemEditor, MeshItemResult, MeshSetupResult, MeshSetupWindow, MeshingJob,
};
use crate::model::{self, Highlight, LoadedModel, Model};
use crate::numeric;
use crate::overlay::{Marker, Overlay};
use crate::properties;
use crate::results::{Deformation, ResultsView, format_legend_value};
use crate::screenshot::{self, Screenshot};
use crate::section::{SectionDialog, SectionResult, SectionView};
use crate::selection::Operation;
use crate::settings::{self, Settings, SettingsWindow, WindowResult};
use crate::setup::{Editor, EditorResult, NewItem};
use crate::sound::{self, ModeSound};
use crate::symbols;
use crate::transformation_dialog::{TransformationAction, TransformationDialog};
use crate::tree::{self, AnalysisAction, TreeItem, TreeResponse, TreeState, TreeView};
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
    /// The CAD geometry of the FE model as shown on the Geometry tab.
    geometry: Option<Model>,
    /// Open window of the default meshing parameters.
    mesh_setup: Option<MeshSetupWindow>,
    /// Open dialog of a mesh setup item.
    mesh_item_editor: Option<MeshItemEditor>,
    meshing: Option<MeshingJob>,
    /// The results workspace: every results file opened in this session, PrePoMax's results
    /// collection. One of them is shown on the Results tab.
    results: Vec<Model>,
    current_result: usize,
    /// Camera of the workspace not shown, so switching tabs keeps each view.
    parked_camera: Option<plx_render::Camera>,
    tree: TreeState,
    /// Item whose properties window is open.
    dialog: Option<TreeItem>,
    /// Name typed in the properties window of a part, with why it cannot be taken.
    part_name: (String, Option<String>),
    /// Part the open context menu of the 3D view belongs to.
    menu_part: Option<usize>,
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
    /// Open dialog creating or editing a field output derived from the shown results.
    field_output_dialog: Option<FieldOutputDialog>,
    /// Open dialog creating or editing a history output of the shown results.
    history_dialog: Option<HistoryOutputDialog>,
    /// Open table of a history output component.
    history_table: Option<HistoryTable>,
    /// The tree selection whose region is highlighted.
    highlighted: Option<(TreeView, TreeItem)>,
    analysis: Option<Analysis>,
    /// Results file the user asked to open; read by the app on a worker thread.
    open_results: Option<PathBuf>,
    screenshot: Screenshot,
    /// Hot spot whose paths are shown in the FE model, edited or selected, with the paths.
    hot_spot_preview: Option<(plx_model::HotSpot, Vec<Vec<glam::Vec3>>)>,
    /// The table of hot spot values is open on the Results tab.
    hot_spot_window: bool,
    /// Audio output of the sound window, opened when it first plays.
    audio: Option<sound::Player>,
    /// The section view, while it is on; it cuts whatever the 3D view shows.
    section: Option<SectionView>,
    section_dialog: Option<SectionDialog>,
    /// Open dialog of the transformations of the current results.
    transformation_dialog: Option<TransformationDialog>,
    /// The section view shown in the scene of the given version, to rebuild it on changes.
    section_shown: Option<(u64, SectionView)>,
    /// The boundary conditions and loads, with the shown parts, whose symbols are drawn.
    symbols_shown: Option<(Vec<symbols::Item>, Vec<bool>)>,
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
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, settings::STORAGE_KEY))
            .unwrap_or_default();
        plx_mesher::set_library_path(settings.gmsh.library());
        let app = Self {
            workbench: Workbench {
                settings,
                settings_window: None,
                viewport: Viewport::new(render_state),
                model: None,
                geometry: None,
                mesh_setup: None,
                mesh_item_editor: None,
                meshing: None,
                results: Vec::new(),
                current_result: 0,
                parked_camera: None,
                tree: TreeState::default(),
                dialog: None,
                part_name: Default::default(),
                menu_part: None,
                tree_view: TreeView::FeModel,
                output,
                view_command: None,
                results_changed: false,
                frame_changed: false,
                frame_cache: Default::default(),
                editor: None,
                keyword_editor: None,
                material_library: None,
                field_output_dialog: None,
                history_dialog: None,
                history_table: None,
                highlighted: None,
                analysis: None,
                open_results: None,
                screenshot: Screenshot::default(),
                hot_spot_preview: None,
                hot_spot_window: false,
                audio: None,
                section: None,
                section_dialog: None,
                transformation_dialog: None,
                section_shown: None,
                symbols_shown: None,
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
                    "Projekt, CalculiX-Modell, -Ergebnisse oder Geometrie",
                    &[
                        "plx", "PLX", "inp", "INP", "frd", "FRD", "step", "STEP", "stp", "STP",
                        "iges", "IGES", "igs", "IGS", "brep", "BREP",
                    ],
                )
                .add_filter("prepolix-Projekt (*.plx)", &["plx", "PLX"])
                .add_filter("Eingabedatei (*.inp)", &["inp", "INP"])
                .add_filter("Ergebnisdatei (*.frd)", &["frd", "FRD"])
                .add_filter(GEOMETRY_FILTER.0, GEOMETRY_FILTER.1)
                .pick_file();
            if let Some(path) = picked {
                load_in_background(path, sender, ctx);
            }
        });
    }

    /// PrePoMax's Geometry > Import: a STEP, IGES or BREP file.
    fn import_dialog(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() {
            return;
        }
        let sender = self.load_events.0.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Geometrie importieren")
                .add_filter(GEOMETRY_FILTER.0, GEOMETRY_FILTER.1)
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
                ui.menu_button("Ansicht senkrecht zu", |ui| {
                    for axis in Axis::ALL {
                        if ui.button(axis.label()).clicked() {
                            self.workbench.view_command = Some(ViewCommand::AxisView(axis));
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
                ui.separator();
                let has_model = self.workbench.shown().is_some();
                if ui
                    .add_enabled(has_model, egui::Button::new("Schnittansicht …"))
                    .clicked()
                {
                    self.workbench.open_section_dialog();
                }
                let active = self.workbench.section.is_some();
                if ui
                    .add_enabled(active, egui::Button::new("Schnittansicht aus"))
                    .clicked()
                {
                    self.workbench.section = None;
                    self.workbench.section_dialog = None;
                }
            });
            ui.menu_button("Geometrie", |ui| {
                let import = egui::Button::new("Importieren …");
                if ui.add_enabled(self.loading.is_none(), import).clicked() {
                    self.import_dialog(ui.ctx());
                }
            });
            ui.menu_button("Netz", |ui| self.workbench.mesh_menu(ui));
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
            let import = "Geometrie importieren (STEP, IGES, BREP)";
            if icons::button(ui, Icon::Import, import, can_open, false).clicked() {
                self.import_dialog(ui.ctx());
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
            ui.separator();
            let shown = self.workbench.shown().is_some();
            let sectioned =
                self.workbench.section.is_some() || self.workbench.section_dialog.is_some();
            if icons::button(ui, Icon::SectionView, "Schnittansicht", shown, sectioned).clicked() {
                self.workbench.open_section_dialog();
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match (&self.loading, self.workbench.shown()) {
            (Some(path), _) => {
                ui.spinner();
                ui.label(format!("Lade {} …", path.display()));
            }
            (None, _) if self.workbench.meshing.is_some() => {
                ui.spinner();
                ui.label("Netz wird erzeugt …");
            }
            (None, Some(model)) if model.is_geometry() => {
                ui.label(format!(
                    "{}: {} Parts",
                    model.file_name(),
                    model.parts.len()
                ));
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

/// Whether a region of the FE model consists of picked nodes or element faces, which a new
/// mesh does not keep.
fn picks_mesh_entities(fe: &plx_model::FeModel) -> bool {
    use plx_model::Region;
    let picked = |region: &Region| matches!(region, Region::Nodes(_) | Region::Faces(_));
    fe.sections.iter().any(|s| picked(&s.region))
        || fe.steps.iter().any(|step| {
            step.boundary_conditions.iter().any(|b| picked(&b.region))
                || step.loads.iter().any(|l| picked(&l.region))
        })
}

/// File dialog filter of the CAD formats Gmsh imports.
const GEOMETRY_FILTER: (&str, &[&str]) = (
    "Geometrie (*.step, *.stp, *.iges, *.igs, *.brep)",
    &[
        "step", "STEP", "stp", "STP", "iges", "IGES", "igs", "IGS", "brep", "BREP",
    ],
);

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
        self.workbench.play_sound(&ctx);
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
                let selected_tab = self.workbench.tree_tabs(ui);
                // Tabs sit flush on the pane: their bottom border is the pane's top border.
                ui.add_space(-ui.spacing().item_spacing.y - 1.0);
                pane.inner_margin(4).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let view = self.workbench.tree_view;
                    self.workbench.model_tree(ui, view);
                });
                // The selected tab opens into the pane like a Windows tab control.
                if let Some(tab) = selected_tab {
                    ui.painter().hline(
                        tab.x_range().shrink(1.0),
                        tab.bottom() - 0.5,
                        egui::Stroke::new(1.0, crate::style::WINDOW),
                    );
                }
            });
        egui::Panel::bottom("output")
            .resizable(true)
            .default_size(140.0)
            .size_range(40.0..=600.0)
            .frame(pane.inner_margin(4))
            .show(ui, |ui| self.workbench.output(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.workbench.viewport.selecting = self.workbench.picking()
                || self.workbench.section_picks()
                || self.workbench.transformation_picks();
            self.workbench.viewport.gizmo =
                match (&self.workbench.section_dialog, self.workbench.shown()) {
                    (Some(dialog), Some(model)) => Some(dialog.gizmo(model)),
                    _ => None,
                };
            let response = self.workbench.viewport.ui(ui);
            if let (Some(drag), Some(dialog)) = (response.gizmo, &mut self.workbench.section_dialog)
            {
                dialog.drag(drag);
            }
            if let Some(command) = response.command {
                self.workbench.view_command = Some(command);
            }
            if let Some(click) = response.click {
                self.workbench.click(click);
            }
            if let Some(view) = &response.response {
                self.workbench.viewport_menu(view, response.secondary_click);
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
        self.workbench.section_window(&ctx);
        self.workbench.keyword_editor_window(&ctx);
        self.workbench.material_library_window(&ctx);
        self.workbench.mesh_setup_window(&ctx);
        self.workbench.mesh_item_window(&ctx);
        self.workbench.poll_meshing();
        self.workbench.field_output_window(&ctx);
        self.workbench.history_output_window(&ctx);
        self.workbench.history_table_window(&ctx);
        self.workbench.transformation_window(&ctx);
        self.workbench.run_analysis(&ctx);
        if let Some(path) = self.workbench.open_results.take() {
            self.open_path(path, &ctx);
        }
        self.workbench.update_highlight();
        self.workbench.update_symbols(&ctx);
        self.workbench.update_hot_spot_preview();
        self.workbench.hot_spot_window(&ctx);
        self.workbench.settings_window(&ctx);
        self.workbench.rebuild_if_results_changed();
        self.workbench.update_section();

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
                    geometry_view,
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
                match &geometry_view {
                    Some(view) if model.mesh.element_count() == 0 => {
                        self.output.push(format!(
                            "{} importiert: {} Parts ({} ms)",
                            path.display(),
                            view.parts.len(),
                            model.load_time.as_millis()
                        ));
                    }
                    _ => self.output.push(format!(
                        "{} geladen: {} Knoten, {} Elemente, {} Parts ({} ms)",
                        path.display(),
                        model.mesh.node_count(),
                        model.mesh.element_count(),
                        model.parts.len(),
                        model.load_time.as_millis()
                    )),
                }
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
                self.field_output_dialog = None;
                self.close_history_windows();
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
                    // Results of the FE model get its hot spots evaluated right away.
                    self.evaluate_hot_spots(true);
                } else {
                    self.tree.selected = None;
                    self.editor = None;
                    self.highlighted = None;
                    self.mesh_setup = None;
                    self.mesh_item_editor = None;
                    self.meshing = None;
                    // A model without a mesh yet opens on the Geometry tab.
                    let view = if geometry_view.is_some() && model.mesh.element_count() == 0 {
                        TreeView::Geometry
                    } else {
                        TreeView::FeModel
                    };
                    self.set_tree_view(view);
                    self.model = Some(model);
                    self.symbols_shown = None;
                    self.geometry = geometry_view;
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
    /// Returns the rect of the selected tab.
    fn tree_tabs(&mut self, ui: &mut egui::Ui) -> Option<egui::Rect> {
        let mut selected_rect = None;
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
                    selected_rect = Some(rect);
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
        selected_rect
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
            TreeView::Geometry => self.geometry.is_none().then_some("Keine Geometrie geladen"),
            TreeView::FeModel => self.model.is_none().then_some("Kein Modell geladen"),
            TreeView::Results => self
                .results
                .is_empty()
                .then_some("Keine Ergebnisse geladen"),
        };
        if let Some(text) = empty {
            let hint = if view == TreeView::Geometry {
                "Geometrie > Importieren oder eine STEP-, IGES- oder BREP-Datei ins Fenster ziehen."
            } else {
                "Datei > Öffnen (Strg+O) oder eine .plx-, .inp- oder .frd-Datei ins Fenster ziehen."
            };
            ui.weak(format!("{text}.\n{hint}"));
            ui.separator();
        }
        let mesh_items: Vec<String> = (self.model.as_ref())
            .and_then(|m| m.geometry.as_ref())
            .map(|g| g.mesh_items.iter().map(|i| i.name.clone()).collect())
            .unwrap_or_default();
        let shown = match view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        };
        let job = self.analysis.as_ref().map(|a| tree::JobState {
            status: a.status(),
            results: a.results().is_some(),
        });
        let response = tree::show(ui, view, shown, &mesh_items, job, &mut self.tree);
        self.tree_response(ui.ctx(), view, response);
    }

    /// Acts on what the user picked in the tree or in a part's context menu in the 3D view.
    fn tree_response(&mut self, ctx: &egui::Context, view: TreeView, response: TreeResponse) {
        for (index, visible) in response.visibility {
            if let Some(part) = self.shown_mut().and_then(|m| m.parts.get_mut(index)) {
                part.visible = visible;
            }
            self.viewport.set_part_visible(index, visible);
        }
        if let (Some((field, component)), Some(results)) =
            (response.component, self.shown_results_mut())
        {
            results.field = field;
            results.component = component;
            self.results_changed = true;
        }
        if let Some(TreeItem::Part(index)) = response.open {
            let name = self.shown().and_then(|m| m.parts.get(index));
            self.part_name = (name.map(|p| p.name.clone()).unwrap_or_default(), None);
            self.dialog = Some(TreeItem::Part(index));
        } else if let Some(TreeItem::ResultFieldOutput(field)) = response.open {
            self.edit_field_output(field);
        } else if let Some(TreeItem::HistorySet(set)) = response.open {
            self.edit_history_output(set);
        } else if let Some(TreeItem::HistoryComponent(set, field, component)) = response.open {
            self.history_table = Some(HistoryTable {
                set,
                field,
                component,
            });
        } else if let Some(TreeItem::MeshItem(index)) = response.open {
            let geometry = self.model.as_ref().and_then(|m| m.geometry.as_ref());
            self.mesh_item_editor = geometry.and_then(|g| MeshItemEditor::edit(g, index));
        } else if let Some(item) = response.open {
            let model = self.model.as_ref().filter(|_| view != TreeView::Results);
            match model.and_then(|m| Editor::edit(&item, &m.fe, &m.mesh)) {
                Some(editor) => self.editor = Some(editor),
                None => self.dialog = Some(item),
            }
        }
        if let Some(kind) = response.create {
            self.create(kind);
        }
        if let Some(TreeItem::ResultFieldOutput(field)) = response.delete {
            self.delete_field_output(field);
        } else if let Some(TreeItem::HistorySet(set)) = response.delete {
            self.delete_history_output(set);
        } else if let Some(TreeItem::MeshItem(index)) = response.delete {
            if let Some(geometry) = self.model.as_mut().and_then(|m| m.geometry.as_mut())
                && index < geometry.mesh_items.len()
            {
                geometry.mesh_items.remove(index);
                self.tree.selected = None;
                self.mesh_item_editor = None;
            }
        } else if let (Some(item), Some(model)) = (response.delete, self.model.as_mut())
            && crate::setup::delete(&mut model.fe, &item)
        {
            self.tree.selected = None;
            self.editor = None;
        }
        if let (Some(item), Some(model)) = (response.toggle_active, self.model.as_mut()) {
            crate::setup::toggle_active(&mut model.fe, &item);
        }
        if let Some(action) = response.analysis {
            self.analysis_action(action);
        }
        if response.material_library {
            self.open_material_library();
        }
        if response.mesh_defaults {
            self.open_mesh_setup();
        }
        if response.generate_mesh {
            self.generate_mesh(ctx, None);
        }
        if let Some(index) = response.mesh_part {
            let name = (self.geometry.as_ref()).and_then(|g| g.parts.get(index));
            if let Some(name) = name.map(|p| p.name.clone()) {
                self.generate_mesh(ctx, Some(vec![name]));
            }
        }
        if response.evaluate_hot_spots {
            self.evaluate_hot_spots(false);
        }
    }

    /// Evaluates the hot spots of the FE model on the current results file, writes the values
    /// next to it and opens the table. `automatic` skips the messages when there is nothing
    /// to evaluate.
    fn evaluate_hot_spots(&mut self, automatic: bool) {
        let fe = self.model.as_ref().filter(|m| !m.fe.hot_spots.is_empty());
        let results = self.results.get_mut(self.current_result);
        let (Some(fe), Some(results)) = (fe, results) else {
            if !automatic {
                self.output.push(
                    "Hot Spots: erst Hot Spots im FE-Modell definieren und Ergebnisse öffnen."
                        .into(),
                );
            }
            return;
        };
        match crate::hot_spots::evaluate(fe, results) {
            Ok(reports) => {
                let file = match crate::hot_spots::write(&results.path, &reports) {
                    Ok(file) => {
                        self.output
                            .push(format!("Hot Spots ausgewertet: {}", file.display()));
                        Some(file)
                    }
                    Err(error) => {
                        self.output.push(error);
                        None
                    }
                };
                self.output.extend(crate::hot_spots::summary(&reports));
                for report in &reports {
                    self.output
                        .extend(report.warnings.iter().map(|w| format!("Warnung: {w}")));
                }
                results.hot_spots = Some(crate::hot_spots::Evaluation { reports, file });
                self.hot_spot_window = true;
                self.set_tree_view(TreeView::Results);
                self.update_contour();
            }
            Err(error) => {
                if !automatic || !fe.fe.hot_spots.is_empty() {
                    self.output
                        .push(format!("Hot Spots nicht ausgewertet: {error}"));
                }
            }
        }
    }

    /// The table of hot spot values of the shown results file.
    fn hot_spot_window(&mut self, ctx: &egui::Context) {
        if !self.hot_spot_window || self.tree_view != TreeView::Results {
            return;
        }
        let Some(model) = self.results.get(self.current_result) else {
            return;
        };
        let Some(evaluation) = &model.hot_spots else {
            return;
        };
        let step = (model.results.as_ref())
            .and_then(ResultsView::current_increment)
            .map(|i| (i.step, i.increment));
        if !crate::hot_spots::window(ctx, evaluation, step) {
            self.hot_spot_window = false;
            self.update_contour();
        }
    }

    /// Shows the paths of the hot spot being edited or selected in the FE model.
    fn update_hot_spot_preview(&mut self) {
        let wanted = match (&self.editor, &self.tree.selected) {
            _ if self.tree_view == TreeView::Results => None,
            (Some(editor), _) => editor.hot_spot(),
            (None, Some((TreeView::FeModel, TreeItem::HotSpot(i)))) => self
                .model
                .as_ref()
                .and_then(|m| m.fe.hot_spots.get(*i).cloned()),
            _ => None,
        };
        if wanted.as_ref() == self.hot_spot_preview.as_ref().map(|(h, _)| h) {
            return;
        }
        let paths = match (&wanted, &self.model) {
            (Some(hot_spot), Some(model)) => crate::hot_spots::preview(model, hot_spot),
            _ => Vec::new(),
        };
        self.hot_spot_preview = wanted.map(|h| (h, paths));
        self.viewport.overlay.paths = self.overlay_paths();
    }

    /// Hot spot paths drawn over the 3D view: of the evaluated results while their table is
    /// open, otherwise of the hot spot edited or selected in the FE model.
    fn overlay_paths(&self) -> Vec<Vec<glam::Vec3>> {
        if self.tree_view == TreeView::Results {
            let model = self.results.get(self.current_result);
            return match model.and_then(|m| Some((m, m.hot_spots.as_ref()?))) {
                Some((model, evaluation)) if self.hot_spot_window => {
                    crate::hot_spots::result_paths(model, evaluation)
                }
                _ => Vec::new(),
            };
        }
        self.hot_spot_preview
            .as_ref()
            .map_or_else(Vec::new, |(_, paths)| paths.clone())
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
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        }
    }

    fn shown_mut(&mut self) -> Option<&mut Model> {
        match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        }
    }

    fn shown_results_mut(&mut self) -> Option<&mut ResultsView> {
        self.shown_mut()?.results.as_mut()
    }

    /// Whether clicks in the 3D view pick for an open dialog: an item dialog of the FE model
    /// or a history output dialog of the results.
    fn picking(&self) -> bool {
        match self.tree_view {
            TreeView::Results => self
                .history_dialog
                .as_ref()
                .is_some_and(HistoryOutputDialog::picks),
            TreeView::FeModel => self.editor.as_ref().is_some_and(Editor::picks),
            TreeView::Geometry => {
                (self.mesh_item_editor.as_ref()).is_some_and(MeshItemEditor::picks)
            }
        }
    }

    /// The results shown on the Results tab.
    fn shown_results_view(&self) -> Option<&ResultsView> {
        self.results.get(self.current_result)?.results.as_ref()
    }

    fn close_history_windows(&mut self) {
        self.history_dialog = None;
        self.history_table = None;
    }

    /// Switches the tree tab. The Results tab is a workspace of its own, as in PrePoMax: the
    /// 3D view swaps between the FE model and the current results, each with its own camera.
    fn set_tree_view(&mut self, view: TreeView) {
        if self.tree_view == view {
            return;
        }
        let was_results = self.tree_view == TreeView::Results;
        if was_results == (view == TreeView::Results) {
            // Geometry and FE model share the camera; the scene changes.
            self.tree_view = view;
            self.results_changed = true;
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
            self.field_output_dialog = None;
            self.transformation_dialog = None;
            self.close_history_windows();
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
        self.field_output_dialog = None;
        self.transformation_dialog = None;
        self.close_history_windows();
        if self.tree_view == TreeView::Results {
            self.dialog = None;
            self.tree.selected = None;
            self.results_changed = true;
        }
    }

    /// PrePoMax's Results menu.
    fn results_menu(&mut self, ui: &mut egui::Ui) {
        let any = !self.results.is_empty();
        let hot_spots = self
            .model
            .as_ref()
            .is_some_and(|m| !m.fe.hot_spots.is_empty());
        if ui
            .add_enabled(any && hot_spots, egui::Button::new("Hot Spots auswerten"))
            .clicked()
        {
            self.evaluate_hot_spots(false);
        }
        let evaluated =
            (self.results.get(self.current_result)).is_some_and(|m| m.hot_spots.is_some());
        if ui
            .add_enabled(evaluated, egui::Button::new("Hot-Spot-Tabelle"))
            .clicked()
        {
            self.hot_spot_window = true;
            self.set_tree_view(TreeView::Results);
            self.update_contour();
        }
        ui.separator();
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
            let transformed = !view.transformations.is_empty();
            let response = results_tool_bar(ui, view, transformed);
            if response.changed && enabled {
                self.results_changed = true;
            }
            if response.transformations && enabled {
                self.open_transformation_dialog();
            }
        });
    }

    fn open_transformation_dialog(&mut self) {
        let index = self.current_result;
        if let Some(model) = self.results.get(index)
            && let Some(view) = &model.results
        {
            let dialog = TransformationDialog::new(index, &view.transformations, model);
            self.transformation_dialog = Some(dialog);
            self.update_contour();
        }
    }

    fn transformation_picks(&self) -> bool {
        (self.transformation_dialog.as_ref())
            .is_some_and(|d| d.picks() && self.tree_view == TreeView::Results)
    }

    fn transformation_window(&mut self, ctx: &egui::Context) {
        if self.tree_view != TreeView::Results {
            self.transformation_dialog = None;
        }
        let Some(dialog) = &mut self.transformation_dialog else {
            return;
        };
        let Some(model) = self.results.get_mut(dialog.result) else {
            self.transformation_dialog = None;
            return;
        };
        let shown = (dialog.lines(model), dialog.points(model));
        let action = dialog.show(ctx, model);
        // The overlay shows the points and axis of the selected item.
        let moved = shown != (dialog.lines(model), dialog.points(model));
        let (transformations, close) = match action {
            TransformationAction::Open => {
                if moved {
                    self.update_contour();
                }
                return;
            }
            TransformationAction::Cancel => {
                self.transformation_dialog = None;
                self.viewport.preview = Default::default();
                self.update_contour();
                return;
            }
            TransformationAction::Apply {
                transformations,
                close,
            } => (transformations, close),
        };
        if let Some(view) = &mut model.results {
            view.transformations = transformations;
            self.results_changed = true;
            // The copies extend the scene; show all of it.
            self.view_command = Some(ViewCommand::Fit);
        }
        if close {
            self.transformation_dialog = None;
            self.viewport.preview = Default::default();
        }
        self.update_contour();
    }

    fn create(&mut self, kind: NewItem) {
        if kind == NewItem::ResultHistoryOutput {
            if let Some(view) = self.shown_results_view() {
                self.history_dialog = Some(HistoryOutputDialog::create(view));
            }
            return;
        }
        if kind == NewItem::MeshSetupItem {
            if let Some(geometry) = self.model.as_ref().and_then(|m| m.geometry.as_ref()) {
                self.mesh_item_editor = Some(MeshItemEditor::create(geometry));
                self.set_tree_view(TreeView::Geometry);
            }
            return;
        }
        if kind == NewItem::ResultFieldOutput {
            if let Some(model) = self.results.get(self.current_result)
                && let Some(view) = &model.results
            {
                self.field_output_dialog = Some(FieldOutputDialog::create(view, &model.mesh));
            }
            return;
        }
        if let Some(model) = self.setup_model() {
            self.editor = Editor::create(kind, &model.fe);
            self.set_tree_view(TreeView::FeModel);
        }
    }

    /// PrePoMax's Mesh menu: the mesh setup and mesh generation for the geometry.
    fn mesh_menu(&mut self, ui: &mut egui::Ui) {
        let has_geometry = self.model.as_ref().is_some_and(|m| m.geometry.is_some());
        if !has_geometry {
            ui.label("Zuerst eine Geometrie importieren");
            return;
        }
        if ui.button("Mesh-Setup-Eintrag erstellen …").clicked() {
            self.create(NewItem::MeshSetupItem);
        }
        if ui.button("Standard-Netzparameter …").clicked() {
            self.open_mesh_setup();
        }
        ui.separator();
        let mesh = egui::Button::new("Alle Parts vernetzen");
        if ui.add_enabled(self.meshing.is_none(), mesh).clicked() {
            self.generate_mesh(ui.ctx(), None);
        }
    }

    fn open_mesh_setup(&mut self) {
        if let Some(geometry) = self.model.as_ref().and_then(|m| m.geometry.as_ref()) {
            self.mesh_setup = Some(MeshSetupWindow::new(&geometry.meshing));
        }
    }

    fn mesh_setup_window(&mut self, ctx: &egui::Context) {
        let Some(window) = &mut self.mesh_setup else {
            return;
        };
        let (setup, mesh) = match window.show(ctx) {
            MeshSetupResult::Open => return,
            MeshSetupResult::Cancel => (None, false),
            MeshSetupResult::Ok(setup) => (Some(setup), false),
            MeshSetupResult::Mesh(setup) => (Some(setup), true),
        };
        self.mesh_setup = None;
        if let (Some(setup), Some(geometry)) =
            (setup, self.model.as_mut().and_then(|m| m.geometry.as_mut()))
        {
            geometry.meshing = setup;
        }
        if mesh {
            self.generate_mesh(ctx, None);
        }
    }

    /// The dialog of a mesh setup item; OK stores the item in the geometry.
    fn mesh_item_window(&mut self, ctx: &egui::Context) {
        let (Some(editor), Some(geometry)) = (
            &mut self.mesh_item_editor,
            self.model.as_mut().and_then(|m| m.geometry.as_mut()),
        ) else {
            self.mesh_item_editor = None;
            return;
        };
        match editor.show(ctx, geometry, self.geometry.as_ref()) {
            MeshItemResult::Open => return,
            MeshItemResult::Cancel => {}
            MeshItemResult::Ok(index, item) => {
                let index = match index.filter(|&i| i < geometry.mesh_items.len()) {
                    Some(index) => {
                        geometry.mesh_items[index] = item;
                        index
                    }
                    None => {
                        geometry.mesh_items.push(item);
                        geometry.mesh_items.len() - 1
                    }
                };
                self.tree.selected = Some((TreeView::Geometry, TreeItem::MeshItem(index)));
            }
        }
        self.mesh_item_editor = None;
        self.viewport.preview = Default::default();
    }

    /// Meshes the named parts of the geometry, or all with `None`, on a worker thread;
    /// [`Self::poll_meshing`] takes the result.
    fn generate_mesh(&mut self, ctx: &egui::Context, parts: Option<Vec<String>>) {
        if self.meshing.is_some() {
            return;
        }
        let Some(geometry) = self.model.as_ref().and_then(|m| m.geometry.clone()) else {
            return;
        };
        let what = match &parts {
            Some(parts) => parts.join(", "),
            None => geometry.source.clone(),
        };
        self.output.push(format!("Vernetze {what} …"));
        self.meshing = Some(MeshingJob::start(geometry, parts, ctx));
    }

    fn poll_meshing(&mut self) {
        let Some(result) = self.meshing.as_ref().and_then(MeshingJob::poll) else {
            return;
        };
        let started = self.meshing.take().map(|job| job.started);
        let Some(model) = self.model.as_mut() else {
            return;
        };
        match result {
            Ok(generated) => {
                let had_mesh = model.mesh.element_count() > 0;
                let mut mesh = model.mesh.clone();
                for part in generated.meshes {
                    for warning in &part.warnings {
                        self.output.push(format!("Gmsh: {warning}"));
                    }
                    for p in &part.mesh.parts {
                        let parameters = model.geometry.as_ref().map(|g| g.parameters(&p.name));
                        self.output.push(format!(
                            "{}: {} Elemente ({}. Ordnung, Elementgröße {} bis {})",
                            p.name,
                            p.elements.len(),
                            if parameters.is_some_and(|s| s.second_order) {
                                2
                            } else {
                                1
                            },
                            parameters.map_or(0.0, |s| s.min_size),
                            parameters.map_or(0.0, |s| s.max_size),
                        ));
                    }
                    mesh = plx_mesher::merge_part(&mesh, part.mesh);
                }
                model.set_mesh(mesh);
                if had_mesh && picks_mesh_entities(&model.fe) {
                    self.output.push(
                        "Hinweis: Ausgewählte Knoten und Elementflächen eines neu vernetzten \
                         Parts beziehen sich noch auf das alte Netz und müssen neu ausgewählt \
                         werden"
                            .into(),
                    );
                }
                self.output.push(format!(
                    "Netz erzeugt: {} Knoten, {} Elemente, {} Parts ({} ms)",
                    model.mesh.node_count(),
                    model.mesh.element_count(),
                    model.parts.len(),
                    started.map_or(0, |s| s.elapsed().as_millis())
                ));
                self.highlighted = None;
                self.mesh_item_editor = None;
                self.set_tree_view(TreeView::FeModel);
                self.results_changed = true;
                if !had_mesh {
                    self.view_command = Some(ViewCommand::Fit);
                }
            }
            Err(error) => self
                .output
                .push(format!("Vernetzung fehlgeschlagen: {error}")),
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
        let takes_loads = model
            .fe
            .steps
            .last()
            .is_some_and(|s| s.kind.supports_loads());
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
                takes_loads,
            ),
            (NewItem::HotSpot, "Hot Spot erstellen …", true),
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
            self.start_analysis(false);
        }
        if ui
            .add_enabled(can_start, egui::Button::new("Modell prüfen"))
            .clicked()
        {
            self.start_analysis(true);
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

    /// An entry of the analysis' context menu in the tree.
    fn analysis_action(&mut self, action: AnalysisAction) {
        match action {
            AnalysisAction::Edit => {
                let window = SettingsWindow::with_page(&self.settings, settings::Page::Solver);
                self.settings_window = Some(window);
            }
            AnalysisAction::Run => self.start_analysis(false),
            AnalysisAction::CheckModel => self.start_analysis(true),
            AnalysisAction::Monitor => match &mut self.analysis {
                Some(analysis) => analysis.monitor = true,
                None => self.output.push(
                    "Die Analyse wurde noch nicht gestartet (Analyse > Analyse starten).".into(),
                ),
            },
            AnalysisAction::Results => {
                self.open_results = self.analysis.as_ref().and_then(Analysis::results);
            }
            AnalysisAction::Kill => {
                if let Some(analysis) = &mut self.analysis {
                    analysis.kill();
                }
            }
        }
    }

    fn start_analysis(&mut self, check_model: bool) {
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
        if model.mesh.element_count() == 0 {
            self.output
                .push("Das Modell hat noch kein Netz: Netz > Netz erzeugen".into());
            return;
        }
        match Analysis::start(&self.settings.solver, model, default_solver, check_model) {
            Ok(analysis) => {
                self.output.push(format!(
                    "{} gestartet: {}",
                    if check_model {
                        "Modellprüfung"
                    } else {
                        "Analyse"
                    },
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
            self.start_analysis(false);
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
        let geometry = model.geometry.as_ref();
        match plx_io::project::save_project(&path, geometry, &model.mesh, &model.fe) {
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

    /// A click in the 3D view picks for the open dialog. Without one, a click on a part
    /// selects it in the tree, as in PrePoMax, and a click into empty space clears the tree
    /// selection and with it the highlighted region.
    fn click(&mut self, click: Click) {
        if self.transformation_picks()
            && let (Some(dialog), Some(model)) = (
                &mut self.transformation_dialog,
                self.results.get(self.current_result),
            )
        {
            let hit = model.pick(click.origin, click.direction);
            dialog.click(
                model,
                hit.as_ref().map(|h| (h, click.precision_at(h.point))),
            );
            return;
        }
        if let Some(dialog) = &mut self.section_dialog
            && dialog.picks()
        {
            let model = match self.tree_view {
                TreeView::Results => self.results.get(self.current_result),
                TreeView::Geometry => self.geometry.as_ref(),
                TreeView::FeModel => self.model.as_ref(),
            };
            if let Some(model) = model {
                let hit = model.pick(click.origin, click.direction);
                dialog.click(
                    model,
                    hit.as_ref().map(|h| (h, click.precision_at(h.point))),
                );
            }
            return;
        }
        if !self.picking() {
            let hit = self
                .shown()
                .and_then(|model| model.pick(click.origin, click.direction));
            match hit {
                Some(hit) => self.select_part(hit.part),
                // On the Results tab the tree shows the current field, which stays.
                None if self.tree_view != TreeView::Results => self.tree.selected = None,
                None if matches!(self.tree.selected, Some((_, TreeItem::Part(_)))) => {
                    self.tree.selected = None;
                }
                None => {}
            }
            return;
        }
        if self.tree_view == TreeView::Results {
            if let (Some(dialog), Some(model)) = (
                &mut self.history_dialog,
                self.results.get(self.current_result),
            ) {
                let hit = model.pick(click.origin, click.direction);
                let pick = hit.as_ref().map(|hit| (hit, click.precision_at(hit.point)));
                let operation = Operation::from_modifiers(click.shift, click.ctrl);
                dialog.click(model, pick, operation);
            }
            return;
        }
        if self.tree_view == TreeView::Geometry {
            if let (Some(editor), Some(view)) = (&mut self.mesh_item_editor, &self.geometry) {
                let hit = view.pick(click.origin, click.direction);
                let pick = hit.as_ref().map(|hit| (hit, click.precision_at(hit.point)));
                editor.click(
                    view,
                    pick,
                    Operation::from_modifiers(click.shift, click.ctrl),
                );
            }
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

    /// Selects a part clicked in the 3D view in the tree shown.
    fn select_part(&mut self, index: usize) {
        let view = self.tree_view;
        self.tree.selected = Some((view, TreeItem::Part(index)));
        self.tree.reveal = true;
    }

    /// The context menu of the 3D view: on a part it starts with the part's menu from the
    /// tree, so it makes no difference where the part is right-clicked.
    fn viewport_menu(&mut self, response: &egui::Response, right_click: Option<Click>) {
        if let Some(click) = right_click {
            self.menu_part = None;
            if !self.picking() {
                self.menu_part = (self.shown())
                    .and_then(|model| model.pick(click.origin, click.direction))
                    .map(|hit| hit.part);
            }
            if let Some(part) = self.menu_part {
                self.select_part(part);
            }
        }
        let part = (self.menu_part)
            .and_then(|index| Some((index, self.shown()?.parts.get(index)?.visible)));
        let mut tree_response = TreeResponse::default();
        let mut command = None;
        response.context_menu(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if let Some((index, visible)) = part {
                let geometry = self.tree_view == TreeView::Geometry;
                tree::part_menu(ui, index, visible, geometry, &mut tree_response);
                ui.separator();
            }
            command = crate::viewport::view_menu(ui);
        });
        if command.is_some() {
            self.view_command = command;
        }
        let view = self.tree_view;
        self.tree_response(&response.ctx, view, tree_response);
    }

    fn box_select(&mut self, area: &BoxSelect) {
        if !self.picking() || self.section_picks() {
            return;
        }
        let operation = Operation::from_modifiers(area.shift, area.ctrl);
        if self.tree_view == TreeView::Results {
            if let (Some(dialog), Some(model)) = (
                &mut self.history_dialog,
                self.results.get(self.current_result),
            ) {
                dialog.box_select(model, area, operation);
            }
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
        if self.transformation_picks()
            && let (Some(dialog), Some(model)) = (
                &self.transformation_dialog,
                self.results.get(self.current_result),
            )
        {
            self.viewport.preview = hover
                .and_then(|click| {
                    let hit = model.pick(click.origin, click.direction)?;
                    Some(dialog.preview(model, &hit, click.precision_at(hit.point)))
                })
                .unwrap_or_default();
            return;
        }
        if let (Some(dialog), Some(model)) = (&self.section_dialog, self.shown())
            && dialog.picks()
        {
            self.viewport.preview = hover
                .and_then(|click| {
                    let hit = model.pick(click.origin, click.direction)?;
                    Some(dialog.preview(model, &hit, click.precision_at(hit.point)))
                })
                .unwrap_or_default();
            return;
        }
        let hover = hover.filter(|_| self.picking());
        if self.tree_view == TreeView::Results {
            let shown = (self.history_dialog.as_ref()).zip(self.results.get(self.current_result));
            self.viewport.preview = match (hover, shown) {
                (Some(click), Some((dialog, model))) => model
                    .pick(click.origin, click.direction)
                    .map(|hit| dialog.preview(model, &hit, click.precision_at(hit.point)))
                    .unwrap_or_default(),
                _ => Default::default(),
            };
            return;
        }
        if self.tree_view == TreeView::Geometry {
            let shown = (self.mesh_item_editor.as_ref()).zip(self.geometry.as_ref());
            self.viewport.preview = match (hover, shown) {
                (Some(click), Some((editor, view))) => view
                    .pick(click.origin, click.direction)
                    .map(|hit| editor.preview(view, &hit, click.precision_at(hit.point)))
                    .unwrap_or_default(),
                _ => Default::default(),
            };
            return;
        }
        let preview = match (hover, &self.editor, &self.model) {
            (Some(click), Some(editor), Some(model)) => model
                .pick(click.origin, click.direction)
                .map(|hit| editor.preview(model, &hit, click.precision_at(hit.point)))
                .unwrap_or_default(),
            _ => Default::default(),
        };
        self.viewport.preview = preview;
    }

    /// Opens the dialog of the derived field output that computes the given field.
    fn edit_field_output(&mut self, field: usize) {
        let Some(model) = self.results.get(self.current_result) else {
            return;
        };
        let Some(view) = &model.results else { return };
        if let Some(index) = view.field_output_index(field) {
            self.field_output_dialog = FieldOutputDialog::edit(view, &model.mesh, index);
        }
    }

    fn delete_field_output(&mut self, field: usize) {
        let Some(model) = self.results.get_mut(self.current_result) else {
            return;
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let Some(index) = view.field_output_index(field) else {
            return;
        };
        let name = view.field_outputs[index].name.clone();
        let warnings = view.remove_field_output(index, &model.mesh);
        self.output.push(format!("Feldausgabe {name} gelöscht"));
        for warning in warnings {
            self.output.push(format!("Warnung: {warning}"));
        }
        self.tree.selected = None;
        self.field_output_dialog = None;
        self.close_history_windows();
        self.results_changed = true;
    }

    fn field_output_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.field_output_dialog else {
            return;
        };
        let (output, next) = match dialog.show(ctx) {
            DialogAction::Open => return,
            DialogAction::Cancel => {
                self.field_output_dialog = None;
                return;
            }
            DialogAction::Ok { output, next } => (output, next),
        };
        let Some(model) = self.results.get_mut(self.current_result) else {
            self.field_output_dialog = None;
            return;
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let name = output.name.clone();
        let edit = dialog.edit;
        match view.set_field_output(edit, output, &model.mesh) {
            Ok(warnings) => {
                let verb = if edit.is_some() {
                    "geändert"
                } else {
                    "erstellt"
                };
                self.output.push(format!("Feldausgabe {name} {verb}"));
                for warning in warnings {
                    self.output.push(format!("Warnung: {warning}"));
                }
                // PrePoMax shows a new field output right away.
                if edit.is_none()
                    && let Some(field) = (view.current_increment())
                        .and_then(|i| i.fields.iter().position(|f| f.name == name))
                {
                    view.field = field;
                    view.component = 0;
                    self.tree.selected = Some((TreeView::Results, TreeItem::Component(field, 0)));
                }
                self.results_changed = true;
                if next {
                    dialog.next(&name);
                } else {
                    self.field_output_dialog = None;
                }
            }
            Err(error) => dialog.error = Some(error),
        }
    }

    fn edit_history_output(&mut self, set: usize) {
        let Some(model) = self.results.get(self.current_result) else {
            return;
        };
        let Some(view) = &model.results else { return };
        if let Some(index) = view.history_output_index(set) {
            self.history_dialog = HistoryOutputDialog::edit(view, &model.mesh, index);
        }
    }

    fn delete_history_output(&mut self, set: usize) {
        let Some(model) = self.results.get_mut(self.current_result) else {
            return;
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let Some(index) = view.history_output_index(set) else {
            return;
        };
        let name = view.history_outputs[index].name.clone();
        let warnings = view.remove_history_output(index, &model.mesh);
        self.output.push(format!("History-Ausgabe {name} gelöscht"));
        for warning in warnings {
            self.output.push(format!("Warnung: {warning}"));
        }
        self.tree.selected = None;
        self.close_history_windows();
    }

    fn history_output_window(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(model)) = (
            &mut self.history_dialog,
            self.results.get_mut(self.current_result),
        ) else {
            return;
        };
        let (output, next) = match dialog.show(ctx, model) {
            DialogAction::Open => return,
            DialogAction::Cancel => {
                self.history_dialog = None;
                return;
            }
            DialogAction::Ok { output, next } => (output, next),
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let edit = dialog.edit;
        match view.set_history_output(edit, output.clone(), &model.mesh) {
            Ok(warnings) => {
                let verb = if edit.is_some() {
                    "geändert"
                } else {
                    "erstellt"
                };
                self.output
                    .push(format!("History-Ausgabe {} {verb}", output.name));
                for warning in warnings {
                    self.output.push(format!("Warnung: {warning}"));
                }
                // The table may show another component now.
                self.history_table = None;
                if next {
                    dialog.next(&output, view);
                } else {
                    self.history_dialog = None;
                }
            }
            Err(error) => dialog.error = Some(error),
        }
    }

    fn history_table_window(&mut self, ctx: &egui::Context) {
        let (Some(table), Some(view)) = (self.history_table, self.shown_results_view()) else {
            return;
        };
        let kind = view
            .current_increment()
            .map_or(plx_results::AnalysisKind::Static, |i| i.kind);
        let unit = (view.history.get(table.set))
            .and_then(|set| view.history_outputs.iter().find(|o| o.name == set.name))
            .and_then(|o| o.unit());
        if !table.show(ctx, &view.history, kind, unit) {
            self.history_table = None;
        }
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

    /// Highlights the region of the open dialog, or of the item selected in the tree; a
    /// selected part is outlined in the FE model and in the results alike.
    fn update_highlight(&mut self) {
        let on_results = self.tree_view == TreeView::Results;
        // The current results show the region of an open history output dialog, else the
        // part selected in the Results tree.
        let current = self.current_result;
        for (index, model) in self.results.iter_mut().enumerate() {
            let dialog = (self.history_dialog.as_ref()).filter(|_| on_results && index == current);
            let transformation = (self.transformation_dialog.as_ref())
                .filter(|d| on_results && d.result == index && index == current);
            let highlight = match (dialog, &self.tree.selected) {
                _ if transformation.is_some() => transformation
                    .map(TransformationDialog::highlight)
                    .unwrap_or_default(),
                (Some(dialog), _) => dialog.highlight(model),
                (None, Some((TreeView::Results, TreeItem::Part(part)))) if index == current => {
                    Highlight::part(*part)
                }
                _ => Highlight::default(),
            };
            if highlight != model.highlight {
                model.highlight = highlight;
                self.results_changed |= on_results && index == current;
            }
        }
        self.update_geometry_highlight();
        let Some(model) = &mut self.model else {
            return;
        };
        let highlight = if let Some(dialog) = &self.section_dialog {
            self.highlighted = None;
            dialog.highlight()
        } else if let Some(editor) = &self.editor {
            editor.highlight(model)
        } else {
            if self.highlighted == self.tree.selected {
                return;
            }
            self.highlighted = self.tree.selected.clone();
            match &self.tree.selected {
                Some((TreeView::FeModel, TreeItem::Part(part))) => Highlight::part(*part),
                Some((TreeView::FeModel, item)) => crate::setup::item_region(&model.fe, item)
                    .map(|region| crate::setup::region_highlight(model, region))
                    .unwrap_or_default(),
                _ => Default::default(),
            }
        };
        if highlight != model.highlight {
            model.highlight = highlight;
            self.results_changed |= !on_results;
        }
    }

    /// The geometry shows the item of an open mesh setup dialog, else the selected part or
    /// mesh setup item.
    fn update_geometry_highlight(&mut self) {
        let Some(view) = &mut self.geometry else {
            return;
        };
        let items = (self.model.as_ref())
            .and_then(|m| m.geometry.as_ref())
            .map_or(&[][..], |g| g.mesh_items.as_slice());
        let highlight = match (&self.mesh_item_editor, &self.tree.selected) {
            (Some(editor), _) => editor.highlight(view),
            (None, Some((TreeView::Geometry, TreeItem::Part(part)))) => Highlight::part(*part),
            (None, Some((TreeView::Geometry, TreeItem::MeshItem(index)))) => items
                .get(*index)
                .map(|item| crate::meshing::item_highlight(view, &item.kind))
                .unwrap_or_default(),
            _ => Highlight::default(),
        };
        if highlight != view.highlight {
            view.highlight = highlight;
            self.results_changed |= self.tree_view == TreeView::Geometry;
        }
    }

    /// Draws the symbols of the boundary conditions and loads of one step in the FE model,
    /// as PrePoMax does for the step chosen to show: the step of the edited or selected
    /// item, else the last step. The selected or edited item is drawn in red.
    fn update_symbols(&mut self, ctx: &egui::Context) {
        let items = self.symbol_items();
        let shown = (self.model.as_ref())
            .map(|m| m.parts.iter().map(|p| p.visible).collect())
            .unwrap_or_default();
        let key = (items, shown);
        if self.symbols_shown.as_ref() == Some(&key) {
            return;
        }
        self.viewport.symbols = (self.model.as_ref())
            .filter(|_| !key.0.is_empty())
            .map(|model| symbols::build(model, &key.0))
            .unwrap_or_default();
        self.symbols_shown = Some(key);
        ctx.request_repaint();
    }

    fn symbol_items(&self) -> Vec<symbols::Item> {
        let Some(model) = self
            .model
            .as_ref()
            .filter(|_| self.tree_view == TreeView::FeModel)
        else {
            return Vec::new();
        };
        let edited = self.editor.as_ref().and_then(Editor::step_item);
        let selected = match &self.tree.selected {
            Some((TreeView::FeModel, item)) => Some(item),
            _ => None,
        };
        let step_of = |item: &TreeItem| match *item {
            TreeItem::Step(s)
            | TreeItem::StepGroup(s, _)
            | TreeItem::BoundaryCondition(s, _)
            | TreeItem::Load(s, _)
            | TreeItem::FieldOutput(s, _) => Some(s),
            _ => None,
        };
        let index = (edited.as_ref().map(|(s, ..)| *s))
            .or_else(|| selected.and_then(step_of))
            .or_else(|| model.fe.steps.len().checked_sub(1));
        let Some((index, step)) = index.and_then(|i| Some((i, model.fe.steps.get(i)?))) else {
            return Vec::new();
        };
        // The edited item replaces its saved version.
        let replaced = |load: bool, i: usize| {
            edited.as_ref().is_some_and(|(_, edited_index, item)| {
                *edited_index == Some(i) && matches!(item.kind, symbols::Kind::Load(_)) == load
            })
        };
        let is_selected = |item: TreeItem| edited.is_none() && selected == Some(&item);
        // Like PrePoMax, deactivated items have no symbols; the edited one is drawn anyway.
        let mut items = Vec::new();
        for (i, bc) in step.boundary_conditions.iter().enumerate() {
            if step.active && bc.active && !replaced(false, i) {
                items.push(symbols::Item {
                    kind: symbols::Kind::Boundary(bc.kind),
                    region: bc.region.clone(),
                    selected: is_selected(TreeItem::BoundaryCondition(index, i)),
                });
            }
        }
        for (i, load) in step.loads.iter().enumerate() {
            if step.active && load.active && !replaced(true, i) {
                items.push(symbols::Item {
                    kind: symbols::Kind::Load(load.kind),
                    region: load.region.clone(),
                    selected: is_selected(TreeItem::Load(index, i)),
                });
            }
        }
        items.extend(edited.map(|(_, _, item)| item));
        items
    }

    fn section_picks(&self) -> bool {
        self.section_dialog
            .as_ref()
            .is_some_and(SectionDialog::picks)
    }

    /// Opens the section view dialog on the current section, or on a new one facing away
    /// from the viewer.
    fn open_section_dialog(&mut self) {
        if self.section_dialog.is_some() {
            return;
        }
        if let Some(model) = self.shown() {
            let forward = self.viewport.camera().forward().as_dvec3();
            let dialog = SectionDialog::new(self.section.clone(), model, forward);
            self.section_dialog = Some(dialog);
        }
    }

    fn section_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.section_dialog.take() else {
            return;
        };
        let Some(model) = self.shown() else {
            return;
        };
        match dialog.show(ctx, model) {
            SectionResult::Open => {
                self.section_dialog = Some(dialog);
                return;
            }
            SectionResult::Ok => self.section = Some(dialog.draft),
            SectionResult::Cancel => self.section = dialog.before,
            SectionResult::Disable => self.section = None,
        }
        self.viewport.preview = Default::default();
    }

    /// Cuts the scene at the section plane being edited or shown, when it or the scene
    /// changed.
    fn update_section(&mut self) {
        let wanted = self
            .section_dialog
            .as_ref()
            .map(|d| &d.draft)
            .or(self.section.as_ref())
            .map(|s| (self.viewport.scene_version(), s.clone()));
        if wanted == self.section_shown {
            return;
        }
        // Only the model in view is cut; picking in the others sees everything.
        let models = self.model.iter_mut().chain(&mut self.geometry);
        for model in models.chain(&mut self.results) {
            model.clip = None;
        }
        let shown = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        };
        match (shown, &wanted) {
            (Some(model), Some((_, section))) => {
                // The model origin is the centre of its bounds, where a principal plane's
                // manipulator sits.
                let anchor = section.anchor(model.origin());
                let normal = section.normal();
                let clip = plx_render::clip_plane(anchor, normal, model.origin());
                let faces = model.section_meshes(anchor, normal, section.lighten);
                model.clip = Some(clip);
                self.viewport.set_section(Some((clip, &faces)));
            }
            _ => self.viewport.set_section(None),
        }
        self.section_shown = wanted;
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
        if new.gmsh != self.settings.gmsh && !plx_mesher::set_library_path(new.gmsh.library()) {
            self.output
                .push("Die geänderte Gmsh-Bibliothek wird nach einem Neustart geladen".into());
        }
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
        let mut accept = false;
        // Parts of the geometry keep their names, which the mesh setup refers to.
        let part = match item {
            TreeItem::Part(index) if self.tree_view != TreeView::Geometry => Some(index),
            _ => None,
        };
        // The model by its fields, so the window can edit the typed name alongside.
        let shown = match self.tree_view {
            TreeView::Results => self.results.get(self.current_result),
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        };
        egui::Window::new(properties::title(shown, &item))
            .id(egui::Id::new("properties window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let (name, error) = &mut self.part_name;
                let name = part.map(|_| name);
                properties::show(ui, shown, &item, name);
                if let Some(error) = error.as_ref().filter(|_| part.is_some()) {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(200, 0, 0), error);
                }
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if part.is_some() {
                        close = ui.button("Abbrechen").clicked();
                        accept = ui.button("OK").clicked();
                    } else {
                        close = ui.button("Schließen").clicked();
                    }
                });
            });
        accept |= part.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Enter));
        if let (true, Some(index)) = (accept, part) {
            let name = self.part_name.0.clone();
            let renamed = match self.shown_mut() {
                Some(model) => model.rename_part(index, &name),
                None => Ok(()),
            };
            match renamed {
                Ok(()) => close = true,
                Err(error) => self.part_name.1 = Some(error),
            }
        }
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
        self.geometry = None;
        self.mesh_setup = None;
        self.mesh_item_editor = None;
        self.meshing = None;
        self.close_results(true);
        self.tree.selected = None;
        self.dialog = None;
        self.section = None;
        self.section_dialog = None;
        self.editor = None;
        self.highlighted = None;
        self.parked_camera = None;
        self.results_changed = true;
    }

    /// Plays the animation and shows its window; marks the scene for rebuilding.
    fn animate(&mut self, ctx: &egui::Context) {
        let model = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
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

    /// Shows the sound window of the shown results and drives its audio output.
    fn play_sound(&mut self, ctx: &egui::Context) {
        let model = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => self.model.as_mut(),
        };
        let Some(view) = model.and_then(|m| m.results.as_mut()) else {
            self.audio = None;
            return;
        };
        let shown = view.increment;
        let Some(sound) = &mut view.sound else {
            // Closing the window releases the audio device.
            self.audio = None;
            if view.superposition.take().is_some() {
                self.results_changed = true;
            }
            return;
        };
        let playing = self.audio.as_ref().is_some_and(|a| a.synth().sounding());
        let actions = sound::window(ctx, sound, shown, playing);
        if actions.play {
            if self.audio.is_none() {
                match sound::Player::open() {
                    Ok(player) => self.audio = Some(player),
                    Err(error) => sound.message = Some(error),
                }
            }
            if let Some(audio) = &self.audio {
                audio.synth().start(sound.tones());
                sound.message = None;
            }
        }
        if let Some(audio) = &self.audio {
            if actions.stop {
                audio.synth().stop();
            } else if actions.changed && playing {
                audio.synth().update(sound.tones());
            }
        }
        if actions.export {
            let picked = rfd::FileDialog::new()
                .set_title("Klang speichern")
                .add_filter("WAV-Datei (*.wav)", &["wav"])
                .set_file_name(format!("Moden-Step-{}.wav", sound.step))
                .save_file();
            if let Some(mut path) = picked {
                if path.extension().is_none() {
                    path.set_extension("wav");
                }
                let data = sound::wav(sound.tones(), sound.export_seconds());
                match std::fs::write(&path, data) {
                    Ok(()) => self.output.push(format!("{} gespeichert", path.display())),
                    Err(error) => sound.message = Some(format!("Nicht gespeichert: {error}")),
                }
            }
        }
        if playing || actions.play {
            // The play button turns back when a struck sound has died away.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let now = ctx.input(|i| i.time);
        if actions.play {
            sound.started = Some(now);
            sound.shape_clock = (0.0, f64::NEG_INFINITY);
        }
        if actions.play || actions.changed {
            sound.mix = None;
        }
        let audible = self.audio.as_ref().is_some_and(|a| a.synth().sounding());
        let overlay = sound.show_shape && audible && !actions.close;
        // The overlay takes the place of an animation.
        if overlay && view.animation.is_some() {
            view.stop_animation();
        }
        if overlay
            && let Some((field, component)) =
                (view.current()).map(|(f, c)| (f.name.clone(), c.name.clone()))
            && let Some(sound) = &mut view.sound
        {
            if sound
                .mix
                .as_ref()
                .is_some_and(|m| !m.shows(&field, &component))
            {
                sound.mix = None;
            }
            if sound.mix.is_none() {
                let mix = sound::ShapeMix::new(sound, &view.increments, &field, &component);
                sound.mix = mix;
            }
            // A new frame at the chosen rate; the swings advance with the chosen speed, so
            // changing it while playing does not make the shape jump.
            let period = 1.0 / sound.shape_fps.max(1.0) as f64;
            let (swings, last) = sound.shape_clock;
            let since = now - last;
            if since >= period || view.superposition.is_none() {
                let step = if last.is_finite() { since } else { 0.0 };
                let swings = swings + step * sound.shape_speed as f64;
                sound.shape_clock = (swings, now);
                let time = now - sound.started.unwrap_or(now);
                view.superposition = sound.mix.as_ref().map(|m| m.frame(swings, time));
                self.results_changed = true;
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(period));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(period - since));
            }
        } else if view.superposition.take().is_some() {
            self.results_changed = true;
        }
        if let Some(increment) = actions.show
            && view.animation.is_none()
            && increment != view.increment
        {
            view.select_increment(increment);
            self.results_changed = true;
        }
        if actions.close {
            view.sound = None;
            self.audio = None;
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
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
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
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
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
        // The open transformation dialog shows the points and axis of the selected item.
        let transformation = (self.transformation_dialog.as_ref())
            .filter(|d| self.tree_view == TreeView::Results && d.result == self.current_result);
        let marker = |label: &str, extreme: Option<(usize, f32, usize)>| {
            let (index, value, item) = extreme?;
            Some(Marker {
                position: model.node_position_on(index, item)?,
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
                .chain(transformation.iter().flat_map(|d| d.points(model)))
                .collect(),
            paths: self.overlay_paths(),
            lines: transformation.map_or_else(Vec::new, |d| d.lines(model)),
        };
    }
}

/// What the results tool bar asks for.
struct ToolBarResponse {
    /// Anything that affects the scene changed.
    changed: bool,
    /// The transformations button was clicked.
    transformations: bool,
}

/// PrePoMax's results tool bar: deformation, colour bands, transformations and the increment
/// with its navigation buttons.
fn results_tool_bar(
    ui: &mut egui::Ui,
    view: &mut ResultsView,
    transformed: bool,
) -> ToolBarResponse {
    let mut changed = false;
    let mut transformations = false;
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
        let tip = "Transformationen: Symmetrien und Muster";
        transformations = icons::button(ui, Icon::Transformation, tip, true, transformed).clicked();
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
        // Only modes of a frequency step have a sound; otherwise the button is greyed out.
        let sounding = view.sound.is_some();
        let is_mode = view
            .current_increment()
            .is_some_and(|i| i.kind == plx_results::AnalysisKind::Frequency);
        let tip = "Klang der Eigenformen";
        if icons::button(ui, Icon::Sound, tip, sounding || is_mode, sounding).clicked() {
            view.sound = match view.sound {
                Some(_) => None,
                None => ModeSound::new(&view.increments, view.increment),
            };
        }
    });
    ToolBarResponse {
        changed,
        transformations,
    }
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
