//! Self test of the CalculiX executable from the settings: prepolix does not ship CalculiX,
//! so the user can check that the configured program runs and computes correct results.
//!
//! The test reads the version and solves small cantilevers through the same path as a real
//! analysis (model, input file writer, job, results reader), comparing with beam theory.

use std::path::Path;
use std::process::Command;

use plx_io::frd::{FrdImport, read_frd};
use plx_job::{Job, JobStatus};
use plx_mesh::{Element, ElementId, ElementShape, FeMesh, NodeId, Part};
use plx_model::{
    BoundaryCondition, BoundaryKind, Elastic, FeModel, Load, LoadKind, Material, Region, Section,
    Step,
};

const LENGTH: f64 = 100.0;
const WIDTH: f64 = 10.0;
const HEIGHT: f64 = 10.0;
const YOUNG: f64 = 210_000.0;
const POISSON: f64 = 0.3;
/// Total tip force in -z.
const TIP_FORCE: f64 = 100.0;
const PRESSURE: f64 = 2.0;

/// Outcome of one check.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckResult {
    pub name: &'static str,
    pub passed: bool,
    pub message: String,
}

/// Runs all checks one after another; stops after the version check if CalculiX does not
/// start at all.
pub fn run(solver: &plx_job::Solver, work_dir: &Path) -> Vec<CheckResult> {
    let version = version(&solver.executable);
    let started = version.passed;
    let mut results = vec![version];
    if started {
        let dir = work_dir.join("Selbsttest");
        results.push(tip_force(solver, &dir));
        results.push(pressure(solver, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
    results
}

fn check(name: &'static str, passed: bool, message: String) -> CheckResult {
    CheckResult {
        name,
        passed,
        message,
    }
}

/// `ccx -v` prints e.g. "This is Version 2.21".
fn version(executable: &Path) -> CheckResult {
    const NAME: &str = "Programm starten";
    match Command::new(executable).arg("-v").output() {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            match text
                .lines()
                .find_map(|l| l.trim().strip_prefix("This is Version"))
            {
                Some(version) => check(NAME, true, format!("CalculiX {}", version.trim())),
                None => check(
                    NAME,
                    false,
                    format!(
                        "{} startet, meldet aber keine CalculiX-Version",
                        executable.display()
                    ),
                ),
            }
        }
        Err(error) => check(
            NAME,
            false,
            format!("{} nicht gestartet: {error}", executable.display()),
        ),
    }
}

/// Cantilever with a surface traction on the free end: the tip deflection must match
/// Timoshenko beam theory and the reactions must balance the load.
fn tip_force(solver: &plx_job::Solver, dir: &Path) -> CheckResult {
    const NAME: &str = "Kragbalken mit Endlast";
    let mesh = beam_mesh();
    let tip = faces_where(&mesh, |p| p[0] == LENGTH);
    let model = beam_model(
        &mesh,
        LoadKind::SurfaceTraction([0.0, 0.0, -TIP_FORCE]),
        Region::Faces(tip),
    );
    let frd = match solve(solver, dir, "Kragbalken", &mesh, &model) {
        Ok(frd) => frd,
        Err(message) => return check(NAME, false, message),
    };
    let tip_nodes = nodes_where(&mesh, |p| p[0] == LENGTH);
    let deflection = -mean(&frd, "DISP", "U3", &tip_nodes);
    let inertia = WIDTH * HEIGHT.powi(3) / 12.0;
    let shear_modulus = YOUNG / (2.0 * (1.0 + POISSON));
    let expected = TIP_FORCE * LENGTH.powi(3) / (3.0 * YOUNG * inertia)
        + TIP_FORCE * LENGTH / (5.0 / 6.0 * shear_modulus * WIDTH * HEIGHT);
    let deviation = (deflection / expected - 1.0).abs();
    let reaction = reaction(&frd, &mesh, "F3");
    let balanced = (reaction - TIP_FORCE).abs() < 1e-3 * TIP_FORCE;
    let passed = deviation < 0.03 && balanced;
    check(
        NAME,
        passed,
        format!(
            "Durchbiegung {deflection:.4} mm (Balkentheorie {expected:.4} mm, Abweichung {:.1} %), Reaktion {reaction:.3} N (Last {TIP_FORCE} N)",
            deviation * 100.0
        ),
    )
}

/// Pressure on the free end: the axial reactions must balance pressure times area. (Loads on
/// clamped nodes do not show up as reactions, so the end face is used, not the top.)
fn pressure(solver: &plx_job::Solver, dir: &Path) -> CheckResult {
    const NAME: &str = "Kragbalken mit Druck";
    let mesh = beam_mesh();
    let tip = faces_where(&mesh, |p| p[0] == LENGTH);
    let model = beam_model(&mesh, LoadKind::Pressure(PRESSURE), Region::Faces(tip));
    let frd = match solve(solver, dir, "Druck", &mesh, &model) {
        Ok(frd) => frd,
        Err(message) => return check(NAME, false, message),
    };
    let expected = PRESSURE * WIDTH * HEIGHT;
    let reaction = reaction(&frd, &mesh, "F1");
    let passed = (reaction - expected).abs() < 1e-3 * expected;
    check(
        NAME,
        passed,
        format!("Reaktion {reaction:.3} N (Druck mal Fläche {expected} N)"),
    )
}

fn solve(
    solver: &plx_job::Solver,
    dir: &Path,
    name: &str,
    mesh: &FeMesh,
    model: &FeModel,
) -> Result<FrdImport, String> {
    let input = plx_io::inp::write_inp(mesh, model, "prepolix Selbsttest")
        .map_err(|e| format!("Eingabedatei nicht geschrieben: {e}"))?;
    let mut job = Job::start(solver, dir, name, &input).map_err(|e| format!("Start: {e}"))?;
    let status = job.wait();
    let output = job.new_output();
    if status != JobStatus::Completed {
        let error = output
            .iter()
            .find(|l| l.contains("*ERROR"))
            .map_or_else(String::new, |l| format!(": {}", l.trim()));
        return Err(format!(
            "Rechnung {}{error}",
            crate::analysis::Analysis::status_text(status)
        ));
    }
    let path = job.results().ok_or("keine Ergebnisdatei geschrieben")?;
    read_frd(&path).map_err(|e| format!("Ergebnisse nicht lesbar: {e}"))
}

/// Mean of a result component over nodes.
fn mean(frd: &FrdImport, field: &str, component: &str, nodes: &[NodeId]) -> f64 {
    let values = component_values(frd, field, component, nodes);
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

/// Sum of the reaction forces at the clamped end.
fn reaction(frd: &FrdImport, mesh: &FeMesh, component: &str) -> f64 {
    let fixed = nodes_where(mesh, |p| p[0] == 0.0);
    component_values(frd, "FORC", component, &fixed)
        .iter()
        .sum()
}

fn component_values(frd: &FrdImport, field: &str, component: &str, nodes: &[NodeId]) -> Vec<f64> {
    let Some(values) = frd
        .increments
        .last()
        .and_then(|i| i.field(field))
        .and_then(|f| f.component(component))
        .map(|c| &c.values)
    else {
        return vec![f64::NAN];
    };
    nodes
        .iter()
        .map(|&node| {
            frd.mesh
                .node_index(node)
                .and_then(|i| values.get(i))
                .map_or(f64::NAN, |&v| f64::from(v))
        })
        .collect()
}

fn beam_model(mesh: &FeMesh, load: LoadKind, region: Region) -> FeModel {
    let mut step = Step::new_static("Step-1");
    step.boundary_conditions.push(BoundaryCondition {
        name: "Fixed-1".into(),
        region: Region::Nodes(nodes_where(mesh, |p| p[0] == 0.0)),
        kind: BoundaryKind::Fixed,
    });
    step.loads.push(Load {
        name: "Load-1".into(),
        region,
        kind: load,
    });
    FeModel {
        materials: vec![Material {
            name: "Steel".into(),
            density: None,
            elastic: Some(Elastic {
                young: YOUNG,
                poisson: POISSON,
            }),
        }],
        sections: vec![Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: Region::Parts(vec!["BEAM".into()]),
        }],
        steps: vec![step],
    }
}

/// The beam as 10 x 2 x 2 quadratic hexahedra (C3D20R), x along the length.
fn beam_mesh() -> FeMesh {
    const DIVISIONS: [u32; 3] = [10, 2, 2];
    let size = [LENGTH, WIDTH, HEIGHT];
    // Nodes on a lattice of half element size; the id encodes the lattice position.
    let points = DIVISIONS.map(|d| 2 * d + 1);
    let id = |p: [u32; 3]| 1 + p[0] + points[0] * (p[1] + points[1] * p[2]);
    let mut mesh = FeMesh::default();
    let add = |mesh: &mut FeMesh, p: [u32; 3]| {
        let coords = [0, 1, 2].map(|k| size[k] * f64::from(p[k]) / f64::from(points[k] - 1));
        mesh.set_node(id(p), coords);
        id(p)
    };
    // CalculiX's C3D20 node order relative to the element's corner, in half element sizes.
    const LOCAL: [[u32; 3]; 20] = [
        [0, 0, 0],
        [2, 0, 0],
        [2, 2, 0],
        [0, 2, 0],
        [0, 0, 2],
        [2, 0, 2],
        [2, 2, 2],
        [0, 2, 2],
        [1, 0, 0],
        [2, 1, 0],
        [1, 2, 0],
        [0, 1, 0],
        [1, 0, 2],
        [2, 1, 2],
        [1, 2, 2],
        [0, 1, 2],
        [0, 0, 1],
        [2, 0, 1],
        [2, 2, 1],
        [0, 2, 1],
    ];
    let mut elements = Vec::new();
    for k in 0..DIVISIONS[2] {
        for j in 0..DIVISIONS[1] {
            for i in 0..DIVISIONS[0] {
                let corner = [2 * i, 2 * j, 2 * k];
                let nodes = LOCAL
                    .iter()
                    .map(|l| add(&mut mesh, [0, 1, 2].map(|d| corner[d] + l[d])))
                    .collect();
                let element_id = elements.len() as ElementId + 1;
                mesh.add_element(Element {
                    id: element_id,
                    type_name: "C3D20R".into(),
                    shape: ElementShape::Hex20,
                    nodes,
                })
                .expect("20 nodes per element");
                elements.push(element_id);
            }
        }
    }
    mesh.parts.push(Part {
        name: "BEAM".into(),
        elements,
    });
    mesh
}

fn nodes_where(mesh: &FeMesh, on: impl Fn([f64; 3]) -> bool) -> Vec<NodeId> {
    mesh.node_ids()
        .iter()
        .zip(mesh.coords())
        .filter(|&(_, &p)| on(p))
        .map(|(&id, _)| id)
        .collect()
}

/// Element faces whose corners all lie where `on` holds.
fn faces_where(mesh: &FeMesh, on: impl Fn([f64; 3]) -> bool) -> Vec<(ElementId, u8)> {
    let mut faces = Vec::new();
    for element in mesh.elements() {
        for (index, face) in element.shape.faces().iter().enumerate() {
            let inside = face
                .corners
                .iter()
                .all(|&l| mesh.node(element.nodes[l]).is_some_and(&on));
            if inside {
                faces.push((element.id, index as u8 + 1));
            }
        }
    }
    faces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beam_mesh_is_a_closed_brick() {
        let mesh = beam_mesh();
        assert_eq!(mesh.element_count(), 40);
        assert!(mesh.missing_nodes().is_empty());
        // 21 x 5 x 5 lattice points minus face and body centres of each element.
        assert_eq!(
            mesh.node_count(),
            21 * 5 * 5 - 40 - (10 * 2 * 3 + 10 * 3 * 2 + 11 * 2 * 2)
        );
        assert_eq!(faces_where(&mesh, |p| p[0] == LENGTH).len(), 4);
        assert_eq!(faces_where(&mesh, |p| p[2] == HEIGHT).len(), 20);
    }

    #[test]
    fn missing_program_fails_the_first_check_only() {
        let solver = plx_job::Solver {
            executable: "plx-gibt-es-nicht".into(),
            threads: 1,
        };
        let results = run(&solver, &std::env::temp_dir());
        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
    }

    #[test]
    fn installed_calculix_passes() {
        if Command::new("ccx").arg("-v").output().is_err() {
            eprintln!("ccx nicht gefunden, Test übersprungen");
            return;
        }
        let dir = std::env::temp_dir().join(format!("plx-selbsttest-{}", std::process::id()));
        let results = run(&plx_job::Solver::default(), &dir);
        let _ = std::fs::remove_dir_all(&dir);
        for result in &results {
            eprintln!("{}: {}", result.name, result.message);
        }
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.passed), "{results:#?}");
    }
}
