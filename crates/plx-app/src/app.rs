use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use plx_render::StandardView;

use crate::animation::{AnimationKind, ColorLimits, Playback};
use crate::icons::{self, Icon};
use crate::model::{self, LoadedModel, Model};
use crate::overlay::{Marker, Overlay};
use crate::properties;
use crate::results::{Deformation, ResultsView, format_legend_value};
use crate::tree::{self, TreeItem, TreeState, TreeView};
use crate::viewport::{ViewCommand, Viewport};
use plx_render::RenderMesh;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tab {
    Tree(TreeView),
    Viewport,
    Output,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Tree(view) => view.title(),
            Tab::Viewport => "3D-Ansicht",
            Tab::Output => "Ausgabe",
        }
    }
}

enum LoadEvent {
    Started(PathBuf),
    Finished(PathBuf, Result<Box<LoadedModel>, String>),
}

struct Workbench {
    viewport: Viewport,
    model: Option<Model>,
    tree: TreeState,
    /// Item whose properties window is open.
    dialog: Option<TreeItem>,
    /// Tree view to bring to the front, after loading a model.
    show_tree: Option<TreeView>,
    output: Vec<String>,
    view_command: Option<ViewCommand>,
    /// The result selection or deformation changed; the scene must be rebuilt.
    results_changed: bool,
    /// Only the animation frame changed; frames already built are reused.
    frame_changed: bool,
    /// Scene of each animation frame shown so far, cleared when anything else changes.
    frame_cache: std::collections::HashMap<usize, Vec<RenderMesh>>,
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
        crate::style::apply(&cc.egui_ctx);
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
                tree: TreeState::default(),
                dialog: None,
                show_tree: None,
                output,
                view_command: None,
                results_changed: false,
                frame_changed: false,
                frame_cache: Default::default(),
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
                .add_filter(
                    "CalculiX-Modell oder -Ergebnisse (*.inp, *.frd)",
                    &["inp", "INP", "frd", "FRD"],
                )
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
                if ui
                    .add_enabled(self.workbench.model.is_some(), new)
                    .clicked()
                {
                    self.workbench.close_model();
                }
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
                if ui.button("Einpassen").clicked() {
                    self.workbench.view_command = Some(ViewCommand::Fit);
                }
                for (view, label) in STANDARD_VIEWS {
                    if ui.button(label).clicked() {
                        self.workbench.view_command = Some(ViewCommand::View(view));
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

    /// PrePoMax's main tool bar: file commands, then views and display options.
    fn tool_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let has_model = self.workbench.model.is_some();
            if icons::button(ui, Icon::New, "Neu (Strg+N)", has_model, false).clicked() {
                self.workbench.close_model();
            }
            let can_open = self.loading.is_none();
            if icons::button(ui, Icon::Open, "Öffnen (Strg+O)", can_open, false).clicked() {
                self.open_dialog(ui.ctx());
            }
            icons::button(
                ui,
                Icon::Save,
                "Speichern (noch nicht implementiert)",
                false,
                false,
            );
            ui.separator();
            if icons::button(ui, Icon::Fit, "Einpassen", true, false).clicked() {
                self.workbench.view_command = Some(ViewCommand::Fit);
            }
            for (view, label) in STANDARD_VIEWS {
                if icons::button(ui, Icon::View(view), label, true, false).clicked() {
                    self.workbench.view_command = Some(ViewCommand::View(view));
                }
            }
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

const STANDARD_VIEWS: [(StandardView, &str); 7] = [
    (StandardView::Front, "Vorne"),
    (StandardView::Back, "Hinten"),
    (StandardView::Top, "Oben"),
    (StandardView::Bottom, "Unten"),
    (StandardView::Left, "Links"),
    (StandardView::Right, "Rechts"),
    (StandardView::Isometric, "Isometrisch"),
];

fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Viewport]);
    let surface = dock.main_surface_mut();
    let trees = [TreeView::Geometry, TreeView::FeModel, TreeView::Results].map(Tab::Tree);
    let [viewport, _] = surface.split_left(NodeIndex::root(), 0.2, trees.to_vec());
    surface.split_below(viewport, 0.82, vec![Tab::Output]);
    if let Some(path) = dock.find_tab(&Tab::Tree(TreeView::FeModel)) {
        let _ = dock.set_active_tab(path);
    }
    dock
}

