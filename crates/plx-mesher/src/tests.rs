//! These tests need the Gmsh library, see [`gmsh::candidates`]. Without it they pass with a
//! note, unless `PREPOLIX_REQUIRE_GMSH` is set, as in the CI.

use std::path::PathBuf;

use plx_model::{Algorithm2d, Algorithm3d, MeshSetupItem, MeshSetupKind};

use super::*;

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(name)
}

/// Whether Gmsh can be used; false if the test should be skipped.
fn gmsh_available() -> bool {
    match self_test() {
        Ok(_) => true,
        Err(error) if std::env::var_os("PREPOLIX_REQUIRE_GMSH").is_none() => {
            eprintln!("Gmsh nicht verfügbar, Test übersprungen: {error}");
            false
        }
        Err(error) => panic!("Gmsh wird verlangt: {error}"),
    }
}

fn corners(mesh: &FeMesh, element: &Element) -> [glam_free::V; 4] {
    [0, 1, 2, 3].map(|k| glam_free::V(mesh.node(element.nodes[k]).unwrap()))
}

/// Just enough vector maths for the checks, without a dependency.
mod glam_free {
    #[derive(Clone, Copy)]
    pub struct V(pub [f64; 3]);

    impl V {
        pub fn sub(self, o: V) -> V {
            V([0, 1, 2].map(|k| self.0[k] - o.0[k]))
        }
        pub fn mid(self, o: V) -> V {
            V([0, 1, 2].map(|k| 0.5 * (self.0[k] + o.0[k])))
        }
        pub fn dot(self, o: V) -> f64 {
            (0..3).map(|k| self.0[k] * o.0[k]).sum()
        }
        pub fn cross(self, o: V) -> V {
            let (a, b) = (self.0, o.0);
            V([
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ])
        }
        pub fn distance(self, o: V) -> f64 {
            let d = self.sub(o);
            d.dot(d).sqrt()
        }
    }
}

/// Volume of a tetrahedron, positive when nodes 1-2-3 run anticlockwise seen from node 4 as
/// CalculiX requires.
fn volume(mesh: &FeMesh, element: &Element) -> f64 {
    let [a, b, c, d] = corners(mesh, element);
    b.sub(a).cross(c.sub(a)).dot(d.sub(a)) / 6.0
}

#[test]
fn step_files_import_with_faces_and_edges() {
    if !gmsh_available() {
        return;
    }
    let import = import_cad(&testdata("platte_mit_loch.step")).unwrap();
    let display = &import.display;
    assert_eq!(display.solids, 1);
    // Four sides, top and bottom with the hole, and the hole's wall.
    assert!(display.faces >= 7, "{} Flächen", display.faces);
    assert!(display.edges >= 12, "{} Kanten", display.edges);
    assert_eq!(display.mesh.parts.len(), 1);
    assert_eq!(display.mesh.parts[0].name, "SOLID-1");
    let types: BTreeSet<&str> = (display.mesh.elements().iter())
        .map(|e| e.type_name.as_str())
        .collect();
    assert_eq!(types, BTreeSet::from(["B31", "S3"]));
    let faces: BTreeSet<i32> = (display.entities.iter())
        .filter_map(|e| match e {
            CadEntity::Face(tag) => Some(*tag),
            CadEntity::Edge(_) => None,
        })
        .collect();
    assert_eq!(faces.len(), display.faces);
    let first = display.mesh.elements()[0].id;
    assert!(matches!(display.entity(first), Some(CadEntity::Face(_))));
    assert_eq!(display.entity(0), None);

    let geometry = &import.geometry;
    assert_eq!(geometry.source, "platte_mit_loch.step");
    assert!(
        geometry.brep.starts_with("DBRep_DrawableShape"),
        "BREP-Text"
    );
    // 5 % of the diagonal of 100 x 40 x 10, rounded.
    assert_eq!(geometry.meshing.max_size, 5.0);
}

#[test]
fn quadratic_tetrahedra_follow_calculix_numbering() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = import_cad(&testdata("platte_mit_loch.step"))
        .unwrap()
        .geometry;
    geometry.meshing.max_size = 8.0;
    let mesh = generate_mesh(&geometry).unwrap().mesh;
    assert_eq!(mesh.parts.len(), 1);
    assert!(mesh.element_count() > 100);
    let mut total = 0.0;
    for element in mesh.elements() {
        assert_eq!(element.type_name, "C3D10");
        let v = volume(&mesh, element);
        assert!(v > 0.0, "Element {} ist verdreht", element.id);
        total += v;
        // Straight midside nodes lie halfway along CalculiX's edges 1-2, 2-3, 3-1, 1-4,
        // 2-4, 3-4.
        let node = |k: usize| glam_free::V(mesh.node(element.nodes[k]).unwrap());
        for (mid, (a, b)) in [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)]
            .into_iter()
            .enumerate()
        {
            let expected = node(a).mid(node(b));
            assert!(
                node(4 + mid).distance(expected) < 1e-6,
                "Element {}",
                element.id
            );
        }
    }
    let exact = 100.0 * 40.0 * 10.0 - std::f64::consts::PI * 8.0 * 8.0 * 10.0;
    assert!((total - exact).abs() / exact < 0.01, "Volumen {total}");
    assert!(mesh.missing_nodes().is_empty());
}

