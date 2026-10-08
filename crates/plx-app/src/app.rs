use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};

use crate::viewport::Viewport;

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

struct Workbench {
    viewport: Viewport,
    output: Vec<String>,
}

pub struct PrepolixApp {
    dock: DockState<Tab>,
    workbench: Workbench,
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
        Ok(Self {
            dock: default_layout(),
            workbench: Workbench {
                viewport: Viewport::new(render_state),
                output,
            },
        })
    }
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
        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Datei", |ui| {
                    if ui.button("Beenden").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                for menu in [
                    "Bearbeiten",
                    "Ansicht",
                    "Geometrie",
                    "Netz",
                    "Modell",
                    "Analyse",
                    "Ergebnisse",
                    "Werkzeuge",
                    "Hilfe",
                ] {
                    ui.menu_button(menu, |ui| {
                        ui.label("Noch nicht implementiert");
                    });
                }
            });
        });
        DockArea::new(&mut self.dock)
            .show_close_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(ui, &mut self.workbench);
    }
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
            Tab::ModelTree => {
                egui::CollapsingHeader::new("Modell")
                    .default_open(true)
                    .show(ui, |ui| {
                        for item in [
                            "Geometrie",
                            "Netz",
                            "Materialien",
                            "Sections",
                            "Constraints",
                            "Steps",
                        ] {
                            ui.label(item);
                        }
                    });
                egui::CollapsingHeader::new("Ergebnisse").show(ui, |ui| {
                    ui.label("Keine Ergebnisse geladen");
                });
            }
            Tab::Properties => {
                ui.weak("Kein Objekt ausgewählt");
            }
            Tab::Viewport => self.viewport.ui(ui),
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
