//! Campbell diagram of a rotor: the complex eigenfrequencies over the rotational speed.
//!
//! PrePoMax has no such tool; CalculiX computes the whirling modes for one speed per run.
//! The sweep runs the analysis once per speed, scaling the centrifugal loads of the model,
//! reads the complex frequencies and the turning direction of every mode from the `.dat`
//! file and plots them over the speed, with the engine order lines (1x, 2x, ...) whose
//! crossings with the modes are the critical speeds.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use egui::Ui;
use plx_job::{Job, JobStatus};
use plx_model::{EquationSolver, FeModel, LoadKind, StepKind};

use crate::model::Model;
use crate::numeric;
use crate::xy_plot::{self, XyData};

/// Folder under the work directory the sweep runs in.
const FOLDER: &str = "Campbell";

/// Default number of speeds between standstill and the full speed.
const DEFAULT_STEPS: u32 = 10;

/// Whirl of a mode as CalculiX reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whirl {
    Forward,
    Backward,
    Unknown,
}

impl Whirl {
    fn letter(self) -> &'static str {
        match self {
            Whirl::Forward => "F",
            Whirl::Backward => "B",
            Whirl::Unknown => "",
        }
    }
}

/// One complex mode at one speed.
#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    /// Real part of the frequency in cycles per time.
    pub frequency: f64,
    pub whirl: Whirl,
}

/// The modes CalculiX found at one speed.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedPoint {
    /// Rotational speed in radians per time.
    pub speed: f64,
    pub modes: Vec<Mode>,
}

/// A crossing of a mode with an engine order line.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticalSpeed {
    pub order: f64,
    /// Mode number, from 1.
    pub mode: usize,
    /// Rotational speed in radians per time.
    pub speed: f64,
    pub whirl: Whirl,
}

/// Settings of the sweep.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    /// Highest speed of the sweep in radians per time.
    pub max_speed: f64,
    /// Number of speed steps from standstill to the highest speed.
    pub steps: u32,
    /// Engine orders drawn as lines, e.g. "1, 2".
    pub orders: String,
}

impl Setup {
    /// The engine orders as numbers, in order and without repeats.
    pub fn order_values(&self) -> Vec<f64> {
        let mut orders: Vec<f64> = self
            .orders
            .split([',', ';', ' '])
            .filter_map(|t| numeric::parse_number(t.trim()))
            .filter(|o| *o > 0.0 && o.is_finite())
            .collect();
        orders.sort_by(f64::total_cmp);
        orders.dedup();
        orders
    }
}

/// The rotational speed of the model: the fastest active centrifugal load of an active step,
/// `None` when nothing rotates.
pub fn rotor_speed(fe: &FeModel) -> Option<f64> {
    (fe.steps
        .iter()
        .filter(|s| s.active && s.kind.supports_loads()))
    .flat_map(|s| &s.loads)
    .filter(|l| l.active)
    .filter_map(|l| match l.kind {
        LoadKind::Centrifugal { speed, .. } => Some(speed.abs()),
        _ => None,
    })
    .reduce(f64::max)
}

/// Number (from 1, among the active steps as CalculiX counts them) of the last active complex
/// frequency step.
pub fn complex_step(fe: &FeModel) -> Option<u32> {
    (fe.steps.iter().filter(|s| s.active).enumerate())
        .filter(|(_, s)| matches!(s.kind, StepKind::ComplexFrequency(_)))
        .map(|(i, _)| i as u32 + 1)
        .last()
}

/// Whether the model can have a Campbell diagram: it rotates and has a complex frequency step.
pub fn applies(fe: &FeModel) -> bool {
    rotor_speed(fe).is_some() && complex_step(fe).is_some()
}

/// The model with every centrifugal load scaled from the model's speed to `speed`.
pub fn with_speed(fe: &FeModel, speed: f64) -> FeModel {
    let mut fe = fe.clone();
    let factor = match rotor_speed(&fe) {
        Some(reference) if reference > 0.0 => speed / reference,
        _ => return fe,
    };
    for load in fe.steps.iter_mut().flat_map(|s| &mut s.loads) {
        if let LoadKind::Centrifugal { speed, .. } = &mut load.kind {
            *speed *= factor;
        }
    }
    fe
}

