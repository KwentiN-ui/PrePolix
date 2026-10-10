use std::path::PathBuf;
use std::process::Command;

use plx_model::{
    BeamOrientation, BeamProfile, BeamSection, BoundaryCondition, Elastic, EquationSolver, Load,
    Material, NodeTie, Section, SectionKind, UserKeyword,
};

use super::*;
use crate::frd::{FrdImport, read_frd};
use crate::inp::{read_inp, read_inp_str};

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(name)
}

/// The cantilever of `kragbalken_c3d8.inp`, set up the way the GUI would.
fn cantilever(load: Load) -> (FeMesh, FeModel) {
    analysis("kragbalken_c3d8.inp", load)
}

/// A test model with steel on part EALL, held at node set FIX.
fn analysis(file: &str, load: Load) -> (FeMesh, FeModel) {
    let mesh = read_inp(&testdata(file)).unwrap().mesh;
    let mut step = Step::new_static("Step-1");
    step.boundary_conditions.push(BoundaryCondition {
        name: "Fixed-1".into(),
        active: true,
        region: Region::NodeSet("FIX".into()),
        kind: BoundaryKind::Fixed,
        amplitude: None,
    });
    step.loads.push(load);
    let model = FeModel {
        properties: Default::default(),
        materials: vec![Material {
            name: "Steel".into(),
            density: Some(7.85e-9),
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
            ..Default::default()
        }],
        sections: vec![Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: Region::Parts(vec!["EALL".into()]),
            thickness: 1.0,
            kind: SectionKind::Solid,
        }],
        steps: vec![step],
        user_keywords: Vec::new(),
        ..FeModel::default()
    };
    (mesh, model)
}

fn tip_force() -> Load {
    Load {
        name: "Force-1".into(),
        active: true,
        region: Region::Nodes(vec![99]),
        kind: LoadKind::ConcentratedForce([0.0, 0.0, -100.0]),
        amplitude: None,
        factor_amplitude: None,
    }
}

#[test]
fn writes_mesh_and_analysis_that_read_back() {
    let (mesh, model) = cantilever(tip_force());
    let text = write_inp(&mesh, &model, "Kragbalken").unwrap();
    for line in [
        "** Nodes +++++++++++++++++++++++++++++++++++++++++++++++++++\n",
        "*Node\n1, 0.00000000E0, 0.00000000E0, 0.00000000E0\n",
        "*Element, Type=C3D8, Elset=EALL\n",
        "*Elset, Elset=Internal_Selection-1_Section-1\nEALL\n",
        "*Nset, Nset=Internal_Selection-1_Force-1\n99\n",
        "*Material, Name=Steel\n*Density\n0.00000000785\n*Elastic\n210000, 0.3\n",
        "*Solid section, Elset=Internal_Selection-1_Section-1, Material=Steel\n",
        "*Step\n*Static\n",
        "*Boundary, op=New\n",
        "*Boundary\nFIX, 1, 6, 0\n",
        "*Cload, op=New\n*Dload, op=New\n",
        "*Cload\nInternal_Selection-1_Force-1, 3, -100\n",
        "*Node file\nRF, U\n",
        "*El file\nS, E, NOE\n",
        "*End step\n",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    let read = read_inp_str(&text, None).unwrap().mesh;
    assert_eq!(read.node_count(), mesh.node_count());
    assert_eq!(read.elements(), mesh.elements());
    assert_eq!(read.node_sets["FIX"], mesh.node_sets["FIX"]);
    assert_eq!(read.surfaces["TIP"], mesh.surfaces["TIP"]);
    // The part set must not be written twice.
    assert_eq!(read.element_sets["EALL"].len(), 40);
}

#[test]
fn the_chosen_solver_is_written_with_the_procedure() {
    let (mesh, mut model) = cantilever(tip_force());
    model.resolve_default_solver(EquationSolver::Pardiso);
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("*Step\n*Static, Solver=Pardiso\n"), "{text}");
    let StepKind::Static(settings) = &mut model.steps[0].kind else {
        unreachable!()
    };
    settings.solver = EquationSolver::IterativeCholesky;
    settings.incrementation = Incrementation::Direct;
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(
        text.contains("*Static, Solver=Iterative Cholesky, Direct\n1, 1\n"),
        "{text}"
    );
    // A solver chosen in the step is kept.
    model.resolve_default_solver(EquationSolver::Pardiso);
    let StepKind::Static(settings) = &model.steps[0].kind else {
        unreachable!()
    };
    assert_eq!(settings.solver, EquationSolver::IterativeCholesky);
}

#[test]
fn picked_faces_become_one_element_set_per_face_number() {
    let load = Load {
        name: "Pressure 1".into(),
        active: true,
        region: Region::Faces(vec![(10, 4), (20, 4), (1, 6)]),
        kind: LoadKind::Pressure(2.5),
        amplitude: None,
        factor_amplitude: None,
    };
    let (mesh, model) = cantilever(load);
    let text = write_inp(&mesh, &model, "").unwrap();
    let side = "Internal-1_Internal_Selection-1_Pressure_1";
    for line in [
        format!("*Elset, Elset={side}_S4\n10, 20\n"),
        format!("*Elset, Elset={side}_S6\n1\n"),
        format!("*Surface, Name=Internal_Selection-1_Pressure_1, Type=Element\n{side}_S4, S4\n"),
        format!("*Dload\n{side}_S4, P4, 2.5\n{side}_S6, P6, 2.5\n"),
    ] {
        assert!(text.contains(&line), "missing {line:?} in\n{text}");
    }
}

#[test]
fn empty_regions_and_unknown_materials_are_errors() {
    let (mesh, mut model) = cantilever(Load {
        name: "Force-1".into(),
        active: true,
        region: Region::Nodes(Vec::new()),
        kind: LoadKind::ConcentratedForce([1.0, 0.0, 0.0]),
        amplitude: None,
        factor_amplitude: None,
    });
    assert_eq!(
        write_inp(&mesh, &model, ""),
        Err(WriteError::EmptyRegion {
            item: "Force-1".into(),
            what: "Knoten"
        })
    );
    model.sections[0].material = "Alu".into();
    assert!(matches!(
        write_inp(&mesh, &model, ""),
        Err(WriteError::UnknownMaterial { .. })
    ));
}

/// Runs CalculiX on the input file, if it is installed; CI does not have it.
fn run_ccx(name: &str, text: &str) -> Option<FrdImport> {
    if Command::new("ccx").arg("-v").output().is_err() {
        eprintln!("ccx nicht gefunden, Test übersprungen");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("plx-write-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.inp")), text).unwrap();
    let output = Command::new("ccx")
        .args(["-i", name])
        .current_dir(&dir)
        .env("OMP_NUM_THREADS", "1")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && !log.contains("*ERROR"), "{log}");
    let frd = read_frd(&dir.join(format!("{name}.frd"))).unwrap();
    // PLX_KEEP_CCX keeps the files for a look at them.
    if std::env::var_os("PLX_KEEP_CCX").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Some(frd)
}

fn node_value(frd: &FrdImport, field: &str, component: &str, node: NodeId) -> f64 {
    let index = frd.mesh.node_index(node).unwrap();
    let increment = frd.increments.last().unwrap();
    let field = increment.field(field).unwrap();
    f64::from(field.component(component).unwrap().values[index])
}

#[test]
fn calculix_reproduces_the_reference_cantilever() {
    let (mesh, model) = cantilever(tip_force());
    let Some(frd) = run_ccx("kragbalken", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let reference = read_frd(&testdata("kragbalken_c3d8.frd")).unwrap();
    let ours = node_value(&frd, "DISP", "U3", 99);
    let expected = node_value(&reference, "DISP", "U3", 99);
    assert!(
        (ours - expected).abs() < 1e-6 * expected.abs(),
        "{ours} != {expected}"
    );
}

#[test]
fn calculix_pressure_on_a_surface_balances_the_reactions() {
    let load = Load {
        name: "Pressure-1".into(),
        active: true,
        region: Region::Surface("TIP".into()),
        kind: LoadKind::Pressure(2.0),
        amplitude: None,
        factor_amplitude: None,
    };
    let (mesh, model) = cantilever(load);
    let Some(frd) = run_ccx("druck", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let reaction: f64 = mesh.node_sets["FIX"]
        .iter()
        .map(|&node| node_value(&frd, "FORC", "F1", node))
        .sum();
    // 2 MPa on the 10 x 10 mm tip face.
    assert!((reaction.abs() - 200.0).abs() < 1e-3, "{reaction}");
}

#[test]
fn calculix_reads_quadratic_elements_over_two_lines() {
    const NODE: NodeId = 141;
    const FORCE: [f64; 3] = [0.0, -500.0, 0.0];
    let load = Load {
        name: "Force-1".into(),
        active: true,
        region: Region::Nodes(vec![NODE]),
        kind: LoadKind::ConcentratedForce(FORCE),
        amplitude: None,
        factor_amplitude: None,
    };
    let (mesh, model) = analysis("block_c3d20r.inp", load);
    let text = write_inp(&mesh, &model, "").unwrap();
    let element = mesh.elements()[0]
        .nodes
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>();
    let first = format!(
        "1, {},\n{}\n",
        element[..15].join(", "),
        element[15..].join(", ")
    );
    assert!(text.contains(&first), "{first}");
    let Some(frd) = run_ccx("block", &text) else {
        return;
    };
    let reference = read_frd(&testdata("block_c3d20r.frd")).unwrap();
    for component in ["U1", "U2", "U3"] {
        let ours = node_value(&frd, "DISP", component, NODE);
        let expected = node_value(&reference, "DISP", component, NODE);
        assert!(
            (ours - expected).abs() <= 1e-6 * expected.abs().max(1e-9),
            "{ours} != {expected}"
        );
    }
}

#[test]
fn surface_traction_spreads_the_total_force_by_area() {
    let mesh = &read_inp(&testdata("block_c3d20r.inp")).unwrap().mesh;
    // Every element face on the x = min side of the block.
    let min_x = mesh.bounds().unwrap().0[0];
    let faces: Vec<(ElementId, u8)> = mesh
        .elements()
        .iter()
        .flat_map(|e| {
            (1..=6u8).filter_map(move |f| {
                let topology = e.shape.faces()[usize::from(f) - 1];
                let on_side = topology
                    .corners
                    .iter()
                    .all(|&l| mesh.node(e.nodes[l]).unwrap()[0] == min_x);
                on_side.then_some((e.id, f))
            })
        })
        .collect();
    assert!(!faces.is_empty());
    let nodal = traction_forces(mesh, &faces, [0.0, -500.0, 30.0], false);
    let total = nodal
        .values()
        .fold([0.0; 3], |sum, f| [0, 1, 2].map(|k| sum[k] + f[k]));
    assert!(
        (total[1] + 500.0).abs() < 1e-9 && (total[2] - 30.0).abs() < 1e-9,
        "{total:?}"
    );
    // Quadratic quads: corners pull against the load direction, midside nodes carry it.
    assert!(nodal.values().any(|f| f[1] > 0.0) && nodal.values().any(|f| f[1] < 0.0));
}

#[test]
fn calculix_balances_a_surface_traction() {
    let load = Load {
        name: "Surface_Traction-1".into(),
        active: true,
        region: Region::Surface("TIP".into()),
        kind: LoadKind::SurfaceTraction([0.0, -80.0, 0.0]),
        amplitude: None,
        factor_amplitude: None,
    };
    let (mesh, model) = cantilever(load);
    let text = write_inp(&mesh, &model, "").unwrap();
    let Some(frd) = run_ccx("traktion", &text) else {
        return;
    };
    let reaction: f64 = mesh.node_sets["FIX"]
        .iter()
        .map(|&node| node_value(&frd, "FORC", "F2", node))
        .sum();
    assert!((reaction - 80.0).abs() < 1e-3, "{reaction}");
}

/// Position of the title `name` among the top-level keywords.
fn top_title(tree: &[Keyword], name: &str) -> usize {
    tree.iter()
        .position(|k| k.kind == KeywordKind::Title(name.into()))
        .unwrap()
}

#[test]
fn user_keywords_are_written_at_their_place_and_read_back() {
    let (mesh, mut model) = cantilever(tip_force());
    let mut tree = model_keywords(&mesh, &model, "").unwrap();
    let amplitudes = top_title(&tree, "Amplitudes");
    let steps = top_title(&tree, "Steps");
    // Step title > *Step > [*Static, Controls, ..., History outputs (index 6), ...].
    let history = &tree[steps].children[0].children[0].children[6];
    assert_eq!(history.kind, KeywordKind::Title("History outputs".into()));
    model.user_keywords = vec![
        UserKeyword {
            position: vec![amplitudes, 0],
            text: "*Amplitude, Name=Ramp\n0, 0, 1, 1".into(),
            active: true,
        },
        UserKeyword {
            position: vec![steps, 0, 0, 6, 0],
            text: "*Node print, Nset=FIX\nRF".into(),
            active: true,
        },
        UserKeyword {
            position: vec![steps, 0, 0, 6, 1],
            text: "*El print, Elset=EALL\nS".into(),
            active: false,
        },
        // A place that no longer exists, for example inside a deleted material.
        UserKeyword {
            position: vec![top_title(&tree, "Materials"), 5, 0],
            text: "*Plastic".into(),
            active: true,
        },
    ];
    let placed = insert_user_keywords(&mut tree, &model.user_keywords);
    assert_eq!(placed, [true, true, true, false]);
    // Collecting them again gives the same positions, without the one left out.
    assert_eq!(user_keywords(&tree), model.user_keywords[..3]);

    let text = write_inp(&mesh, &model, "").unwrap();
    assert_eq!(text, write_keywords(&tree));
    for line in [
        "** Amplitudes ++++++++++++++++++++++++++++++++++++++++++++++\n**\n*Amplitude, Name=Ramp\n0, 0, 1, 1\n",
        "** History outputs +++++++++++++++++++++++++++++++++++++++++\n**\n*Node print, Nset=FIX\nRF\n** *El print, Elset=EALL\n** S\n",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    assert!(!text.contains("*Plastic"));
    // The step's *Node print ends up in the .dat file, so CalculiX accepted it.
    let Some(dat) = run_ccx_dat("benutzer", &text) else {
        return;
    };
    assert!(dat.contains("forces (fx,fy,fz) for set FIX"), "{dat}");
}

/// Runs CalculiX like [`run_ccx`] and returns the `.dat` file.
fn run_ccx_dat(name: &str, text: &str) -> Option<String> {
    if Command::new("ccx").arg("-v").output().is_err() {
        eprintln!("ccx nicht gefunden, Test übersprungen");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("plx-write-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.inp")), text).unwrap();
    let output = Command::new("ccx")
        .args(["-i", name])
        .current_dir(&dir)
        .env("OMP_NUM_THREADS", "1")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && !log.contains("*ERROR"), "{log}");
    let dat = std::fs::read_to_string(dir.join(format!("{name}.dat"))).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    Some(dat)
}

#[test]
fn check_model_replaces_the_procedures_by_no_analysis() {
    let (mesh, mut model) = cantilever(tip_force());
    let text = write_check_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("*Step\n*No analysis\n"), "{text}");
    assert!(!text.contains("*Static"), "{text}");
    // The loads are still checked.
    assert!(text.contains("*Cload\nInternal_Selection-1_Force-1, 3, -100\n"));
    // A deactivated step stays a comment, and without an active step PrePoMax's CheckModel
    // step is added.
    model.steps[0].active = false;
    let text = write_check_inp(&mesh, &model, "").unwrap();
    assert!(
        text.contains("** Name: StaticStep: Deactivated\n"),
        "{text}"
    );
    assert!(text.contains("** CheckModel"), "{text}");
    assert_eq!(text.matches("*No analysis").count(), 1, "{text}");
    model.steps.clear();
    let text = write_check_inp(&mesh, &model, "").unwrap();
    let step = text.find("*Step\n*No analysis\n").expect(&text);
    assert!(text[step..].contains("*End step\n"), "{text}");
}

