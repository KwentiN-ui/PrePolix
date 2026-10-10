use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use plx_model::UnitSystem;
use plx_render::StandardView;
use plx_results::hot_spot::{HotSpot, HotSpotPath};

use crate::analysis::{Analysis, MonitorEvent};
use crate::animation::{AnimationKind, ColorLimits, Playback};
use crate::contact_search::{ContactSearchDialog, SearchResult};
use crate::exploded::{ExplodedDialog, ExplodedResult};
use crate::features::{FeatureDialog, FeatureKind, FeatureResult};
use crate::field_output_dialog::{DialogAction, FieldOutputDialog};
use crate::history_output_dialog::HistoryOutputDialog;
use crate::history_table::HistoryTable;
use crate::hot_spot_dialog::HotSpotDialog;
use crate::icons::{self, Icon};
use crate::keywords::KeywordEditor;
use crate::material_library::{LibraryResult, MaterialLibraryEditor};
use crate::meshing::{
    MeshItemEditor, MeshItemResult, MeshSetupResult, MeshSetupWindow, MeshingJob,
};
use crate::model::{self, Highlight, LoadedModel, Model};
use crate::model_properties::{DialogResult, ModelPropertiesDialog, geometry_check};
use crate::numeric;
use crate::overlay::{Marker, Overlay};
use crate::properties;
use crate::results::{Deformation, ResultsView, format_legend_value};
use crate::screenshot::{self, Screenshot};
use crate::section::{PlaneDefinition, SectionDialog, SectionResult, SectionView};
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
    /// CAD files added to the geometry of the open model.
    Added(Vec<PathBuf>, Result<Box<plx_mesher::CadAddition>, String>),
}

struct Workbench {
    settings: Settings,
    /// Open settings window with its unsaved draft.
    settings_window: Option<SettingsWindow>,
    /// Open dialog of the model space and unit system, for a new or the open model.
    model_dialog: Option<ModelPropertiesDialog>,
    /// The new model asked for the geometry import; the app opens the file dialog.
    import_requested: bool,
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
    /// CAD faces and edges Gmsh named when meshing failed, shown red on the geometry.
    mesh_failure: BTreeSet<plx_mesher::CadEntity>,
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
    /// Item asked to be deleted, waiting for the user's confirmation.
    /// Items to delete once the user agrees: several parts, or one item.
    confirm_delete: Option<(TreeView, Vec<TreeItem>)>,
    /// Open CalculiX keyword editor.
    keyword_editor: Option<KeywordEditor>,
    /// Open search for contact pairs.
    contact_search: Option<ContactSearchDialog>,
    /// Open material library editor.
    material_library: Option<MaterialLibraryEditor>,
    /// Open dialog creating or editing a field output derived from the shown results.
    field_output_dialog: Option<FieldOutputDialog>,
    /// Open dialog creating or editing a history output of the shown results.
    history_dialog: Option<HistoryOutputDialog>,
    /// Open table of a history output component.
    history_table: Option<HistoryTable>,
    /// Diagram of a table selection, PrePoMax's diagram view.
    history_plot: Option<crate::xy_plot::XyData>,
    /// The tree selection whose region is highlighted.
    /// `None` until it is computed, so a dialog's highlight is cleared once it closes.
    highlighted: Option<TreeSelection>,
    analysis: Option<Analysis>,
    /// Problems CalculiX reported when the last analysis failed, shown in the tree.
    solver_findings: Vec<plx_model::Finding>,
    /// The findings of a tree item whose warning sign was clicked, explained in a window.
    findings_window: Option<Vec<plx_model::Finding>>,
    /// Results file the user asked to open; read by the app on a worker thread.
    open_results: Option<PathBuf>,
    screenshot: Screenshot,
    /// Open dialog of a hot spot definition of the current results.
    hot_spot_dialog: Option<HotSpotDialog>,
    /// Hot spot whose paths are shown, edited or selected, with the results file and its
    /// paths there.
    hot_spot_preview: Option<(HotSpot, usize, Vec<HotSpotPath>)>,
    /// The table of hot spot values is open on the Results tab.
    hot_spot_window: bool,
    /// Audio output of the sound window, opened when it first plays.
    audio: Option<sound::Player>,
    /// The section view, while it is on; it cuts whatever the 3D view shows.
    section: Option<SectionView>,
    section_dialog: Option<SectionDialog>,
    /// Open dialog of the transformations of the current results.
    transformation_dialog: Option<TransformationDialog>,
    /// The cut shown in the scene of the given version with the visible parts, and whether
    /// it shows the section faces alone, to rebuild it on changes.
    section_shown: Option<(u64, Vec<bool>, SectionView, bool)>,
    /// The boundary conditions and loads, with the shown parts and the exploded view, whose
    /// symbols are drawn.
    symbols_shown: Option<(Vec<symbols::Item>, Vec<bool>, u64)>,
    /// Open exploded view dialog with the model it edits.
    exploded_dialog: Option<(ExplodedDialog, ShownModel)>,
    /// The exploded view last applied, where the next one starts, as in PrePoMax.
    last_exploded: crate::exploded::Parameters,
    /// Open dialog of a feature or of results on one.
    feature_dialog: Option<FeatureDialog>,
    /// The shown values on the plane of the shown plane result, in model coordinates: where
    /// the plane cuts the parts as shown, and where it cuts the undeformed mesh.
    section_values: Option<plx_render::SectionValues>,
    plane_values: Option<plx_render::SectionValues>,
}