#[test]
fn every_solid_becomes_a_part_and_size_controls_the_count() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = import_cad(&testdata("zwei_bloecke.step")).unwrap().geometry;
    geometry.meshing.second_order = false;
    geometry.meshing.max_size = 5.0;
    let coarse = generate_mesh(&geometry).unwrap().mesh;
    let names: Vec<&str> = coarse.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["SOLID-1", "SOLID-2"]);
    assert!(coarse.elements().iter().all(|e| e.type_name == "C3D4"));
    assert!(coarse.elements().iter().all(|e| volume(&coarse, e) > 0.0));
    // The blocks do not touch, so their meshes share no node.
    let nodes = |part: &Part| -> BTreeSet<NodeId> {
        (part.elements.iter())
            .flat_map(|&id| coarse.element(id).unwrap().nodes.clone())
            .collect()
    };
    assert!(nodes(&coarse.parts[0]).is_disjoint(&nodes(&coarse.parts[1])));

    geometry.meshing.max_size = 1.25;
    let fine = generate_mesh(&geometry).unwrap().mesh;
    assert!(
        fine.element_count() > 3 * coarse.element_count(),
        "{} gegen {}",
        fine.element_count(),
        coarse.element_count()
    );
}

#[test]
fn a_remeshed_part_replaces_its_old_mesh_and_leaves_the_others() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = import_cad(&testdata("zwei_bloecke.step")).unwrap().geometry;
    geometry.meshing.second_order = false;
    geometry.meshing.max_size = 5.0;
    assert_eq!(part_names(&geometry).unwrap(), ["SOLID-1", "SOLID-2"]);
    let whole = generate_mesh(&geometry).unwrap().mesh;
    let old_first = whole.parts[0].elements.len();

    geometry.mesh_items.push(MeshSetupItem {
        name: "Meshing_Parameters-1".into(),
        kind: MeshSetupKind::MeshingParameters {
            parts: vec!["SOLID-1".into()],
            parameters: MeshingParameters {
                max_size: 1.25,
                second_order: false,
                ..geometry.meshing
            },
        },
    });
    let part = generate_part_mesh(&geometry, "SOLID-1").unwrap().mesh;
    let names: Vec<&str> = part.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["SOLID-1"]);
    let merged = merge_part(&whole, part);
    let names: Vec<&str> = merged.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["SOLID-1", "SOLID-2"], "the part keeps its place");
    let new_first = merged.parts[0].elements.len();
    assert!(new_first > 3 * old_first, "{new_first} gegen {old_first}");
    assert!(merged.missing_nodes().is_empty());
    // The second block is untouched, with its numbers.
    assert_eq!(merged.parts[1], whole.parts[1]);
    for &id in &whole.parts[1].elements {
        let element = merged.element(id).unwrap();
        assert_eq!(element, whole.element(id).unwrap());
        for &node in &element.nodes {
            assert_eq!(merged.node(node), whole.node(node));
        }
    }
    // The new elements are numbered after all old ones.
    let old_max = whole.elements().iter().map(|e| e.id).max().unwrap();
    assert!(merged.parts[0].elements.iter().all(|&e| e > old_max));
    let used: BTreeSet<NodeId> = (merged.elements().iter())
        .flat_map(|e| e.nodes.iter().copied())
        .collect();
    assert_eq!(used.len(), merged.node_count(), "no orphaned nodes");
    assert!(generate_part_mesh(&geometry, "SOLID-9").is_err());
}