/// The cantilever of `kragbalken_c3d20r.inp` with a frequency step after the static one.
fn frequency_analysis() -> (FeMesh, FeModel) {
    let (mesh, mut model) = analysis("kragbalken_c3d20r.inp", tip_force());
    let mut step = Step::new_frequency("Step-2");
    step.boundary_conditions = model.steps[0].boundary_conditions.clone();
    // A load left in a frequency step must not reach CalculiX.
    step.loads = model.steps[0].loads.clone();
    model.steps.push(step);
    (mesh, model)
}

#[test]
fn a_frequency_step_is_written_like_prepomax_does() {
    let (mesh, mut model) = frequency_analysis();
    let text = write_inp(&mesh, &model, "").unwrap();
    let frequency = &text[text.find("** Step-2").unwrap()..];
    for line in [
        "*Step\n*Frequency\n10\n",
        "*Boundary\nFIX, 1, 6, 0\n",
        "*Node file\nU\n",
        "*El file\nS, E, NOE\n",
    ] {
        assert!(frequency.contains(line), "missing {line:?} in\n{frequency}");
    }
    assert!(!frequency.contains("*Cload"), "{frequency}");
    let StepKind::Frequency(settings) = &mut model.steps[1].kind else {
        unreachable!()
    };
    settings.perturbation = true;
    settings.storage = true;
    settings.num_frequencies = 4;
    settings.upper_frequency = Some(5000.0);
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(
        text.contains("*Step, Perturbation\n*Frequency, Storage=Yes\n4, 0, 5000\n"),
        "{text}"
    );
    let StepKind::Frequency(settings) = &mut model.steps[1].kind else {
        unreachable!()
    };
    settings.lower_frequency = Some(100.0);
    settings.upper_frequency = None;
    model.resolve_default_solver(EquationSolver::Spooles);
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(
        text.contains("*Frequency, Solver=Spooles, Storage=Yes\n4, 100\n"),
        "{text}"
    );
}