/// Which model the 3D view shows: the geometry, the FE model or one of the results.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShownModel(TreeView, usize);

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
                model_dialog: None,
                import_requested: false,
                viewport: Viewport::new(render_state),
                model: None,
                geometry: None,
                mesh_setup: None,
                mesh_item_editor: None,
                meshing: None,
                mesh_failure: BTreeSet::new(),
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
                confirm_delete: None,
                keyword_editor: None,
                contact_search: None,
                material_library: None,
                field_output_dialog: None,
                history_dialog: None,
                history_table: None,
                history_plot: None,
                highlighted: None,
                analysis: None,
                solver_findings: Vec::new(),
                findings_window: None,
                open_results: None,
                screenshot: Screenshot::default(),
                hot_spot_dialog: None,
                hot_spot_preview: None,
                hot_spot_window: false,
                audio: None,
                section: None,
                section_dialog: None,
                transformation_dialog: None,
                feature_dialog: None,
                section_values: None,
                plane_values: None,
                section_shown: None,
                symbols_shown: None,
                exploded_dialog: None,
                last_exploded: Default::default(),
            },
            load_events: channel(),
            loading: None,
        };
        // Every file on the command line is opened in turn, e.g. a model and its results.
        let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        // Without files on the command line, the project open at the last exit is reopened.
        if paths.is_empty() {
            paths.extend(cc.storage.and_then(stored_last_project));
        }
        if !paths.is_empty() {
            let (sender, ctx) = (app.load_events.0.clone(), cc.egui_ctx.clone());
            let units = app.workbench.import_units();
            std::thread::spawn(move || {
                // Several CAD files make one geometry, as with Geometry > Import.
                if paths.len() > 1 && paths.iter().all(|p| plx_mesher::is_cad_file(p)) {
                    return load_geometry_in_background(paths, units, sender, ctx);
                }
                for path in paths {
                    load_in_background(path, units, sender.clone(), ctx.clone());
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
        let units = self.workbench.import_units();
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
                load_in_background(path, units, sender, ctx);
            }
        });
    }

    /// Geometry > Import. Without a model, PrePoMax first asks for the new model's
    /// properties, so that a 2D geometry lands in a 2D model.
    fn import(&mut self, ctx: &egui::Context) {
        if self.workbench.model.is_none() {
            self.workbench.new_model(true);
        } else {
            self.import_dialog(ctx);
        }
    }

    /// PrePoMax's Geometry > Import: one or several STEP, IGES or BREP files, read into
    /// one geometry. A model that already has geometry or a mesh keeps it; the new parts
    /// are added.
    fn import_dialog(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() {
            return;
        }
        let sender = self.load_events.0.clone();
        let ctx = ctx.clone();
        let units = self.workbench.import_units();
        // What the new parts are added to: the geometry, if any, and the names of the
        // mesh parts they must not take.
        let base = (self.workbench.model.as_ref())
            .filter(|m| m.geometry.is_some() || m.mesh.element_count() > 0)
            .map(|m| {
                let taken: Vec<String> = m.parts.iter().map(|p| p.name.clone()).collect();
                (m.geometry.clone(), taken, m.fe.properties.units)
            });
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Geometrie importieren")
                .add_filter(GEOMETRY_FILTER.0, GEOMETRY_FILTER.1)
                .pick_files();
            let Some(paths) = picked.filter(|p| !p.is_empty()) else {
                return;
            };
            match base {
                Some((geometry, taken, units)) => {
                    let _ = sender.send(LoadEvent::Started(paths[0].clone()));
                    ctx.request_repaint();
                    let result =
                        plx_mesher::add_cad_files(geometry.as_ref(), &paths, units, &taken)
                            .map(Box::new)
                            .map_err(|e| e.to_string());
                    let _ = sender.send(LoadEvent::Added(paths, result));
                    ctx.request_repaint();
                }
                None => load_geometry_in_background(paths, units, sender, ctx),
            }
        });
    }

    fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.loading.is_none() {
            let (sender, ctx) = (self.load_events.0.clone(), ctx.clone());
            let units = self.workbench.import_units();
            std::thread::spawn(move || load_in_background(path, units, sender, ctx));
        }
    }

    fn open_geometry(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if self.loading.is_none() {
            let (sender, ctx) = (self.load_events.0.clone(), ctx.clone());
            let units = self.workbench.import_units();
            std::thread::spawn(move || load_geometry_in_background(paths, units, sender, ctx));
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
                LoadEvent::Added(paths, result) => {
                    self.loading = None;
                    self.workbench.geometry_added(&paths, result);
                }
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Datei", |ui| {
                let new = egui::Button::new("Neu …").shortcut_text("Strg+N");
                if ui.add(new).clicked() {
                    self.workbench.new_model(false);
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
                ui.separator();
                if ui
                    .add_enabled(has_model, egui::Button::new("Explosionsansicht …"))
                    .clicked()
                {
                    self.workbench.open_exploded_dialog();
                }
                let exploded = self.workbench.exploded_applied();
                if ui
                    .add_enabled(exploded, egui::Button::new("Explosionsansicht aus"))
                    .clicked()
                {
                    self.workbench.toggle_exploded();
                }
            });
            ui.menu_button("Geometrie", |ui| {
                let import = egui::Button::new("Importieren …");
                if ui.add_enabled(self.loading.is_none(), import).clicked() {
                    self.import(ui.ctx());
                }
            });
            ui.menu_button("Netz", |ui| self.workbench.mesh_menu(ui));
            ui.menu_button("Modell", |ui| self.workbench.model_menu(ui));
            ui.menu_button("Interaktion", |ui| self.workbench.interaction_menu(ui));
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
            if icons::button(ui, Icon::New, "Neu (Strg+N)", true, false).clicked() {
                self.workbench.new_model(false);
            }
            let can_open = self.loading.is_none();
            if icons::button(ui, Icon::Open, "Öffnen (Strg+O)", can_open, false).clicked() {
                self.open_dialog(ui.ctx());
            }
            let import = "Geometrie importieren (STEP, IGES, BREP)";
            if icons::button(ui, Icon::Import, import, can_open, false).clicked() {
                self.import(ui.ctx());
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
            // PrePoMax: a left click turns the exploded view on and off, a right click opens
            // its dialog.
            let exploded =
                self.workbench.exploded_applied() || self.workbench.exploded_dialog.is_some();
            let tooltip = "Explosionsansicht ein/aus (Rechtsklick: Einstellungen)";
            let button = icons::button(ui, Icon::ExplodedView, tooltip, shown, exploded);
            if button.clicked() {
                self.workbench.toggle_exploded();
            }
            if button.secondary_clicked() {
                self.workbench.open_exploded_dialog();
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // PrePoMax shows the unit system of the model at the right.
            if let Some(model) = &self.workbench.model {
                let properties = model.fe.properties;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "Einheitensystem: {}   Modellraum: {}",
                        properties.units.label(),
                        properties.space.label()
                    ));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        self.status_text(ui);
                    });
                });
            } else {
                self.status_text(ui);
            }
        });
    }

    fn status_text(&self, ui: &mut egui::Ui) {
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

/// Moves the parts of a model between two scales of an exploded view: through the levels of a
/// sequential disassembly one after the other, else in one animation.
fn animate_explosion(
    model: &mut Model,
    parameters: &crate::exploded::Parameters,
    from: f64,
    to: f64,
) {
    let sequential =
        parameters.method == crate::exploded::Method::Disassembly && parameters.sequential;
    let stops = if sequential {
        model.explosion_layout(parameters).sequence(from, to)
    } else {
        vec![to]
    };
    let duration = if stops.len() > 1 {
        crate::exploded::STEP_TIME
    } else {
        crate::exploded::ANIMATION_TIME
    };
    let targets = stops
        .into_iter()
        .map(|scale| {
            let offsets = model
                .explosion_layout(parameters)
                .offsets(scale, parameters.sequential);
            (offsets, duration)
        })
        .collect();
    model.explosion.show_sequence(targets, true);
}

/// Shows the exploded view being edited in a dialog.
fn preview_explosion(model: &mut Model, dialog: &mut ExplodedDialog, animate: bool) {
    let parameters = dialog.draft.clone();
    dialog.step_count = model.explosion_layout(&parameters).step_count();
    let offsets = model.explosion_offsets(&parameters);
    model.explosion.show(offsets, animate);
}

/// Loads a file on this thread; CAD geometry in the length unit of `units`.
fn load_in_background(
    path: PathBuf,
    units: UnitSystem,
    sender: Sender<LoadEvent>,
    ctx: egui::Context,
) {
    let _ = sender.send(LoadEvent::Started(path.clone()));
    ctx.request_repaint();
    let result = model::load(&path, units).map(Box::new);
    let _ = sender.send(LoadEvent::Finished(path, result));
    ctx.request_repaint();
}

/// Imports several CAD files into one geometry on the calling thread, reporting like
/// [`load_in_background`] under the first file's path.
fn load_geometry_in_background(
    paths: Vec<PathBuf>,
    units: UnitSystem,
    sender: Sender<LoadEvent>,
    ctx: egui::Context,
) {
    let path = paths[0].clone();
    let _ = sender.send(LoadEvent::Started(path.clone()));
    ctx.request_repaint();
    let result = model::load_geometry(&paths, units).map(Box::new);
    let _ = sender.send(LoadEvent::Finished(path, result));
    ctx.request_repaint();
}

/// The selected tree item and the parts selected along with it.
type TreeSelection = (Option<(TreeView, TreeItem)>, BTreeSet<usize>);

/// The output line after deleting parts.
fn deleted_message(names: &[String]) -> String {
    match names {
        [name] => format!("Part {name} gelöscht"),
        names => format!("Parts {} gelöscht", names.join(", ")),
    }
}

/// How many regions of the FE model consist of node or element numbers that a new mesh no
/// longer has. Regions picked on the geometry are found on the new mesh again.
fn lost_selections(fe: &plx_model::FeModel, mesh: &plx_mesh::FeMesh) -> usize {
    fe.regions()
        .filter(|r| r.by_mesh_ids() && r.missing_reference(mesh).is_some())
        .count()
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

/// Storage key of the project file open at exit, reopened at the next start.
const LAST_PROJECT_KEY: &str = "last_project";

/// The project file stored at the last exit, if it still exists. It is stored as an
/// `Option`, so it has to be read back as one.
fn stored_last_project(storage: &dyn eframe::Storage) -> Option<PathBuf> {
    eframe::get_value::<Option<PathBuf>>(storage, LAST_PROJECT_KEY)
        .flatten()
        .filter(|path| path.is_file())
}

impl eframe::App for PrepolixApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, settings::STORAGE_KEY, &self.workbench.settings);
        eframe::set_value(storage, LAST_PROJECT_KEY, &self.workbench.last_project());
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
            self.workbench.new_model(false);
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            (i.raw.dropped_files.iter())
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        // Several CAD files dropped together are imported into one geometry, as with
        // Geometry > Import; otherwise the first file opens.
        if dropped.len() > 1 && dropped.iter().all(|p| plx_mesher::is_cad_file(p)) {
            self.open_geometry(dropped, &ctx);
        } else if let Some(path) = dropped.into_iter().next() {
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
                || self.workbench.transformation_picks()
                || self.workbench.feature_picks();
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
        self.workbench.confirm_delete_window(&ctx);
        self.workbench.contact_search_window(&ctx);
        self.workbench.section_window(&ctx);
        self.workbench.exploded_window(&ctx);
        self.workbench.keyword_editor_window(&ctx);
        self.workbench.material_library_window(&ctx);
        self.workbench.mesh_setup_window(&ctx);
        self.workbench.mesh_item_window(&ctx);
        self.workbench.poll_meshing();
        self.workbench.field_output_window(&ctx);
        self.workbench.history_output_window(&ctx);
        self.workbench.hot_spot_dialog_window(&ctx);
        self.workbench.history_table_window(&ctx);
        self.workbench.transformation_window(&ctx);
        self.workbench.feature_window(&ctx);
        self.workbench.sync_result_features();
        self.workbench.run_analysis(&ctx);
        if let Some(path) = self.workbench.open_results.take() {
            self.open_path(path, &ctx);
        }
        self.workbench.update_highlight();
        self.workbench.update_symbols(&ctx);
        self.workbench.update_hot_spot_preview();
        self.workbench.hot_spot_window(&ctx);
        self.workbench.findings_window(&ctx);
        self.workbench.settings_window(&ctx);
        self.workbench.model_dialog_window(&ctx);
        if std::mem::take(&mut self.workbench.import_requested) {
            self.import_dialog(&ctx);
        }
        self.workbench.update_explosion(&ctx);
        self.workbench.rebuild_if_results_changed();
        self.workbench.update_section();
        self.workbench.update_feature_overlay();

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
                // Geometry imported into a model that has nothing yet, such as one just
                // created with File > New, has to fit its model space.
                let into_empty = (self.model.as_ref()).filter(|old| {
                    plx_mesher::is_cad_file(&path)
                        && old.mesh.element_count() == 0
                        && old.geometry.is_none()
                });
                let mut turned_faces = Vec::new();
                if let (Some(old), Some(view)) = (into_empty, &geometry_view) {
                    match geometry_check(old.fe.properties.space, view) {
                        Ok(faces) => turned_faces = faces,
                        Err(error) => {
                            self.output.push(format!(
                                "Fehler beim Import von {}: {error}",
                                path.display()
                            ));
                            return;
                        }
                    }
                }
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
                        // Several files imported together are listed by name.
                        let source = model.geometry.as_ref().map(|g| g.source.as_str());
                        let single = path.file_name().and_then(|n| n.to_str()) == source;
                        let what = match source {
                            Some(names) if !single => names.to_string(),
                            _ => path.display().to_string(),
                        };
                        self.output.push(format!(
                            "{what} importiert: {} Parts ({} ms)",
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
                if !turned_faces.is_empty() {
                    let faces: Vec<String> = turned_faces.iter().map(i32::to_string).collect();
                    self.output.push(format!(
                        "Hinweis: Normale von Fläche {} zeigt in -z; die Elemente werden beim \
                         Vernetzen umgedreht.",
                        faces.join(", ")
                    ));
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
                    if !view.file_history.is_empty() {
                        self.output.push(format!(
                            "{} History-Ausgabe(n) aus der .dat-Datei gelesen",
                            view.file_history.len()
                        ));
                    }
                }
                self.dialog = None;
                self.field_output_dialog = None;
                self.close_history_windows();
                self.viewport.labels = Default::default();
                let mut model = model;
                if let Some(view) = &mut model.results {
                    // Results are in the unit system of the open FE model, as those of its
                    // analyses are; without one in that of new models.
                    view.units = self
                        .model
                        .as_ref()
                        .map_or(self.settings.new_model.units, |m| m.fe.properties.units);
                }
                if let Some(view) = &model.results {
                    // A results file joins the results collection and leaves the FE model
                    // alone; opening the same file again replaces it.
                    self.tree.selected = Some((
                        TreeView::Results,
                        TreeItem::Component(view.field, view.component),
                    ));
                    self.set_tree_view(TreeView::Results);
                    // Results of a 2D model are flat in z and seen from the front.
                    if model
                        .mesh
                        .bounds()
                        .is_some_and(|(min, max)| (max[2] - min[2]).abs() < 1e-9)
                    {
                        self.viewport
                            .apply(ViewCommand::View(StandardView::Front), None);
                    }
                    let mut model = model;
                    // Results get the features of the FE model; reloaded ones keep their own
                    // and their paths.
                    let previous = self.results.iter().position(|r| r.path == model.path);
                    if let Some(old) = previous.map(|i| &self.results[i].fe) {
                        model.fe.add_missing_features(old);
                        model.fe.result_paths = old.result_paths.clone();
                        model.fe.result_planes = old.result_planes.clone();
                    }
                    if let Some(project) = &self.model {
                        model.fe.add_missing_features(&project.fe);
                    }
                    match previous {
                        Some(index) => {
                            // Results read again, e.g. after the analysis ran once more,
                            // keep their hot spots.
                            let mut model = model;
                            let old = std::mem::take(&mut self.results[index].hot_spots);
                            model.hot_spots.definitions = old.definitions;
                            self.results[index] = model;
                            self.current_result = index;
                        }
                        None => {
                            self.results.push(model);
                            self.current_result = self.results.len() - 1;
                        }
                    }
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
                    // Geometry imported into a model that has nothing yet, such as one just
                    // created with File > New, keeps its model space and units.
                    let mut model = model;
                    if let Some(old) = &self.model
                        && plx_mesher::is_cad_file(&path)
                        && old.mesh.element_count() == 0
                        && old.geometry.is_none()
                    {
                        model.fe = old.fe.clone();
                    }
                    // 2D models lie in the x-y plane and are seen from the front.
                    if model.fe.properties.space.is_2d() {
                        self.viewport
                            .apply(ViewCommand::View(StandardView::Front), None);
                    }
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
        let response = tree::show(
            ui,
            view,
            shown,
            &mesh_items,
            job,
            &self.solver_findings,
            &mut self.tree,
        );
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
        if let (Some(shown), Some(results)) = (response.plane_result, self.shown_results_mut()) {
            results.plane_result = shown;
        }
        if let Some(TreeItem::Part(index)) = response.open {
            let name = self.shown().and_then(|m| m.parts.get(index));
            self.part_name = (name.map(|p| p.name.clone()).unwrap_or_default(), None);
            self.dialog = Some(TreeItem::Part(index));
        } else if let Some(TreeItem::ResultFieldOutput(field)) = response.open {
            self.edit_field_output(field);
        } else if let Some(TreeItem::HistorySet(set)) = response.open {
            self.edit_history_output(set);
        } else if let Some(TreeItem::HotSpot(index)) = response.open {
            self.edit_hot_spot(index);
        } else if let Some(TreeItem::HistoryComponent(set, field, component)) = response.open {
            let same = (self.history_table.as_ref())
                .is_some_and(|t| (t.set, t.field, t.component) == (set, field, component));
            if !same {
                self.history_table = Some(HistoryTable::new(set, field, component));
            }
        } else if let (Some(TreeItem::Model), TreeView::FeModel) = (&response.open, view) {
            self.edit_model_properties();
        } else if let Some(TreeItem::MeshItem(index)) = response.open {
            self.mesh_item_editor = self.model.as_ref().and_then(|m| {
                MeshItemEditor::edit(m.geometry.as_ref()?, index, m.fe.properties.units)
            });
        } else if let Some(feature) = response.open.as_ref().and_then(tree::feature) {
            let results = (view == TreeView::Results).then_some(self.current_result);
            if let Some(model) = self.feature_model(results) {
                self.feature_dialog = FeatureDialog::edit(feature, results, model);
            }
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
        if let Some(item) = response.delete {
            // A part among several selected ones deletes them all.
            let parts = self.tree.selected_parts(view);
            let items = match item {
                TreeItem::Part(index) if parts.contains(&index) => {
                    parts.into_iter().map(TreeItem::Part).collect()
                }
                item => vec![item],
            };
            self.confirm_delete = Some((view, items));
        }
        if let (Some(item), Some(model)) = (response.toggle_active, self.model.as_mut()) {
            crate::setup::toggle_active(&mut model.fe, &item);
        }
        if let (Some(item), Some(model)) = (response.swap_master_slave, self.model.as_mut())
            && crate::setup::swap_master_slave(&mut model.fe, &item)
        {
            // The selection stays, but master and slave colours change places.
            self.highlighted = None;
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
        if response.hot_spot_table {
            self.show_hot_spot_table();
        }
        if response.search_contacts {
            self.open_contact_search();
        }
        if let Some(findings) = response.findings {
            self.findings_window = Some(findings);
        }
    }

    /// Explains the findings of the tree item whose warning sign was clicked: what is wrong,
    /// why CalculiX cannot cope with it and how to fix it.
    fn findings_window(&mut self, ctx: &egui::Context) {
        let Some(findings) = &self.findings_window else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new("Modellprüfung")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.6)
                    .show(ui, |ui| {
                        for (i, finding) in findings.iter().enumerate() {
                            if i > 0 {
                                ui.separator();
                            }
                            let (kind, color) = match finding.severity() {
                                plx_model::Severity::Error => {
                                    ("Fehler", egui::Color32::from_rgb(200, 0, 0))
                                }
                                plx_model::Severity::Warning => {
                                    ("Warnung", egui::Color32::from_rgb(170, 110, 0))
                                }
                            };
                            ui.horizontal(|ui| {
                                ui.colored_label(color, egui::RichText::new(kind).strong());
                                ui.label(egui::RichText::new(finding.problem.title()).strong());
                            });
                            ui.add(
                                egui::Label::new(egui::RichText::new(&finding.detail).weak())
                                    .wrap(),
                            );
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(finding.problem.explanation()).wrap());
                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(format!("Abhilfe: {}", finding.problem.fix()))
                                    .wrap(),
                            );
                        }
                    });
                ui.separator();
                ui.vertical_centered(|ui| {
                    close = ui.button("Schließen").clicked();
                });
            });
        if !open || close {
            self.findings_window = None;
        }
    }

    /// Evaluates the hot spots of the current results file and writes the values next to
    /// it; `table` opens their table.
    fn evaluate_hot_spots(&mut self, table: bool) {
        let Some(results) = self.results.get_mut(self.current_result) else {
            return;
        };
        let reports = crate::hot_spots::evaluate(results);
        let hot_spots = &mut results.hot_spots;
        hot_spots.reports = Vec::new();
        hot_spots.file = None;
        if reports.is_empty() {
            self.hot_spot_window = false;
            self.update_contour();
            return;
        }
        match crate::hot_spots::write(&results.path, &reports) {
            Ok(file) => {
                self.output
                    .push(format!("Hot Spots ausgewertet: {}", file.display()));
                hot_spots.file = Some(file);
            }
            Err(error) => self.output.push(error),
        }
        self.output.extend(crate::hot_spots::summary(&reports));
        for report in &reports {
            self.output
                .extend(report.warnings.iter().map(|w| format!("Warnung: {w}")));
        }
        hot_spots.reports = reports;
        if table {
            self.show_hot_spot_table();
        } else {
            self.update_contour();
        }
    }

    fn show_hot_spot_table(&mut self) {
        let evaluated = (self.results.get(self.current_result))
            .is_some_and(|m| !m.hot_spots.reports.is_empty());
        if evaluated {
            self.hot_spot_window = true;
            self.set_tree_view(TreeView::Results);
            self.update_contour();
        }
    }

    fn edit_hot_spot(&mut self, index: usize) {
        if let Some(model) = self.results.get(self.current_result) {
            self.hot_spot_dialog =
                HotSpotDialog::edit(&model.hot_spots.definitions, index, &model.mesh);
        }
    }

    fn delete_hot_spot(&mut self, index: usize) {
        let Some(model) = self.results.get_mut(self.current_result) else {
            return;
        };
        if index >= model.hot_spots.definitions.len() {
            return;
        }
        let removed = model.hot_spots.definitions.remove(index);
        self.output
            .push(format!("Hot Spot {} gelöscht", removed.name));
        self.hot_spot_dialog = None;
        self.tree.selected = None;
        self.evaluate_hot_spots(self.hot_spot_window);
    }

    fn hot_spot_dialog_window(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(model)) = (
            &mut self.hot_spot_dialog,
            self.results.get_mut(self.current_result),
        ) else {
            return;
        };
        let (hot_spot, next) = match dialog.show(ctx, model) {
            DialogAction::Open => return,
            DialogAction::Cancel => {
                self.hot_spot_dialog = None;
                return;
            }
            DialogAction::Ok { output, next } => (output, next),
        };
        let definitions = &mut model.hot_spots.definitions;
        let verb = match dialog.edit.filter(|&i| i < definitions.len()) {
            Some(index) => {
                definitions[index] = hot_spot.clone();
                "geändert"
            }
            None => {
                definitions.push(hot_spot.clone());
                "erstellt"
            }
        };
        self.output
            .push(format!("Hot Spot {} {verb}", hot_spot.name));
        if next {
            dialog.next(&hot_spot);
        } else {
            self.hot_spot_dialog = None;
        }
        // The table would cover the selection window while the next one is picked.
        self.evaluate_hot_spots(!next);
    }

    /// The table of hot spot values of the shown results file.
    fn hot_spot_window(&mut self, ctx: &egui::Context) {
        if !self.hot_spot_window || self.tree_view != TreeView::Results {
            return;
        }
        let Some(model) = self.results.get(self.current_result) else {
            return;
        };
        if model.hot_spots.reports.is_empty() {
            return;
        }
        let step = (model.results.as_ref())
            .and_then(ResultsView::current_increment)
            .map(|i| (i.step, i.increment));
        if !crate::hot_spots::window(ctx, &model.hot_spots, step) {
            self.hot_spot_window = false;
            self.update_contour();
        }
    }

    /// Finds the paths of the hot spot being edited or selected in the Results tree.
    fn update_hot_spot_preview(&mut self) {
        let current = self.current_result;
        let model = self.results.get(current);
        let wanted = match (&self.hot_spot_dialog, &self.tree.selected) {
            _ if self.tree_view != TreeView::Results => None,
            (Some(dialog), _) => Some(dialog.hot_spot()),
            (None, Some((TreeView::Results, TreeItem::HotSpot(i)))) => {
                model.and_then(|m| m.hot_spots.definitions.get(*i).cloned())
            }
            _ => None,
        };
        let shown = (self.hot_spot_preview.as_ref()).map(|(h, index, _)| (h, *index));
        if wanted.as_ref().map(|h| (h, current)) == shown {
            return;
        }
        self.hot_spot_preview = match (wanted, model) {
            (Some(hot_spot), Some(model)) => {
                let paths = crate::hot_spots::paths(model, &hot_spot);
                Some((hot_spot, current, paths))
            }
            _ => None,
        };
        self.update_contour();
    }

    /// Hot spot paths drawn over the results, deformed like them: of the hot spot edited or
    /// selected, else of all evaluated ones while their table is open.
    fn overlay_paths(&self) -> Vec<Vec<glam::Vec3>> {
        let model = self.results.get(self.current_result);
        let Some(model) = model.filter(|_| self.tree_view == TreeView::Results) else {
            return Vec::new();
        };
        match &self.hot_spot_preview {
            Some((_, index, paths)) if *index == self.current_result => {
                crate::hot_spots::render_paths(model, paths)
            }
            _ if self.hot_spot_window => {
                let reports = model.hot_spots.reports.iter();
                crate::hot_spots::render_paths(model, reports.flat_map(|r| &r.paths))
            }
            _ => Vec::new(),
        }
    }

    /// The y axis of an axisymmetric model as PrePoMax shows it, from below to above the
    /// shown model, in render coordinates. Results show it when they are flat like the 2D
    /// model they come from.
    fn axis_line(&self, shown: &Model) -> Option<[glam::Vec3; 2]> {
        let space = self.model.as_ref()?.fe.properties.space;
        if space != plx_model::ModelSpace::Axisymmetric {
            return None;
        }
        if shown.is_results() {
            let (min, max) = shown.mesh.bounds()?;
            let flat = (max[2] - min[2]).abs() <= 1e-6 * (max[0] - min[0]).abs().max(1e-30);
            if !flat {
                return None;
            }
        }
        let (min, max) = shown.visible_bounds()?;
        let origin = shown.global_origin();
        let margin = 0.1 * (max.y - min.y).max(max.x - min.x);
        Some([
            glam::Vec3::new(origin.x, min.y - margin, origin.z),
            glam::Vec3::new(origin.x, max.y + margin, origin.z),
        ])
    }

    /// The FE model, which can be set up.
    fn setup_model(&self) -> Option<&Model> {
        self.model.as_ref()
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
    /// or a history output or hot spot dialog of the results.
    fn picking(&self) -> bool {
        match self.tree_view {
            TreeView::Results => {
                (self.history_dialog.as_ref()).is_some_and(HistoryOutputDialog::picks)
                    || (self.hot_spot_dialog.as_ref()).is_some_and(HotSpotDialog::picks)
            }
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
        self.hot_spot_dialog = None;
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
        if ui
            .add_enabled(any, egui::Button::new("Hot Spot erstellen …"))
            .clicked()
        {
            self.create(NewItem::ResultHotSpot);
        }
        let evaluated = (self.results.get(self.current_result))
            .is_some_and(|m| !m.hot_spots.reports.is_empty());
        if ui
            .add_enabled(evaluated, egui::Button::new("Hot-Spot-Tabelle"))
            .clicked()
        {
            self.show_hot_spot_table();
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
        if kind == NewItem::ResultHotSpot {
            if let Some(model) = self.results.get(self.current_result) {
                self.hot_spot_dialog = Some(HotSpotDialog::create(&model.hot_spots.definitions));
                self.set_tree_view(TreeView::Results);
            }
            return;
        }
        if let NewItem::Feature(kind) = kind {
            // Paths read results; features belong to the tab they are created on.
            let on_results = matches!(kind, FeatureKind::ResultPath | FeatureKind::ResultPlane);
            let results =
                (on_results || self.tree_view == TreeView::Results).then_some(self.current_result);
            if let Some(model) = self.feature_model(results) {
                self.feature_dialog = Some(FeatureDialog::create(kind, results, model));
                if results.is_some() {
                    self.set_tree_view(TreeView::Results);
                }
            }
            return;
        }
        if kind == NewItem::ResultHistoryOutput {
            if let Some(view) = self.shown_results_view() {
                self.history_dialog = Some(HistoryOutputDialog::create(view));
            }
            return;
        }
        if kind == NewItem::MeshSetupItem {
            if let Some(model) = &self.model
                && let Some(geometry) = &model.geometry
            {
                let units = model.fe.properties.units;
                self.mesh_item_editor = Some(MeshItemEditor::create(geometry, units));
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
        if let Some(model) = &self.model
            && let Some(geometry) = &model.geometry
        {
            let units = model.fe.properties.units;
            self.mesh_setup = Some(MeshSetupWindow::new(&geometry.meshing, units));
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
        self.mesh_failure.clear();
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
                let space = model.fe.properties.space;
                for mut part in generated.meshes {
                    if let Err(error) = space.prepare_generated_mesh(&mut part.mesh) {
                        self.output
                            .push(format!("Vernetzung fehlgeschlagen: {error}"));
                        return;
                    }
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
                let lost = lost_selections(&model.fe, &model.mesh);
                if had_mesh && lost > 0 {
                    self.output.push(format!(
                        "Hinweis: {lost} Auswahlen aus Knoten- oder Elementnummern beziehen \
                         sich auf das alte Netz und müssen neu ausgewählt werden"
                    ));
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
            Err(error) => {
                let message = error.to_string();
                self.mesh_failure = plx_mesher::named_entities(&message).into_iter().collect();
                self.output
                    .push(format!("Vernetzung fehlgeschlagen: {message}"));
                // The geometry shows the faces and edges Gmsh names in red.
                if !self.mesh_failure.is_empty() {
                    self.output.push(
                        "Die betroffenen Flächen und Kanten sind in der Geometrie rot markiert"
                            .into(),
                    );
                    self.tree.selected = None;
                    self.set_tree_view(TreeView::Geometry);
                }
            }
        }
    }

    /// PrePoMax's Model menu: create items of the FE model.
    /// PrePoMax's Interaction menu: constraints, contacts and the search for contact pairs.
    fn interaction_menu(&mut self, ui: &mut egui::Ui) {
        if self.setup_model().is_none() {
            ui.label("Zuerst eine .inp-Datei öffnen");
            return;
        }
        let mut kind = None;
        for (item, label) in [
            (NewItem::Constraint, "Constraint erstellen …"),
            (
                NewItem::SurfaceInteraction,
                "Surface Interaction erstellen …",
            ),
            (NewItem::ContactPair, "Kontaktpaar erstellen …"),
        ] {
            if ui.button(label).clicked() {
                kind = Some(item);
            }
        }
        if let Some(kind) = kind {
            self.create(kind);
        }
        ui.separator();
        if ui.button("Kontaktpaare suchen …").clicked() {
            self.open_contact_search();
        }
    }

    fn model_menu(&mut self, ui: &mut egui::Ui) {
        if self.setup_model().is_none() {
            ui.label("Zuerst ein Modell anlegen oder öffnen");
            return;
        }
        if ui.button("Modelleigenschaften …").clicked() {
            self.edit_model_properties();
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
            (
                NewItem::InitialCondition,
                "Anfangsbedingung erstellen …",
                true,
            ),
            (NewItem::Amplitude, "Amplitude erstellen …", true),
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
            (
                NewItem::HistoryOutput(last_step.unwrap_or(0)),
                "History Output erstellen …",
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
            self.material_library = Some(MaterialLibraryEditor::new(
                &model.fe.materials,
                model.fe.properties.units,
            ));
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
        let findings = model.findings();
        match Analysis::start(&self.settings.solver, model, default_solver, check_model) {
            Ok(mut analysis) => {
                // Problems the checks found go into the monitor first, so that an abort is
                // explained even before CalculiX says anything.
                for finding in
                    (findings.iter()).filter(|f| f.severity() == plx_model::Severity::Error)
                {
                    analysis.note(format!(
                        "Modellprüfung: {}: {}",
                        finding.problem.title(),
                        finding.detail
                    ));
                }
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
                self.solver_findings.clear();
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
            if matches!(
                status,
                plx_job::JobStatus::Failed | plx_job::JobStatus::FailedWithResults
            ) && let Some(model) = self.model.as_ref()
            {
                self.solver_findings =
                    plx_model::diagnose_solver_output(analysis.output(), &model.mesh);
                for finding in (self.solver_findings.iter())
                    .filter(|f| f.item == plx_model::ModelItem::Analysis)
                {
                    analysis.note(format!(
                        "Mögliche Ursache: {}. {}",
                        finding.problem.title(),
                        finding.problem.fix()
                    ));
                }
                if !self.solver_findings.is_empty() {
                    analysis
                        .note("Das Warnsymbol an der Analyse im Baum erklärt die Ursache.".into());
                }
            }
        }
        if analysis.monitor
            && let MonitorEvent::OpenResults(path) = analysis.window(ctx)
        {
            self.open_results = Some(path);
        }
    }

    /// The project file of the open model, if it was opened from or saved to one that exists.
    /// Unsaved models and models from input or result files are not reopened at the next start.
    fn last_project(&self) -> Option<PathBuf> {
        self.model
            .as_ref()
            .filter(|model| model.is_project() && model.path.is_file())
            .map(|model| model.path.clone())
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
        if self.feature_picks()
            && let Some(dialog) = &mut self.feature_dialog
        {
            let model = match dialog.results {
                Some(index) => self.results.get(index),
                None => self.model.as_ref(),
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
        if self.transformation_picks()
            && let (Some(dialog), Some(model)) = (
                &mut self.transformation_dialog,
                self.results.get(self.current_result),
            )
        {
            let hit = model.pick_click(&click);
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
                let hit = model.pick_click(&click);
                dialog.click(
                    model,
                    hit.as_ref().map(|h| (h, click.precision_at(h.point))),
                );
            }
            return;
        }
        if !self.picking() {
            let hit = self.shown().and_then(|model| model.pick_click(&click));
            let view = self.tree_view;
            match hit {
                // Ctrl adds a part or takes it out again, Shift adds it.
                Some(hit) if click.ctrl => self.tree.toggle_part(view, hit.part),
                Some(hit) if click.shift => self.tree.add_parts(view, [hit.part]),
                Some(hit) => self.select_part(hit.part),
                // A click beside the model with Ctrl or Shift keeps the selection.
                None if click.ctrl || click.shift => {}
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
            if let Some(model) = self.results.get(self.current_result) {
                let hit = model.pick_click(&click);
                let pick = hit.as_ref().map(|hit| (hit, click.precision_at(hit.point)));
                let operation = Operation::from_modifiers(click.shift, click.ctrl);
                if let Some(dialog) = &mut self.hot_spot_dialog {
                    dialog.click(model, pick, operation);
                } else if let Some(dialog) = &mut self.history_dialog {
                    dialog.click(model, pick, operation);
                }
            }
            return;
        }
        if self.tree_view == TreeView::Geometry {
            if let (Some(editor), Some(view)) = (&mut self.mesh_item_editor, &self.geometry) {
                let hit = view.pick_click(&click);
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
        let hit = model.pick_click(&click);
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
        self.tree.select_part(view, index);
        self.tree.reveal = true;
    }

    /// The context menu of the 3D view: on a part it starts with the part's menu from the
    /// tree, so it makes no difference where the part is right-clicked.
    fn viewport_menu(&mut self, response: &egui::Response, right_click: Option<Click>) {
        if let Some(click) = right_click {
            self.menu_part = None;
            if !self.picking() {
                self.menu_part = (self.shown())
                    .and_then(|model| model.pick_click(&click))
                    .map(|hit| hit.part);
            }
            // A right click on one of several selected parts keeps them, as in the tree.
            if let Some(part) = self.menu_part
                && !self.tree.selected_parts(self.tree_view).contains(&part)
            {
                self.select_part(part);
            }
        }
        let part = (self.menu_part)
            .and_then(|index| Some((index, self.shown()?.parts.get(index)?.visible)));
        let mut tree_response = TreeResponse::default();
        let mut command = None;
        let selected = self.tree.selected_parts(self.tree_view);
        response.context_menu(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if let Some((index, visible)) = part {
                let view = self.tree_view;
                tree::part_menu(ui, index, visible, view, &selected, &mut tree_response);
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
            if let Some(model) = self.results.get(self.current_result) {
                if let Some(dialog) = &mut self.hot_spot_dialog {
                    dialog.box_select(model, area, operation);
                } else if let Some(dialog) = &mut self.history_dialog {
                    dialog.box_select(model, area, operation);
                }
            }
            return;
        }
        if self.tree_view == TreeView::Geometry {
            if let (Some(editor), Some(view)) = (&mut self.mesh_item_editor, &self.geometry) {
                editor.box_select(view, area, operation);
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
        if self.feature_picks()
            && let Some(dialog) = &self.feature_dialog
            && let Some(model) = self.feature_model(dialog.results)
        {
            self.viewport.preview = hover
                .and_then(|click| {
                    let hit = model.pick(click.origin, click.direction)?;
                    Some(dialog.preview(model, &hit, click.precision_at(hit.point)))
                })
                .unwrap_or_default();
            return;
        }
        if self.transformation_picks()
            && let (Some(dialog), Some(model)) = (
                &self.transformation_dialog,
                self.results.get(self.current_result),
            )
        {
            self.viewport.preview = hover
                .and_then(|click| {
                    let hit = model.pick_click(&click)?;
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
                    let hit = model.pick_click(&click)?;
                    Some(dialog.preview(model, &hit, click.precision_at(hit.point)))
                })
                .unwrap_or_default();
            return;
        }
        let hover = hover.filter(|_| self.picking());
        if self.tree_view == TreeView::Results {
            let model = self.results.get(self.current_result);
            self.viewport.preview = match (hover, model) {
                (Some(click), Some(model)) => model
                    .pick_click(&click)
                    .map(|hit| {
                        let precision = click.precision_at(hit.point);
                        match (&self.hot_spot_dialog, &self.history_dialog) {
                            (Some(dialog), _) => dialog.preview(model, &hit, precision),
                            (None, Some(dialog)) => dialog.preview(model, &hit, precision),
                            (None, None) => Default::default(),
                        }
                    })
                    .unwrap_or_default(),
                _ => Default::default(),
            };
            return;
        }
        if self.tree_view == TreeView::Geometry {
            let shown = (self.mesh_item_editor.as_ref()).zip(self.geometry.as_ref());
            self.viewport.preview = match (hover, shown) {
                (Some(click), Some((editor, view))) => view
                    .pick_click(&click)
                    .map(|hit| editor.preview(view, &hit, click.precision_at(hit.point)))
                    .unwrap_or_default(),
                _ => Default::default(),
            };
            return;
        }
        let preview = match (hover, &self.editor, &self.model) {
            (Some(click), Some(editor), Some(model)) => model
                .pick_click(&click)
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
        if let Some(data) = &self.history_plot
            && !crate::xy_plot::window(ctx, data)
        {
            self.history_plot = None;
        }
        let Some(mut table) = self.history_table.take() else {
            return;
        };
        let Some(view) = self.shown_results_view() else {
            return;
        };
        let kind = view
            .current_increment()
            .map_or(plx_results::AnalysisKind::Static, |i| i.kind);
        let unit = (view.history_output_index(table.set))
            .and_then(|i| view.history_outputs.get(i))
            .and_then(|o| o.unit());
        match table.show(ctx, &view.history, kind, unit) {
            crate::history_table::TableAction::Close => return,
            crate::history_table::TableAction::Plot(data) => self.history_plot = Some(data),
            crate::history_table::TableAction::None => {}
        }
        self.history_table = Some(table);
    }

    /// PrePoMax's Search Contact Pairs; it replaces an open item dialog.
    fn open_contact_search(&mut self) {
        if let Some(model) = self.setup_model() {
            self.contact_search = Some(ContactSearchDialog::new(&model.fe));
            self.editor = None;
            self.set_tree_view(TreeView::FeModel);
        }
    }

    fn contact_search_window(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(model)) = (&mut self.contact_search, &mut self.model) else {
            return;
        };
        match dialog.show(ctx, model) {
            SearchResult::Open => {}
            SearchResult::Ok(ties, pairs, joints) => {
                let mut created = format!(
                    "Kontaktsuche: {} Ties und {} Kontaktpaare erstellt",
                    ties.len(),
                    pairs.len()
                );
                if !joints.is_empty() {
                    created.push_str(&format!(", {} Node Ties", joints.len()));
                }
                // The first new item shows in the tree, even in a collapsed branch.
                let first = if !pairs.is_empty() {
                    Some(TreeItem::ContactPair(model.fe.contact_pairs.len()))
                } else if !joints.is_empty() {
                    Some(TreeItem::NodeTie(model.fe.node_ties.len()))
                } else {
                    (!ties.is_empty()).then_some(TreeItem::Constraint(model.fe.constraints.len()))
                };
                if let Some(item) = first {
                    self.tree.selected = Some((TreeView::FeModel, item));
                    self.tree.reveal = true;
                }
                model.fe.constraints.extend(ties);
                model.fe.contact_pairs.extend(pairs);
                model.fe.node_ties.extend(joints);
                self.output.push(created);
                self.contact_search = None;
                self.highlighted = None;
            }
            SearchResult::Cancel => {
                self.contact_search = None;
                self.highlighted = None;
            }
        }
    }

    /// PrePoMax's question before deleting, for the context menu and the Delete key alike.
    fn confirm_delete_window(&mut self, ctx: &egui::Context) {
        let Some((view, items)) = self.confirm_delete.clone() else {
            return;
        };
        let mut answer = None;
        // Like PrePoMax, the parts are named in the question.
        let parts: Vec<String> = (items.iter())
            .filter_map(|item| match item {
                TreeItem::Part(index) => (self.tree_model(view))
                    .and_then(|m| m.parts.get(*index))
                    .map(|p| p.name.clone()),
                _ => None,
            })
            .collect();
        let question = match parts.as_slice() {
            [] => "Ausgewähltes Element löschen?".into(),
            [name] => format!("Ausgewähltes Part löschen?\n{name}"),
            names => format!("Ausgewählte Parts löschen?\n{}", names.join(", ")),
        };
        egui::Modal::new(egui::Id::new("confirm delete")).show(ctx, |ui| {
            ui.label(question);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    answer = Some(true);
                }
                if ui.button("Abbrechen").clicked() {
                    answer = Some(false);
                }
            });
        });
        // Enter confirms and Escape cancels, as with the default buttons of a message box.
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Enter) {
                answer = Some(true);
            } else if i.key_pressed(egui::Key::Escape) {
                answer = Some(false);
            }
        });
        match answer {
            Some(true) => {
                self.confirm_delete = None;
                // The tree may have changed meanwhile, e.g. by another tab's selection.
                let selected = self.tree.selected_parts(view);
                let parts: Vec<usize> = (items.iter())
                    .filter_map(|item| match item {
                        TreeItem::Part(index) => Some(*index),
                        _ => None,
                    })
                    .collect();
                if !parts.is_empty() && parts.iter().all(|p| selected.contains(p)) {
                    self.delete_parts(view, &parts);
                } else if let [item] = items.as_slice()
                    && self.tree.selected.as_ref() == Some(&(view, item.clone()))
                {
                    self.delete_item(view, item.clone());
                }
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }

    /// The model a tree shows.
    fn tree_model(&self, view: TreeView) -> Option<&Model> {
        match view {
            TreeView::Results => self.results.get(self.current_result),
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        }
    }

    fn delete_parts(&mut self, view: TreeView, parts: &[usize]) {
        match view {
            TreeView::Geometry => self.delete_geometry_parts(parts),
            TreeView::FeModel => self.delete_mesh_parts(parts),
            TreeView::Results => {}
        }
    }

    fn delete_item(&mut self, view: TreeView, item: TreeItem) {
        if let TreeItem::Part(index) = item {
            self.delete_parts(view, &[index]);
        } else if let TreeItem::ResultFieldOutput(field) = item {
            self.delete_field_output(field);
        } else if let TreeItem::HistorySet(set) = item {
            self.delete_history_output(set);
        } else if let TreeItem::HotSpot(index) = item {
            self.delete_hot_spot(index);
        } else if let TreeItem::MeshItem(index) = item {
            if let Some(geometry) = self.model.as_mut().and_then(|m| m.geometry.as_mut())
                && index < geometry.mesh_items.len()
            {
                geometry.mesh_items.remove(index);
                self.tree.selected = None;
                self.mesh_item_editor = None;
            }
        } else if let Some(feature) = tree::feature(&item) {
            let model = match view {
                TreeView::Results => self.results.get_mut(self.current_result),
                _ => self.model.as_mut(),
            };
            if let Some(model) = model
                && crate::features::delete(&mut model.fe, feature)
            {
                self.tree.selected = None;
                self.feature_dialog = None;
                if let Some(view) = &mut model.results
                    && feature.kind == FeatureKind::ResultPlane
                {
                    view.plane_result = match view.plane_result {
                        Some(i) if i == feature.index => None,
                        Some(i) if i > feature.index => Some(i - 1),
                        other => other,
                    };
                }
            }
        } else if let Some(model) = self.model.as_mut()
            && crate::setup::delete(&mut model.fe, &item)
        {
            self.tree.selected = None;
            self.editor = None;
        }
    }

    /// PrePoMax's import into a model with geometry or a mesh: the new parts join the
    /// geometry. The parts already there keep their names, and the selections on the
    /// geometry and the mesh follow Gmsh's new numbering of the faces, edges and vertices.
    fn geometry_added(
        &mut self,
        paths: &[PathBuf],
        result: Result<Box<plx_mesher::CadAddition>, String>,
    ) {
        let files: Vec<String> = (paths.iter())
            .map(|p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            })
            .collect();
        let files = files.join(", ");
        let addition = match result {
            Ok(addition) => *addition,
            Err(error) => {
                self.output
                    .push(format!("Fehler beim Import von {files}: {error}"));
                return;
            }
        };
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let plx_mesher::CadAddition {
            import,
            renumbered,
            added,
        } = addition;
        let view = Model::geometry_view(&model.path, import.display);
        match geometry_check(model.fe.properties.space, &view) {
            Ok(faces) if !faces.is_empty() => {
                let faces: Vec<String> = faces.iter().map(i32::to_string).collect();
                self.output.push(format!(
                    "Hinweis: Normale von Fläche {} zeigt in -z; die Elemente werden beim \
                     Vernetzen umgedreht.",
                    faces.join(", ")
                ));
            }
            Ok(_) => {}
            Err(error) => {
                self.output
                    .push(format!("Fehler beim Import von {files}: {error}"));
                return;
            }
        }
        if model.geometry.is_some() {
            model.fe.renumber_cad(&renumbered);
            if model.has_cad() {
                let mut mesh = model.mesh.clone();
                mesh.cad = mesh.cad.renumbered(&renumbered);
                model.set_mesh(mesh);
            }
        }
        model.geometry = Some(import.geometry);
        for warning in &import.warnings {
            self.output.push(format!("Warnung: {warning}"));
        }
        self.output.push(format!(
            "{files} importiert: {} Part(s) hinzugefügt ({})",
            added.len(),
            added.join(", ")
        ));
        self.geometry = Some(view);
        self.after_parts_changed();
        self.set_tree_view(TreeView::Geometry);
        self.view_command = Some(ViewCommand::Fit);
    }

    /// PrePoMax's Delete of geometry parts: the geometry loses the solids or faces; a mesh
    /// already generated from them stays, as parts of the FE model.
    fn delete_geometry_parts(&mut self, indices: &[usize]) {
        let names: Vec<String> = (self.geometry.as_ref())
            .map(|g| {
                (indices.iter())
                    .filter_map(|&i| g.parts.get(i))
                    .map(|p| p.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let Some(geometry) = &model.geometry else {
            return;
        };
        if names.is_empty() {
            return;
        }
        let name = names.join(", ");
        // All in one go, so that Gmsh numbers the faces, edges and vertices anew once.
        let smaller = match plx_mesher::delete_parts_renumbered(geometry, &names) {
            Ok((smaller, tags)) => {
                // Gmsh numbers the faces, edges and vertices anew; the mesh and the
                // selections on the geometry follow.
                if model.has_cad() {
                    let mut mesh = model.mesh.clone();
                    mesh.cad = mesh.cad.renumbered(&tags);
                    model.fe.renumber_cad(&tags);
                    model.set_mesh(mesh);
                }
                smaller
            }
            Err(error) => {
                self.output
                    .push(format!("{name} kann nicht gelöscht werden: {error}"));
                return;
            }
        };
        self.geometry = match &smaller {
            Some(geometry) => match plx_mesher::tessellate(geometry) {
                Ok(display) => Some(Model::geometry_view(&model.path, display)),
                Err(error) => {
                    self.output
                        .push(format!("Geometrie kann nicht angezeigt werden: {error}"));
                    None
                }
            },
            None => None,
        };
        model.geometry = smaller;
        self.output.push(deleted_message(&names));
        self.after_parts_changed();
    }

    /// PrePoMax's Delete of mesh parts: their elements go, with the nodes no other part has.
    fn delete_mesh_parts(&mut self, indices: &[usize]) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let names: Vec<String> = (indices.iter())
            .filter_map(|&i| model.parts.get(i))
            .map(|p| p.name.clone())
            .collect();
        if names.is_empty() {
            return;
        }
        // By name, which stays while the others go.
        let mut mesh = model.mesh.clone();
        for name in &names {
            mesh = plx_mesher::delete_mesh_part(&mesh, name);
        }
        model.set_mesh(mesh);
        self.output.push(deleted_message(&names));
        self.after_parts_changed();
    }

    fn after_parts_changed(&mut self) {
        self.tree.selected = None;
        self.menu_part = None;
        self.dialog = None;
        self.mesh_item_editor = None;
        self.highlighted = None;
        self.symbols_shown = None;
        self.frame_cache.clear();
        self.results_changed = true;
    }

    fn editor_window(&mut self, ctx: &egui::Context) {
        let (Some(editor), Some(model)) = (&mut self.editor, &mut self.model) else {
            return;
        };
        match editor.show(ctx, model) {
            EditorResult::Open => {}
            EditorResult::Ok => {
                if let Some(editor) = self.editor.take() {
                    let created = editor.new_interaction_item(&model.fe);
                    editor.apply(&mut model.fe);
                    // The new item shows in the tree, even in a collapsed branch.
                    if let Some(item) = created {
                        self.tree.selected = Some((TreeView::FeModel, item));
                        self.tree.reveal = true;
                    }
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
            let shown = on_results && index == current;
            let dialog = (self.history_dialog.as_ref()).filter(|_| shown);
            let hot_spot = (self.hot_spot_dialog.as_ref()).filter(|_| shown);
            let transformation = (self.transformation_dialog.as_ref())
                .filter(|d| on_results && d.result == index && index == current);
            let feature = (self.feature_dialog.as_ref())
                .filter(|d| on_results && d.results == Some(index) && index == current);
            let highlight = match (dialog, &self.tree.selected) {
                _ if feature.is_some() => feature.map(FeatureDialog::highlight).unwrap_or_default(),
                _ if transformation.is_some() => transformation
                    .map(TransformationDialog::highlight)
                    .unwrap_or_default(),
                _ if hot_spot.is_some() => hot_spot.map(|d| d.highlight(model)).unwrap_or_default(),
                (Some(dialog), _) => dialog.highlight(model),
                (None, Some((TreeView::Results, TreeItem::Part(_)))) if index == current => {
                    Highlight::parts(self.tree.selected_parts(TreeView::Results))
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
        let feature = self.feature_dialog.as_ref().filter(|d| d.results.is_none());
        let highlight = if let Some(dialog) = &self.section_dialog {
            self.highlighted = None;
            dialog.highlight()
        } else if let Some(dialog) = feature {
            self.highlighted = None;
            dialog.highlight()
        } else if let Some(dialog) = &self.contact_search {
            self.highlighted = None;
            dialog.highlight(model)
        } else if let Some(editor) = &self.editor {
            self.highlighted = None;
            editor.highlight(model)
        } else {
            let parts = self.tree.selected_parts(TreeView::FeModel);
            let selection = (self.tree.selected.clone(), parts.clone());
            if self.highlighted.as_ref() == Some(&selection) {
                return;
            }
            self.highlighted = Some(selection);
            match &self.tree.selected {
                Some((TreeView::FeModel, TreeItem::Part(_))) => Highlight::parts(parts),
                Some((TreeView::FeModel, item)) => crate::setup::item_highlight(model, item),
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
            (None, Some((TreeView::Geometry, TreeItem::Part(_)))) => {
                Highlight::parts(self.tree.selected_parts(TreeView::Geometry))
            }
            (None, Some((TreeView::Geometry, TreeItem::MeshItem(index)))) => items
                .get(*index)
                .map(|item| crate::meshing::item_highlight(view, &item.kind))
                .unwrap_or_default(),
            // Without a selection, the faces and edges meshing failed on.
            _ => crate::meshing::entities_highlight(view, &self.mesh_failure),
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
        // As in PrePoMax, symbols rest while the exploded view is edited or moves.
        let exploding = self.exploded_dialog.is_some()
            || (self.model.as_ref()).is_some_and(|m| m.explosion.is_animating());
        let items = if exploding {
            Vec::new()
        } else {
            self.symbol_items()
        };
        let shown = (self.model.as_ref())
            .map(|m| m.parts.iter().map(|p| p.visible).collect())
            .unwrap_or_default();
        let version = (self.model.as_ref()).map_or(0, |m| m.explosion.version());
        let key = (items, shown, version);
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
            | TreeItem::FieldOutput(s, _)
            | TreeItem::HistoryOutput(s, _) => Some(s),
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

    fn shown_model(&self) -> ShownModel {
        ShownModel(self.tree_view, self.current_result)
    }

    fn model_mut(&mut self, which: ShownModel) -> Option<&mut Model> {
        match which.0 {
            TreeView::Results => self.results.get_mut(which.1),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        }
    }

    /// Whether the shown model is exploded.
    fn exploded_applied(&self) -> bool {
        self.shown().is_some_and(|m| m.explosion.applied.is_some())
    }

    /// PrePoMax's toolbar button: explodes the shown model by the last used parameters at
    /// half the scale, or puts it back together, animated either way.
    fn toggle_exploded(&mut self) {
        if self.exploded_dialog.is_some() {
            return;
        }
        let last = self.last_exploded.clone();
        let Some(model) = self.shown_mut() else {
            return;
        };
        let (from, to, parameters) = match model.explosion.applied.take() {
            Some(parameters) => (parameters.scale(), 0.0, parameters),
            None if model.parts.len() > 1 => {
                let parameters = crate::exploded::Parameters {
                    scale_factor: 0.5,
                    ..last
                };
                model.explosion.applied = Some(parameters.clone());
                (0.0, parameters.scale(), parameters)
            }
            None => return,
        };
        animate_explosion(model, &parameters, from, to);
        if model.explosion.applied.is_some() {
            self.last_exploded = parameters;
        }
        self.viewport.preview = Default::default();
    }

    /// Opens the exploded view dialog on the shown model and previews it at once.
    fn open_exploded_dialog(&mut self) {
        if self.exploded_dialog.is_some() {
            return;
        }
        let which = self.shown_model();
        let last = self.last_exploded.clone();
        let Some(model) = self.model_mut(which) else {
            return;
        };
        let center = model.mesh.bounds().map_or(glam::DVec3::ZERO, |(a, b)| {
            (glam::DVec3::from(a) + glam::DVec3::from(b)) * 0.5
        });
        let mut dialog = ExplodedDialog::new(model.explosion.applied.clone(), &last, center);
        preview_explosion(model, &mut dialog, true);
        self.exploded_dialog = Some((dialog, which));
    }

    fn exploded_window(&mut self, ctx: &egui::Context) {
        let Some((mut dialog, which)) = self.exploded_dialog.take() else {
            return;
        };
        // Switching to another model ends the dialog as if it was cancelled.
        let (result, change) = if which == self.shown_model() {
            dialog.show(ctx)
        } else {
            (ExplodedResult::Cancel, None)
        };
        let Some(model) = self.model_mut(which) else {
            return;
        };
        if let Some(change) = change {
            let animate = change == crate::exploded::dialog::Change::Parameter;
            preview_explosion(model, &mut dialog, animate);
        }
        match result {
            ExplodedResult::Open => {
                self.exploded_dialog = Some((dialog, which));
                return;
            }
            ExplodedResult::Ok => {
                let parameters = dialog.draft.clone();
                let offsets = model.explosion_offsets(&parameters);
                let nothing = offsets.iter().all(|o| *o == glam::DVec3::ZERO);
                // A scale factor of zero shows the model assembled, which is no exploded view.
                model.explosion.applied = (!nothing).then(|| parameters.clone());
                if !model.explosion.is_animating() {
                    model.explosion.show(offsets, false);
                }
                self.last_exploded = parameters;
            }
            ExplodedResult::Cancel => {
                match &dialog.before {
                    Some(before) => {
                        let offsets = model.explosion_offsets(before);
                        model.explosion.show(offsets, true);
                    }
                    None => model.explosion.show(Vec::new(), true),
                }
                model.explosion.applied = dialog.before.clone();
            }
            ExplodedResult::Disable => {
                model.explosion.show(Vec::new(), true);
                model.explosion.applied = None;
            }
        }
        self.viewport.preview = Default::default();
    }

    /// Advances the exploded view animations; the shown model is rebuilt when it moved.
    fn update_explosion(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        let shown = self.shown_model();
        let mut animating = false;
        let mut moved = false;
        let models = (self.model.iter_mut().map(|m| (TreeView::FeModel, 0, m)))
            .chain(self.geometry.iter_mut().map(|m| (TreeView::Geometry, 0, m)))
            .chain((self.results.iter_mut().enumerate()).map(|(i, m)| (TreeView::Results, i, m)));
        for (view, index, model) in models {
            let changed = model.explosion.tick(now);
            animating |= model.explosion.is_animating();
            let is_shown = view == shown.0 && (view != TreeView::Results || index == shown.1);
            moved |= changed && is_shown;
        }
        if moved {
            self.results_changed = true;
            self.viewport.preview = Default::default();
        }
        if animating {
            ctx.request_repaint();
        }
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

    /// The model whose features a dialog edits: a results file, or the FE model.
    fn feature_model(&self, results: Option<usize>) -> Option<&Model> {
        match results {
            Some(index) => self.results.get(index),
            None => self.model.as_ref(),
        }
    }

    fn feature_picks(&self) -> bool {
        (self.feature_dialog.as_ref()).is_some_and(|d| d.picks() && self.feature_dialog_shown())
    }

    /// Whether the open feature dialog belongs to what the 3D view shows.
    fn feature_dialog_shown(&self) -> bool {
        match self.feature_dialog.as_ref().map(|d| d.results) {
            Some(Some(index)) => {
                self.tree_view == TreeView::Results && index == self.current_result
            }
            Some(None) => self.tree_view == TreeView::FeModel,
            None => false,
        }
    }

    fn feature_window(&mut self, ctx: &egui::Context) {
        if self.feature_dialog.is_some() && !self.feature_dialog_shown() {
            self.feature_dialog = None;
            self.viewport.preview = Default::default();
        }
        let Some(mut dialog) = self.feature_dialog.take() else {
            return;
        };
        let model = match dialog.results {
            Some(index) => self.results.get_mut(index),
            None => self.model.as_mut(),
        };
        let Some(model) = model else {
            return;
        };
        let cut = self.plane_values.as_ref();
        match dialog.show(ctx, model, cut) {
            FeatureResult::Open => self.feature_dialog = Some(dialog),
            FeatureResult::Ok => {
                dialog.apply(&mut model.fe);
                self.viewport.preview = Default::default();
            }
            FeatureResult::Cancel => self.viewport.preview = Default::default(),
        }
    }

    /// Results take the coordinate systems of their features, for transformed field outputs.
    fn sync_result_features(&mut self) {
        for model in &mut self.results {
            let Some(view) = &mut model.results else {
                continue;
            };
            if view.coordinate_systems == model.fe.coordinate_systems {
                continue;
            }
            let systems = model.fe.coordinate_systems.clone();
            let warnings = view.set_coordinate_systems(systems, &model.mesh);
            self.output.extend(warnings);
            self.results_changed = true;
        }
    }

    /// Reference points, coordinate systems and the result path drawn over the shown model.
    fn update_feature_overlay(&mut self) {
        let Some(model) = self.shown() else {
            self.viewport.overlay.features.clear();
            self.viewport.overlay.result_path = None;
            return;
        };
        let dialog = self
            .feature_dialog
            .as_ref()
            .filter(|_| self.feature_dialog_shown());
        let selected = match &self.tree.selected {
            Some((view, item)) if *view == self.tree_view => tree::feature(item),
            _ => None,
        };
        let edited = dialog.and_then(FeatureDialog::item);
        let mut features = crate::features::marks(model, selected, edited);
        features.extend(dialog.and_then(|d| d.mark(model)));
        let result_path = match dialog {
            Some(dialog) if dialog.kind() == FeatureKind::ResultPath => dialog.path_line(model),
            _ => selected
                .filter(|s| s.kind == FeatureKind::ResultPath)
                .and_then(|s| model.fe.result_paths.get(s.index))
                .and_then(|p| crate::features::path_line(model, p)),
        };
        self.viewport.overlay.features = features;
        self.viewport.overlay.result_path = result_path;
    }

    /// The plane whose cut the shown results show alone: that of a plane result being
    /// edited, or of the checked one; a point on it and its unit normal.
    fn shown_plane_cut(&self) -> Option<(glam::DVec3, glam::DVec3)> {
        if self.tree_view != TreeView::Results {
            return None;
        }
        let model = self.results.get(self.current_result)?;
        let view = model.results.as_ref()?;
        let edited = (self.feature_dialog.as_ref())
            .filter(|d| d.results == Some(self.current_result))
            .and_then(FeatureDialog::result_plane);
        let name = match edited {
            Some(name) => name,
            None => &model.fe.result_planes.get(view.plane_result?)?.plane,
        };
        let (point, normal) = model.fe.plane(name)?.resolve(&model.fe).ok()?;
        Some((point.into(), normal.into()))
    }

    /// Cuts the scene at the section plane being edited or shown, or at the plane of shown
    /// plane results, when it or the scene changed.
    fn update_section(&mut self) {
        // Planes of features follow them.
        if let Some(fe) = self.shown().map(|m| m.fe.clone()) {
            if let Some(dialog) = &mut self.section_dialog {
                dialog.draft.resolve(&fe);
            }
            if let Some(section) = &mut self.section {
                section.resolve(&fe);
            }
        }
        // As in PrePoMax, the section view rests while the exploded view is edited or moves.
        let exploding = self.exploded_dialog.is_some()
            || self.shown().is_some_and(|m| m.explosion.is_animating());
        // Plane results show the cut alone, in place of the section view.
        let plane = self.shown_plane_cut().map(|(point, normal)| {
            let definition = PlaneDefinition::PointNormal { point, normal };
            let cut = SectionView {
                definition,
                flipped: false,
                lighten: false,
            };
            (cut, true)
        });
        let section = (self.section_dialog.as_ref())
            .map(|d| &d.draft)
            .or(self.section.as_ref())
            .map(|s| (s.clone(), false));
        let visible = (self.shown())
            .map(|m| m.parts.iter().map(|p| p.visible).collect())
            .unwrap_or_default();
        let wanted = plane
            .or(section)
            .filter(|_| !exploding)
            .map(|(cut, alone)| (self.viewport.scene_version(), visible, cut, alone));
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
        let (mut values, mut plane_values) = (None, None);
        match (shown, &wanted) {
            (Some(model), Some((_, _, section, alone))) => {
                // The model origin is the centre of its bounds, where a principal plane's
                // manipulator sits.
                let anchor = section.anchor(model.origin());
                let normal = section.normal();
                let clip = plx_render::clip_plane(anchor, normal, model.origin());
                let faces = model.section_meshes(anchor, normal, section.lighten);
                model.clip = Some(clip);
                self.viewport.set_section(Some((clip, &faces)));
                let alone = *alone && model.results.is_some();
                self.viewport.set_sections_only(alone);
                if alone {
                    values = model.section_values(anchor, normal, true);
                    plane_values = model.section_values(anchor, normal, false);
                }
            }
            _ => {
                self.viewport.set_section(None);
                self.viewport.set_sections_only(false);
            }
        }
        // The legend of the shown results takes the range on the plane; the others their own.
        let shown_range = values
            .as_ref()
            .map(|v| v.extremes.map(|[min, max]| (min.1, max.1)));
        let on_results = self.tree_view == TreeView::Results;
        for (index, model) in self.results.iter_mut().enumerate() {
            let range = match shown_range {
                Some(range) if on_results && index == self.current_result => range,
                _ => None,
            };
            if let Some(view) = &mut model.results
                && view.section_range != range
            {
                view.section_range = range;
                self.results_changed = true;
            }
        }
        self.section_values = values;
        self.plane_values = plane_values;
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

    /// The unit system CAD geometry is read in: that of an empty model it goes into, such as
    /// one just created with File > New, else that of new models.
    fn import_units(&self) -> UnitSystem {
        match &self.model {
            Some(model) if model.mesh.element_count() == 0 && model.geometry.is_none() => {
                model.fe.properties.units
            }
            _ => self.settings.new_model.units,
        }
    }

    /// PrePoMax's File > New: asks for the model space and unit system of the new model,
    /// with the last choice proposed. `then_import` opens the geometry import afterwards.
    fn new_model(&mut self, then_import: bool) {
        self.model_dialog = Some(ModelPropertiesDialog::new_model(
            self.settings.new_model,
            then_import,
        ));
    }

    fn edit_model_properties(&mut self) {
        if let Some(model) = &self.model {
            self.model_dialog = Some(ModelPropertiesDialog::edit(model.fe.properties));
        }
    }

    fn model_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.model_dialog else {
            return;
        };
        let mesh = (self.model.as_ref())
            .filter(|_| dialog.editing)
            .map(|m| &m.mesh);
        let geometry = self.geometry.as_ref().filter(|_| dialog.editing);
        let result = dialog.show(ctx, mesh, geometry);
        let (editing, then_import, convert) = (dialog.editing, dialog.then_import, dialog.convert);
        match result {
            DialogResult::Open => return,
            DialogResult::Cancel => {}
            DialogResult::Ok(properties) if editing => {
                self.set_model_properties(properties, convert)
            }
            DialogResult::Ok(properties) => {
                self.settings.new_model = properties;
                self.create_model(properties);
                self.import_requested = then_import;
            }
        }
        self.model_dialog = None;
    }

    /// Starts an empty model, to import geometry into or to save as a project.
    fn create_model(&mut self, properties: plx_model::ModelProperties) {
        self.close_model();
        let mut model = Model::new(
            std::path::Path::new("Unbenannt"),
            plx_mesh::FeMesh::default(),
        );
        model.fe.properties = properties;
        self.output.push(format!(
            "Neues Modell: {}, {}",
            properties.space.label(),
            properties.units.label()
        ));
        self.model = Some(model);
        self.set_tree_view(TreeView::Geometry);
        self.viewport.set_parts(&[]);
        self.frame_cache.clear();
        self.update_contour();
    }

    /// Takes over changed model properties; a new model space retypes the mesh's surface
    /// elements, as PrePoMax does. A new unit system converts the model with `convert`.
    fn set_model_properties(&mut self, properties: plx_model::ModelProperties, convert: bool) {
        let Some(model) = &mut self.model else {
            return;
        };
        let old = model.fe.properties;
        if convert && old.units != properties.units {
            self.convert_units(properties.units);
        }
        let Some(model) = &mut self.model else {
            return;
        };
        model.fe.properties = properties;
        if old.space != properties.space {
            let mut mesh = model.mesh.clone();
            if properties.space.convert_mesh(&mut mesh) {
                model.set_mesh(mesh);
                self.results_changed = true;
                self.highlighted = None;
            }
            self.output
                .push(format!("Modellraum: {}", properties.space.label()));
        }
        if old.units != properties.units {
            self.output
                .push(format!("Einheitensystem: {}", properties.units.label()));
        }
        self.results_changed = true;
    }

    /// Converts the open model into another unit system: its values, its mesh and its
    /// geometry, which are scaled for the new length unit.
    fn convert_units(&mut self, units: UnitSystem) {
        let Some(model) = &mut self.model else {
            return;
        };
        let conversion = plx_model::convert::Conversion::new(model.fe.properties.units, units);
        let notes = model.fe.convert_units(units);
        let factor = conversion.length_factor();
        if factor != 1.0 {
            if model.mesh.node_count() > 0 {
                let mut mesh = model.mesh.clone();
                mesh.scale(factor);
                model.set_mesh(mesh);
                self.highlighted = None;
            }
            if let Some(geometry) = &mut model.geometry {
                geometry.convert_sizes(&conversion);
                match plx_mesher::scale_geometry(geometry, factor) {
                    Ok(scaled) => {
                        *geometry = scaled;
                        match plx_mesher::tessellate(geometry) {
                            Ok(display) => {
                                self.geometry = Some(Model::geometry_view(&model.path, display))
                            }
                            Err(error) => self
                                .output
                                .push(format!("Geometrie kann nicht angezeigt werden: {error}")),
                        }
                    }
                    Err(error) => self.output.push(format!(
                        "Geometrie konnte nicht umgerechnet werden: {error}"
                    )),
                }
            }
            self.frame_cache.clear();
            self.view_command = Some(ViewCommand::Fit);
        }
        self.output.extend(notes);
        self.output.push(format!(
            "Modell in {} umgerechnet (Längen × {factor})",
            units.label()
        ));
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
        self.exploded_dialog = None;
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
        if model.explosion.is_shown()
            && let Some(bounds) = model.visible_bounds()
        {
            self.viewport.cover(bounds);
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
        // Results have their own unit system; the geometry is in that of the FE model.
        let units = match view {
            Some(view) => view.units,
            None => self
                .model
                .as_ref()
                .map_or(model.fe.properties.units, |m| m.fe.properties.units),
        };
        let axis = self.axis_line(model);
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
        // On the section plane the extremes lie between nodes.
        let section = (self.section_values.as_ref())
            .and_then(|v| v.extremes)
            .filter(|_| view.is_some_and(|v| v.section_range.is_some()));
        let section_marker = |label: &str, (position, value): (glam::DVec3, f32)| {
            Some(Marker {
                position: model.to_render(position.to_array()),
                text: format!("{label}: {}\nSection plane", format_legend_value(value)),
            })
        };
        self.viewport.overlay = Overlay {
            legend: view.and_then(ResultsView::legend),
            status: view
                .filter(|_| post.status_block)
                .map_or_else(Vec::new, |v| v.status_lines(&model.file_name())),
            maximum: view.filter(|_| post.max_label).and_then(|v| match section {
                Some([_, max]) => section_marker("Max", max),
                None => marker("Max", v.maximum()),
            }),
            minimum: view.filter(|_| post.min_label).and_then(|v| match section {
                Some([min, _]) => section_marker("Min", min),
                None => marker("Min", v.minimum()),
            }),
            global_origin: graphics.global_axes.then(|| model.global_origin()),
            show_scale_bar: graphics.scale_bar,
            length_unit: units.unit(plx_model::Quantity::Length),
            show_view_triad: graphics.view_triad,
            nodes: (model.highlight.nodes.iter())
                .filter_map(|&id| model.node_position(model.mesh.node_index(id)?))
                .chain(transformation.iter().flat_map(|d| d.points(model)))
                .collect(),
            edges: render_lines(model, &model.highlight.lines),
            secondary_edges: render_lines(model, &model.highlight.secondary_lines),
            axis,
            paths: self.overlay_paths(),
            lines: transformation.map_or_else(Vec::new, |d| d.lines(model)),
            features: Vec::new(),
            result_path: None,
        };
        self.update_feature_overlay();
    }
}

/// Lines between nodes in render coordinates; lines with an unknown node are left out.
fn render_lines(model: &Model, lines: &[[plx_mesh::NodeId; 2]]) -> Vec<[glam::Vec3; 2]> {
    (lines.iter())
        .filter_map(|ends| {
            let [a, b] = ends.map(|id| {
                model
                    .mesh
                    .node_index(id)
                    .and_then(|n| model.node_position(n))
            });
            Some([a?, b?])
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn the_project_stored_at_exit_is_read_back() {
        let path = std::env::temp_dir().join("prepolix_test_letztes_projekt.plx");
        std::fs::write(&path, "").unwrap();
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, LAST_PROJECT_KEY, &Some(path.clone()));
        assert_eq!(stored_last_project(&storage), Some(path.clone()));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(stored_last_project(&storage), None);
        eframe::set_value(&mut storage, LAST_PROJECT_KEY, &None::<PathBuf>);
        assert_eq!(stored_last_project(&storage), None);
        assert_eq!(stored_last_project(&MemoryStorage::default()), None);
    }
}
