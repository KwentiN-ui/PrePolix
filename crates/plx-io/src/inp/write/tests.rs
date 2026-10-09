use std::path::PathBuf;
use std::process::Command;

use plx_model::{BoundaryCondition, Elastic, EquationSolver, Load, Material, Section, UserKeyword};

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
        region: Region::NodeSet("FIX".into()),
        kind: BoundaryKind::Fixed,
    });
    step.loads.push(load);
    let model = FeModel {
        materials: vec![Material {
            name: "Steel".into(),
            density: Some(7.85e-9),
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
        }],
        sections: vec![Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: Region::Parts(vec!["EALL".into()]),
        }],
        steps: vec![step],
        user_keywords: Vec::new(),
        hot_spots: Vec::new(),
    };
    (mesh, model)
}

fn tip_force() -> Load {
    Load {
        name: "Force-1".into(),
        region: Region::Nodes(vec![99]),
        kind: LoadKind::ConcentratedForce([0.0, 0.0, -100.0]),
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
    let StepKind::Static(settings) = &mut model.steps[0].kind;
    settings.solver = EquationSolver::IterativeCholesky;
    settings.incrementation = Incrementation::Direct;
    let text = write_inp(&mesh, &model, "").unwrap();
    assert!(
        text.contains("*Static, Solver=Iterative Cholesky, Direct\n1, 1\n"),
        "{text}"
    );
    // A solver chosen in the step is kept.
    model.resolve_default_solver(EquationSolver::Pardiso);
    let StepKind::Static(settings) = &model.steps[0].kind;
    assert_eq!(settings.solver, EquationSolver::IterativeCholesky);
}

#[test]
fn picked_faces_become_one_element_set_per_face_number() {
    let load = Load {
        name: "Pressure 1".into(),
        region: Region::Faces(vec![(10, 4), (20, 4), (1, 6)]),
        kind: LoadKind::Pressure(2.5),
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
        region: Region::Nodes(Vec::new()),
        kind: LoadKind::ConcentratedForce([1.0, 0.0, 0.0]),
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
    let _ = std::fs::remove_dir_all(&dir);
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
        region: Region::Surface("TIP".into()),
        kind: LoadKind::Pressure(2.0),
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
        region: Region::Nodes(vec![NODE]),
        kind: LoadKind::ConcentratedForce(FORCE),
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
    let nodal = traction_forces(mesh, &faces, [0.0, -500.0, 30.0]);
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
        region: Region::Surface("TIP".into()),
        kind: LoadKind::SurfaceTraction([0.0, -80.0, 0.0]),
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
    model.steps.clear();
    let text = write_check_inp(&mesh, &model, "").unwrap();
    let step = text.find("*Step\n*No analysis\n").expect(&text);
    assert!(text[step..].contains("*End step\n"), "{text}");
}