/// The complex frequencies and whirls of step `step` in the text of a `.dat` file.
pub fn modes_of(dat: &str, step: u32) -> Vec<Mode> {
    let import = plx_io::dat::parse_dat(dat);
    let name = format!("STEP_{step}");
    let Some(set) = import.sets.iter().find(|s| s.name == name) else {
        return Vec::new();
    };
    let Some(output) = set.field("EIGENVALUE_OUTPUT") else {
        return Vec::new();
    };
    let values = |component: &str| -> Vec<f64> {
        output
            .component(component)
            .and_then(|c| c.entries.first())
            .map(|e| e.values.clone())
            .unwrap_or_default()
    };
    let frequencies = values("FREQUENCY");
    let whirls = values("TURNING_DIRECTION");
    (frequencies.iter().enumerate())
        .map(|(i, &frequency)| Mode {
            frequency,
            whirl: match whirls.get(i) {
                Some(w) if *w > 0.0 => Whirl::Forward,
                Some(w) if *w < 0.0 => Whirl::Backward,
                _ => Whirl::Unknown,
            },
        })
        .collect()
}

/// The speeds of a sweep: `steps` equal steps up to `max_speed`. Standstill is left out:
/// without rotation CalculiX merges the coinciding modes of a pair into one, which would
/// shift every curve of the diagram at its start.
pub fn speeds(max_speed: f64, steps: u32) -> Vec<f64> {
    let steps = steps.max(1);
    (1..=steps)
        .map(|i| max_speed * f64::from(i) / f64::from(steps))
        .collect()
}

/// Radians per time to revolutions per minute, for a time unit of seconds.
pub fn rpm(speed: f64) -> f64 {
    speed * 60.0 / std::f64::consts::TAU
}

/// Revolutions per minute to radians per time.
pub fn from_rpm(rpm: f64) -> f64 {
    rpm * std::f64::consts::TAU / 60.0
}

/// The diagram: one curve per mode over the speed in rpm, then the engine order lines.
/// Modes are matched between speeds by their order of frequency; curves of modes that cross
/// swap at the crossing.
pub fn diagram(points: &[SpeedPoint], orders: &[f64]) -> XyData {
    let x: Vec<f64> = points.iter().map(|p| rpm(p.speed)).collect();
    let count = points.iter().map(|p| p.modes.len()).max().unwrap_or(0);
    let mut curves = Vec::new();
    for mode in 0..count {
        let values: Vec<f64> = points
            .iter()
            .map(|p| p.modes.get(mode).map_or(f64::NAN, |m| m.frequency))
            .collect();
        let whirls: Vec<Whirl> = points
            .iter()
            .filter_map(|p| p.modes.get(mode).map(|m| m.whirl))
            .collect();
        let name = match whirls.first() {
            Some(&first) if first != Whirl::Unknown && whirls.iter().all(|w| *w == first) => {
                format!("Mode {} ({})", mode + 1, first.letter())
            }
            _ => format!("Mode {}", mode + 1),
        };
        curves.push((name, values));
    }
    for &order in orders {
        let values = points
            .iter()
            .map(|p| order * p.speed / std::f64::consts::TAU)
            .collect();
        curves.push((format!("{}x", numeric::format_physical(order)), values));
    }
    XyData {
        title: "Campbell diagram".into(),
        x_label: "Speed [rpm]".into(),
        x,
        curves,
    }
}

/// The crossings of the modes with the engine order lines, by linear interpolation between
/// the speeds of the sweep.
pub fn critical_speeds(points: &[SpeedPoint], orders: &[f64]) -> Vec<CriticalSpeed> {
    let mut found = Vec::new();
    let count = points.iter().map(|p| p.modes.len()).max().unwrap_or(0);
    for &order in orders {
        for mode in 0..count {
            for pair in points.windows(2) {
                let (Some(a), Some(b)) = (pair[0].modes.get(mode), pair[1].modes.get(mode)) else {
                    continue;
                };
                let line = |speed: f64| order * speed / std::f64::consts::TAU;
                let (da, db) = (
                    a.frequency - line(pair[0].speed),
                    b.frequency - line(pair[1].speed),
                );
                // A crossing has the mode above the line at one end and below at the other;
                // standstill itself (speed 0, line 0) is no critical speed.
                if da == db || da * db > 0.0 || (pair[0].speed == 0.0 && da == 0.0) {
                    continue;
                }
                let t = da / (da - db);
                let speed = pair[0].speed + t * (pair[1].speed - pair[0].speed);
                if speed <= 0.0 {
                    continue;
                }
                found.push(CriticalSpeed {
                    order,
                    mode: mode + 1,
                    speed,
                    whirl: if t < 0.5 { a.whirl } else { b.whirl },
                });
            }
        }
    }
    found.sort_by(|a, b| a.speed.total_cmp(&b.speed));
    found
}