#[test]
fn calculix_finds_the_bending_frequency_of_the_cantilever() {
    let (mesh, mut model) = frequency_analysis();
    // The preload of the static step is small; CalculiX must accept the perturbation step.
    let StepKind::Frequency(settings) = &mut model.steps[1].kind else {
        unreachable!()
    };
    settings.perturbation = true;
    let Some(frd) = run_ccx("eigenfrequenz", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let modes: Vec<f64> = (frd.increments.iter())
        .filter(|i| i.kind == plx_results::AnalysisKind::Frequency)
        .map(|i| i.value)
        .collect();
    assert_eq!(modes.len(), 10, "{modes:?}");
    // Euler-Bernoulli: f1 = 1.875² / (2π L²) √(EI / ρA) = 835 Hz for the 10 x 10 x 100 steel
    // beam; shear makes the real beam a little softer. The square section bends alike in
    // both directions.
    let euler = 1.875_f64.powi(2) / (2.0 * std::f64::consts::PI * 100.0_f64.powi(2))
        * (210_000.0 * 10.0_f64.powi(4) / 12.0 / (7.85e-9 * 100.0)).sqrt();
    for mode in &modes[..2] {
        assert!(
            (0.95 * euler..1.001 * euler).contains(mode),
            "{mode} Hz vs. {euler} Hz"
        );
    }
}

/// The cantilever of `kragbalken_c3d20r.inp` in a buckle step, pressed along its axis by
/// 1 N on each of the 21 nodes of its free end.
fn buckle_analysis() -> (FeMesh, FeModel) {
    let (mesh, mut model) = analysis(
        "kragbalken_c3d20r.inp",
        Load {
            name: "Force-1".into(),
            active: true,
            region: Region::NodeSet("TIP".into()),
            kind: LoadKind::ConcentratedForce([-1.0, 0.0, 0.0]),
            amplitude: None,
            factor_amplitude: None,
        },
    );
    let mut step = Step::new_buckle("Step-1");
    step.boundary_conditions = model.steps[0].boundary_conditions.clone();
    step.loads = model.steps[0].loads.clone();
    model.steps = vec![step];
    (mesh, model)
}

#[test]
fn a_buckle_step_is_written_like_prepomax_does() {
    let (mesh, mut model) = buckle_analysis();
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Step\n*Buckle\n1, 0.0001\n",
        "*Boundary\nFIX, 1, 6, 0\n",
        "*Cload\nTIP, 1, -1\n",
        "*Node file\nRF, U\n",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    let StepKind::Buckle(settings) = &mut model.steps[0].kind else {
        unreachable!()
    };
    settings.perturbation = true;
    settings.num_factors = 3;
    settings.accuracy = 0.01;
    model.resolve_default_solver(EquationSolver::Spooles);
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(
        text.contains("*Step, Perturbation\n*Buckle, Solver=Spooles\n3, 0.01\n"),
        "{text}"
    );
}

#[test]
fn calculix_finds_the_euler_load_of_the_cantilever() {
    let (mesh, mut model) = buckle_analysis();
    let StepKind::Buckle(settings) = &mut model.steps[0].kind else {
        unreachable!()
    };
    settings.num_factors = 2;
    let Some(frd) = run_ccx("beulen", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let buckling: Vec<_> = (frd.increments.iter())
        .filter(|i| i.kind == plx_results::AnalysisKind::Buckling)
        .collect();
    // The static solution of the reference load comes first as increment 0, then one
    // increment per buckling mode with its factor and mode shape.
    let (reference, buckling) = buckling.split_first().unwrap();
    assert_eq!((reference.increment, reference.value), (0, 0.0));
    assert!(reference.field("STRESS").is_some());
    let factors: Vec<f64> = buckling.iter().map(|i| i.value).collect();
    assert_eq!(factors.len(), 2, "{factors:?}");
    let modes: Vec<u32> = buckling.iter().map(|i| i.increment).collect();
    assert_eq!(modes, [1, 2]);
    assert!(buckling.iter().all(|i| i.displacements().is_some()));
    // Euler case 1: P = π² EI / (4 L²) for the 10 x 10 x 100 steel beam, against the 21 N
    // of the load; shear makes the real beam a little softer. The square section buckles
    // alike in both directions.
    let euler = std::f64::consts::PI.powi(2) * 210_000.0 * 10.0_f64.powi(4)
        / 12.0
        / (4.0 * 100.0_f64.powi(2))
        / 21.0;
    for factor in &factors {
        assert!(
            (0.95 * euler..1.001 * euler).contains(factor),
            "{factor} vs. {euler}"
        );
    }
}

#[test]
fn the_buckling_mode_exports_as_a_deformed_mesh() {
    let (mesh, model) = buckle_analysis();
    let Some(frd) = run_ccx("beulform", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let mode = frd.increments.iter().find(|i| i.increment == 1).unwrap();
    let displacements = mode.displacements().unwrap();
    let text = write_deformed_mesh_inp(&frd.mesh, &displacements, 0.5, "Beulform").unwrap();
    // Only the mesh: no material, section or step.
    for keyword in ["*Material", "*Solid section", "*Step", "*Boundary"] {
        assert!(!text.contains(keyword), "{keyword} in\n{text}");
    }
    let read = read_inp_str(&text, None).unwrap().mesh;
    assert_eq!(read.node_ids(), frd.mesh.node_ids());
    assert_eq!(read.element_count(), frd.mesh.element_count());
    for ((moved, start), u) in read
        .coords()
        .iter()
        .zip(frd.mesh.coords())
        .zip(&displacements)
    {
        for k in 0..3 {
            let expected = start[k] + 0.5 * f64::from(u[k]);
            assert!(
                (moved[k] - expected).abs() < 1e-6,
                "{moved:?} vs. {start:?} + {u:?}"
            );
        }
    }
    // The free end moves sideways in the mode, the clamped end stays.
    let tip = frd.mesh.node_index(41).unwrap();
    let fixed = frd.mesh.node_index(1).unwrap();
    assert!(read.coords()[tip] != frd.mesh.coords()[tip]);
    assert_eq!(read.coords()[fixed], frd.mesh.coords()[fixed]);
}

#[test]
fn a_preloaded_buckle_step_counts_its_modes_from_one() {
    let (mesh, mut model) = buckle_analysis();
    // A static step with a side load before the buckle step, taken over as preload.
    let mut preload = Step::new_static("Step-1");
    preload.boundary_conditions = model.steps[0].boundary_conditions.clone();
    preload.loads.push(tip_force());
    model.steps.insert(0, preload);
    model.steps[1].name = "Step-2".into();
    let StepKind::Buckle(settings) = &mut model.steps[1].kind else {
        unreachable!()
    };
    settings.perturbation = true;
    let Some(frd) = run_ccx("vorspannung-beulen", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let ids: Vec<_> = (frd.increments.iter())
        .map(|i| (i.step, i.increment, i.kind))
        .collect();
    use plx_results::AnalysisKind::{Buckling, Static};
    assert_eq!(ids, [(1, 1, Static), (2, 0, Buckling), (2, 1, Buckling)]);
}

#[test]
fn deactivated_items_are_left_out_as_comments() {
    let (mesh, mut model) = cantilever(tip_force());
    model.steps[0].loads.push(Load {
        name: "Empty-1".into(),
        active: false,
        // Not even an empty region stops the export of a deactivated item.
        region: Region::Nodes(Vec::new()),
        kind: LoadKind::ConcentratedForce([1.0, 0.0, 0.0]),
        amplitude: None,
        factor_amplitude: None,
    });
    model.steps[0].loads[0].active = false;
    model.steps[0].boundary_conditions[0].active = false;
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Boundary, op=New\n** Name: Fixed-1: Deactivated\n",
        "*Cload, op=New\n*Dload, op=New\n** Name: Force-1: Deactivated\n** Name: Empty-1: Deactivated\n",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    assert!(!text.contains("FIX, 1, 6, 0"), "{text}");
    assert!(!text.contains("_Force-1"), "{text}");
}

#[test]
fn a_deactivated_step_is_written_as_comments_only() {
    let (mesh, mut model) = frequency_analysis();
    model.steps[0].active = false;
    let text = write_inp(&mesh, &model, "").unwrap();
    let step = &text[text.find("** Step-1").unwrap()..text.find("** Step-2").unwrap()];
    for line in [
        "** Name: Step-1: Deactivated\n** Name: StaticStep: Deactivated\n",
        "** Name: Fixed-1: Deactivated\n",
        "** Name: Force-1: Deactivated\n",
        "** Name: NF-Output-1: Deactivated\n",
    ] {
        assert!(step.contains(line), "missing {line:?} in\n{step}");
    }
    let keywords = step
        .lines()
        .filter(|l| l.starts_with('*') && !l.starts_with("**"));
    assert_eq!(keywords.count(), 0, "{step}");
    // The node set of the load in the deactivated step is not written either.
    assert!(!text.contains("_Force-1"), "{text}");
    // CalculiX runs the remaining frequency step alone.
    let Some(frd) = run_ccx("deaktiviert", &text) else {
        return;
    };
    assert!(
        (frd.increments.iter()).all(|i| i.kind == plx_results::AnalysisKind::Frequency),
        "{:?}",
        frd.increments.len()
    );
}

/// A rectangle of quadratic quadrilaterals in the x-y plane from (`x0`, 0) to
/// (`x0 + width`, `height`), `nx` by `ny` elements, typed as shells (`S8`) so the model space
/// decides the element type. Elements count row by row from 1.
fn rectangle(x0: f64, width: f64, height: f64, nx: u32, ny: u32) -> FeMesh {
    let mut mesh = FeMesh::default();
    let columns = 2 * nx + 1;
    let id = |i: u32, j: u32| j * columns + i + 1;
    for j in 0..=2 * ny {
        for i in 0..=2 * nx {
            if i % 2 == 1 && j % 2 == 1 {
                continue;
            }
            let x = x0 + width * f64::from(i) / f64::from(2 * nx);
            let y = height * f64::from(j) / f64::from(2 * ny);
            mesh.set_node(id(i, j), [x, y, 0.0]);
        }
    }
    let mut part = plx_mesh::Part {
        name: "PLATE".into(),
        elements: Vec::new(),
    };
    for row in 0..ny {
        for column in 0..nx {
            let (i, j) = (2 * column, 2 * row);
            let element = row * nx + column + 1;
            let nodes = vec![
                id(i, j),
                id(i + 2, j),
                id(i + 2, j + 2),
                id(i, j + 2),
                id(i + 1, j),
                id(i + 2, j + 1),
                id(i + 1, j + 2),
                id(i, j + 1),
            ];
            mesh.add_element(plx_mesh::Element {
                id: element,
                type_name: "S8".into(),
                shape: plx_mesh::ElementShape::Quad8,
                nodes,
            })
            .unwrap();
            part.elements.push(element);
        }
    }
    mesh.parts.push(part);
    mesh
}

/// Nodes of the mesh where `coordinate` of the axis is `value`.
fn nodes_at(mesh: &FeMesh, axis: usize, value: f64) -> Vec<NodeId> {
    (mesh.node_ids().iter().zip(mesh.coords()))
        .filter(|(_, c)| (c[axis] - value).abs() < 1e-9)
        .map(|(&id, _)| id)
        .collect()
}

/// Edges of the rectangle's elements lying on the line where `axis` is `value`, as faces.
fn edges_at(mesh: &FeMesh, axis: usize, value: f64) -> Vec<(ElementId, u8)> {
    let mut faces = Vec::new();
    for element in mesh.elements() {
        for (k, edge) in element.shape.edges().iter().enumerate() {
            let on = (edge.corners.iter())
                .all(|&l| (mesh.node(element.nodes[l]).unwrap()[axis] - value).abs() < 1e-9);
            if on {
                faces.push((element.id, k as u8 + 1));
            }
        }
    }
    faces
}

/// A 2D model of the rectangle: steel, `thickness`, the given boundary conditions and load.
fn plane_model(
    space: ModelSpace,
    thickness: f64,
    bcs: Vec<(Vec<NodeId>, [Option<f64>; 6])>,
    load: LoadKind,
    region: Region,
) -> FeModel {
    let mut step = Step::new_static("Step-1");
    for (k, (nodes, values)) in bcs.into_iter().enumerate() {
        step.boundary_conditions.push(BoundaryCondition {
            name: format!("Displacement_Rotation-{}", k + 1),
            active: true,
            region: Region::Nodes(nodes),
            kind: BoundaryKind::Displacement(values),
            amplitude: None,
        });
    }
    step.loads.push(Load {
        name: "Load-1".into(),
        active: true,
        region,
        kind: load,
        amplitude: None,
        factor_amplitude: None,
    });
    FeModel {
        properties: plx_model::ModelProperties {
            space,
            ..Default::default()
        },
        materials: vec![Material {
            name: "Steel".into(),
            density: None,
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
            ..Default::default()
        }],
        sections: vec![Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: Region::Parts(vec!["PLATE".into()]),
            thickness,
            kind: SectionKind::Solid,
        }],
        steps: vec![step],
        ..FeModel::default()
    }
}

const FIX_X: [Option<f64>; 6] = [Some(0.0), None, None, None, None, None];
const FIX_Y: [Option<f64>; 6] = [None, Some(0.0), None, None, None, None];

#[test]
fn two_d_models_write_their_element_types_thickness_and_dofs() {
    let mesh = rectangle(0.0, 10.0, 2.0, 2, 1);
    let left = nodes_at(&mesh, 0, 0.0);
    let mut model = plane_model(
        ModelSpace::PlaneStress,
        2.5,
        vec![(left, FIX_X)],
        LoadKind::ConcentratedForce([1.0, 2.0, 3.0]),
        Region::Nodes(vec![5]),
    );
    model.steps[0].boundary_conditions[0].kind = BoundaryKind::Fixed;
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Element, Type=CPS8, Elset=PLATE\n",
        "*Solid section, Elset=Internal_Selection-1_Section-1, Material=Steel\n2.5\n",
        ", 1, 2, 0\n",
        "*Cload\nInternal_Selection-1_Load-1, 1, 1\nInternal_Selection-1_Load-1, 2, 2\n**\n",
    ] {
        assert!(text.contains(line), "{line:?} fehlt in\n{text}");
    }
    model.properties.space = ModelSpace::Axisymmetric;
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("*Element, Type=CAX8, Elset=PLATE\n"));
    assert!(text.contains("Material=Steel\n**"), "{text}");
}

#[test]
fn edge_tractions_follow_length_and_radius() {
    // One quadratic edge from x = 1 to x = 3: by length 1/6, 4/6, 1/6.
    let weights = edge_weights(&[[1.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 0.0, 0.0]], false);
    for (w, expected) in weights.iter().zip([2.0 / 6.0, 2.0 / 6.0, 8.0 / 6.0]) {
        assert!((w - expected).abs() < 1e-12, "{weights:?}");
    }
    // Times the radius: the integral of r over the edge is 4.
    let weights = edge_weights(&[[1.0, 0.0, 0.0], [3.0, 0.0, 0.0]], true);
    assert!((weights.iter().sum::<f64>() - 4.0).abs() < 1e-12);
    assert!((weights[0] - 5.0 / 3.0).abs() < 1e-12, "{weights:?}");
}

/// Plane stress: a strip pulled by a surface traction stretches by sigma L / E.
#[test]
fn calculix_stretches_a_plane_stress_strip() {
    let (length, height, thickness, force) = (10.0, 2.0, 2.0, 400.0);
    let mesh = rectangle(0.0, length, height, 5, 2);
    let model = plane_model(
        ModelSpace::PlaneStress,
        thickness,
        vec![(nodes_at(&mesh, 0, 0.0), FIX_X), (vec![1], FIX_Y)],
        LoadKind::SurfaceTraction([force, 0.0, 0.0]),
        Region::Faces(edges_at(&mesh, 0, length)),
    );
    let Some(frd) = run_ccx(
        "ebener-spannungszustand",
        &write_inp(&mesh, &model, "").unwrap(),
    ) else {
        return;
    };
    let stress = force / (height * thickness);
    let expected = stress * length / 210_000.0;
    for node in nodes_at(&mesh, 0, length) {
        let u = node_value(&frd, "DISP", "U1", node);
        assert!((u - expected).abs() < 1e-5 * expected, "{u} != {expected}");
        let s = node_value(&frd, "STRESS", "S11", node);
        assert!((s - stress).abs() < 1e-3 * stress, "{s} != {stress}");
    }
}

/// Plane strain: the same strip under pressure is stiffer by 1 - nu^2 and has a stress
/// nu sigma across its plane.
#[test]
fn calculix_stretches_a_plane_strain_strip() {
    let (length, height, pressure) = (10.0, 2.0, -100.0);
    let mesh = rectangle(0.0, length, height, 5, 2);
    let model = plane_model(
        ModelSpace::PlaneStrain,
        1.0,
        vec![(nodes_at(&mesh, 0, 0.0), FIX_X), (vec![1], FIX_Y)],
        LoadKind::Pressure(pressure),
        Region::Faces(edges_at(&mesh, 0, length)),
    );
    let Some(frd) = run_ccx(
        "ebener-verzerrungszustand",
        &write_inp(&mesh, &model, "").unwrap(),
    ) else {
        return;
    };
    let expected = (1.0 - 0.3 * 0.3) * -pressure * length / 210_000.0;
    for node in nodes_at(&mesh, 0, length) {
        let u = node_value(&frd, "DISP", "U1", node);
        assert!((u - expected).abs() < 1e-5 * expected, "{u} != {expected}");
        let szz = node_value(&frd, "STRESS", "S33", node);
        assert!((szz - 30.0).abs() < 1e-2, "{szz}");
    }
}

/// Axisymmetric: a thick-walled tube under internal pressure, held axially, widens as Lamé's
/// solution for plane strain says.
#[test]
fn calculix_widens_a_tube_under_internal_pressure() {
    let (a, b, pressure) = (10.0, 20.0, 100.0);
    let mesh = rectangle(a, b - a, 2.0, 10, 1);
    let model = plane_model(
        ModelSpace::Axisymmetric,
        1.0,
        vec![(mesh.node_ids().to_vec(), FIX_Y)],
        LoadKind::Pressure(pressure),
        Region::Faces(edges_at(&mesh, 0, a)),
    );
    let Some(frd) = run_ccx("rohr-innendruck", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let (e, nu) = (210_000.0, 0.3);
    let lame = |r: f64| {
        (1.0 + nu) / e * a * a * pressure / (b * b - a * a) * ((1.0 - 2.0 * nu) * r + b * b / r)
    };
    for r in [a, b] {
        let expected = lame(r);
        for node in nodes_at(&mesh, 0, r) {
            let u = node_value(&frd, "DISP", "U1", node);
            assert!(
                (u - expected).abs() < 2e-3 * expected,
                "r = {r}: {u} != {expected}"
            );
        }
    }
}

/// Axisymmetric: a total axial force on the end of a tube is the force on the whole
/// revolution, so the axial stress is F / (pi (b^2 - a^2)).
#[test]
fn calculix_pulls_a_tube_by_its_total_force() {
    let (a, b, height, force) = (10.0, 12.0, 5.0, 10_000.0);
    let mesh = rectangle(a, b - a, height, 2, 3);
    let model = plane_model(
        ModelSpace::Axisymmetric,
        1.0,
        vec![(nodes_at(&mesh, 1, 0.0), FIX_Y)],
        LoadKind::SurfaceTraction([0.0, force, 0.0]),
        Region::Faces(edges_at(&mesh, 1, height)),
    );
    let Some(frd) = run_ccx("rohr-zug", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let stress = force / (std::f64::consts::PI * (b * b - a * a));
    for node in mesh.node_ids() {
        let s = node_value(&frd, "STRESS", "S22", *node);
        assert!(
            (s - stress).abs() < 1e-3 * stress,
            "{node}: {s} != {stress}"
        );
    }
}

type Faces = Vec<(ElementId, u8)>;

/// Two blocks of 10 x 10 x 10 mm stacked along z, parts LOWER and UPPER, each 2 x 2 x 1
/// C3D8 elements with own nodes, the upper one `gap` above the lower one. Returns the mesh
/// with the top faces of LOWER and the bottom faces of UPPER.
fn two_blocks(gap: f64) -> (FeMesh, Faces, Faces) {
    let mut mesh = FeMesh::default();
    let mut faces = [Vec::new(), Vec::new()];
    for (block, z0) in [(0u32, 0.0), (1, 10.0 + gap)] {
        let node = |i: u32, j: u32, k: u32| 100 * block + 1 + i + 3 * j + 9 * k;
        for k in 0..2 {
            for j in 0..3 {
                for i in 0..3 {
                    let coords = [
                        5.0 * f64::from(i),
                        5.0 * f64::from(j),
                        z0 + 10.0 * f64::from(k),
                    ];
                    mesh.set_node(node(i, j, k), coords);
                }
            }
        }
        let mut elements = Vec::new();
        for j in 0..2 {
            for i in 0..2 {
                let id = 100 * block + 1 + i + 2 * j;
                let mut nodes = Vec::new();
                for k in 0..2 {
                    nodes.extend([node(i, j, k), node(i + 1, j, k), node(i + 1, j + 1, k)]);
                    nodes.push(node(i, j + 1, k));
                }
                mesh.add_element(plx_mesh::Element {
                    id,
                    type_name: "C3D8".into(),
                    shape: plx_mesh::ElementShape::Hex8,
                    nodes,
                })
                .unwrap();
                elements.push(id);
                // Top face S2 of the lower block, bottom face S1 of the upper one.
                faces[block as usize].push((id, if block == 0 { 2 } else { 1 }));
            }
        }
        let name = if block == 0 { "LOWER" } else { "UPPER" };
        mesh.parts.push(plx_mesh::Part {
            name: name.into(),
            elements,
        });
    }
    let [lower, upper] = faces;
    (mesh, lower, upper)
}

/// Steel on both blocks, the lower one held at its bottom and 10 MPa pressing on the top of
/// the upper one.
fn stacked_blocks(mesh: &FeMesh) -> FeModel {
    let mut model = FeModel::default();
    let bottom: Vec<NodeId> = (mesh.node_ids().iter().zip(mesh.coords()))
        .filter(|(_, c)| c[2] == 0.0)
        .map(|(&id, _)| id)
        .collect();
    let mut step = Step::new_static("Step-1");
    step.boundary_conditions.push(BoundaryCondition {
        name: "Fixed-1".into(),
        active: true,
        region: Region::Nodes(bottom),
        kind: BoundaryKind::Fixed,
        amplitude: None,
    });
    step.loads.push(Load {
        name: "Pressure-1".into(),
        active: true,
        region: Region::Faces((101..=104).map(|e| (e, 2)).collect()),
        kind: LoadKind::Pressure(10.0),
        amplitude: None,
        factor_amplitude: None,
    });
    model.materials.push(Material {
        name: "Steel".into(),
        density: None,
        elastic: Some(Elastic {
            young: 210_000.0,
            poisson: 0.3,
        }),
        ..Default::default()
    });
    model.sections.push(Section {
        name: "Section-1".into(),
        material: "Steel".into(),
        region: Region::Parts(vec!["LOWER".into(), "UPPER".into()]),
        thickness: 1.0,
        kind: SectionKind::Solid,
    });
    model.steps.push(step);
    model
}

#[test]
fn a_tie_writes_its_surfaces_slave_first() {
    let (mesh, lower, upper) = two_blocks(0.0);
    let mut model = stacked_blocks(&mesh);
    let mut tie = plx_model::Tie::new("Tie-1");
    tie.master = Region::Faces(lower);
    tie.slave = Region::Faces(upper);
    tie.position_tolerance = Some(0.5);
    model.constraints.push(Constraint::Tie(tie));
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Surface, Name=Internal_Selection-1_Tie-1_Master, Type=Element\n",
        "*Elset, Elset=Internal-1_Internal_Selection-1_Tie-1_Slave_S1\n101, 102, 103, 104\n",
        "*Tie, Name=Tie-1, Position tolerance=0.5\n\
         Internal_Selection-1_Tie-1_Slave, Internal_Selection-1_Tie-1_Master\n",
    ] {
        assert!(text.contains(line), "{line:?} fehlt in\n{text}");
    }
    let constraints = top_title(&model_keywords(&mesh, &model, "").unwrap(), "Constraints");
    assert_eq!(constraints, 11);
}

#[test]
fn surface_interactions_and_contact_pairs_are_written_like_prepomax() {
    let (mesh, lower, upper) = two_blocks(0.0);
    let mut model = stacked_blocks(&mesh);
    model.surface_interactions.push(SurfaceInteraction {
        name: "Surface_Interaction-1".into(),
        properties: vec![
            InteractionProperty::SurfaceBehavior(SurfaceBehavior::Linear {
                k: 1e7,
                sigma_inf: 2.86,
                c0: None,
            }),
            InteractionProperty::Friction(plx_model::Friction {
                coefficient: 0.2,
                stick_slope: Some(5000.0),
            }),
            InteractionProperty::GapConductance(GapConductance::Constant(3.0)),
        ],
    });
    let mut pair = ContactPair::new("Contact_Pair-1", "Surface_Interaction-1");
    pair.master = Region::Faces(lower);
    pair.slave = Region::Faces(upper);
    pair.method = ContactMethod::NodeToSurface;
    pair.small_sliding = true;
    pair.adjust = true;
    model.contact_pairs.push(pair.clone());
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Surface interaction, Name=Surface_Interaction-1\n\
         *Surface behavior, Pressure-overclosure=Linear\n10000000, 2.86\n\
         *Friction\n0.2, 5000\n*Gap conductance\n3\n",
        "*Contact pair, Interaction=Surface_Interaction-1, Type=Node to surface, \
         Small sliding, Adjust=0\n\
         Internal_Selection-1_Contact_Pair-1_Slave, Internal_Selection-1_Contact_Pair-1_Master\n",
    ] {
        assert!(text.contains(line), "{line:?} fehlt in\n{text}");
    }
    // Deactivated, the pair keeps its place as a comment and gets no surfaces.
    model.contact_pairs[0].active = false;
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("** Name: Contact_Pair-1: Deactivated\n"));
    assert!(!text.contains("Contact_Pair-1_Master"));
    // A pair needs its interaction.
    model.contact_pairs[0] = ContactPair::new("Contact_Pair-1", "Gone");
    assert!(matches!(
        write_inp(&mesh, &model, ""),
        Err(WriteError::UnknownInteraction { .. })
    ));
}