impl eframe::App for PrepolixApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_load_events();
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.open_dialog(&ctx);
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
            if let Some(view) = self
                .workbench
                .model
                .as_mut()
                .and_then(|m| m.results.as_mut())
            {
                ui.separator();
                if results_tool_bar(ui, view) {
                    self.workbench.results_changed = true;
                }
            }
        });
        self.workbench.animate(&ctx);
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if let Some(view) = self.workbench.show_tree.take()
            && let Some(path) = self.dock.find_tab(&Tab::Tree(view))
        {
            let _ = self.dock.set_active_tab(path);
        }
        let dock_style = crate::style::dock_style(ui.style());
        DockArea::new(&mut self.dock)
            .style(dock_style)
            .show_close_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_leaf_collapse_buttons(false)
            .show_inside(ui, &mut self.workbench);
        self.workbench.properties_window(&ctx);
        self.workbench.rebuild_if_results_changed();

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
                if let Some(view) = &model.results {
                    self.output.push(format!(
                        "{} Ergebnis-Inkrement(e) gelesen",
                        view.increments.len()
                    ));
                }
                self.viewport.set_parts(&render_meshes);
                self.viewport
                    .apply(ViewCommand::Fit, model.visible_bounds());
                self.show_tree = Some(if model.results.is_some() {
                    TreeView::Results
                } else {
                    TreeView::FeModel
                });
                self.tree.selected = model
                    .results
                    .as_ref()
                    .map(|v| (TreeView::Results, TreeItem::Component(v.field, v.component)));
                self.dialog = None;
                self.model = Some(model);
                self.update_contour();
            }
            Err(error) => self.output.push(format!("Fehler beim Laden: {error}")),
        }
    }

    fn model_tree(&mut self, ui: &mut egui::Ui, view: TreeView) {
        if self.model.is_none() && view != TreeView::Geometry {
            ui.weak("Kein Modell geladen.\nDatei > Öffnen (Strg+O) oder eine .inp- oder .frd-Datei ins Fenster ziehen.");
            ui.separator();
        }
        let response = tree::show(ui, view, self.model.as_mut(), &mut self.tree);
        for (index, visible) in response.visibility {
            self.viewport.set_part_visible(index, visible);
        }
        if let (Some((field, component)), Some(results)) = (
            response.component,
            self.model.as_mut().and_then(|m| m.results.as_mut()),
        ) {
            results.field = field;
            results.component = component;
            self.results_changed = true;
        }
        if let Some(item) = response.open {
            self.dialog = Some(item);
        }
    }

    /// PrePoMax-style properties dialog of the double-clicked tree item.
    fn properties_window(&mut self, ctx: &egui::Context) {
        let Some(item) = self.dialog.clone() else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new(properties::title(self.model.as_ref(), &item))
            .id(egui::Id::new("properties window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                properties::show(ui, self.model.as_ref(), &item);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close = ui.button("Schließen").clicked();
                });
            });
        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dialog = None;
        }
    }

    /// Removes the model and its results, like PrePoMax's File > New.
    fn close_model(&mut self) {
        if let Some(model) = self.model.take() {
            self.output
                .push(format!("{} geschlossen", model.file_name()));
        }
        self.tree.selected = None;
        self.dialog = None;
        self.viewport.set_parts(&[]);
        self.update_contour();
    }

    /// Plays the animation and shows its window; marks the scene for rebuilding.
    fn animate(&mut self, ctx: &egui::Context) {
        let Some(view) = self.model.as_mut().and_then(|m| m.results.as_mut()) else {
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
        let Some(model) = &mut self.model else { return };
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
        let Some(model) = &self.model else {
            self.viewport.options.contour_levels = None;
            self.viewport.overlay = Overlay::default();
            return;
        };
        let view = model.results.as_ref();
        self.viewport.options.contour_levels =
            view.filter(|v| v.current().is_some()).map(|v| v.levels);
        self.viewport.overlay = Overlay {
            legend: view.and_then(ResultsView::legend),
            status: view.map_or_else(Vec::new, |v| v.status_lines(&model.file_name())),
            maximum: view
                .and_then(ResultsView::maximum)
                .and_then(|(index, value)| {
                    let value = value * view.map_or(1.0, ResultsView::amplitude);
                    Some(Marker {
                        position: model.node_position(index)?,
                        text: format!(
                            "Max: {}\nNode id: {}",
                            format_legend_value(value),
                            model.mesh.node_ids()[index]
                        ),
                    })
                }),
            global_origin: Some(model.global_origin()),
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
            egui::DragValue::new(&mut factor).speed(0.1).max_decimals(4),
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
            .add(egui::DragValue::new(&mut view.levels).range(2..=plx_render::contour::MAX_LEVELS))
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
                        let frames = egui::DragValue::new(&mut animation.frames).range(2..=200);
                        if ui.add(frames).changed() {
                            animation.go_to(animation.frame);
                            event = WindowEvent::Settings;
                        }
                        ui.end_row();
                    }
                    ui.label("Bilder pro Sekunde");
                    ui.add(egui::DragValue::new(&mut animation.fps).range(1.0..=60.0));
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
            Tab::Tree(view) => self.model_tree(ui, *view),
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

    fn clear_background(&self, _tab: &Tab) -> bool {
        true
    }
}
