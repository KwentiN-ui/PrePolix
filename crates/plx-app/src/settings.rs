//! User settings, grouped like PrePoMax's settings dialog and stored by eframe in the user's
//! data directory (Linux `~/.local/share/prepolix`, Windows `%APPDATA%\prepolix\data`).

use serde::{Deserialize, Serialize};

/// Key of the settings in eframe's storage.
pub const STORAGE_KEY: &str = "settings";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub graphics: Graphics,
    pub post: PostProcessing,
    pub solver: Solver,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphics {
    /// Axis triad at the global origin of the model.
    pub global_axes: bool,
    /// Axis triad in the bottom right corner showing the view direction.
    pub view_triad: bool,
    pub scale_bar: bool,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            global_axes: true,
            view_triad: true,
            scale_bar: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostProcessing {
    /// Label at the node with the largest value (PrePoMax: on).
    pub max_label: bool,
    /// Label at the node with the smallest value (PrePoMax: off).
    pub min_label: bool,
    /// Information block with file, step and deformation.
    pub status_block: bool,
    /// Outline of the undeformed shape behind deformed results, for newly opened results.
    pub undeformed_outline: bool,
    /// Colour bands of the legend for newly opened results.
    pub levels: u32,
}

impl Default for PostProcessing {
    fn default() -> Self {
        Self {
            max_label: true,
            min_label: false,
            status_block: true,
            undeformed_outline: true,
            levels: plx_render::contour::DEFAULT_LEVELS,
        }
    }
}

/// How CalculiX is run, PrePoMax's "CalculiX" settings page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Solver {
    /// The CalculiX executable; a bare name is looked up on `PATH`.
    pub executable: String,
    /// Threads for the solver (`OMP_NUM_THREADS`).
    pub threads: u32,
    /// Where analyses run; empty for the default, see [`default_work_dir`].
    pub work_dir: String,
}

impl Default for Solver {
    fn default() -> Self {
        Self {
            executable: "ccx".into(),
            threads: 1,
            work_dir: String::new(),
        }
    }
}

impl Solver {
    pub fn work_dir(&self) -> std::path::PathBuf {
        if self.work_dir.trim().is_empty() {
            default_work_dir()
        } else {
            std::path::PathBuf::from(self.work_dir.trim())
        }
    }

    pub fn job_solver(&self) -> plx_job::Solver {
        plx_job::Solver {
            executable: self.executable.trim().into(),
            threads: self.threads.max(1),
        }
    }
}

/// PrePoMax's default work directory: a `Temp` folder next to the program. Where that is
/// not writable, e.g. for a binary installed to `/usr/bin`, a `prepolix` folder in the
/// system's temporary directory.
pub fn default_work_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("Temp")))
        .filter(|dir| is_writable(dir))
        .unwrap_or_else(|| std::env::temp_dir().join("prepolix"))
}

/// Creates the directory if needed and checks that files can be written into it.
fn is_writable(dir: &std::path::Path) -> bool {
    let probe = dir.join(".prepolix-schreibtest");
    let writable = std::fs::create_dir_all(dir).is_ok() && std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(probe);
    writable
}

/// Pages of the settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Graphics,
    PostProcessing,
    Solver,
}

impl Page {
    const ALL: [Page; 3] = [Page::Graphics, Page::PostProcessing, Page::Solver];

    fn title(self) -> &'static str {
        match self {
            Page::Graphics => "Grafik",
            Page::PostProcessing => "Postprocessing",
            Page::Solver => "CalculiX",
        }
    }
}

/// The settings window while it is open: a draft that OK or Apply copies into the settings.
pub struct SettingsWindow {
    page: Page,
    draft: Settings,
    default_work_dir: String,
}

/// What the user decided in the settings window.
pub enum WindowResult {
    Open,
    /// Take over the draft and keep the window open.
    Apply(Settings),
    /// Take over the draft and close.
    Ok(Settings),
    Cancel,
}

impl SettingsWindow {
    pub fn new(settings: &Settings) -> Self {
        Self {
            page: Page::PostProcessing,
            draft: settings.clone(),
            default_work_dir: default_work_dir().display().to_string(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> WindowResult {
        let mut open = true;
        let mut result = WindowResult::Open;
        egui::Window::new("Einstellungen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    egui::Frame::new()
                        .fill(crate::style::WINDOW)
                        .stroke(egui::Stroke::new(1.0, crate::style::BORDER))
                        .inner_margin(4)
                        .show(ui, |ui| {
                            ui.set_min_size(egui::vec2(130.0, 220.0));
                            ui.vertical(|ui| {
                                for page in Page::ALL {
                                    if ui
                                        .selectable_label(self.page == page, page.title())
                                        .clicked()
                                    {
                                        self.page = page;
                                    }
                                }
                            });
                        });
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.set_min_width(300.0);
                        ui.heading(self.page.title());
                        ui.add_space(4.0);
                        self.page_ui(ui);
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Standardwerte").clicked() {
                        self.draft = Settings::default();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Übernehmen").clicked() {
                            result = WindowResult::Apply(self.draft.clone());
                        }
                        if ui.button("Abbrechen").clicked() {
                            result = WindowResult::Cancel;
                        }
                        if ui.button("OK").clicked() {
                            result = WindowResult::Ok(self.draft.clone());
                        }
                    });
                });
            });
        if !open {
            result = WindowResult::Cancel;
        }
        result
    }

    fn page_ui(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::Graphics => {
                let g = &mut self.draft.graphics;
                ui.checkbox(&mut g.global_axes, "Achsenkreuz am globalen Ursprung");
                ui.checkbox(&mut g.view_triad, "Achsenkreuz in der Ecke (Blickrichtung)");
                ui.checkbox(&mut g.scale_bar, "Maßstab");
            }
            Page::PostProcessing => {
                let p = &mut self.draft.post;
                ui.checkbox(&mut p.max_label, "Label am Maximum anzeigen");
                ui.checkbox(&mut p.min_label, "Label am Minimum anzeigen");
                ui.checkbox(&mut p.status_block, "Infoblock anzeigen");
                ui.checkbox(
                    &mut p.undeformed_outline,
                    "Unverformte Kontur zeigen (neu geöffnete Ergebnisse)",
                );
                ui.horizontal(|ui| {
                    ui.label("Farbstufen (neu geöffnete Ergebnisse)");
                    ui.add(
                        egui::DragValue::new(&mut p.levels)
                            .range(2..=plx_render::contour::MAX_LEVELS),
                    );
                });
            }
            Page::Solver => {
                let solver = &mut self.draft.solver;
                egui::Grid::new("solver settings")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Programm");
                        ui.text_edit_singleline(&mut solver.executable);
                        ui.end_row();
                        ui.label("Threads");
                        ui.add(egui::DragValue::new(&mut solver.threads).range(1..=256));
                        ui.end_row();
                        ui.label("Arbeitsverzeichnis");
                        ui.add(
                            egui::TextEdit::singleline(&mut solver.work_dir)
                                .hint_text(self.default_work_dir.as_str()),
                        );
                        ui.end_row();
                    });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_their_defaults() {
        // Settings files of older versions lack newer fields.
        let settings: Settings = ron::from_str("(post: (min_label: true))").unwrap();
        assert!(settings.post.min_label);
        assert!(settings.post.max_label);
        assert!(settings.graphics.global_axes);
    }

    #[test]
    fn default_work_dir_is_next_to_the_binary_if_writable() {
        let dir = default_work_dir();
        let next_to_binary = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("Temp");
        assert_eq!(dir, next_to_binary, "the test binary's folder is writable");
    }
}