/// Sum of a component of a nodal field over nodes.
fn node_sum(frd: &FrdImport, field: &str, component: &str, nodes: &[NodeId]) -> f64 {
    nodes
        .iter()
        .map(|&n| node_value(frd, field, component, n))
        .sum()
}

#[test]
fn calculix_carries_the_load_across_a_tie() {
    // A small gap within the position tolerance; adjust closes it.
    let (mesh, lower, upper) = two_blocks(0.01);
    let mut model = stacked_blocks(&mesh);
    let mut tie = plx_model::Tie::new("Tie-1");
    tie.master = Region::Faces(lower);
    tie.slave = Region::Faces(upper);
    tie.position_tolerance = Some(0.05);
    model.constraints.push(Constraint::Tie(tie));
    let Some(frd) = run_ccx("tie", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let bottom: Vec<NodeId> = (1..=9).collect();
    let reaction = node_sum(&frd, "FORC", "F3", &bottom);
    // 10 MPa on 10 x 10 mm.
    assert!((reaction - 1000.0).abs() < 1.0, "{reaction}");
    // The tied faces move together: middle node of the lower top and the upper bottom.
    let below = node_value(&frd, "DISP", "U3", 14);
    let above = node_value(&frd, "DISP", "U3", 105);
    assert!(
        (below - above).abs() < 1e-3 * below.abs(),
        "{below} != {above}"
    );
}

#[test]
fn calculix_carries_the_load_across_a_contact() {
    // PrePoMax's default hard contact and a linear spring.
    let linear = SurfaceBehavior::Linear {
        k: 1e7,
        sigma_inf: 2.86,
        c0: None,
    };
    for (name, behavior) in [("hart", SurfaceBehavior::Hard), ("linear", linear)] {
        let (mesh, lower, upper) = two_blocks(0.0);
        let mut model = stacked_blocks(&mesh);
        // Held sideways at its top, the upper block rests on the lower one only by contact.
        let top: Vec<NodeId> = (119..=127).collect();
        model.steps[0].boundary_conditions.push(BoundaryCondition {
            name: "Side-1".into(),
            active: true,
            region: Region::Nodes(top),
            kind: BoundaryKind::Displacement([Some(0.0), Some(0.0), None, None, None, None]),
            amplitude: None,
        });
        model.surface_interactions.push(SurfaceInteraction {
            name: "Surface_Interaction-1".into(),
            properties: vec![InteractionProperty::SurfaceBehavior(behavior)],
        });
        let mut pair = ContactPair::new("Contact_Pair-1", "Surface_Interaction-1");
        pair.master = Region::Faces(lower);
        pair.slave = Region::Faces(upper);
        model.contact_pairs.push(pair);
        let text = write_inp(&mesh, &model, "").unwrap();
        let Some(frd) = run_ccx(&format!("kontakt-{name}"), &text) else {
            return;
        };
        let bottom: Vec<NodeId> = (1..=9).collect();
        let reaction = node_sum(&frd, "FORC", "F3", &bottom);
        assert!((reaction - 1000.0).abs() < 10.0, "{name}: {reaction}");
    }
}

/// Plane strain: a strip of 4 x 1 resting on another of the same size only by contact on
/// their edges carries the pressure on its top into the supports of the lower one.
#[test]
fn calculix_carries_the_load_across_a_contact_in_2d() {
    let (width, pressure) = (4.0, 50.0);
    let mut mesh = rectangle(0.0, width, 1.0, 4, 1);
    // The upper strip: the same rectangle with own nodes and elements, one higher.
    let upper = rectangle(0.0, width, 1.0, 4, 1);
    for (&id, c) in upper.node_ids().iter().zip(upper.coords()) {
        mesh.set_node(id + 1000, [c[0], c[1] + 1.0, c[2]]);
    }
    for element in upper.elements() {
        let mut element = element.clone();
        element.id += 100;
        element.nodes.iter_mut().for_each(|n| *n += 1000);
        mesh.add_element(element).unwrap();
    }
    mesh.parts.push(plx_mesh::Part {
        name: "UPPER".into(),
        elements: (101..=104).collect(),
    });
    let on_line = |y: f64, upper: bool| -> Vec<(ElementId, u8)> {
        (edges_at(&mesh, 1, y).into_iter())
            .filter(|&(element, _)| (element > 100) == upper)
            .collect()
    };
    let (lower_top, upper_bottom, upper_top) =
        (on_line(1.0, false), on_line(1.0, true), on_line(2.0, true));
    let bottom = nodes_at(&mesh, 1, 0.0);
    // Held sideways at its top, the upper strip rests on the lower one only by contact.
    let top: Vec<NodeId> = (nodes_at(&mesh, 1, 2.0).into_iter())
        .filter(|&n| n > 1000)
        .collect();
    let mut model = plane_model(
        ModelSpace::PlaneStrain,
        1.0,
        vec![(bottom.clone(), FIX_Y), (vec![1], FIX_X), (top, FIX_X)],
        LoadKind::Pressure(pressure),
        Region::Faces(upper_top),
    );
    model.sections[0].region = Region::Parts(vec!["PLATE".into(), "UPPER".into()]);
    model.surface_interactions.push(SurfaceInteraction {
        name: "Surface_Interaction-1".into(),
        properties: vec![InteractionProperty::SurfaceBehavior(SurfaceBehavior::Hard)],
    });
    let mut pair = ContactPair::new("Contact_Pair-1", "Surface_Interaction-1");
    pair.master = Region::Faces(lower_top);
    pair.slave = Region::Faces(upper_bottom);
    model.contact_pairs.push(pair);
    let Some(frd) = run_ccx("kontakt-2d", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let reaction = node_sum(&frd, "FORC", "F2", &bottom);
    let expected = pressure * width;
    assert!(
        (reaction - expected).abs() < 0.01 * expected,
        "{reaction} != {expected}"
    );
}

/// Two steel blocks of 1 x 1 x 1 hexahedra, `a` from z = 0 to 1 and `b` from z = 1.1 to 2.1,
/// with `n` elements per edge each. Returns the mesh and the bottom and top faces of
/// each block.
struct Blocks {
    mesh: FeMesh,
    a_bottom: Vec<(ElementId, u8)>,
    a_top: Vec<(ElementId, u8)>,
    b_bottom: Vec<(ElementId, u8)>,
    b_top: Vec<(ElementId, u8)>,
}

fn blocks(na: u32, nb: u32) -> Blocks {
    use plx_mesh::{Element, ElementShape, Part};
    let mut mesh = FeMesh::default();
    let mut next_node = 1;
    let mut next_element = 1;
    let mut block = |mesh: &mut FeMesh, n: u32, z0: f64, name: &str| {
        let mut ids = BTreeMap::new();
        for k in 0..=n {
            for j in 0..=n {
                for i in 0..=n {
                    let h = 1.0 / f64::from(n);
                    mesh.set_node(
                        next_node,
                        [f64::from(i) * h, f64::from(j) * h, z0 + f64::from(k) * h],
                    );
                    ids.insert((i, j, k), next_node);
                    next_node += 1;
                }
            }
        }
        let (mut bottom, mut top, mut elements) = (Vec::new(), Vec::new(), Vec::new());
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    let corner = |di, dj, dk| ids[&(i + di, j + dj, k + dk)];
                    let nodes = vec![
                        corner(0, 0, 0),
                        corner(1, 0, 0),
                        corner(1, 1, 0),
                        corner(0, 1, 0),
                        corner(0, 0, 1),
                        corner(1, 0, 1),
                        corner(1, 1, 1),
                        corner(0, 1, 1),
                    ];
                    mesh.add_element(Element {
                        id: next_element,
                        type_name: "C3D8".into(),
                        shape: ElementShape::Hex8,
                        nodes,
                    })
                    .unwrap();
                    if k == 0 {
                        bottom.push((next_element, 1));
                    }
                    if k == n - 1 {
                        top.push((next_element, 2));
                    }
                    elements.push(next_element);
                    next_element += 1;
                }
            }
        }
        mesh.parts.push(Part {
            name: name.into(),
            elements,
        });
        (bottom, top)
    };
    let (a_bottom, a_top) = block(&mut mesh, na, 0.0, "A");
    let (b_bottom, b_top) = block(&mut mesh, nb, 1.1, "B");
    Blocks {
        mesh,
        a_bottom,
        a_top,
        b_bottom,
        b_top,
    }
}

/// Steel on both blocks and one static step with a surface traction on top of block `b`,
/// block `a` held at its bottom.
fn blocks_model(
    blocks: &Blocks,
    constraints: Vec<plx_model::Constraint>,
    force: [f64; 3],
) -> FeModel {
    let mut step = Step::new_static("Step-1");
    step.boundary_conditions.push(BoundaryCondition {
        name: "Fixed-1".into(),
        active: true,
        region: Region::Faces(blocks.a_bottom.clone()),
        kind: BoundaryKind::Fixed,
        amplitude: None,
    });
    step.loads.push(Load {
        name: "Surface_Traction-1".into(),
        active: true,
        region: Region::Faces(blocks.b_top.clone()),
        kind: LoadKind::SurfaceTraction(force),
        amplitude: None,
        factor_amplitude: None,
    });
    FeModel {
        materials: vec![Material {
            name: "Steel".into(),
            density: None,
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
            ..Default::default()
        }],
        sections: vec![Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: Region::Parts(vec!["A".into(), "B".into()]),
            thickness: 1.0,
            kind: SectionKind::Solid,
        }],
        constraints,
        steps: vec![step],
        ..FeModel::default()
    }
}

/// Mean displacement component of the top nodes of block `b`.
fn mean_top(blocks: &Blocks, frd: &FrdImport, component: &str) -> f64 {
    let nodes = Region::Faces(blocks.b_top.clone()).nodes(&blocks.mesh);
    let sum: f64 = nodes
        .iter()
        .map(|&n| node_value(frd, "DISP", component, n))
        .sum();
    sum / nodes.len() as f64
}