#[test]
fn local_mesh_sizes_refine_faces_and_edges() {
    if !gmsh_available() {
        return;
    }
    let import = import_cad(&testdata("platte_mit_loch.step")).unwrap();
    let mut geometry = import.geometry;
    geometry.meshing.max_size = 8.0;
    geometry.meshing.second_order = false;
    let count = |geometry: &Geometry| {
        (generate_part_mesh(geometry, "SOLID-1").unwrap().mesh).element_count()
    };
    let coarse = count(&geometry);
    // The faces of the plate in the display; the first one is refined.
    let face = (import.display.entities.iter())
        .find_map(|e| match e {
            CadEntity::Face(tag) => Some(*tag),
            CadEntity::Edge(_) => None,
        })
        .unwrap();
    let local = |faces: Vec<i32>, edges: Vec<i32>, size| MeshSetupItem {
        name: "Local_Mesh_Size-1".into(),
        kind: MeshSetupKind::LocalMeshSize { faces, edges, size },
    };
    geometry.mesh_items = vec![local(vec![face], vec![], 1.0)];
    let on_face = count(&geometry);
    assert!(on_face > 2 * coarse, "{on_face} gegen {coarse}");
    let edge = (import.display.entities.iter())
        .find_map(|e| match e {
            CadEntity::Edge(tag) => Some(*tag),
            CadEntity::Face(_) => None,
        })
        .unwrap();
    geometry.mesh_items = vec![local(vec![], vec![edge], 0.5)];
    let on_edge = count(&geometry);
    assert!(on_edge > coarse, "{on_edge} gegen {coarse}");
    // Tags of other parts or faces that do not exist change nothing.
    geometry.mesh_items = vec![local(vec![9999], vec![], 1.0)];
    assert_eq!(count(&geometry), coarse);
}

#[test]
fn every_gmsh_algorithm_gives_valid_tetrahedra() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = import_cad(&testdata("platte_mit_loch.step"))
        .unwrap()
        .geometry;
    geometry.meshing.max_size = 8.0;
    let exact = 100.0 * 40.0 * 10.0 - std::f64::consts::PI * 8.0 * 8.0 * 10.0;
    for (algorithm_2d, algorithm_3d) in [
        (Algorithm2d::MeshAdapt, Algorithm3d::Hxt),
        (Algorithm2d::Delaunay, Algorithm3d::Frontal),
        (Algorithm2d::Automatic, Algorithm3d::Delaunay),
    ] {
        geometry.mesh_items = vec![MeshSetupItem {
            name: "Tetrahedral_Gmsh-1".into(),
            kind: MeshSetupKind::TetrahedralGmsh {
                parts: vec!["SOLID-1".into()],
                algorithm_2d,
                algorithm_3d,
            },
        }];
        let mesh = generate_mesh(&geometry).unwrap().mesh;
        let total: f64 = mesh.elements().iter().map(|e| volume(&mesh, e)).sum();
        assert!(
            mesh.elements().iter().all(|e| volume(&mesh, e) > 0.0),
            "{algorithm_2d:?} {algorithm_3d:?}"
        );
        assert!(
            (total - exact).abs() / exact < 0.01,
            "{algorithm_2d:?} {algorithm_3d:?}: {total}"
        );
    }
}

#[test]
fn invalid_sizes_and_files_are_reported() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = import_cad(&testdata("zwei_bloecke.step")).unwrap().geometry;
    geometry.meshing.max_size = 0.0;
    assert!(generate_mesh(&geometry).is_err());
    assert!(import_cad(&testdata("wuerfel_c3d10.inp")).is_err());
    assert!(import_cad(&testdata("gibt_es_nicht.step")).is_err());
}

#[test]
fn names_are_made_calculix_safe() {
    assert_eq!(
        calculix_name("Bracket v2.1").as_deref(),
        Some("BRACKET_V2_1")
    );
    assert_eq!(calculix_name("  "), None);
    assert_eq!(calculix_name("3D-Teil"), None);
    assert!(is_cad_file(std::path::Path::new("a/B.STEP")));
    assert!(!is_cad_file(std::path::Path::new("a/b.inp")));
}

/// A geometry of one rounded rectangle in the x-y plane, as a 2D model has it.
fn rectangle(origin: [f64; 3], size: [f64; 2], radius: f64) -> Geometry {
    let file = TempFile::new("brep");
    with_gmsh(|gmsh| {
        gmsh.add_rectangle(origin, size, radius)?;
        gmsh.write(&file.0)
    })
    .unwrap();
    Geometry {
        source: "rechteck.brep".into(),
        brep: std::fs::read_to_string(&file.0).unwrap(),
        meshing: MeshingParameters::for_diagonal(size[0].hypot(size[1])),
        mesh_items: Vec::new(),
        part_names: Vec::new(),
    }
}