/// The sweep as text with tabs, for a spreadsheet.
pub fn table_text(points: &[SpeedPoint]) -> String {
    let count = points.iter().map(|p| p.modes.len()).max().unwrap_or(0);
    let mut text = String::from("Speed [rad/s]\tSpeed [rpm]");
    for mode in 1..=count {
        text.push_str(&format!("\tMode {mode} [Hz]\tWhirl {mode}"));
    }
    text.push('\n');
    for point in points {
        text.push_str(&format!("{}\t{}", point.speed, rpm(point.speed)));
        for mode in 0..count {
            match point.modes.get(mode) {
                Some(m) => text.push_str(&format!("\t{}\t{}", m.frequency, m.whirl.letter())),
                None => text.push_str("\t\t"),
            }
        }
        text.push('\n');
    }
    text
}

enum Message {
    /// A speed is done, with its modes or the reason it failed.
    Done(Result<SpeedPoint, String>),
    Finished,
}

/// The running sweep: CalculiX runs in a thread, one speed after the other.
struct Sweep {
    receiver: Receiver<Message>,
    cancel: Arc<AtomicBool>,
    total: usize,
    points: Vec<SpeedPoint>,
    log: Vec<String>,
    finished: bool,
}

/// What the sweep thread works on.
struct SweepInput {
    solver: plx_job::Solver,
    work_dir: std::path::PathBuf,
    mesh: Arc<plx_mesh::FeMesh>,
    /// The model at its own speed, with the default solver resolved.
    fe: FeModel,
    /// Number of the complex frequency step in the `.dat` file.
    step: u32,
    speeds: Vec<f64>,
}

/// Runs CalculiX for every speed and sends the modes back; stops when `cancel` is set.
fn run_sweep(input: SweepInput, sender: Sender<Message>, cancel: Arc<AtomicBool>) {
    for (i, &speed) in input.speeds.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let result = run_speed(&input, speed, i, &cancel);
        if sender.send(Message::Done(result)).is_err() {
            return;
        }
    }
    let _ = sender.send(Message::Finished);
}