#[test]
fn calculix_carries_a_block_on_springs_between_two_surfaces() {
    use plx_model::SurfaceToSurfaceSpring;
    // Different meshes on the two sides: the springs end on points between nodes.
    let blocks = blocks(2, 3);
    let spring = Constraint::SurfaceToSurfaceSpring(SurfaceToSurfaceSpring {
        name: "Bearing-1".into(),
        active: true,
        master: Region::Faces(blocks.a_top.clone()),
        slave: Region::Faces(blocks.b_bottom.clone()),
        stiffness: [1000.0, 1000.0, 2000.0],
        per_area: false,
    });
    let model = blocks_model(&blocks, vec![spring], [0.0, 0.0, 100.0]);
    let text = write_inp(&blocks.mesh, &model, "").unwrap();
    assert!(
        text.contains("*Element, Type=SPRING2, Elset=Bearing-1_All\n"),
        "{text}"
    );
    assert!(text.contains("*Equation\n"));
    let Some(frd) = run_ccx("feder-flaeche", &text) else {
        return;
    };
    // 100 N on 2000 N/mm, the steel itself barely stretches.
    let lift = mean_top(&blocks, &frd, "U3");
    assert!((lift - 0.05).abs() < 0.001, "{lift}");
}

#[test]
fn calculix_carries_a_block_on_surface_and_point_springs() {
    use plx_model::{PointSpring, SurfaceSpring};
    let blocks = blocks(2, 2);
    let surface = Constraint::SurfaceSpring(SurfaceSpring {
        name: "Surface_Spring-1".into(),
        active: true,
        region: Region::Faces(blocks.b_bottom.clone()),
        stiffness: [1000.0, 1000.0, 1000.0],
        per_area: false,
    });
    // Nine nodes on the bottom of block b, 100 N/mm each in z.
    let point = Constraint::PointSpring(PointSpring {
        name: "Point_Spring-1".into(),
        active: true,
        region: Region::Faces(blocks.b_bottom.clone()),
        stiffness: [0.0, 0.0, 100.0],
    });
    let model = blocks_model(&blocks, vec![surface, point], [0.0, 0.0, 95.0]);
    let text = write_inp(&blocks.mesh, &model, "").unwrap();
    assert!(text.contains("*Element, Type=SPRING1, Elset=Point_Spring-1_All\n"));
    assert!(
        text.contains("*Spring, Elset=Point_Spring-1_DOF_3\n3\n100.\n"),
        "{text}"
    );
    let Some(frd) = run_ccx("federn", &text) else {
        return;
    };
    let lift = mean_top(&blocks, &frd, "U3");
    assert!((lift - 0.05).abs() < 0.001, "{lift}");
}

#[test]
fn calculix_compression_only_support_takes_only_pressure() {
    use plx_model::{CompressionOnly, SurfaceSpring};
    let blocks = blocks(2, 2);
    let support = |force: [f64; 3]| {
        let springs = Constraint::SurfaceSpring(SurfaceSpring {
            name: "Surface_Spring-1".into(),
            active: true,
            region: Region::Faces(blocks.b_bottom.clone()),
            stiffness: [1000.0, 1000.0, 1000.0],
            per_area: false,
        });
        let gaps = Constraint::CompressionOnly(CompressionOnly {
            name: "Compression_Only-1".into(),
            active: true,
            region: Region::Faces(blocks.b_bottom.clone()),
            clearance: 0.0,
            spring_stiffness: None,
            tensile_force: None,
            offset: 0.0,
            nonlinear: true,
        });
        blocks_model(&blocks, vec![springs, gaps], force)
    };
    let pulled = write_inp(&blocks.mesh, &support([0.0, 0.0, 100.0]), "").unwrap();
    assert!(pulled.contains("*Element, Type=GAPUNI\n"));
    assert!(pulled.contains("*Plastic\n"));
    let Some(frd) = run_ccx("druck-nur-zug", &pulled) else {
        return;
    };
    // Pulled away, only the springs hold the block.
    let lift = mean_top(&blocks, &frd, "U3");
    assert!((lift - 0.1).abs() < 0.002, "{lift}");
    let pushed = write_inp(&blocks.mesh, &support([0.0, 0.0, -100.0]), "").unwrap();
    let frd = run_ccx("druck-nur-druck", &pushed).unwrap();
    // Pushed down, the gaps carry the load.
    let sink = mean_top(&blocks, &frd, "U3");
    assert!(sink.abs() < 0.002, "{sink}");
}

/// A straight chain of `n` line elements along `axis` from 0 to `length`, numbered from 1;
/// quadratic ones have a midside node.
fn line_chain(n: u32, axis: usize, length: f64, quadratic: bool) -> FeMesh {
    use plx_mesh::{Element, ElementShape, Part};
    let mut mesh = FeMesh::default();
    let per_element = if quadratic { 2 } else { 1 };
    let count = n * per_element;
    for i in 0..=count {
        let mut coords = [0.0; 3];
        coords[axis] = length * f64::from(i) / f64::from(count);
        mesh.set_node(i + 1, coords);
    }
    let mut elements = Vec::new();
    for e in 0..n {
        let first = e * per_element + 1;
        let (shape, type_name, nodes) = if quadratic {
            (
                ElementShape::Line3,
                "B32",
                vec![first, first + 1, first + 2],
            )
        } else {
            (ElementShape::Line2, "B31", vec![first, first + 1])
        };
        mesh.add_element(Element {
            id: e + 1,
            type_name: type_name.into(),
            shape,
            nodes,
        })
        .unwrap();
        elements.push(e + 1);
    }
    mesh.parts.push(Part {
        name: "BEAM".into(),
        elements,
    });
    mesh
}

/// Steel on part BEAM with the given section, node 1 fixed and `load` at the last node.
fn line_model(mesh: &FeMesh, kind: SectionKind, load: LoadKind) -> FeModel {
    let tip = *mesh.node_ids().last().unwrap();
    let mut step = Step::new_static("Step-1");
    step.boundary_conditions.push(BoundaryCondition {
        name: "Fixed-1".into(),
        active: true,
        region: Region::Nodes(vec![1]),
        kind: BoundaryKind::Fixed,
        amplitude: None,
    });
    step.loads.push(Load {
        name: "Force-1".into(),
        active: true,
        region: Region::Nodes(vec![tip]),
        kind: load,
        amplitude: None,
        factor_amplitude: None,
    });
    FeModel {
        materials: vec![Material {
            name: "Steel".into(),
            density: None,
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
            conductivity: None,
            specific_heat: None,
            expansion: None,
        }],
        sections: vec![Section {
            name: "Beam-1".into(),
            material: "Steel".into(),
            region: Region::Parts(vec!["BEAM".into()]),
            thickness: 1.0,
            kind,
        }],
        steps: vec![step],
        ..FeModel::default()
    }
}

fn rect_beam(orientation: BeamOrientation) -> SectionKind {
    SectionKind::Beam(BeamSection {
        profile: BeamProfile::Rect { a: 10.0, b: 5.0 },
        orientation,
        offset: [0.0, 0.0],
    })
}

#[test]
fn beam_sections_type_their_elements_and_write_the_normal() {
    let mesh = line_chain(2, 0, 100.0, true);
    let model = line_model(
        &mesh,
        rect_beam(BeamOrientation::Direction([0.0, 1.0, 0.0])),
        LoadKind::ConcentratedForce([0.0, -100.0, 0.0]),
    );
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Element, Type=B32, Elset=BEAM\n1, 1, 2, 3\n2, 3, 4, 5\n",
        "*Beam section, Elset=Internal_Selection-1_Beam-1, Material=Steel, Section=RECT\n\
         10, 5\n0, 1, 0\n",
        "*Boundary\nInternal_Selection-1_Fixed-1, 1, 6, 0\n",
    ] {
        assert!(text.contains(line), "{line} fehlt in\n{text}");
    }
    // Pipes and boxes need B32R and carry the offsets.
    let mut model = model;
    model.sections[0].kind = SectionKind::Beam(BeamSection {
        profile: BeamProfile::Pipe {
            radius: 5.0,
            thickness: 1.0,
        },
        orientation: BeamOrientation::Automatic,
        offset: [0.5, 0.0],
    });
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("*Element, Type=B32R, Elset=BEAM\n"), "{text}");
    // CalculiX 2.21 reads the set name of a pipe or box 20 characters wide, so it is short.
    assert!(text.contains("*Elset, Elset=Beam-1\n1, 2\n"), "{text}");
    assert!(
        text.contains(
            "*Beam section, Elset=Beam-1, Material=Steel, Section=PIPE, Offset1=0.5\n\
             5, 1\n0, 0, 1\n"
        ),
        "{text}"
    );
    // A circle is written by its diameter, as CalculiX reads it.
    model.sections[0].kind = SectionKind::Beam(BeamSection {
        profile: BeamProfile::Circ { radius: 2.0 },
        ..BeamSection::DEFAULT
    });
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("Section=CIRC\n4, 4\n0, 0, 1\n"), "{text}");
}

#[test]
fn automatic_normals_split_a_frame_into_groups() {
    use plx_mesh::{Element, ElementShape};
    // An L: two elements along x, then two up along z.
    let mut mesh = line_chain(2, 0, 100.0, false);
    mesh.set_node(4, [100.0, 0.0, 50.0]);
    mesh.set_node(5, [100.0, 0.0, 100.0]);
    for (id, nodes) in [(3, vec![3, 4]), (4, vec![4, 5])] {
        mesh.add_element(Element {
            id,
            type_name: "B31".into(),
            shape: ElementShape::Line2,
            nodes,
        })
        .unwrap();
        mesh.parts[0].elements.push(id);
    }
    let model = line_model(
        &mesh,
        rect_beam(BeamOrientation::Automatic),
        LoadKind::ConcentratedForce([100.0, 0.0, 0.0]),
    );
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Elset, Elset=Internal_Selection-1_Beam-1\n1, 2\n",
        "*Elset, Elset=Internal_Selection-2_Beam-1\n3, 4\n",
        "*Beam section, Elset=Internal_Selection-1_Beam-1, Material=Steel, Section=RECT\n\
         10, 5\n0, 0, 1\n\
         *Beam section, Elset=Internal_Selection-2_Beam-1, Material=Steel, Section=RECT\n\
         10, 5\n1, 0, 0\n",
    ] {
        assert!(text.contains(line), "{line} fehlt in\n{text}");
    }
    // A given normal parallel to the columns is refused.
    let mut model = model;
    model.sections[0].kind = rect_beam(BeamOrientation::Direction([0.0, 0.0, 1.0]));
    let error = write_inp(&mesh, &model, "").unwrap_err();
    assert_eq!(
        error,
        WriteError::InvalidSection {
            item: "Beam-1".into(),
            reason: "Die Normale ist parallel zur Achse von Element 3".into()
        }
    );
}

#[test]
fn sections_must_fit_their_elements() {
    let mesh = line_chain(2, 0, 100.0, false);
    let mut model = line_model(
        &mesh,
        SectionKind::Solid,
        LoadKind::ConcentratedForce([0.0, -100.0, 0.0]),
    );
    let error = write_inp(&mesh, &model, "").unwrap_err();
    assert!(
        matches!(&error, WriteError::InvalidSection { item, reason }
            if item == "Beam-1" && reason.contains("Linienelement")),
        "{error}"
    );
    model.sections[0].kind = SectionKind::Beam(BeamSection {
        profile: BeamProfile::ALL[3],
        ..BeamSection::DEFAULT
    });
    let error = write_inp(&mesh, &model, "").unwrap_err();
    assert!(
        matches!(&error, WriteError::InvalidSection { reason, .. } if reason.contains("B32R")),
        "{error}"
    );
    let (solid_mesh, mut solid_model) = cantilever(tip_force());
    solid_model.sections[0].kind = SectionKind::Truss { area: 1.0 };
    let error = write_inp(&solid_mesh, &solid_model, "").unwrap_err();
    assert!(
        matches!(&error, WriteError::InvalidSection { reason, .. }
            if reason.contains("kein Linienelement")),
        "{error}"
    );
}

#[test]
fn trusses_are_written_as_t3d2_with_translations_only() {
    let mesh = line_chain(2, 0, 100.0, true);
    let model = line_model(
        &mesh,
        SectionKind::Truss { area: 50.0 },
        LoadKind::ConcentratedForce([1000.0, 0.0, 0.0]),
    );
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        // The midside nodes of the quadratic lines are dropped.
        "*Element, Type=T3D2, Elset=BEAM\n1, 1, 3\n2, 3, 5\n",
        "*Solid section, Elset=Internal_Selection-1_Beam-1, Material=Steel\n50\n",
        "*Boundary\nInternal_Selection-1_Fixed-1, 1, 3, 0\n",
    ] {
        assert!(text.contains(line), "{line} fehlt in\n{text}");
    }
}

/// Two chains of `n` line elements each along x, part BEAM from 0 to 50 with nodes from 1
/// and part BEAM2 from 50 to 100 with nodes from 101, meeting at x = 50 with a node each.
fn two_chains(n: u32, quadratic: bool) -> FeMesh {
    use plx_mesh::{Element, ElementShape, Part};
    let mut mesh = line_chain(n, 0, 50.0, quadratic);
    let per_element = if quadratic { 2 } else { 1 };
    let count = n * per_element;
    for i in 0..=count {
        let x = 50.0 + 50.0 * f64::from(i) / f64::from(count);
        mesh.set_node(101 + i, [x, 0.0, 0.0]);
    }
    let mut elements = Vec::new();
    for e in 0..n {
        let first = 101 + e * per_element;
        let (shape, type_name, nodes) = if quadratic {
            (
                ElementShape::Line3,
                "B32",
                vec![first, first + 1, first + 2],
            )
        } else {
            (ElementShape::Line2, "B31", vec![first, first + 1])
        };
        mesh.add_element(Element {
            id: 101 + e,
            type_name: type_name.into(),
            shape,
            nodes,
        })
        .unwrap();
        elements.push(101 + e);
    }
    mesh.parts.push(Part {
        name: "BEAM2".into(),
        elements,
    });
    mesh
}

