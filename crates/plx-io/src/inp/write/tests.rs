use std::path::PathBuf;
use std::process::Command;

use plx_model::{
    BeamOrientation, BeamProfile, BeamSection, BoundaryCondition, Elastic, EquationSolver, Load,
    Material, Section, SectionKind, UserKeyword,
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
        hot_spots: Vec::new(),
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
        active: true,
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
        active: true,
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

#[test]
fn deactivated_items_are_left_out_as_comments() {
    let (mesh, mut model) = cantilever(tip_force());
    model.steps[0].loads.push(Load {
        name: "Empty-1".into(),
        active: false,
        // Not even an empty region stops the export of a deactivated item.
        region: Region::Nodes(Vec::new()),
        kind: LoadKind::ConcentratedForce([1.0, 0.0, 0.0]),
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
        });
    }
    step.loads.push(Load {
        name: "Load-1".into(),
        active: true,
        region,
        kind: load,
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
    });
    step.loads.push(Load {
        name: "Pressure-1".into(),
        active: true,
        region: Region::Faces((101..=104).map(|e| (e, 2)).collect()),
        kind: LoadKind::Pressure(10.0),
    });
    model.materials.push(Material {
        name: "Steel".into(),
        density: None,
        elastic: Some(Elastic {
            young: 210_000.0,
            poisson: 0.3,
        }),
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
    });
    step.loads.push(Load {
        name: "Surface_Traction-1".into(),
        active: true,
        region: Region::Faces(blocks.b_top.clone()),
        kind: LoadKind::SurfaceTraction(force),
    });
    FeModel {
        materials: vec![Material {
            name: "Steel".into(),
            density: None,
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
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
    });
    step.loads.push(Load {
        name: "Force-1".into(),
        active: true,
        region: Region::Nodes(vec![tip]),
        kind: load,
    });
    FeModel {
        materials: vec![Material {
            name: "Steel".into(),
            density: None,
            elastic: Some(Elastic {
                young: 210_000.0,
                poisson: 0.3,
            }),
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
