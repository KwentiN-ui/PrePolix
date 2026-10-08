use super::*;

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(name)
}

fn value(import: &FrdImport, field: &str, component: &str, node: u32) -> f32 {
    let index = import.mesh.node_index(node).unwrap();
    import.increments[0]
        .field(field)
        .unwrap()
        .component(component)
        .unwrap()
        .values[index]
}

#[test]
fn reads_ascii_mesh_and_results() {
    let import = read_frd(&testdata("kragbalken_c3d8.frd")).unwrap();
    assert!(import.warnings.is_empty(), "{:?}", import.warnings);
    assert_eq!(import.mesh.node_count(), 99);
    assert_eq!(import.mesh.element_count(), 40);
    assert_eq!(import.mesh.parts.len(), 1);
    assert_eq!(import.mesh.parts[0].name, "STEEL");
    assert_eq!(import.mesh.node(99), Some([100.0, 10.0, 10.0]));

    assert_eq!(import.increments.len(), 1);
    let increment = &import.increments[0];
    assert_eq!((increment.step, increment.increment), (1, 1));
    assert_eq!(increment.kind, AnalysisKind::Static);
    assert_eq!(increment.value, 1.0);
    let names: Vec<&str> = increment.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["DISP", "STRESS", "ERROR"]);

    let disp: Vec<&str> = increment.fields[0]
        .components
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(disp, ["ALL", "U1", "U2", "U3"]);
    assert_eq!(value(&import, "DISP", "U3", 99), -1.34512e-1);
    assert_eq!(value(&import, "STRESS", "S33", 99), -1.20928e1);
    assert_eq!(value(&import, "STRESS", "S13", 99), -3.41818);
    assert!(value(&import, "STRESS", "MISES", 99) > 0.0);
    assert_eq!(increment.field("ERROR").unwrap().components[0].name, "STR");
}

#[test]
fn quadratic_hexahedra_get_input_file_node_order() {
    let import = read_frd(&testdata("block_c3d20r.frd")).unwrap();
    let inp = crate::inp::read_inp(&testdata("block_c3d20r.inp")).unwrap();
    for element in inp.mesh.elements() {
        assert_eq!(
            import.mesh.element(element.id).unwrap().nodes,
            element.nodes
        );
    }
}

/// Writes the mesh and the given nodal components as a binary frd with floats of `size` bytes.
fn binary_frd(import: &FrdImport, components: &[(&str, &str)], size: usize) -> Vec<u8> {
    let mesh = &import.mesh;
    let float = |out: &mut Vec<u8>, v: f64| {
        if size == 8 {
            out.extend(v.to_le_bytes());
        } else {
            out.extend((v as f32).to_le_bytes());
        }
    };
    let flag = if size == 8 { 3 } else { 2 };
    let mut out = b"    1C\n    1UMAT    1STEEL\n".to_vec();
    out.extend(format!("    2C{:>30}{:>37}\n", mesh.node_count(), flag).bytes());
    for (&id, xyz) in mesh.node_ids().iter().zip(mesh.coords()) {
        out.extend((id as i32).to_le_bytes());
        for &c in xyz {
            float(&mut out, c);
        }
    }
    out.extend(format!("    3C{:>30}{:>37}\n", mesh.element_count(), flag).bytes());
    for element in mesh.elements() {
        for v in [element.id as i32, 1, 0, 1] {
            out.extend(v.to_le_bytes());
        }
        for &n in &element.nodes {
            out.extend((n as i32).to_le_bytes());
        }
    }
    out.extend(b"    1PSTEP                         1           1           1\n");
    out.extend(
        format!(
            "  100CL  101 1.000000000{:>12}{:20}{:>2}{:>5}{:10}{:>2}\n",
            mesh.node_count(),
            "",
            0,
            1,
            "",
            flag
        )
        .bytes(),
    );
    out.extend(format!(" -4  DISP    {:>5}    1\n", components.len() + 1).bytes());
    for (k, (name, _)) in components.iter().enumerate() {
        let frd_name = ["D1", "D2", "D3"][k];
        let _ = name;
        out.extend(format!(" -5  {frd_name:<8}    1    2{:>5}    0\n", k + 1).bytes());
    }
    out.extend(b" -5  ALL         1    2    0    0    1ALL\n");
    let columns: Vec<&[f32]> = components
        .iter()
        .map(|(field, name)| {
            import.increments[0]
                .field(field)
                .unwrap()
                .component(name)
                .unwrap()
                .values
                .as_slice()
        })
        .collect();
    for (index, &id) in mesh.node_ids().iter().enumerate() {
        out.extend((id as i32).to_le_bytes());
        for column in &columns {
            float(&mut out, column[index] as f64);
        }
    }
    out.extend(b"9999\n");
    out
}

#[test]
fn reads_binary_files_with_single_and_double_precision() {
    let ascii = read_frd(&testdata("kragbalken_c3d8.frd")).unwrap();
    let disp = [("DISP", "U1"), ("DISP", "U2"), ("DISP", "U3")];
    for size in [4, 8] {
        let binary = read_frd_bytes(&binary_frd(&ascii, &disp, size)).unwrap();
        assert_eq!(binary.mesh.node_count(), ascii.mesh.node_count());
        assert_eq!(binary.mesh.element_count(), ascii.mesh.element_count());
        assert_eq!(binary.mesh.parts[0].name, "STEEL");
        for node in [1, 50, 99] {
            assert_eq!(
                value(&binary, "DISP", "U3", node),
                value(&ascii, "DISP", "U3", node)
            );
            assert_eq!(
                value(&binary, "DISP", "ALL", node),
                value(&ascii, "DISP", "ALL", node)
            );
        }
    }
}

#[test]
fn numbers_without_separating_blanks_are_split_by_column() {
    assert_eq!(
        fixed_floats(" -1        99-1.79459E-01-6.31509E-01", 13, 2),
        [-1.79459e-1, -6.31509e-1]
    );
    // Three-digit exponents of old Windows builds make negative numbers 13 columns wide.
    assert_eq!(
        fixed_floats(" -1         1-1.35800E+003 1.0000E+000", 13, 2),
        [-1358.0, 1.0]
    );
}

#[test]
fn truncated_binary_data_is_an_error() {
    let ascii = read_frd(&testdata("kragbalken_c3d8.frd")).unwrap();
    let mut bytes = binary_frd(&ascii, &[("DISP", "U1")], 4);
    bytes.truncate(400);
    assert!(read_frd_bytes(&bytes).is_err());
}
