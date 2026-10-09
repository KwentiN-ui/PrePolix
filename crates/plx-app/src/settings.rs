//! User settings, grouped like PrePoMax's settings dialog and stored by eframe in the user's
//! data directory (Linux `~/.local/share/prepolix`, Windows `%APPDATA%\prepolix\data`).

use crate::numeric;
use plx_model::EquationSolver;
use serde::{Deserialize, Serialize};

/// Key of the settings in eframe's storage.
pub const STORAGE_KEY: &str = "settings";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub graphics: Graphics,
    pub post: PostProcessing,
    pub solver: Solver,
    pub gmsh: Gmsh,
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
    /// Optional solvers found in the executable; probed again when it changes.
    pub detected: Option<DetectedSolvers>,
}

/// Result of [`crate::solver_check::available_solvers`] for one executable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectedSolvers {
    pub executable: String,
    pub solvers: Vec<EquationSolver>,
}

impl Default for Solver {
    fn default() -> Self {
        Self {
            executable: "ccx".into(),
            threads: 1,
            work_dir: String::new(),
            detected: None,
        }
    }
}

impl Solver {
    pub fn work_dir(&self) -> std::path::PathBuf {
        match clean_path(&self.work_dir) {
            "" => default_work_dir(),
            dir => std::path::PathBuf::from(dir),
        }
    }

    /// The solver for steps left at [`EquationSolver::Default`]: Pardiso if the executable
    /// has it, otherwise CalculiX's own choice. The executable is probed the first time.
    pub fn default_solver(&mut self) -> EquationSolver {
        if self.solvers().is_none()
            && let Some(solvers) =
                crate::solver_check::available_solvers(&self.job_solver(), &self.work_dir())
        {
            self.detected = Some(DetectedSolvers {
                executable: self.executable.clone(),
                solvers,
            });
        }
        match self.solvers() {
            Some(solvers) if solvers.contains(&EquationSolver::Pardiso) => EquationSolver::Pardiso,
            _ => EquationSolver::Default,
        }
    }

    /// The detected solvers, if they belong to the current executable.
    pub fn solvers(&self) -> Option<&[EquationSolver]> {
        self.detected
            .as_ref()
            .filter(|d| d.executable == self.executable)
            .map(|d| d.solvers.as_slice())
    }

    pub fn job_solver(&self) -> plx_job::Solver {
        plx_job::Solver {
            executable: clean_path(&self.executable).into(),
            threads: self.threads.max(1),
        }
    }
}

/// Where the Gmsh library comes from, for geometry import and meshing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gmsh {
    /// The library file; empty to search next to the program and on the system.
    pub library: String,
}

impl Gmsh {
    pub fn library(&self) -> Option<std::path::PathBuf> {
        Some(clean_path(&self.library))
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
    }
}

fn solvers_text(solvers: Option<&[EquationSolver]>) -> String {
    match solvers {
        None => "noch nicht geprüft".into(),
        Some([]) => "kein direkter Löser gefunden".into(),
        Some(solvers) => {
            let names: Vec<_> = solvers.iter().filter_map(|s| s.keyword()).collect();
            names.join(", ")
        }
    }
}

/// A path as typed or pasted: without surrounding spaces and the quotes that Windows'
/// "Als Pfad kopieren" adds.
pub fn clean_path(text: &str) -> &str {
    let text = text.trim();
    ['"', '\'']
        .iter()
        .find_map(|&quote| text.strip_prefix(quote)?.strip_suffix(quote))
        .map_or(text, str::trim)
}

/// Why the executable cannot be started, if it is given as a path to a missing file;
/// a bare name is left to the `PATH` lookup.
pub fn missing_executable(executable: &std::path::Path) -> Option<String> {
    let is_path = executable.components().count() > 1 || executable.is_absolute();
    (is_path && !executable.is_file())
        .then(|| format!("Datei nicht gefunden: {}", executable.display()))
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
    Gmsh,
}

impl Page {
    const ALL: [Page; 4] = [
        Page::Graphics,
        Page::PostProcessing,
        Page::Solver,
        Page::Gmsh,
    ];