/// Signed area of a triangle or quadrilateral in the x-y plane, positive counter-clockwise.
fn signed_area(mesh: &FeMesh, element: &Element) -> f64 {
    let n = if element.shape.edges().len() == 3 {
        3
    } else {
        4
    };
    let p: Vec<[f64; 3]> = (0..n)
        .map(|k| mesh.node(element.nodes[k]).unwrap())
        .collect();
    (0..n)
        .map(|k| {
            let (a, b) = (p[k], p[(k + 1) % n]);
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        / 2.0
}

#[test]
fn faces_outside_solids_are_meshed_as_shell_parts() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = rectangle([10.0, 0.0, 0.0], [20.0, 10.0], 2.0);
    assert_eq!(part_names(&geometry).unwrap(), ["SHELL-1"]);
    let display = tessellate(&geometry).unwrap();
    assert_eq!(display.mesh.parts[0].name, "SHELL-1");

    geometry.meshing.max_size = 2.0;
    let mesh = generate_mesh(&geometry).unwrap().mesh;
    assert_eq!(mesh.parts.len(), 1);
    assert!(mesh.elements().iter().all(|e| e.type_name == "S6"));
    // Midside nodes lie between their corners, as CalculiX numbers them.
    for element in mesh.elements() {
        for edge in element.shape.edges() {
            let p = |l: usize| mesh.node(element.nodes[l]).unwrap();
            let (a, b, m) = (p(edge.corners[0]), p(edge.corners[1]), p(edge.mids[0]));
            let off = (0..3)
                .map(|k| (m[k] - (a[k] + b[k]) / 2.0).abs())
                .fold(0.0, f64::max);
            assert!(off < 0.05, "Element {}: {off}", element.id);
        }
    }
    let area: f64 = mesh
        .elements()
        .iter()
        .map(|e| signed_area(&mesh, e).abs())
        .sum();
    let exact = 200.0 - (4.0 - std::f64::consts::PI) * 4.0;
    assert!((area - exact).abs() / exact < 0.01, "Fläche {area}");
    assert!(mesh.coords().iter().all(|c| c[2].abs() < 1e-9));

    geometry.meshing.second_order = false;
    geometry.meshing.quad_dominated = true;
    let quads = generate_mesh(&geometry).unwrap().mesh;
    let count = |name: &str| {
        quads
            .elements()
            .iter()
            .filter(|e| e.type_name == name)
            .count()
    };
    assert!(
        count("S4") > 10 * count("S3"),
        "{} S4, {} S3",
        count("S4"),
        count("S3")
    );
    geometry.meshing.second_order = true;
    let quadratic = generate_mesh(&geometry).unwrap().mesh;
    assert!(quadratic.elements().iter().any(|e| e.type_name == "S8"));
}

#[test]
fn a_deleted_part_leaves_the_others_with_their_names_and_local_sizes() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = import_cad(&testdata("zwei_bloecke.step")).unwrap().geometry;
    geometry.meshing.second_order = false;
    geometry.meshing.max_size = 5.0;
    // A face of each block; the second block's face must follow it into the new numbering.
    let file = TempFile::with_contents("brep", &geometry.brep).unwrap();
    let (first, second, second_box) = with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        let volumes = gmsh.entities(3)?;
        let first = gmsh.adjacencies(3, volumes[0])?.1[0];
        let second = *gmsh.adjacencies(3, volumes[1])?.1.last().unwrap();
        Ok((first, second, gmsh.entity_bounding_box(2, second)?))
    })
    .unwrap();
    geometry.mesh_items.push(MeshSetupItem {
        name: "Local_Mesh_Size-1".into(),
        kind: MeshSetupKind::LocalMeshSize {
            faces: vec![first, second],
            edges: Vec::new(),
            size: 1.0,
        },
    });

    let smaller = delete_part(&geometry, "SOLID-1").unwrap().unwrap();
    // Gmsh alone would call the block that is left SOLID-1.
    assert_eq!(part_names(&smaller).unwrap(), ["SOLID-2"]);
    let mesh = generate_mesh(&smaller).unwrap().mesh;
    let names: Vec<&str> = mesh.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["SOLID-2"]);
    let display = tessellate(&smaller).unwrap();
    assert_eq!(display.mesh.parts[0].name, "SOLID-2");
    let MeshSetupKind::LocalMeshSize { faces, .. } = &smaller.mesh_items[0].kind else {
        panic!("local mesh size expected");
    };
    assert_eq!(faces.len(), 1, "the face of the deleted block is dropped");
    let file = TempFile::with_contents("brep", &smaller.brep).unwrap();
    let moved = with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        gmsh.entity_bounding_box(2, faces[0])
    })
    .unwrap();
    assert!(same_box(&moved, &second_box));

    assert!(delete_part(&smaller, "SOLID-1").is_err());
    assert_eq!(delete_part(&smaller, "SOLID-2").unwrap(), None);
}

#[test]
fn a_deleted_mesh_part_takes_its_elements_and_nodes() {
    if !gmsh_available() {
        return;
    }
    let mut geometry = import_cad(&testdata("zwei_bloecke.step")).unwrap().geometry;
    geometry.meshing.second_order = false;
    geometry.meshing.max_size = 5.0;
    let whole = generate_mesh(&geometry).unwrap().mesh;
    let smaller = delete_mesh_part(&whole, "solid-1");
    assert_eq!(smaller.parts, whole.parts[1..]);
    assert_eq!(smaller.element_count(), whole.parts[1].elements.len());
    assert!(smaller.missing_nodes().is_empty());
    assert!(smaller.node_count() < whole.node_count());
}
