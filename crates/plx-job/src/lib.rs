//! Start und Überwachung des CalculiX-Solvers.
//!
//! A job runs `ccx <name>` in its work directory the way PrePoMax's `AnalysisJob` does: old
//! result files are removed, the input file is written, and the solver output is collected
//! while it runs. Progress comes from the `.sta` file CalculiX updates after each increment.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Files of a previous run that would otherwise be mistaken for results of this one.
const OLD_FILES: [&str; 7] = ["inp", "dat", "sta", "cvg", "12d", "cel", "frd"];

/// Smallest `.frd` that holds results; PrePoMax uses the same limit.
const MIN_RESULT_SIZE: u64 = 300;

/// How to run CalculiX.
#[derive(Clone, Debug, PartialEq)]
pub struct Solver {
    /// The CalculiX executable, a path or a name found on `PATH`.
    pub executable: PathBuf,
    /// Threads for the solver (`OMP_NUM_THREADS`).
    pub threads: u32,
}

impl Default for Solver {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("ccx"),
            threads: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    /// Finished with results.
    Completed,
    /// CalculiX reported an error but wrote results, e.g. of earlier increments.
    FailedWithResults,
    Failed,
    Killed,
}

/// One line of the `.sta` file: the last increment CalculiX finished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub step: u32,
    pub increment: u32,
    pub attempt: u32,
    pub iterations: u32,
    pub total_time: f64,
    pub step_time: f64,
    pub increment_time: f64,
}

/// A running or finished CalculiX analysis.
pub struct Job {
    name: String,
    work_dir: PathBuf,
    child: Child,
    output: Arc<Mutex<Vec<String>>>,
    readers: Vec<JoinHandle<()>>,
    /// Lines of output already handed out by [`Job::new_output`].
    read: usize,
    status: JobStatus,
}

impl Job {
    /// Writes `<work_dir>/<name>.inp` and starts CalculiX on it.
    pub fn start(
        solver: &Solver,
        work_dir: &Path,
        name: &str,
        input: &str,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(work_dir)?;
        for extension in OLD_FILES {
            let path = work_dir.join(format!("{name}.{extension}"));
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        std::fs::write(work_dir.join(format!("{name}.inp")), input)?;
        let mut child = Command::new(&solver.executable)
            .arg(name)
            .current_dir(work_dir)
            .env("OMP_NUM_THREADS", solver.threads.max(1).to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(collect(stdout, Arc::clone(&output)));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(collect(stderr, Arc::clone(&output)));
        }
        Ok(Self {
            name: name.to_owned(),
            work_dir: work_dir.to_owned(),
            child,
            output,
            readers,
            read: 0,
            status: JobStatus::Running,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Checks whether CalculiX has finished, without waiting.
    pub fn poll(&mut self) -> JobStatus {
        if self.status == JobStatus::Running {
            match self.child.try_wait() {
                Ok(Some(_)) => self.finish(),
                Ok(None) => {}
                Err(_) => self.status = JobStatus::Failed,
            }
        }
        self.status
    }

    /// Waits until CalculiX has finished.
    pub fn wait(&mut self) -> JobStatus {
        if self.status == JobStatus::Running {
            match self.child.wait() {
                Ok(_) => self.finish(),
                Err(_) => self.status = JobStatus::Failed,
            }
        }
        self.status
    }

    /// Stops the solver.
    pub fn kill(&mut self) {
        if self.status == JobStatus::Running {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.join_readers();
            self.status = JobStatus::Killed;
        }
    }

    /// Output lines written since the last call.
    pub fn new_output(&mut self) -> Vec<String> {
        let output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        let lines = output[self.read.min(output.len())..].to_vec();
        self.read = output.len();
        lines
    }

    /// The last increment CalculiX reported in the `.sta` file.
    pub fn progress(&self) -> Option<Progress> {
        let text = std::fs::read_to_string(self.file("sta")).ok()?;
        parse_sta(&text)
    }

    /// The result file, once CalculiX has written results to it.
    pub fn results(&self) -> Option<PathBuf> {
        let path = self.file("frd");
        let size = std::fs::metadata(&path).ok()?.len();
        (size > MIN_RESULT_SIZE).then_some(path)
    }

    pub fn file(&self, extension: &str) -> PathBuf {
        self.work_dir.join(format!("{}.{extension}", self.name))
    }

    /// Status after the process ended, decided as in PrePoMax.
    fn finish(&mut self) {
        self.join_readers();
        let error = self
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|line| line.contains("*ERROR"));
        self.status = match (self.results().is_some(), error) {
            (false, _) => JobStatus::Failed,
            (true, true) => JobStatus::FailedWithResults,
            (true, false) => JobStatus::Completed,
        };
    }

    fn join_readers(&mut self) {
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.kill();
    }
}

fn collect(stream: impl Read + Send + 'static, output: Arc<Mutex<Vec<String>>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        // CalculiX output is not always valid UTF-8; read bytes and convert lossily.
        while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            let text = String::from_utf8_lossy(&line).trim_end().to_owned();
            output.lock().unwrap_or_else(|e| e.into_inner()).push(text);
            line.clear();
        }
    })
}

/// The last increment line of a `.sta` file: `step inc att iter tot_time step_time inc_time`.
pub fn parse_sta(text: &str) -> Option<Progress> {
    text.lines().rev().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [
            step,
            increment,
            attempt,
            iterations,
            total,
            step_time,
            increment_time,
        ] = fields[..]
        else {
            return None;
        };
        Some(Progress {
            step: step.parse().ok()?,
            // CalculiX marks increments of linear perturbation steps with a "U".
            increment: increment.trim_end_matches('U').parse().ok()?,
            attempt: attempt.parse().ok()?,
            iterations: iterations.parse().ok()?,
            total_time: total.parse().ok()?,
            step_time: step_time.parse().ok()?,
            increment_time: increment_time.parse().ok()?,
        })
    })
}

#[cfg(test)]
mod tests;
