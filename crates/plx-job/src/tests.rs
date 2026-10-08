use super::*;

const STA: &str = "\
SUMMARY OF JOB INFORMATION
  STEP      INC     ATT  ITRS     TOT TIME     STEP TIME         INC TIME
     1       1     1     2   0.250000E+00  0.250000E+00  0.250000E+00
     1       2     1     3   0.625000E+00  0.625000E+00  0.375000E+00
";

#[test]
fn reads_the_last_increment_of_the_status_file() {
    let progress = parse_sta(STA).unwrap();
    assert_eq!(
        (progress.step, progress.increment, progress.iterations),
        (1, 2, 3)
    );
    assert_eq!(progress.step_time, 0.625);
    assert_eq!(parse_sta("SUMMARY OF JOB INFORMATION\n"), None);
}

fn work_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("plx-job-{name}-{}", std::process::id()))
}

fn ccx_installed() -> bool {
    let found = Command::new("ccx").arg("-v").output().is_ok();
    if !found {
        eprintln!("ccx nicht gefunden, Test übersprungen");
    }
    found
}

#[test]
fn runs_calculix_and_finds_the_results() {
    if !ccx_installed() {
        return;
    }
    let input = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/kragbalken_c3d8.inp"),
    )
    .unwrap();
    let dir = work_dir("ok");
    let mut job = Job::start(&Solver::default(), &dir, "Analysis-1", &input).unwrap();
    assert_eq!(job.wait(), JobStatus::Completed);
    assert!(job.new_output().iter().any(|l| l.contains("Job finished")));
    assert!(job.new_output().is_empty());
    assert_eq!(job.progress().map(|p| p.step), Some(1));
    assert!(job.results().is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn input_errors_fail_the_job() {
    if !ccx_installed() {
        return;
    }
    let dir = work_dir("error");
    let mut job = Job::start(&Solver::default(), &dir, "Analysis-1", "*STEP\n*STATIC\n").unwrap();
    assert_eq!(job.wait(), JobStatus::Failed);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_missing_solver_is_reported_at_start() {
    let solver = Solver {
        executable: PathBuf::from("plx-kein-ccx"),
        threads: 1,
    };
    let dir = work_dir("missing");
    assert!(Job::start(&solver, &dir, "Analysis-1", "").is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