/// One run of CalculiX at `speed`, as job `Campbell-<index>` in the work directory.
fn run_speed(
    input: &SweepInput,
    speed: f64,
    index: usize,
    cancel: &AtomicBool,
) -> Result<SpeedPoint, String> {
    let label = format!("{} rpm", numeric::format_physical(rpm(speed)));
    let fe = with_speed(&input.fe, speed);
    let heading = format!("prepolix Campbell sweep: {label}");
    let text = plx_io::inp::write_inp(&input.mesh, &fe, &heading)
        .map_err(|e| format!("{label}: input file not written: {e}"))?;
    let name = format!("Campbell-{index}");
    let mut job = Job::start(&input.solver, &input.work_dir, &name, &text)
        .map_err(|e| format!("{label}: CalculiX not started: {e}"))?;
    let status = loop {
        let status = job.poll();
        if status != JobStatus::Running {
            break status;
        }
        if cancel.load(Ordering::Relaxed) {
            job.kill();
            return Err(format!("{label}: cancelled"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let dat = std::fs::read_to_string(job.file("dat")).unwrap_or_default();
    let modes = modes_of(&dat, input.step);
    if modes.is_empty() {
        let output = job.new_output();
        let error = output
            .iter()
            .find(|l| l.contains("*ERROR"))
            .cloned()
            .unwrap_or_else(|| {
                format!("CalculiX {status:?}, no complex frequencies in {name}.dat")
            });
        return Err(format!("{label}: {error}"));
    }
    Ok(SpeedPoint { speed, modes })
}

/// The Campbell diagram window: the setup of the sweep, its progress and the diagram.
pub struct CampbellWindow {
    pub setup: Setup,
    sweep: Option<Sweep>,
    points: Vec<SpeedPoint>,
    log: Vec<String>,
    /// Orders the shown diagram was made with.
    orders: Vec<f64>,
}

impl CampbellWindow {
    /// A window for the model: the sweep runs up to the speed of its centrifugal load.
    pub fn new(fe: &FeModel) -> Self {
        Self {
            setup: Setup {
                max_speed: rotor_speed(fe).unwrap_or(0.0),
                steps: DEFAULT_STEPS,
                orders: "1".into(),
            },
            sweep: None,
            points: Vec::new(),
            log: Vec::new(),
            orders: vec![1.0],
        }
    }

    pub fn is_running(&self) -> bool {
        self.sweep.as_ref().is_some_and(|s| !s.finished)
    }

    /// Starts the sweep on the model; steps at the default solver use `default_solver`.
    pub fn start(
        &mut self,
        solver: &crate::settings::Solver,
        model: &Model,
        default_solver: EquationSolver,
    ) -> Result<(), String> {
        let Some(step) = complex_step(&model.fe) else {
            return Err("The model has no active Complex Frequency step.".into());
        };
        if rotor_speed(&model.fe).is_none_or(|s| s <= 0.0) {
            return Err("The model has no active Centrifugal load.".into());
        }
        if self.setup.max_speed <= 0.0 {
            return Err("The highest speed must be greater than zero.".into());
        }
        let job_solver = solver.job_solver();
        if let Some(message) = crate::settings::missing_executable(&job_solver.executable) {
            return Err(format!("CalculiX not started: {message}"));
        }
        let mut fe = model.fe.clone();
        fe.resolve_default_solver(default_solver);
        let speeds = speeds(self.setup.max_speed, self.setup.steps);
        let (sender, receiver) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let work_dir = solver.work_dir().join(FOLDER);
        let mesh = Arc::new(model.mesh.clone());
        let total = speeds.len();
        let thread_cancel = Arc::clone(&cancel);
        let input = SweepInput {
            solver: job_solver,
            work_dir,
            mesh,
            fe,
            step,
            speeds,
        };
        std::thread::Builder::new()
            .name("campbell sweep".into())
            .spawn(move || run_sweep(input, sender, thread_cancel))
            .map_err(|e| format!("Sweep not started: {e}"))?;
        self.orders = self.setup.order_values();
        self.points.clear();
        self.log = vec![format!(
            "Sweep started: {total} speeds up to {} rpm in {}",
            numeric::format_physical(rpm(self.setup.max_speed)),
            solver.work_dir().join(FOLDER).display()
        )];
        self.sweep = Some(Sweep {
            receiver,
            cancel,
            total,
            points: Vec::new(),
            log: Vec::new(),
            finished: false,
        });
        Ok(())
    }

    pub fn cancel(&mut self) {
        if let Some(sweep) = &self.sweep {
            sweep.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Takes what the sweep thread has sent.
    fn poll(&mut self) {
        let Some(sweep) = &mut self.sweep else {
            return;
        };
        while let Ok(message) = sweep.receiver.try_recv() {
            match message {
                Message::Done(Ok(point)) => {
                    sweep.log.push(format!(
                        "{} rpm: {} modes, first {} Hz",
                        numeric::format_physical(rpm(point.speed)),
                        point.modes.len(),
                        point.modes.first().map_or_else(
                            || "-".to_string(),
                            |m| numeric::format_physical(m.frequency)
                        )
                    ));
                    sweep.points.push(point);
                }
                Message::Done(Err(error)) => sweep.log.push(error),
                Message::Finished => sweep.finished = true,
            }
        }
        self.points = sweep.points.clone();
        if !sweep.log.is_empty() {
            self.log.append(&mut sweep.log);
        }
        if sweep.finished {
            self.log.push("Sweep finished.".into());
            self.sweep = None;
        }
    }

    /// Shows the window; returns false when it was closed. `can_start` tells whether the
    /// model is ready; `start` is called with the setup when the user starts the sweep.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        can_start: bool,
        mut start: impl FnMut(&mut Self) -> Result<(), String>,
    ) -> bool {
        self.poll();
        let mut open = true;
        let running = self.is_running();
        egui::Window::new("Campbell diagram")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 560.0])
            .min_size([480.0, 360.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                self.setup_row(ui, running);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_start && !running, egui::Button::new("Start sweep"))
                        .clicked()
                        && let Err(error) = start(self)
                    {
                        self.log.push(error);
                    }
                    if ui
                        .add_enabled(running, egui::Button::new("Cancel"))
                        .clicked()
                    {
                        self.cancel();
                    }
                    if running {
                        ui.spinner();
                        let total = self.sweep.as_ref().map_or(0, |s| s.total);
                        ui.label(format!("{} of {total} speeds done", self.points.len()));
                    }
                    if ui
                        .add_enabled(!self.points.is_empty(), egui::Button::new("Copy table"))
                        .clicked()
                    {
                        ui.ctx().copy_text(table_text(&self.points));
                    }
                });
                ui.separator();
                if self.points.len() >= 2 {
                    self.diagram_and_table(ui);
                } else {
                    ui.weak(
                        "The sweep runs the analysis once per speed, with the centrifugal \
                         load scaled to it, and plots the complex frequencies of the last \
                         Complex Frequency step over the speed. The crossings with the engine \
                         order lines are the critical speeds. CalculiX computes the Coriolis \
                         modes in the rotating frame of reference; the frequencies and the \
                         whirl directions (F forward, B backward) are CalculiX's.",
                    );
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("campbell log")
                    .max_height(80.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.monospace(line);
                        }
                    });
            });
        if running {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        open
    }

    fn setup_row(&mut self, ui: &mut Ui, running: bool) {
        ui.add_enabled_ui(!running, |ui| {
            egui::Grid::new("campbell setup")
                .num_columns(4)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Highest speed [rpm]");
                    let mut max_rpm = rpm(self.setup.max_speed);
                    if ui
                        .add(
                            numeric::drag_value(&mut max_rpm)
                                .range(0.0..=f64::MAX)
                                .speed(10.0),
                        )
                        .changed()
                    {
                        self.setup.max_speed = from_rpm(max_rpm);
                    }
                    ui.label(format!(
                        "= {} rad/s",
                        numeric::format_physical(self.setup.max_speed)
                    ));
                    ui.end_row();
                    ui.label("Speed steps");
                    ui.add(numeric::drag_value(&mut self.setup.steps).range(1..=200));
                    ui.weak("CalculiX runs once per speed.");
                    ui.end_row();
                    ui.label("Engine orders");
                    ui.add(egui::TextEdit::singleline(&mut self.setup.orders).desired_width(120.0));
                    ui.weak("Lines f = order x speed, e.g. 1, 2, 3");
                    ui.end_row();
                });
        });
    }

    fn diagram_and_table(&mut self, ui: &mut Ui) {
        // Orders can change after the sweep; the diagram follows the text field.
        let orders = if self.is_running() {
            self.orders.clone()
        } else {
            self.setup.order_values()
        };
        let data = diagram(&self.points, &orders);
        let critical = critical_speeds(&self.points, &orders);
        let table_height = if critical.is_empty() {
            24.0
        } else {
            (critical.len().min(6) as f32 + 1.5) * 20.0
        };
        let plot_height = (ui.available_height() - table_height - 110.0).max(220.0);
        ui.allocate_ui(egui::vec2(ui.available_width(), plot_height), |ui| {
            xy_plot::show(ui, &data);
        });
        ui.add_space(4.0);
        if critical.is_empty() {
            ui.weak("No crossing of a mode with an engine order line in the sweep.");
            return;
        }
        ui.strong("Critical speeds");
        egui::ScrollArea::vertical()
            .id_salt("critical speeds")
            .max_height(table_height)
            .show(ui, |ui| {
                egui::Grid::new("critical speeds grid")
                    .num_columns(5)
                    .striped(true)
                    .spacing([16.0, 2.0])
                    .show(ui, |ui| {
                        for title in ["Order", "Mode", "Whirl", "Speed [rpm]", "Frequency [Hz]"] {
                            ui.strong(title);
                        }
                        ui.end_row();
                        for c in &critical {
                            ui.label(format!("{}x", numeric::format_physical(c.order)));
                            ui.label(c.mode.to_string());
                            ui.label(match c.whirl {
                                Whirl::Forward => "forward",
                                Whirl::Backward => "backward",
                                Whirl::Unknown => "-",
                            });
                            ui.label(numeric::format_physical(rpm(c.speed)));
                            ui.label(numeric::format_physical(
                                c.order * c.speed / std::f64::consts::TAU,
                            ));
                            ui.end_row();
                        }
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(speed: f64, frequencies: &[(f64, Whirl)]) -> SpeedPoint {
        SpeedPoint {
            speed,
            modes: frequencies
                .iter()
                .map(|&(frequency, whirl)| Mode { frequency, whirl })
                .collect(),
        }
    }

    #[test]
    fn critical_speeds_are_the_crossings_with_the_order_lines() {
        // A backward mode falling from 100 Hz and a forward mode rising; the 1x line
        // f = speed / 2 pi crosses the backward mode where 100 - 10 t = 60 t / 2pi.
        let points: Vec<SpeedPoint> = (0..=10)
            .map(|i| {
                let t = f64::from(i);
                point(
                    60.0 * t,
                    &[
                        (100.0 - 10.0 * t, Whirl::Backward),
                        (100.0 + 10.0 * t, Whirl::Forward),
                    ],
                )
            })
            .collect();
        let critical = critical_speeds(&points, &[1.0]);
        assert_eq!(critical.len(), 1, "{critical:?}");
        let c = &critical[0];
        assert_eq!((c.order, c.mode, c.whirl), (1.0, 1, Whirl::Backward));
        let t = 100.0 / (10.0 + 60.0 / std::f64::consts::TAU);
        assert!((c.speed - 60.0 * t).abs() < 1e-9, "{}", c.speed);
        // With the 20x line both modes are crossed.
        assert_eq!(critical_speeds(&points, &[20.0]).len(), 2);
        // Standstill, where every order line starts at zero, is no critical speed.
        let still = vec![
            point(0.0, &[(0.0, Whirl::Unknown)]),
            point(60.0, &[(5.0, Whirl::Forward)]),
        ];
        assert!(critical_speeds(&still, &[1.0]).is_empty());
    }

    #[test]
    fn the_diagram_names_modes_by_their_whirl_and_adds_order_lines() {
        let points = vec![
            point(0.0, &[(100.0, Whirl::Unknown), (100.0, Whirl::Unknown)]),
            point(
                std::f64::consts::TAU,
                &[(90.0, Whirl::Backward), (110.0, Whirl::Forward)],
            ),
            point(
                2.0 * std::f64::consts::TAU,
                &[(80.0, Whirl::Backward), (120.0, Whirl::Forward)],
            ),
        ];
        let data = diagram(&points, &[1.0, 2.0]);
        for (x, expected) in data.x.iter().zip([0.0, 60.0, 120.0]) {
            assert!((x - expected).abs() < 1e-9, "{:?}", data.x);
        }
        let names: Vec<&str> = data.curves.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Mode 1", "Mode 2", "1x", "2x"]);
        assert_eq!(data.curves[2].1, [0.0, 1.0, 2.0]);
        assert_eq!(data.curves[3].1, [0.0, 2.0, 4.0]);
        let steady = vec![
            point(1.0, &[(10.0, Whirl::Forward)]),
            point(2.0, &[(11.0, Whirl::Forward)]),
        ];
        assert_eq!(diagram(&steady, &[]).curves[0].0, "Mode 1 (F)");
    }

    #[test]
    fn orders_are_parsed_and_speeds_spread_evenly() {
        let setup = Setup {
            max_speed: 100.0,
            steps: 4,
            orders: "2, 1; 1 x".into(),
        };
        assert_eq!(setup.order_values(), [1.0, 2.0]);
        assert_eq!(speeds(100.0, 4), [25.0, 50.0, 75.0, 100.0]);
        assert!((rpm(from_rpm(3000.0)) - 3000.0).abs() < 1e-9);
    }

    /// The spinning test cantilever, swept over three speeds with CalculiX: the bending
    /// modes split more the faster it turns, and the sweep tells forward from backward.
    #[test]
    fn calculix_sweeps_the_speed_of_the_cantilever() {
        if std::process::Command::new("ccx")
            .arg("-v")
            .output()
            .is_err()
        {
            eprintln!("ccx not found, test skipped");
            return;
        }
        let testdata = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/kragbalken_c3d8.inp");
        let mesh = plx_io::inp::read_inp(&testdata).unwrap().mesh;
        let mut fe = FeModel {
            materials: vec![plx_model::Material {
                name: "Steel".into(),
                density: Some(7.85e-9),
                elastic: Some(plx_model::Elastic {
                    young: 210_000.0,
                    poisson: 0.3,
                }),
                ..Default::default()
            }],
            sections: vec![plx_model::Section {
                name: "Section-1".into(),
                material: "Steel".into(),
                region: plx_model::Region::Parts(vec!["EALL".into()]),
                thickness: 1.0,
                kind: plx_model::SectionKind::Solid,
            }],
            ..FeModel::default()
        };
        let fixed = plx_model::BoundaryCondition {
            name: "Fixed-1".into(),
            active: true,
            region: plx_model::Region::NodeSet("FIX".into()),
            kind: plx_model::BoundaryKind::Fixed,
            amplitude: None,
        };
        let mut spin = plx_model::Step::new_static("Step-1");
        spin.boundary_conditions.push(fixed.clone());
        spin.loads.push(plx_model::Load {
            name: "Centrifugal-1".into(),
            active: true,
            region: plx_model::Region::Parts(vec!["EALL".into()]),
            kind: LoadKind::Centrifugal {
                point: [0.0, 5.0, 5.0],
                axis: [1.0, 0.0, 0.0],
                speed: 200.0,
            },
            amplitude: None,
            factor_amplitude: None,
        });
        let mut frequency = plx_model::Step::new_frequency("Step-2");
        frequency.boundary_conditions.push(fixed.clone());
        if let StepKind::Frequency(f) = &mut frequency.kind {
            f.perturbation = true;
            f.storage = true;
            f.num_frequencies = 4;
        }
        let mut complex = plx_model::Step::new_complex_frequency("Step-3");
        complex.boundary_conditions.push(fixed);
        if let StepKind::ComplexFrequency(c) = &mut complex.kind {
            c.num_frequencies = 4;
        }
        fe.steps = vec![spin, frequency, complex];
        assert!(applies(&fe));
        assert_eq!(
            (rotor_speed(&fe), complex_step(&fe)),
            (Some(200.0), Some(3))
        );
        fe.resolve_default_solver(EquationSolver::Spooles);

        let work_dir = std::env::temp_dir().join(format!("plx-campbell-{}", std::process::id()));
        let input = SweepInput {
            solver: plx_job::Solver::default(),
            work_dir: work_dir.clone(),
            mesh: Arc::new(mesh),
            fe,
            step: 3,
            speeds: speeds(200.0, 3),
        };
        let cancel = AtomicBool::new(false);
        let points: Vec<SpeedPoint> = (input.speeds.iter().enumerate())
            .map(|(i, &speed)| run_speed(&input, speed, i, &cancel).unwrap())
            .collect();
        let _ = std::fs::remove_dir_all(&work_dir);
        assert_eq!(points.len(), 3);
        // In the rotating frame the pair splits by twice the speed: 2 x 200 rad/s = 63.7 Hz
        // at the top speed, a third of it at the first.
        let split = |p: &SpeedPoint| p.modes[1].frequency - p.modes[0].frequency;
        assert!((18.0..25.0).contains(&split(&points[0])), "{:?}", points[0]);
        assert!((55.0..72.0).contains(&split(&points[2])), "{points:?}");
        // CalculiX computes in the rotating frame of reference, where the forward whirl
        // appears below the backward one; both directions occur in the pair.
        let whirls = [points[2].modes[0].whirl, points[2].modes[1].whirl];
        assert!(whirls.contains(&Whirl::Forward) && whirls.contains(&Whirl::Backward));
        let critical = critical_speeds(&points, &[1.0]);
        assert!(critical.is_empty(), "{critical:?}");
        assert_eq!(diagram(&points, &[1.0]).curves.len(), 5);
        assert!(table_text(&points).lines().count() == 4);
    }

    #[test]
    fn complex_frequencies_are_read_from_the_dat_text() {
        let dat = "
                        S T E P       3

     E I G E N V A L U E   O U T P U T

 MODE NO                     FREQUENCY
                      REAL PART         IMAGINARY PART
             (RAD/TIME)   (CYCLES/TIME)   (RAD/TIME)

      1   0.4602752E+04   0.7325508E+03   0.4018004E-11
      2   0.6595139E+04   0.1049649E+04   0.5608028E-12

     E I G E N M O D E   T U R N I N G   D I R E C T I O N

    Axis reference direction:  0.1000E+01  0.0000E+00  0.0000E+00

 MODE NO     TURNING DIRECTION (F=FORWARD,B=BACKWARD)

      1          F
      2          B
";
        let modes = modes_of(dat, 3);
        assert_eq!(
            modes,
            [
                Mode {
                    frequency: 732.5508,
                    whirl: Whirl::Forward
                },
                Mode {
                    frequency: 1049.649,
                    whirl: Whirl::Backward
                }
            ]
        );
        assert!(modes_of(dat, 2).is_empty());
    }
}
