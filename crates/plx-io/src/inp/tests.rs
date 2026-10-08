use std::path::PathBuf;

use plx_mesh::{ElementShape, SurfaceDefinition, extract_part_skin};

use super::*;

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(name)
}

#[test]
fn reads_hex_cantilever_with_sets_and_surface() {
    let import = read_inp(&testdata("kragbalken_c3d8.inp")).unwrap();
    let mesh = &import.mesh;
    assert_eq!(mesh.node_count(), 11 * 3 * 3);
    assert_eq!(mesh.element_count(), 40);
    assert_eq!(mesh.parts.len(), 1);
    assert_eq!(mesh.parts[0].name, "EALL");
    assert_eq!(mesh.node_sets["FIX"].len(), 9);
    assert_eq!(mesh.node_sets["NALL"].len(), 99);
    assert_eq!(mesh.element_sets["TIP_ELEMENTS"], vec![10, 20, 30, 40]);
    assert_eq!(
        mesh.surfaces["TIP"],
        SurfaceDefinition::ElementFaces(vec![(10, 4), (20, 4), (30, 4), (40, 4)])
    );
    assert_eq!(mesh.bounds(), Some(([0.0, 0.0, 0.0], [100.0, 10.0, 10.0])));
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    for keyword in [
        "HEADING",
        "MATERIAL",
        "ELASTIC",
        "SOLIDSECTION",
        "STEP",
        "STATIC",
        "BOUNDARY",
        "ENDSTEP",
    ] {
        assert!(import.skipped_keywords.contains_key(keyword), "{keyword}");
    }
}

#[test]
fn reads_quadratic_hexes_spanning_two_lines() {
    let import = read_inp(&testdata("block_c3d20r.inp")).unwrap();
    let mesh = &import.mesh;
    assert_eq!(mesh.element_count(), 16);
    assert!(
        mesh.elements()
            .iter()
            .all(|e| e.shape == ElementShape::Hex20)
    );
    assert!(mesh.missing_nodes().is_empty());
    let skin = extract_part_skin(mesh, &mesh.parts[0], 30.0);
    assert_eq!(skin.faces.len(), 2 * (4 * 2 + 4 * 2 + 2 * 2));
    assert!(skin.faces.iter().all(|f| f.mids.len() == 4));
}

#[test]
fn reads_quadratic_tets() {
    let import = read_inp(&testdata("wuerfel_c3d10.inp")).unwrap();
    let mesh = &import.mesh;
    assert_eq!(mesh.element_count(), 27 * 6);
    let skin = extract_part_skin(mesh, &mesh.parts[0], 30.0);
    // Each of the 6 cube sides has 3 × 3 squares, each split into 2 triangles.
    assert_eq!(skin.faces.len(), 6 * 9 * 2);
    assert_eq!(skin.edges.iter().filter(|e| e.feature).count(), 12 * 3);
}

#[test]
fn reads_includes_shells_and_beams_as_separate_parts() {
    let import = read_inp(&testdata("platte_mit_stuetzen.inp")).unwrap();
    let mesh = &import.mesh;
    assert_eq!(import.files.len(), 2);
    assert_eq!(mesh.node_count(), 27);
    let names: Vec<_> = mesh.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["PLATTE", "STUETZEN"]);
    let beams = extract_part_skin(mesh, &mesh.parts[1], 30.0);
    assert_eq!(beams.lines.len(), 2);
    assert!(beams.faces.is_empty());
}

#[test]
fn elements_without_elset_form_a_part_per_type() {
    let text = "*NODE\n1,0,0,0\n2,1,0,0\n3,0,1,0\n4,0,0,1\n*ELEMENT,TYPE=C3D4\n1,1,2,3,4\n";
    let import = read_inp_str(text, None).unwrap();
    assert_eq!(import.mesh.parts[0].name, "C3D4");
    assert_eq!(import.mesh.element_sets["C3D4"], vec![1]);
}

#[test]
fn generate_and_set_references() {
    let text = "*NODE\n1,0,0,0\n*NSET,NSET=A,GENERATE\n1,9,2\n*NSET,NSET=B\nA, 20\n*NSET,NSET=C\nMISSING\n";
    let import = read_inp_str(text, None).unwrap();
    assert_eq!(import.mesh.node_sets["A"], vec![1, 3, 5, 7, 9]);
    assert_eq!(import.mesh.node_sets["B"], vec![1, 3, 5, 7, 9, 20]);
    assert_eq!(import.warnings.len(), 1, "{:?}", import.warnings);
}

#[test]
fn unsupported_elements_are_skipped_with_a_warning() {
    let text = "*NODE\n1,0,0,0\n2,1,0,0\n*ELEMENT,TYPE=SPRINGA,ELSET=S\n1,1,2\n*ELEMENT,TYPE=T3D2,ELSET=T\n2,1,2\n";
    let import = read_inp_str(text, None).unwrap();
    assert_eq!(import.mesh.element_count(), 1);
    assert!(import.warnings.iter().any(|w| w.contains("SPRINGA")));
}

#[test]
fn errors_name_file_and_line() {
    let err = read_inp_str(
        "*NODE\n1,0,0,0\n*ELEMENT,TYPE=C3D4\n1,1,2,3\n*NSET,NSET=X\n1\n",
        None,
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "<text>:4: Element 1 vom Typ C3D4 ist unvollständig"
    );
    let err = read_inp_str("*NODE\n1,0,abc,0\n", None).unwrap_err();
    assert_eq!(err.to_string(), "<text>:2: Ungültige Koordinate 'abc'");
    let err = read_inp_str("*ELEMENT\n", None).unwrap_err();
    assert_eq!(err.to_string(), "<text>:1: *ELEMENT ohne TYPE=");
}

#[test]
fn missing_include_is_an_io_error() {
    let err = read_inp_str("*INCLUDE, INPUT=gibt_es_nicht.inp\n", Some(&testdata(""))).unwrap_err();
    assert!(matches!(err, InpError::Io { .. }));
}
