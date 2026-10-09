//! Running the analysis: writes the input file, starts CalculiX and shows PrePoMax's monitor
//! window with the solver output, progress and the button to open the results.

use std::path::PathBuf;
use std::time::Instant;

use plx_job::{Job, JobStatus};

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

impl Analysis {
    /// Writes `Analysis-1.inp` into the work directory and starts CalculiX on it.
    pub fn start(solver: &Solver, model: &Model) -> Result<Self, String> {
        if model.fe.steps.is_empty() {
            return Err(
                "Analyse nicht gestartet: Das Modell hat keinen Step (Modell > Step erstellen)."
                    .into(),
            );
        }
        let heading = format!("prepolix: {}", model.file_name());
        let input = plx_io::inp::write_inp(&model.mesh, &model.fe, &heading)
            .map_err(|e| format!("Eingabedatei nicht geschrieben: {e}"))?;
        let work_dir = solver.work_dir();
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
        Ok(Self {
            output: vec![format!(
                "{} gestartet in {}",
                ANALYSIS_NAME,
                work_dir.display()
            )],
            job,
            started: Instant::now(),
            finished: None,
            status: JobStatus::Running,
            monitor: true,
        })
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
