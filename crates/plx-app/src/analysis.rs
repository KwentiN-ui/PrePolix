//! Running the analysis: writes the input file, starts CalculiX and shows PrePoMax's monitor
//! window with the solver output, progress and the button to open the results.

use std::path::PathBuf;
use std::time::Instant;

use plx_job::{Job, JobStatus};
use plx_model::EquationSolver;

use crate::model::Model;
use crate::settings::Solver;
use crate::tree::ANALYSIS_NAME;

/// Lines of solver output kept for the monitor window.
const MAX_OUTPUT_LINES: usize = 5000;

pub struct Analysis {
    job: Job,
    output: Vec<String>,
    started: Instant,
    finished: Option<f32>,
    status: JobStatus,
    pub monitor: bool,
}

pub enum MonitorEvent {
    None,
    OpenResults(PathBuf),
}

/// Copies the results file of a submodel's global model next to the input file `<name>.inp`
/// in `dir`, as `<name>-global.frd`, and lets `fe` refer to the copy: CalculiX reads it from
/// the directory of the input file and cannot take paths with spaces. Does nothing unless a
/// submodel boundary condition needs the file.
pub fn stage_global_results(
    fe: &mut plx_model::FeModel,
    dir: &std::path::Path,
    name: &str,
) -> Result<(), String> {
    if !fe.uses_global_results() {
        return Ok(());
    }
    // Without a file the input file reports the missing global results.
    let Some(source) = fe
        .properties
        .submodel_input()
        .map(std::path::Path::to_path_buf)
    else {
        return Ok(());
    };
    let target = dir.join(format!("{name}-global.frd"));
    let same = |a: &std::path::Path, b: &std::path::Path| {
        a.canonicalize()
            .ok()
            .is_some_and(|a| b.canonicalize().ok() == Some(a))
    };
    if same(&source, &dir.join(format!("{name}.frd"))) {
        return Err(format!(
            "The global results {} are this analysis's own results file, which the run \
             overwrites. Copy the file elsewhere and pick the copy in Model > Model Properties.",
            source.display()
        ));
    }
    if !same(&source, &target) {
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::copy(&source, &target))
            .map_err(|e| format!("Global results {} not copied: {e}", source.display()))?;
    }
    fe.properties.global_results = Some(target);
    Ok(())
}

impl Analysis {
    /// Writes `Analysis-1.inp` into the work directory and starts CalculiX on it; steps left
    /// at the default solver use `default_solver`, see [`Solver::default_solver`]. With
    /// `check_model` CalculiX only reads and checks the model (PrePoMax's "Check Model").
    pub fn start(
        solver: &Solver,
        model: &Model,
        default_solver: EquationSolver,
        check_model: bool,
    ) -> Result<Self, String> {
        if model.fe.steps.is_empty() && !check_model {
            return Err(
                "Analyse nicht gestartet: Das Modell hat keinen Step (Modell > Step erstellen)."
                    .into(),
            );
        }
        if !check_model && !model.fe.steps.iter().any(|s| s.active) {
            return Err("Analyse nicht gestartet: Alle Steps sind deaktiviert.".into());
        }
        let heading = format!("prepolix: {}", model.file_name());
        let mut fe = model.fe.clone();
        fe.resolve_default_solver(default_solver);
        let work_dir = solver.work_dir();
        stage_global_results(&mut fe, &work_dir, ANALYSIS_NAME)?;
        let write = if check_model {
            plx_io::inp::write_check_inp
        } else {
            plx_io::inp::write_inp
        };
        let input = write(&model.mesh, &fe, &heading)
            .map_err(|e| format!("Eingabedatei nicht geschrieben: {e}"))?;
        let copied = copy_result_files(&fe, &work_dir);
        let job_solver = solver.job_solver();
        let hint = "Programm unter Werkzeuge > Einstellungen > CalculiX prüfen.";
        if let Some(message) = crate::settings::missing_executable(&job_solver.executable) {
            return Err(format!("CalculiX nicht gestartet: {message}. {hint}"));
        }
        let job = Job::start(&job_solver, &work_dir, ANALYSIS_NAME, &input).map_err(|e| {
            format!(
                "CalculiX ({}) nicht gestartet: {e}. {hint}",
                job_solver.executable.display()
            )
        })?;
        let mut output = vec![format!(
            "{} {} in {}",
            ANALYSIS_NAME,
            if check_model {
                "Modellprüfung gestartet"
            } else {
                "gestartet"
            },
            work_dir.display()
        )];
        output.extend(copied);
        Ok(Self {
            output,
            job,
            started: Instant::now(),
            finished: None,
            status: JobStatus::Running,
            monitor: true,
        })
    }