    fn title(self) -> &'static str {
        match self {
            Page::Graphics => "Grafik",
            Page::PostProcessing => "Postprocessing",
            Page::Solver => "CalculiX",
            Page::Gmsh => "Gmsh",
        }
    }
}

/// The settings window while it is open: a draft that OK or Apply copies into the settings.
pub struct SettingsWindow {
    page: Page,
    draft: Settings,
    default_work_dir: String,
    solver_check: SolverCheck,
    gmsh_check: GmshCheck,
}

/// State of the Gmsh test on the settings page.
#[derive(Default)]
enum GmshCheck {
    #[default]
    NotRun,
    Running(std::sync::mpsc::Receiver<Result<String, String>>),
    Done(Result<String, String>),
}

/// State of the CalculiX self test on the settings page.
#[derive(Default)]
enum SolverCheck {
    #[default]
    NotRun,
    Running(std::sync::mpsc::Receiver<crate::solver_check::Report>),
    Done(Vec<crate::solver_check::CheckResult>),
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
            solver_check: SolverCheck::NotRun,
            gmsh_check: GmshCheck::NotRun,
        }
    }

    /// The window opened at `page`.
    pub fn with_page(settings: &Settings, page: Page) -> Self {
        Self {
            page,
            ..Self::new(settings)
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
                        numeric::drag_value(&mut p.levels)
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
                        ui.add(numeric::drag_value(&mut solver.threads).range(1..=256));
                        ui.end_row();
                        ui.label("Arbeitsverzeichnis");
                        ui.add(
                            egui::TextEdit::singleline(&mut solver.work_dir)
                                .hint_text(self.default_work_dir.as_str()),
                        );
                        ui.end_row();
                        ui.label("Gleichungslöser");
                        ui.label(solvers_text(solver.solvers()))
                            .on_hover_text("Steps mit Standard-Löser rechnen mit Pardiso, wenn CalculiX es enthält. Geprüft beim ersten Rechnen und mit \"CalculiX testen\".");
                        ui.end_row();
                    });
                ui.add_space(8.0);
                self.solver_check_ui(ui);
            }
            Page::Gmsh => self.gmsh_ui(ui),
        }
    }

    /// Gmsh imports STEP, IGES and BREP and meshes; prepolix loads its library at run time.
    fn gmsh_ui(&mut self, ui: &mut egui::Ui) {
        let loaded = plx_mesher::loaded_library();
        egui::Grid::new("gmsh settings")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Bibliothek");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.gmsh.library)
                        .hint_text("automatisch suchen")
                        .desired_width(260.0),
                );
                ui.end_row();
                ui.label("Geladen");
                match &loaded {
                    Some(info) => {
                        ui.label(format!("Gmsh {}\n{}", info.version, info.path.display()))
                    }
                    None => ui.weak("noch nicht"),
                };
                ui.end_row();
            });
        ui.add(
            egui::Label::new(
                "Ohne Angabe wird die Bibliothek neben dem Programm gesucht (libgmsh.so bzw. \
                 gmsh-4.15.dll), dann im Suchpfad des Systems. Eine andere Bibliothek wird erst \
                 nach einem Neustart geladen.",
            )
            .wrap(),
        );
        ui.add_space(8.0);
        if let GmshCheck::Running(receiver) = &self.gmsh_check
            && let Ok(result) = receiver.try_recv()
        {
            self.gmsh_check = GmshCheck::Done(result);
        }
        let running = matches!(self.gmsh_check, GmshCheck::Running(_));
        ui.horizontal(|ui| {
            let button = ui
                .add_enabled(!running, egui::Button::new("Gmsh testen"))
                .on_hover_text("Lädt Gmsh und vernetzt einen Würfel.");
            if button.clicked() {
                if loaded.is_none() {
                    plx_mesher::set_library_path(self.draft.gmsh.library());
                }
                let (sender, receiver) = std::sync::mpsc::channel();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = plx_mesher::self_test()
                        .map(|(info, elements)| {
                            let mut text = format!(
                                "Gmsh {} arbeitet korrekt, Würfel mit {elements} Elementen vernetzt.",
                                info.version
                            );
                            if info.untested {
                                let (major, minor) = plx_mesher::gmsh::TESTED_VERSION;
                                text += &format!(
                                    " Getestet ist prepolix mit Gmsh {major}.{minor}."
                                );
                            }
                            text
                        })
                        .map_err(|e| e.to_string());
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
                self.gmsh_check = GmshCheck::Running(receiver);
            }
            if running {
                ui.spinner();
                ui.label("Test läuft …");
            }
        });
        match &self.gmsh_check {
            GmshCheck::Done(Ok(text)) => {
                ui.colored_label(egui::Color32::from_rgb(0, 128, 0), text);
            }
            GmshCheck::Done(Err(text)) => {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(text).color(egui::Color32::from_rgb(200, 0, 0)),
                    )
                    .wrap(),
                );
            }
            _ => {}
        }
    }

    /// Button and results of the CalculiX self test, run with the settings in the window.
    fn solver_check_ui(&mut self, ui: &mut egui::Ui) {
        if let SolverCheck::Running(receiver) = &self.solver_check
            && let Ok(report) = receiver.try_recv()
        {
            if let Some(solvers) = report.solvers {
                self.draft.solver.detected = Some(DetectedSolvers {
                    executable: self.draft.solver.executable.clone(),
                    solvers,
                });
            }
            self.solver_check = SolverCheck::Done(report.checks);
        }
        let running = matches!(self.solver_check, SolverCheck::Running(_));
        ui.horizontal(|ui| {
            let button = ui
                .add_enabled(!running, egui::Button::new("CalculiX testen"))
                .on_hover_text(
                    "Startet das Programm und rechnet kleine Kragbalken, deren Ergebnisse mit der Balkentheorie verglichen werden, und prüft, welche Gleichungslöser vorhanden sind.",
                );
            if button.clicked() {
                let (sender, receiver) = std::sync::mpsc::channel();
                let (solver, work_dir) = (self.draft.solver.job_solver(), self.draft.solver.work_dir());
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let _ = sender.send(crate::solver_check::run(&solver, &work_dir));
                    ctx.request_repaint();
                });
                self.solver_check = SolverCheck::Running(receiver);
            }
            if running {
                ui.spinner();
                ui.label("Test läuft …");
            }
        });
        if let SolverCheck::Done(results) = &self.solver_check {
            egui::Grid::new("solver check")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for result in results {
                        if result.passed {
                            ui.colored_label(egui::Color32::from_rgb(0, 128, 0), "OK");
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(200, 0, 0), "Fehler");
                        }
                        ui.vertical(|ui| {
                            ui.strong(result.name);
                            ui.add(egui::Label::new(&result.message).wrap());
                        });
                        ui.end_row();
                    }
                });
            if results.iter().all(|r| r.passed) {
                ui.label("CalculiX arbeitet korrekt.");
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
    fn paths_lose_quotes_and_spaces() {
        assert_eq!(
            clean_path(r#" "C:\Program Files\ccx.exe" "#),
            r"C:\Program Files\ccx.exe"
        );
        assert_eq!(clean_path("'/opt/ccx' "), "/opt/ccx");
        assert_eq!(clean_path("ccx"), "ccx");
        assert_eq!(clean_path("\"ccx"), "\"ccx");
        let solver = Solver {
            executable: "\"/opt/ccx\"".into(),
            work_dir: " \"/tmp/plx\" ".into(),
            ..Solver::default()
        };
        assert_eq!(
            solver.job_solver().executable,
            std::path::Path::new("/opt/ccx")
        );
        assert_eq!(solver.work_dir(), std::path::Path::new("/tmp/plx"));
    }

    #[test]
    fn missing_files_are_reported_but_names_are_looked_up() {
        assert!(missing_executable(std::path::Path::new("ccx")).is_none());
        let missing = std::env::temp_dir().join("plx-gibt-es-nicht").join("ccx");
        let message = missing_executable(&missing).unwrap();
        assert!(message.starts_with("Datei nicht gefunden"), "{message}");
        let exe = std::env::current_exe().unwrap();
        assert!(missing_executable(&exe).is_none());
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