/// The two chains with their section on both parts and a node tie at x = 50.
fn tied_chains(n: u32, quadratic: bool, kind: SectionKind, load: LoadKind) -> (FeMesh, FeModel) {
    let mesh = two_chains(n, quadratic);
    let mut model = line_model(&mesh, kind, load);
    model.sections[0].region = Region::Parts(vec!["BEAM".into(), "BEAM2".into()]);
    let middle = n * if quadratic { 2 } else { 1 } + 1;
    model.node_ties.push(NodeTie {
        region: Region::Nodes(vec![middle, 101]),
        ..NodeTie::new("Node_Tie-1")
    });
    (mesh, model)
}

#[test]
fn node_ties_merge_the_nodes_or_hinge_beams_with_equations() {
    let (mesh, mut model) = tied_chains(
        2,
        false,
        rect_beam(BeamOrientation::Automatic),
        LoadKind::ConcentratedForce([0.0, -100.0, 0.0]),
    );
    // Tied rigidly, the second beam starts at the first one's end node; the node set of a
    // boundary condition on the merged node names the node that stays.
    model.steps[0].boundary_conditions.push(BoundaryCondition {
        name: "Held".into(),
        active: true,
        amplitude: None,
        region: Region::Nodes(vec![101]),
        kind: BoundaryKind::Displacement([None, None, Some(0.0), None, None, None]),
    });
    let text = write_inp(&mesh, &model, "").unwrap();
    for line in [
        "*Element, Type=B31, Elset=BEAM2\n101, 3, 102\n102, 102, 103\n",
        "** Name: Node_Tie-1\n** Knoten 3 (zusammengelegt)\n",
        "*Nset, Nset=Internal_Selection-1_Held\n3\n",
    ] {
        assert!(text.contains(line), "{line} fehlt in\n{text}");
    }
    assert!(!text.contains("*Node\n101,") && !text.contains("\n101, 50, 0, 0\n"));
    assert!(!text.contains("*Equation"));
    // A hinge between beams keeps the nodes and ties their translations.
    model.node_ties[0].rotations = false;
    let text = write_inp(&mesh, &model, "").unwrap();
    let expected: String = (1..=3)
        .map(|dof| format!("*Equation\n2\n101, {dof}, 1, 3, {dof}, -1\n"))
        .collect();
    assert!(
        text.contains(&format!("** Name: Node_Tie-1\n{expected}")) && !text.contains("101, 4, 1"),
        "{text}"
    );
    assert!(text.contains("*Element, Type=B31, Elset=BEAM2\n101, 101, 102\n"));
    // Trusses have no rotations: a hinge is the same as a rigid tie, one node.
    model.sections[0].kind = SectionKind::Truss { area: 50.0 };
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("*Element, Type=T3D2, Elset=BEAM2\n101, 3, 102\n"));
    assert!(!text.contains("*Equation"));
    // A deactivated tie is a comment, an empty one an error.
    model.node_ties[0] = NodeTie {
        active: false,
        ..NodeTie::new("Node_Tie-1")
    };
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("** Name: Node_Tie-1: Deactivated\n"));
    assert!(text.contains("*Element, Type=T3D2, Elset=BEAM2\n101, 101, 102\n"));
    model.node_ties[0] = NodeTie::new("Node_Tie-1");
    assert!(matches!(
        write_inp(&mesh, &model, ""),
        Err(WriteError::EmptyRegion { .. })
    ));
}