    /// Adds a line of prepolix's own to the monitor output.
    pub fn note(&mut self, line: String) {
        self.output.push(line);
    }

    /// The solver output collected so far.
    pub fn output(&self) -> &[String] {
        &self.output
    }

    pub fn is_running(&self) -> bool {
        self.finished.is_none()
    }

    /// Collects new output; returns the final status once, when the job has just ended.
    pub fn poll(&mut self) -> Option<JobStatus> {
        self.output.extend(self.job.new_output());
        if self.output.len() > MAX_OUTPUT_LINES {
            self.output.drain(..self.output.len() - MAX_OUTPUT_LINES);
        }
        if self.finished.is_some() {
            return None;
        }
        let status = self.job.poll();
        if status == JobStatus::Running {
            return None;
        }
        self.output.extend(self.job.new_output());
        self.finished = Some(self.started.elapsed().as_secs_f32());
        self.status = status;
        Some(status)
    }

    pub fn status(&self) -> JobStatus {
        self.status
    }

    pub fn kill(&mut self) {
        self.job.kill();
        self.poll();
    }

    pub fn status_text(status: JobStatus) -> &'static str {
        match status {
            JobStatus::Running => "läuft",
            JobStatus::Completed => "abgeschlossen",
            JobStatus::FailedWithResults => "mit Fehlern beendet, Ergebnisse vorhanden",
            JobStatus::Failed => "fehlgeschlagen",
            JobStatus::Killed => "abgebrochen",
        }
    }

    /// PrePoMax's monitor window.
    pub fn window(&mut self, ctx: &egui::Context) -> MonitorEvent {
        let mut event = MonitorEvent::None;
        let mut open = self.monitor;
        let status = self.job.poll();
        egui::Window::new(format!("Monitor: {ANALYSIS_NAME}"))
            .open(&mut open)
            .collapsible(false)
            .default_size([620.0, 380.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if status == JobStatus::Running {
                        ui.spinner();
                    }
                    let seconds = self
                        .finished
                        .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
                    ui.label(format!(
                        "Status: {} ({seconds:.1} s)",
                        Self::status_text(status)
                    ));
                    if let Some(progress) = self.job.progress() {
                        ui.separator();
                        ui.label(format!(
                            "Step {}, Inkrement {}, Iterationen {}, Zeit {}",
                            progress.step,
                            progress.increment,
                            progress.iterations,
                            progress.total_time
                        ));
                    }
                });
                ui.separator();
                let height = ui.available_height() - 36.0;
                egui::ScrollArea::both()
                    .max_height(height.max(80.0))
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.output {
                            ui.monospace(line);
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(status == JobStatus::Running, egui::Button::new("Abbrechen"))
                        .clicked()
                    {
                        self.kill();
                    }
                    let results = self.job.results().filter(|_| status != JobStatus::Running);
                    if ui
                        .add_enabled(results.is_some(), egui::Button::new("Ergebnisse"))
                        .clicked()
                        && let Some(path) = results
                    {
                        event = MonitorEvent::OpenResults(path);
                    }
                });
            });
        self.monitor = open;
        if status == JobStatus::Running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        event
    }

    pub fn results(&self) -> Option<PathBuf> {
        self.job.results()
    }
}

/// Copies the result files the defined fields read next to the input file, where CalculiX
/// looks for them; a file already there is left alone. Returns a line per file for the
/// monitor output.
pub fn copy_result_files(fe: &plx_model::FeModel, dir: &std::path::Path) -> Vec<String> {
    let mut notes = Vec::new();
    let files = fe.result_files();
    if !files.is_empty()
        && let Err(error) = std::fs::create_dir_all(dir)
    {
        return vec![format!("{} not created: {error}", dir.display())];
    }
    for file in files {
        let Some(name) = file.file_name() else {
            continue;
        };
        let target = dir.join(name);
        if target == file {
            continue;
        }
        match std::fs::copy(file, &target) {
            Ok(_) => notes.push(format!("{} copied to {}", file.display(), dir.display())),
            Err(error) => notes.push(format!("{} not copied: {error}", file.display())),
        }
    }
    notes
}