/// Two beams of five elements each tied rigidly at their ends bend like one cantilever.
#[test]
fn calculix_bends_two_beams_tied_at_a_node_like_one() {
    let expected = cantilever_deflection(100.0, 100.0, 10.0 * 125.0 / 12.0);
    let (mesh, model) = tied_chains(
        5,
        false,
        rect_beam(BeamOrientation::Automatic),
        LoadKind::ConcentratedForce([0.0, -100.0, 0.0]),
    );
    let Some(frd) = run_ccx("balken_node_tie", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let deflection = -min_u2(&frd);
    assert!(
        (deflection - expected).abs() < 0.005 * expected,
        "{deflection} statt {expected}"
    );
}

/// Two beams clamped at their far ends and hinged together at x = 50 move together there,
/// but bend less stiffly than tied rigidly: a hinge carries no moment.
#[test]
fn calculix_hinges_two_beams_at_a_node() {
    let deflection = |rotations: bool| {
        let (mesh, mut model) = tied_chains(
            5,
            false,
            rect_beam(BeamOrientation::Automatic),
            LoadKind::ConcentratedForce([0.0, -100.0, 0.0]),
        );
        model.node_ties[0].rotations = rotations;
        let step = &mut model.steps[0];
        step.boundary_conditions[0].region = Region::Nodes(vec![1, 106]);
        step.loads[0].region = Region::Nodes(vec![3]);
        let name = if rotations {
            "balken_starr"
        } else {
            "balken_gelenk"
        };
        let frd = run_ccx(name, &write_inp(&mesh, &model, "").unwrap())?;
        // The expanded nodes at x = 50 of both beams move alike; tied rigidly, the beams
        // share the node there and CalculiX expands it once.
        let at_joint: Vec<f64> = (frd.mesh.node_ids().iter().zip(frd.mesh.coords()))
            .filter(|(_, c)| (c[0] - 50.0).abs() < 1e-6)
            .map(|(&n, _)| node_value(&frd, "DISP", "U2", n))
            .collect();
        assert_eq!(
            at_joint.len(),
            if rotations { 4 } else { 8 },
            "{at_joint:?}"
        );
        let mean = at_joint.iter().sum::<f64>() / at_joint.len() as f64;
        assert!(
            at_joint
                .iter()
                .all(|u| (u - mean).abs() < 1e-3 * mean.abs()),
            "{at_joint:?}"
        );
        Some(-mean)
    };
    let (Some(hinged), Some(rigid)) = (deflection(false), deflection(true)) else {
        return;
    };
    assert!(hinged > 1.2 * rigid && rigid > 0.0, "{hinged} vs {rigid}");
}

/// Two trusses tied at their ends stretch like one bar of their whole length.
#[test]
fn calculix_stretches_two_trusses_tied_at_a_node_like_one() {
    let (mesh, mut model) = tied_chains(
        5,
        true,
        SectionKind::Truss { area: 50.0 },
        LoadKind::ConcentratedForce([1000.0, 0.0, 0.0]),
    );
    // A straight chain of trusses is a mechanism sideways; hold the nodes in the axis.
    let nodes: Vec<NodeId> = (mesh.node_ids().iter().copied())
        .filter(|&n| n % 2 == 1)
        .collect();
    model.steps[0].boundary_conditions.push(BoundaryCondition {
        name: "Sideways".into(),
        active: true,
        amplitude: None,
        region: Region::Nodes(nodes),
        kind: BoundaryKind::Displacement([None, Some(0.0), Some(0.0), None, None, None]),
    });
    let Some(frd) = run_ccx("stab_node_tie", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let expected = 1000.0 * 100.0 / (210_000.0 * 50.0);
    let stretch = node_value(&frd, "DISP", "U1", 111);
    assert!(
        (stretch - expected).abs() < 1e-6 * expected,
        "{stretch} statt {expected}"
    );
}

/// Deflection of a cantilever of length `l` under a tip force `f` after beam theory.
fn cantilever_deflection(f: f64, l: f64, inertia: f64) -> f64 {
    f * l.powi(3) / (3.0 * 210_000.0 * inertia)
}

/// The most negative `U2` in the results: the tip of a beam bent down.
fn min_u2(frd: &FrdImport) -> f64 {
    let increment = frd.increments.last().unwrap();
    let u2 = increment.field("DISP").unwrap().component("U2").unwrap();
    f64::from(u2.values.iter().copied().fold(f32::MAX, f32::min))
}

#[test]
fn calculix_bends_a_rectangular_beam_like_beam_theory() {
    // Rectangle 10 high (1-direction z) and 5 wide, loaded in y: I = 10 * 5^3 / 12.
    let expected = cantilever_deflection(100.0, 100.0, 10.0 * 125.0 / 12.0);
    for (quadratic, tolerance) in [(false, 0.005), (true, 0.03)] {
        let mesh = line_chain(10, 0, 100.0, quadratic);
        let model = line_model(
            &mesh,
            rect_beam(BeamOrientation::Automatic),
            LoadKind::ConcentratedForce([0.0, -100.0, 0.0]),
        );
        let name = if quadratic {
            "balken_b32"
        } else {
            "balken_b31"
        };
        let Some(frd) = run_ccx(name, &write_inp(&mesh, &model, "").unwrap()) else {
            return;
        };
        // CalculiX expands the beam to solids with new node numbers.
        assert!(frd.mesh.element_count() == 10 && frd.mesh.node(1).is_none());
        let deflection = -min_u2(&frd);
        assert!(
            (deflection - expected).abs() < tolerance * expected,
            "{name}: {deflection} statt {expected}"
        );
    }
}

#[test]
fn calculix_bends_a_pipe_like_beam_theory() {
    let (r, t) = (5.0f64, 1.0f64);
    let inertia = std::f64::consts::PI * (r.powi(4) - (r - t).powi(4)) / 4.0;
    let expected = cantilever_deflection(100.0, 100.0, inertia);
    let mesh = line_chain(10, 0, 100.0, true);
    let model = line_model(
        &mesh,
        SectionKind::Beam(BeamSection {
            profile: BeamProfile::Pipe {
                radius: r,
                thickness: t,
            },
            ..BeamSection::DEFAULT
        }),
        LoadKind::ConcentratedForce([0.0, -100.0, 0.0]),
    );
    let Some(frd) = run_ccx("rohr_b32r", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let deflection = -min_u2(&frd);
    assert!(
        (deflection - expected).abs() < 0.01 * expected,
        "{deflection} statt {expected}"
    );
}

#[test]
fn calculix_stretches_a_truss_by_f_l_over_e_a() {
    let mesh = line_chain(5, 0, 100.0, true);
    let mut model = line_model(
        &mesh,
        SectionKind::Truss { area: 50.0 },
        LoadKind::ConcentratedForce([1000.0, 0.0, 0.0]),
    );
    // A straight chain of trusses is a mechanism sideways; hold the nodes in the axis.
    let nodes: Vec<NodeId> = mesh
        .node_ids()
        .iter()
        .copied()
        .filter(|&n| n % 2 == 1)
        .collect();
    model.steps[0].boundary_conditions.push(BoundaryCondition {
        name: "Sideways".into(),
        active: true,
        region: Region::Nodes(nodes),
        kind: BoundaryKind::Displacement([None, Some(0.0), Some(0.0), None, None, None]),
        amplitude: None,
    });
    let Some(frd) = run_ccx("stab_t3d2", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let expected = 1000.0 * 100.0 / (210_000.0 * 50.0);
    let stretch = node_value(&frd, "DISP", "U1", 11);
    assert!(
        (stretch - expected).abs() < 1e-6 * expected,
        "{stretch} statt {expected}"
    );
}

/// The cantilever bar (x from 0 to 100, 5 x 5 cross-section) as a thermal model of steel in
/// mm, t, s: conductivity 50 W/(m K), specific heat 460 J/(kg K), expansion 1e-5 from 20, and
/// a step of the given kind.
fn thermal_bar(kind: StepKind, bcs: Vec<BoundaryCondition>, loads: Vec<Load>) -> (FeMesh, FeModel) {
    let (mesh, mut model) = cantilever(tip_force());
    let material = &mut model.materials[0];
    material.conductivity = Some(CONDUCTIVITY);
    material.specific_heat = Some(SPECIFIC_HEAT);
    material.expansion = Some(plx_model::Expansion {
        coefficient: 1e-5,
        zero_temperature: 20.0,
    });
    let step = &mut model.steps[0];
    step.field_outputs = match kind {
        StepKind::HeatTransfer(_) => FieldOutput::heat_transfer_defaults(),
        _ => FieldOutput::coupled_defaults(),
    };
    step.kind = kind;
    step.boundary_conditions = bcs;
    step.loads = loads;
    (mesh, model)
}

const CONDUCTIVITY: f64 = 50.0;
const SPECIFIC_HEAT: f64 = 4.6e8;

fn temperature(name: &str, region: Region, value: f64) -> BoundaryCondition {
    BoundaryCondition {
        name: name.into(),
        active: true,
        region,
        kind: BoundaryKind::Temperature(value),
        amplitude: None,
    }
}

fn heat_load(name: &str, region: Region, kind: LoadKind) -> Load {
    Load {
        name: name.into(),
        active: true,
        region,
        kind,
        amplitude: None,
        factor_amplitude: None,
    }
}

fn steady() -> StepKind {
    StepKind::HeatTransfer(HeatTransferStep::default())
}

/// Temperatures at the nodes of the cross-section at x.
fn temperatures_at(mesh: &FeMesh, frd: &FrdImport, x: f64) -> Vec<f64> {
    let nodes = nodes_at(mesh, 0, x);
    assert!(!nodes.is_empty());
    (nodes.iter())
        .map(|&n| node_value(frd, "NDTEMP", "T", n))
        .collect()
}

#[track_caller]
fn assert_all_close(values: &[f64], expected: f64, tolerance: f64) {
    for &v in values {
        assert!(
            (v - expected).abs() <= tolerance,
            "{v} != {expected} ({values:?})"
        );
    }
}

#[test]
fn writes_heat_transfer_keywords() {
    let (mesh, mut model) = thermal_bar(
        StepKind::HeatTransfer(HeatTransferStep {
            steady_state: false,
            deltmx: Some(5.0),
            ..HeatTransferStep::default()
        }),
        vec![
            temperature("Temperature-1", Region::NodeSet("FIX".into()), 20.0),
            BoundaryCondition {
                name: "Fixed-1".into(),
                active: true,
                region: Region::NodeSet("FIX".into()),
                kind: BoundaryKind::Fixed,
                amplitude: None,
            },
        ],
        vec![
            heat_load(
                "Flux-1",
                Region::Nodes(vec![99]),
                LoadKind::ConcentratedFlux(3.0),
            ),
            heat_load(
                "Film-1",
                Region::Surface("TIP".into()),
                LoadKind::Film {
                    sink: 25.0,
                    coefficient: 0.01,
                },
            ),
            heat_load(
                "Body-1",
                Region::Parts(vec!["EALL".into()]),
                LoadKind::BodyFlux(0.5),
            ),
            tip_force(),
        ],
    );
    model.properties.absolute_zero = Some(-273.15);
    model.properties.stefan_boltzmann = Some(5.67e-11);
    model.initial_conditions.push(plx_model::InitialCondition {
        name: "Initial_Temperature-1".into(),
        active: true,
        region: Region::Parts(vec!["EALL".into()]),
        kind: plx_model::InitialConditionKind::Temperature(20.0),
    });
    let text = write_inp(&mesh, &model, "").unwrap();
    for expected in [
        "*Physical constants, Absolute zero=-273.15, Stefan Boltzmann=0.0000000000567\n",
        "*Expansion, Zero=20\n0.00001\n*Conductivity\n50\n*Specific heat\n460000000\n",
        "** Name: Initial_Temperature-1\n*Initial conditions, Type=Temperature\n",
        "*Heat transfer, Deltmx=5\n",
        "** Name: Temperature-1\n*Boundary\nFIX, 11, 11, 20\n",
        // Displacements and forces do not act in a heat transfer step.
        "** Name: Fixed-1: Deactivated\n",
        "** Name: Force-1: Deactivated\n",
        "*Cflux, op=New\n*Dflux, op=New\n*Film, op=New\n",
        "*Cflux\nInternal_Selection-1_Flux-1, 11, 3\n",
        "*Dflux\nInternal_Selection-1_Body-1, BF, 0.5\n",
        "*Film\nInternal-1_TIP_S4, F4, 25, 0.01\n",
        "*Node file\nNT, RFL\n",
        "*El file\nHFL\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert!(
        !text.contains("Cload") && !text.contains("Radiate"),
        "{text}"
    );
}

/// Steady conduction along the bar, held at 0 at x = 0: a heat flux q into the far end
/// gives T = q x / k.
#[test]
fn calculix_conducts_heat_along_a_bar() {
    let q = 1.0;
    let (mesh, model) = thermal_bar(
        steady(),
        vec![temperature(
            "Temperature-1",
            Region::NodeSet("FIX".into()),
            0.0,
        )],
        vec![heat_load(
            "Surface_Flux-1",
            Region::Surface("TIP".into()),
            LoadKind::SurfaceFlux(q),
        )],
    );
    let Some(frd) = run_ccx("waermeleitung", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    for x in [50.0, 100.0] {
        assert_all_close(&temperatures_at(&mesh, &frd, x), q * x / CONDUCTIVITY, 1e-6);
    }
    let hfl = node_value(&frd, "FLUX", "F1", 99);
    assert!((hfl.abs() - q).abs() < 1e-3, "{hfl}");
}

/// Heat generated in the bar flows out at x = 0: T = Q (L x - x^2 / 2) / k.
#[test]
fn calculix_conducts_body_heat() {
    let (body, length) = (0.01, 100.0);
    let (mesh, model) = thermal_bar(
        steady(),
        vec![temperature(
            "Temperature-1",
            Region::NodeSet("FIX".into()),
            0.0,
        )],
        vec![heat_load(
            "Body_Flux-1",
            Region::Parts(vec!["EALL".into()]),
            LoadKind::BodyFlux(body),
        )],
    );
    let Some(frd) = run_ccx("koerperwaerme", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    for x in [50.0, 100.0] {
        let expected = body * (length * x - x * x / 2.0) / CONDUCTIVITY;
        assert_all_close(&temperatures_at(&mesh, &frd, x), expected, 1e-3 * expected);
    }
}

/// The far end gives its heat to the surroundings by convection: k T_L / L = h (T_s - T_L).
#[test]
fn calculix_cools_the_bar_by_convection() {
    let (h, sink, length) = (0.5, 100.0, 100.0);
    let (mesh, model) = thermal_bar(
        steady(),
        vec![temperature(
            "Temperature-1",
            Region::NodeSet("FIX".into()),
            0.0,
        )],
        vec![heat_load(
            "Film-1",
            Region::Surface("TIP".into()),
            LoadKind::Film {
                sink,
                coefficient: h,
            },
        )],
    );
    let Some(frd) = run_ccx("konvektion", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let expected = h * sink / (h + CONDUCTIVITY / length);
    assert_all_close(
        &temperatures_at(&mesh, &frd, length),
        expected,
        1e-6 * expected,
    );
}

/// The far end radiates to the surroundings: k (T_0 - T_L) / L = e s (T_L^4 - T_s^4) in
/// kelvin, with the physical constants of the model.
#[test]
fn calculix_cools_the_bar_by_radiation() {
    let (hot, sink, emissivity, length) = (500.0, 20.0, 0.8, 100.0);
    let (mesh, mut model) = thermal_bar(
        steady(),
        vec![temperature(
            "Temperature-1",
            Region::NodeSet("FIX".into()),
            hot,
        )],
        vec![heat_load(
            "Radiation-1",
            Region::Surface("TIP".into()),
            LoadKind::Radiation { sink, emissivity },
        )],
    );
    let (zero, sigma) =
        plx_model::ModelProperties::standard_constants(model.properties.units).unwrap();
    model.properties.absolute_zero = Some(zero);
    model.properties.stefan_boltzmann = Some(sigma);
    // Radiation is nonlinear; CalculiX iterates from the initial temperature.
    model.initial_conditions.push(plx_model::InitialCondition {
        name: "Initial_Temperature-1".into(),
        active: true,
        region: Region::Parts(vec!["EALL".into()]),
        kind: plx_model::InitialConditionKind::Temperature(hot),
    });
    let Some(frd) = run_ccx("strahlung", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let kelvin = |t: f64| t - zero;
    let balance = |t: f64| {
        CONDUCTIVITY * (hot - t) / length
            - emissivity * sigma * (kelvin(t).powi(4) - kelvin(sink).powi(4))
    };
    let (mut low, mut high) = (sink, hot);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if balance(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    assert_all_close(
        &temperatures_at(&mesh, &frd, length),
        low,
        1e-3 * (hot - low),
    );
}

/// A bar heated at x = 0 and insulated elsewhere warms up as the series solution of the
/// heat equation says: T(L, t) / T_0 = 1 - sum 4 (-1)^n / ((2n+1) pi) exp(-l_n^2 a t).
#[test]
fn calculix_warms_the_bar_transiently() {
    let (hot, length, density) = (100.0, 100.0, 7.85e-9);
    let mut settings = HeatTransferStep {
        steady_state: false,
        ..HeatTransferStep::default()
    };
    // Thermal diffusivity.
    let a = CONDUCTIVITY / (density * SPECIFIC_HEAT);
    // Half way to the end temperature at the far end.
    let period = 0.4 * length * length / a;
    settings.increments.incrementation = Incrementation::Direct;
    settings.increments.time_period = period;
    settings.increments.initial_increment = period / 200.0;
    settings.increments.max_increments = 1000;
    let (mesh, mut model) = thermal_bar(
        StepKind::HeatTransfer(settings),
        vec![temperature(
            "Temperature-1",
            Region::NodeSet("FIX".into()),
            hot,
        )],
        Vec::new(),
    );
    model.initial_conditions.push(plx_model::InitialCondition {
        name: "Initial_Temperature-1".into(),
        active: true,
        region: Region::Parts(vec!["EALL".into()]),
        kind: plx_model::InitialConditionKind::Temperature(0.0),
    });
    let Some(frd) = run_ccx("instationaer", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let series: f64 = (0..50)
        .map(|n| {
            let k = f64::from(2 * n + 1);
            let lambda = k * std::f64::consts::PI / (2.0 * length);
            let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
            4.0 * sign / (k * std::f64::consts::PI) * (-lambda * lambda * a * period).exp()
        })
        .sum();
    let expected = hot * (1.0 - series);
    assert!(expected > 20.0 && expected < 80.0, "{expected}");
    assert_all_close(&temperatures_at(&mesh, &frd, length), expected, 0.02 * hot);
}

/// A bar held only against rigid body motion and heated evenly by 100 from its stress free
/// temperature grows by alpha dT L and stays free of stress.
#[test]
fn calculix_expands_the_bar_in_a_coupled_step() {
    let length = 100.0;
    let (mesh, _) = cantilever(tip_force());
    let at = |p: [f64; 3]| -> NodeId {
        let index = (mesh.coords().iter())
            .position(|c| (0..3).all(|k| (c[k] - p[k]).abs() < 1e-9))
            .unwrap();
        mesh.node_ids()[index]
    };
    let support = |name: &str, nodes: Vec<NodeId>, values: [Option<f64>; 6]| BoundaryCondition {
        name: name.into(),
        active: true,
        region: Region::Nodes(nodes),
        kind: BoundaryKind::Displacement(values),
        amplitude: None,
    };
    let (origin, y, z) = (at([0.0; 3]), at([0.0, 5.0, 0.0]), at([0.0, 0.0, 5.0]));
    let (mesh, mut model) = thermal_bar(
        StepKind::CoupledTempDisp(HeatTransferStep::default()),
        vec![
            support("Support-1", nodes_at(&mesh, 0, 0.0), FIX_X),
            support(
                "Support-2",
                vec![origin],
                [None, Some(0.0), Some(0.0), None, None, None],
            ),
            support(
                "Support-3",
                vec![y],
                [None, None, Some(0.0), None, None, None],
            ),
            support(
                "Support-4",
                vec![z],
                [None, Some(0.0), None, None, None, None],
            ),
            temperature("Temperature-1", Region::NodeSet("NALL".into()), 120.0),
        ],
        Vec::new(),
    );
    model.initial_conditions.push(plx_model::InitialCondition {
        name: "Initial_Temperature-1".into(),
        active: true,
        region: Region::Parts(vec!["EALL".into()]),
        kind: plx_model::InitialConditionKind::Temperature(20.0),
    });
    let Some(frd) = run_ccx("waermedehnung", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let expected = 1e-5 * 100.0 * length;
    for node in nodes_at(&mesh, 0, length) {
        let u = node_value(&frd, "DISP", "U1", node);
        assert!((u - expected).abs() < 1e-6 * expected, "{u} != {expected}");
        let s = node_value(&frd, "STRESS", "S11", node);
        assert!(s.abs() < 1e-6, "{s}");
    }
}

/// In a plane stress model the faces are edges: a strip held at 0 at x = 0 and cooled by a
/// film at its far edge reaches k T_L / L = h (T_s - T_L), as the bar does.
#[test]
fn calculix_cools_a_plane_strip_by_convection() {
    let (length, h, sink) = (10.0, 5.0, 100.0);
    let mesh = rectangle(0.0, length, 2.0, 5, 2);
    let mut model = plane_model(
        ModelSpace::PlaneStress,
        2.0,
        Vec::new(),
        LoadKind::Film {
            sink,
            coefficient: h,
        },
        Region::Faces(edges_at(&mesh, 0, length)),
    );
    model.materials[0].conductivity = Some(CONDUCTIVITY);
    let step = &mut model.steps[0];
    step.kind = steady();
    step.field_outputs = FieldOutput::heat_transfer_defaults();
    step.boundary_conditions = vec![temperature(
        "Temperature-1",
        Region::Nodes(nodes_at(&mesh, 0, 0.0)),
        0.0,
    )];
    let text = write_inp(&mesh, &model, "").unwrap();
    let Some(frd) = run_ccx("film-2d", &text) else {
        return;
    };
    let expected = h * sink / (h + CONDUCTIVITY / length);
    assert_all_close(
        &temperatures_at(&mesh, &frd, length),
        expected,
        1e-6 * expected,
    );
}

fn amplitude(name: &str, points: Vec<[f64; 2]>) -> plx_model::Amplitude {
    plx_model::Amplitude {
        points,
        ..plx_model::Amplitude::new(name)
    }
}

#[test]
fn writes_amplitudes_and_their_references() {
    let mut force = tip_force();
    force.amplitude = Some("Ramp up".into());
    let (mesh, mut model) = cantilever(force);
    let mut ramp = amplitude(
        "Ramp up",
        vec![[0.0, 0.0], [0.2, 0.5], [0.4, 1.0], [0.6, 1.0], [1.0, 0.25]],
    );
    ramp.time_span = plx_model::AmplitudeTime::Total;
    ramp.shift_time = 0.1;
    ramp.shift_amplitude = -0.5;
    model.amplitudes.push(ramp);
    model.amplitudes.push(amplitude("Half", vec![[0.0, 0.5]]));
    let step = &mut model.steps[0];
    // Fixed supports stay zero; the amplitude is left out.
    step.boundary_conditions[0].amplitude = Some("Half".into());
    step.boundary_conditions.push(BoundaryCondition {
        name: "Displacement-1".into(),
        active: true,
        region: Region::Nodes(vec![99]),
        kind: BoundaryKind::Displacement([None, Some(0.5), None, None, None, None]),
        amplitude: Some("Half".into()),
    });
    let text = write_inp(&mesh, &model, "").unwrap();
    for expected in [
        "** Amplitudes ",
        "*Amplitude, Name=Ramp_up, Time=Total time, Shiftx=0.1, Shifty=-0.5\n\
         0, 0, 0.2, 0.5, 0.4, 1, 0.6, 1\n1, 0.25\n",
        "*Amplitude, Name=Half\n0, 0.5\n",
        "** Name: Fixed-1\n*Boundary\nFIX, 1, 6, 0\n",
        "** Name: Displacement-1\n*Boundary, Amplitude=Half\n",
        "*Cload, Amplitude=Ramp_up\nInternal_Selection-1_Force-1, 3, -100\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    // The amplitudes precede the steps, as CalculiX reads them before their references.
    assert!(text.find("*Amplitude,").unwrap() < text.find("*Step").unwrap());

    model.steps[0].loads[0].amplitude = Some("Gone".into());
    assert_eq!(
        write_inp(&mesh, &model, ""),
        Err(WriteError::UnknownAmplitude {
            item: "Force-1".into(),
            amplitude: "Gone".into(),
        })
    );
    model.steps[0].loads[0].amplitude = None;
    model.amplitudes[1].points.clear();
    assert!(matches!(
        write_inp(&mesh, &model, ""),
        Err(WriteError::InvalidAmplitude { .. })
    ));
}

#[test]
fn writes_both_amplitudes_of_films_and_radiation() {
    let mut film = heat_load(
        "Film-1",
        Region::Surface("TIP".into()),
        LoadKind::Film {
            sink: 25.0,
            coefficient: 0.01,
        },
    );
    film.amplitude = Some("Sink".into());
    film.factor_amplitude = Some("Coefficient".into());
    let mut radiation = heat_load(
        "Radiation-1",
        Region::Surface("TIP".into()),
        LoadKind::Radiation {
            sink: 25.0,
            emissivity: 0.8,
        },
    );
    radiation.factor_amplitude = Some("Coefficient".into());
    let mut flux = heat_load(
        "Flux-1",
        Region::Nodes(vec![99]),
        LoadKind::ConcentratedFlux(3.0),
    );
    flux.amplitude = Some("Sink".into());
    let mut surface_flux = heat_load(
        "Surface_Flux-1",
        Region::Surface("TIP".into()),
        LoadKind::SurfaceFlux(1.0),
    );
    surface_flux.amplitude = Some("Sink".into());
    let mut temperature = temperature("Temperature-1", Region::NodeSet("FIX".into()), 20.0);
    temperature.amplitude = Some("Sink".into());
    let (mesh, mut model) = thermal_bar(
        steady(),
        vec![temperature],
        vec![film, radiation, flux, surface_flux],
    );
    model.properties.absolute_zero = Some(-273.15);
    model.properties.stefan_boltzmann = Some(5.67e-11);
    model.amplitudes = vec![
        amplitude("Sink", vec![[0.0, 1.0], [1.0, 2.0]]),
        amplitude("Coefficient", vec![[0.0, 1.0]]),
    ];
    let text = write_inp(&mesh, &model, "").unwrap();
    for expected in [
        "*Boundary, Amplitude=Sink\nFIX, 11, 11, 20\n",
        "*Film, Amplitude=Sink, Film amplitude=Coefficient\n",
        "*Radiate, Radiation amplitude=Coefficient\n",
        "*Cflux, Amplitude=Sink\n",
        "*Dflux, Amplitude=Sink\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    // CalculiX takes every one of them.
    run_ccx("amplitudes_thermal", &text);
}

/// A force that follows an amplitude up to its full value at half the step and back to half
/// of it at the end: the cantilever bends as far as with the full force at first, then half.
/// The deflection is small, so the geometrically nonlinear steps stay within a percent of
/// the linear solution.
#[test]
fn calculix_scales_a_load_by_its_amplitude() {
    let (mesh, reference) = cantilever(tip_force());
    let Some(full) = run_ccx(
        "amplitude_reference",
        &write_inp(&mesh, &reference, "").unwrap(),
    ) else {
        return;
    };
    let full = node_value(&full, "DISP", "U3", 99);
    let mut model = reference.clone();
    model.amplitudes.push(amplitude(
        "Up_and_down",
        vec![[0.0, 0.0], [0.5, 1.0], [1.0, 0.5]],
    ));
    model.steps[0].loads[0].amplitude = Some("Up_and_down".into());
    let StepKind::Static(settings) = &mut model.steps[0].kind else {
        unreachable!()
    };
    // A linear static step is solved once, at its end; increments need a nonlinear one.
    settings.nlgeom = true;
    settings.incrementation = Incrementation::Direct;
    settings.initial_increment = 0.25;
    let frd = run_ccx("amplitude_force", &write_inp(&mesh, &model, "").unwrap()).unwrap();
    let index = frd.mesh.node_index(99).unwrap();
    let u3: Vec<f64> = (frd.increments.iter())
        .filter_map(|i| i.field("DISP"))
        .map(|f| f64::from(f.component("U3").unwrap().values[index]))
        .collect();
    assert_eq!(u3.len(), 4, "{u3:?}");
    for (value, factor) in u3.iter().zip([0.5, 1.0, 0.75, 0.5]) {
        assert!(
            (value - factor * full).abs() < 1e-2 * full.abs(),
            "{u3:?} statt {factor} * {full}"
        );
    }
}

/// A temperature held by an amplitude that ends at half: the bar settles at half the value.
#[test]
fn calculix_scales_a_temperature_by_its_amplitude() {
    let mut held = temperature("Temperature-1", Region::NodeSet("FIX".into()), 100.0);
    held.amplitude = Some("Half".into());
    let (mesh, mut model) = thermal_bar(steady(), vec![held], Vec::new());
    model
        .amplitudes
        .push(amplitude("Half", vec![[0.0, 1.0], [1.0, 0.5]]));
    let Some(frd) = run_ccx(
        "amplitude_temperature",
        &write_inp(&mesh, &model, "").unwrap(),
    ) else {
        return;
    };
    assert_all_close(&temperatures_at(&mesh, &frd, 100.0), 50.0, 1e-6);
}

/// Beyond its last point an amplitude keeps the last value, as [`plx_model::Amplitude::value_at`]
/// assumes for the dialog's curve.
#[test]
fn calculix_holds_an_amplitude_after_its_last_point() {
    let (mesh, reference) = cantilever(tip_force());
    let Some(full) = run_ccx(
        "amplitude_hold_ref",
        &write_inp(&mesh, &reference, "").unwrap(),
    ) else {
        return;
    };
    let full = node_value(&full, "DISP", "U3", 99);
    let mut model = reference;
    model
        .amplitudes
        .push(amplitude("Early", vec![[0.0, 0.0], [0.5, 1.0]]));
    model.steps[0].loads[0].amplitude = Some("Early".into());
    let frd = run_ccx("amplitude_hold", &write_inp(&mesh, &model, "").unwrap()).unwrap();
    let held = node_value(&frd, "DISP", "U3", 99);
    assert!(
        (held - full).abs() < 1e-6 * full.abs(),
        "{held} statt {full}"
    );
}

#[test]
fn history_outputs_become_print_keywords_with_their_sets() {
    let (mesh, mut model) = cantilever(tip_force());
    let step = &mut model.steps[0];
    let mut nodes = HistoryOutput::node("NH_Output-1", Region::Nodes(vec![99]));
    nodes.totals = Totals::Yes;
    step.history_outputs.push(nodes);
    let mut elements = HistoryOutput::element("EH_Output-1", Region::Parts(vec!["EALL".into()]));
    elements.variables = vec!["S".into(), "EVOL".into()];
    step.history_outputs.push(elements);
    let mut off = HistoryOutput::node("NH_Output-2", Region::NodeSet("FIX".into()));
    off.active = false;
    step.history_outputs.push(off);
    let mut empty = HistoryOutput::node("NH_Output-3", Region::NodeSet("FIX".into()));
    empty.variables.clear();
    step.history_outputs.push(empty);
    let text = write_inp(&mesh, &model, "").unwrap();
    for expected in [
        "*Nset, Nset=Internal_Selection-1_NH_Output-1\n99\n",
        "*Elset, Elset=Internal_Selection-1_EH_Output-1\nEALL\n",
        "** History outputs +++++++++++++++++++++++++++++++++++++++++\n**\n\
         ** Name: NH_Output-1\n\
         *Node print, Nset=Internal_Selection-1_NH_Output-1, Totals=Yes\nRF, U\n\
         ** Name: EH_Output-1\n\
         *El print, Elset=Internal_Selection-1_EH_Output-1\nS, EVOL\n\
         ** Name: NH_Output-2: Deactivated\n**\n",
    ] {
        assert!(text.contains(expected), "{expected}\nfehlt in\n{text}");
    }
    assert!(!text.contains("NH_Output-3"));
    // A deactivated step lists its history outputs as comments.
    model.steps[0].active = false;
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("** Name: NH_Output-1: Deactivated\n"));
    assert!(!text.contains("*Node print"));
}

#[test]
fn contact_forces_need_an_active_contact_pair() {
    let (mesh, mut model) = cantilever(tip_force());
    let mut output = HistoryOutput::contact("CH_Output-1", "Contact_Pair-1");
    model.steps[0].history_outputs.push(output.clone());
    // Without contact forces no surfaces are needed.
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains("** Name: CH_Output-1\n*Contact print\nCDIS, CSTR\n"));
    output.variables.push("CF".into());
    model.steps[0].history_outputs[0] = output;
    assert_eq!(
        write_inp(&mesh, &model, ""),
        Err(WriteError::UnknownContactPair {
            item: "CH_Output-1".into(),
            pair: "Contact_Pair-1".into(),
        })
    );
}

#[test]
fn calculix_prints_the_history_outputs_into_the_dat_file() {
    let (mesh, mut model) = cantilever(tip_force());
    let step = &mut model.steps[0];
    let mut reactions = HistoryOutput::node("NH_Output-1", Region::NodeSet("FIX".into()));
    reactions.variables = vec!["RF".into()];
    reactions.totals = Totals::Only;
    step.history_outputs.push(reactions);
    step.history_outputs
        .push(HistoryOutput::node("NH_Output-2", Region::Nodes(vec![99])));
    let mut volume = HistoryOutput::element("EH_Output-1", Region::Parts(vec!["EALL".into()]));
    volume.variables = vec!["S".into(), "EVOL".into()];
    volume.totals = Totals::Yes;
    step.history_outputs.push(volume);
    let Some(dat) = run_ccx_dat("historie", &write_inp(&mesh, &model, "").unwrap()) else {
        return;
    };
    let import = crate::dat::parse_dat(&dat);
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let set = |name: &str| import.sets.iter().find(|s| s.name == name).unwrap();
    let total = set("FIX").field("TOTAL_FORCE").unwrap();
    let rf3 = &total.component("RF3").unwrap().entries[0].values;
    assert!((rf3[0] - 100.0).abs() < 1e-6, "{rf3:?}");
    let tip = set("NH_OUTPUT-2").field("DISPLACEMENTS").unwrap();
    let u3 = tip.component("U3").unwrap();
    assert_eq!(u3.entries[0].name, "99");
    assert!(u3.entries[0].values[0] < 0.0);
    let elements = set("EH_OUTPUT-1");
    assert!(
        elements
            .field("STRESSES")
            .unwrap()
            .component("MISES")
            .is_some()
    );
    let volume = elements.field("TOTAL_VOLUME").unwrap().components[0].entries[0].values[0];
    let bounds = mesh.bounds().unwrap();
    let expected: f64 = (0..3).map(|i| bounds.1[i] - bounds.0[i]).product();
    assert!(
        (volume - expected).abs() < 1e-6 * expected,
        "{volume} {expected}"
    );
}

#[test]
fn calculix_prints_the_contact_force_of_a_pair() {
    let (mesh, lower, upper) = two_blocks(0.0);
    let mut model = stacked_blocks(&mesh);
    let top: Vec<NodeId> = (119..=127).collect();
    model.steps[0].boundary_conditions.push(BoundaryCondition {
        name: "Side-1".into(),
        active: true,
        region: Region::Nodes(top),
        kind: BoundaryKind::Displacement([Some(0.0), Some(0.0), None, None, None, None]),
        amplitude: None,
    });
    model.surface_interactions.push(SurfaceInteraction {
        name: "Surface_Interaction-1".into(),
        properties: vec![InteractionProperty::SurfaceBehavior(SurfaceBehavior::Hard)],
    });
    let mut pair = ContactPair::new("Contact_Pair-1", "Surface_Interaction-1");
    pair.master = Region::Faces(lower);
    pair.slave = Region::Faces(upper);
    model.contact_pairs.push(pair);
    let mut output = HistoryOutput::contact("CH_Output-1", "Contact_Pair-1");
    output.variables = ["CDIS", "CSTR", "CNUM", "CF"].map(String::from).to_vec();
    model.steps[0].history_outputs.push(output);
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(text.contains(
        "*Contact print, Master=Internal_Selection-1_Contact_Pair-1_Master, \
         Slave=Internal_Selection-1_Contact_Pair-1_Slave\nCDIS, CSTR, CNUM, CF\n"
    ));
    let Some(dat) = run_ccx_dat("kontakt-historie", &text) else {
        return;
    };
    let import = crate::dat::parse_dat(&dat);
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    let pair = import
        .sets
        .iter()
        .find(|s| s.name == "CONTACT_PAIR-1")
        .unwrap();
    let force = pair
        .field("TOTAL_SURFACE_FORCE")
        .unwrap()
        .component("FZ")
        .unwrap();
    let fz = *force.entries[0].values.last().unwrap();
    // 10 MPa on 10 x 10 mm press the blocks together.
    assert!((fz.abs() - 1000.0).abs() < 10.0, "{fz}");
    let all = (import.sets.iter())
        .find(|s| s.name == crate::dat::ALL_CONTACT_ELEMENTS)
        .unwrap();
    assert!(all.field("CONTACT_STRESS").is_some());
    assert!(all.field("TOTAL_NUMBER_OF_CONTACT_ELEMENTS").is_some());
}
