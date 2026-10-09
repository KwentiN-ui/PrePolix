//! Selections on a mesh generated from CAD geometry by its faces, edges and vertices, as
//! PrePoMax's geometry based selection does. They are stored as CAD entities and found on
//! the mesh again after remeshing, see [`plx_model::Region::Geometry`].

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::Vec3;
use plx_mesh::{CadEntity, ElementId, FeMesh, NodeId};

use crate::model::{Highlight, Hit, Model};
use crate::selection::Target;
use crate::viewport::Preview;

/// Lookups from the mesh to the CAD entities, built once per mesh.
#[derive(Debug, Default)]
pub struct CadIndex {
    /// The CAD face of each element face; shell and 2D elements with face 1.
    face_of: HashMap<(ElementId, u8), i32>,
    /// The CAD edges bounding each CAD face.
    face_edges: BTreeMap<i32, Vec<i32>>,
    /// The CAD vertices at the ends of each CAD edge.
    edge_vertices: BTreeMap<i32, Vec<i32>>,
}

impl CadIndex {
    pub fn new(mesh: &FeMesh) -> Self {
        let cad = &mesh.cad;
        let nodes_of = |entity| -> BTreeSet<NodeId> {
            cad.nodes
                .get(&entity)
                .into_iter()
                .flatten()
                .copied()
                .collect()
        };
        let mut face_edges: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
        let mut edge_vertices: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
        let vertices: Vec<(i32, NodeId)> = (cad.nodes.iter())
            .filter_map(|(entity, nodes)| match entity {
                CadEntity::Vertex(tag) => Some((*tag, *nodes.first()?)),
                _ => None,
            })
            .collect();
        for &face in cad.faces.keys() {
            let on_face = nodes_of(CadEntity::Face(face));
            for (&edge, segments) in &cad.segments {
                if segments
                    .first()
                    .is_some_and(|s| s.iter().all(|n| on_face.contains(n)))
                {
                    face_edges.entry(face).or_default().push(edge);
                }
            }
        }
        for &edge in cad.segments.keys() {
            let on_edge = nodes_of(CadEntity::Edge(edge));
            edge_vertices.insert(
                edge,
                (vertices.iter())
                    .filter(|(_, node)| on_edge.contains(node))
                    .map(|&(tag, _)| tag)
                    .collect(),
            );
        }
        Self {
            face_of: cad.face_of().into_iter().collect(),
            face_edges,
            edge_vertices,
        }
    }

    /// The CAD face an element face of the skin lies on.
    fn face(&self, mesh: &FeMesh, (element, face): (ElementId, u8)) -> Option<i32> {
        if let Some(&tag) = self.face_of.get(&(element, face)) {
            return Some(tag);
        }
        // Shell and 2D elements lie on their CAD face as a whole.
        let element = mesh.element(element)?;
        (element.shape.faces().len() == 1)
            .then(|| self.face_of.get(&(element.id, 1)).copied())
            .flatten()
    }
}

/// What a click selects by geometry on a mesh generated from CAD geometry: the vertex or
/// edge near the click for regions of nodes, else the face under it; for the edges of 2D
/// models the nearest edge of the face. `None` where the mesh has no CAD entities.
pub fn pick(model: &Model, hit: &Hit, target: Target, precision: f32) -> Option<CadEntity> {
    let index = model.cad_index();
    let skin_face = model.skin(hit.part).faces.get(hit.face)?;
    let element = model.mesh.elements()[skin_face.element].id;
    let face = index.face(&model.mesh, (element, skin_face.face as u8 + 1))?;
    let edges = index.face_edges.get(&face).map_or(&[][..], Vec::as_slice);
    let nearest_edge = || {
        (edges.iter())
            .filter_map(|&edge| Some((edge, edge_distance(model, edge, hit.point)?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    };
    match target {
        Target::Faces => Some(CadEntity::Face(face)),
        Target::Edges => nearest_edge().map(|(edge, _)| CadEntity::Edge(edge)),
        Target::Nodes => {
            let vertices: BTreeSet<i32> = (edges.iter())
                .filter_map(|e| index.edge_vertices.get(e))
                .flatten()
                .copied()
                .collect();
            let vertex = (vertices.into_iter())
                .filter_map(|v| {
                    let node = *model.mesh.cad.nodes.get(&CadEntity::Vertex(v))?.first()?;
                    let position = model.render_position(model.mesh.node_index(node)?);
                    Some((v, position.distance(hit.point)))
                })
                .filter(|&(_, d)| d <= precision)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((vertex, _)) = vertex {
                return Some(CadEntity::Vertex(vertex));
            }
            match nearest_edge() {
                Some((edge, d)) if d <= precision => Some(CadEntity::Edge(edge)),
                _ => Some(CadEntity::Face(face)),
            }
        }
    }
}

/// Distance of a point to the mesh segments along a CAD edge, in render coordinates.
fn edge_distance(model: &Model, edge: i32, point: Vec3) -> Option<f32> {
    segment_positions(model, &[edge])
        .map(|[a, b]| {
            let ab = b - a;
            let t =
                ((point - a).dot(ab) / ab.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
            (a + ab * t).distance(point)
        })
        .min_by(f32::total_cmp)
}

fn segment_positions<'a>(
    model: &'a Model,
    edges: &'a [i32],
) -> impl Iterator<Item = [Vec3; 2]> + 'a {
    let position = |id| Some(model.render_position(model.mesh.node_index(id)?));
    (edges.iter())
        .filter_map(|e| model.mesh.cad.segments.get(e))
        .flatten()
        .filter_map(move |&[a, b]| Some([position(a)?, position(b)?]))
}

/// The CAD edges of faces and the edges themselves.
fn outline(model: &Model, entities: &BTreeSet<CadEntity>) -> Vec<i32> {
    let index = model.cad_index();
    let mut edges = BTreeSet::new();
    for entity in entities {
        match *entity {
            CadEntity::Face(face) => {
                edges.extend(index.face_edges.get(&face).into_iter().flatten().copied());
            }
            CadEntity::Edge(edge) => {
                edges.insert(edge);
            }
            CadEntity::Vertex(_) => {}
        }
    }
    edges.into_iter().collect()
}

/// The hover preview: faces by their outline, edges as lines and vertices as points.
pub fn preview(model: &Model, entities: &BTreeSet<CadEntity>) -> Preview {
    let edges = outline(model, entities);
    Preview {
        lines: segment_positions(model, &edges).collect(),
        points: vertex_nodes(model, entities)
            .filter_map(|n| model.mesh.node_index(n))
            .map(|n| model.render_position(n))
            .collect(),
    }
}

fn vertex_nodes<'a>(
    model: &'a Model,
    entities: &'a BTreeSet<CadEntity>,
) -> impl Iterator<Item = NodeId> + 'a {
    (entities.iter())
        .filter(|e| matches!(e, CadEntity::Vertex(_)))
        .filter_map(|e| model.mesh.cad.nodes.get(e))
        .flatten()
        .copied()
}

/// How a region of CAD entities shows selected: faces shaded, edges as lines, vertices as
/// points.
pub fn highlight(model: &Model, entities: &[CadEntity]) -> Highlight {
    let entities: BTreeSet<CadEntity> = entities.iter().copied().collect();
    let mut highlight = Highlight::default();
    let faces: Vec<CadEntity> = (entities.iter())
        .filter(|e| matches!(e, CadEntity::Face(_)))
        .copied()
        .collect();
    if !faces.is_empty() {
        let on_faces: BTreeSet<(ElementId, u8)> = (faces.iter())
            .filter_map(|e| match e {
                CadEntity::Face(tag) => model.mesh.cad.faces.get(tag),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        let elements: BTreeSet<ElementId> = on_faces.iter().map(|(e, _)| *e).collect();
        // Solid element faces show as they are; shell and 2D elements with all their faces.
        highlight.faces = model
            .skin_faces()
            .filter(|f| {
                on_faces.contains(f)
                    || (elements.contains(&f.0)
                        && model
                            .mesh
                            .element(f.0)
                            .is_some_and(|e| e.shape.faces().len() == 1))
            })
            .collect();
    }
    let edges: Vec<i32> = (entities.iter())
        .filter_map(|e| match e {
            CadEntity::Edge(tag) => Some(*tag),
            _ => None,
        })
        .collect();
    highlight.lines = (edges.iter())
        .filter_map(|e| model.mesh.cad.segments.get(e))
        .flatten()
        .copied()
        .collect();
    highlight.nodes = vertex_nodes(model, &entities).collect();
    highlight
}

#[cfg(test)]
mod tests {
    use plx_io::frd::read_frd;
    use plx_model::{
        BoundaryCondition, BoundaryKind, Elastic, FeModel, Load, LoadKind, Material, Region,
        Section, Step,
    };

    use super::*;
    use crate::model::load;
    use crate::selection::{Operation, Picker};
    use crate::setup::{FACE_SOURCES, NODE_SOURCES, RegionDraft};

    fn testdata(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata")
            .join(name)
    }

    fn gmsh_available() -> bool {
        match plx_mesher::self_test() {
            Ok(_) => true,
            Err(error) if std::env::var_os("PREPOLIX_REQUIRE_GMSH").is_none() => {
                eprintln!("Gmsh nicht verfügbar, Test übersprungen: {error}");
                false
            }
            Err(error) => panic!("Gmsh wird verlangt: {error}"),
        }
    }

    /// The region a click into the 3D view picks, from `from` along `direction` in model
    /// coordinates.
    fn click(model: &Model, target: Target, from: [f64; 3], direction: Vec3) -> Region {
        let sources = if target == Target::Nodes {
            NODE_SOURCES
        } else {
            FACE_SOURCES
        };
        let mut draft = RegionDraft::new(sources, target);
        let hit = model.pick(model.to_render(from), direction).unwrap();
        draft.click(
            model,
            &Picker::default(),
            Some((&hit, 0.1)),
            Operation::Replace,
        );
        draft.region()
    }

    fn coords(mesh: &FeMesh, nodes: &[NodeId]) -> Vec<[f64; 3]> {
        nodes.iter().map(|&n| mesh.node(n).unwrap()).collect()
    }

    /// The plate with a hole (100 x 40 x 10) fixed at x = 0 and pulled by a pressure on its
    /// end at x = 100, both picked on the geometry: meshed coarse, then the part remeshed
    /// fine. The regions find the end faces on both meshes, and CalculiX gives about the
    /// same elongation.
    #[test]
    fn selections_on_the_geometry_survive_remeshing() {
        if !gmsh_available() {
            return;
        }
        let mut model = load(
            &testdata("platte_mit_loch.step"),
            plx_model::UnitSystem::MmTonSC,
        )
        .unwrap()
        .model;
        let mut geometry = model.geometry.clone().unwrap();
        geometry.meshing.max_size = 10.0;
        model.set_mesh(plx_mesher::generate_mesh(&geometry).unwrap().mesh);

        let fixed = click(&model, Target::Nodes, [-50.0, 5.0, 5.0], Vec3::X);
        let pulled = click(&model, Target::Faces, [150.0, 5.0, 5.0], Vec3::NEG_X);
        assert!(
            matches!(&fixed, Region::Geometry(e) if e.len() == 1),
            "{fixed:?}"
        );
        assert!(
            matches!(&pulled, Region::Geometry(e) if e.len() == 1),
            "{pulled:?}"
        );
        // Near an edge of the end the click picks the edge, near a corner the vertex.
        let edge = click(&model, Target::Nodes, [-50.0, 5.0, 9.95], Vec3::X);
        assert!(matches!(&edge, Region::Geometry(e) if matches!(e[..], [CadEntity::Edge(_)])));
        let corner = click(&model, Target::Nodes, [-50.0, 0.05, 9.95], Vec3::X);
        assert_eq!(corner.nodes(&model.mesh).len(), 1, "{corner:?}");
        assert_eq!(
            coords(&model.mesh, &corner.nodes(&model.mesh)),
            [[0.0, 0.0, 10.0]]
        );

        // Mixed with picked nodes the geometry turns into nodes; undo goes back to it.
        let mut draft = RegionDraft::from_region(&fixed, NODE_SOURCES, Target::Nodes, &model.mesh);
        let extra = model.mesh.node_ids()[0];
        let items = crate::selection::Items::Nodes(std::collections::BTreeSet::from([extra]));
        draft.take(&model.mesh, items, Operation::Add);
        let mut expected = fixed.nodes(&model.mesh);
        expected.push(extra);
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(draft.region(), Region::Nodes(expected));
        let mut draft = RegionDraft::from_region(&fixed, NODE_SOURCES, Target::Nodes, &model.mesh);
        let hit = model
            .pick(model.to_render([150.0, 5.0, 5.0]), Vec3::NEG_X)
            .unwrap();
        draft.click(
            &model,
            &Picker::default(),
            Some((&hit, 0.1)),
            Operation::Add,
        );
        assert!(matches!(draft.region(), Region::Geometry(e) if e.len() == 2));
        draft.action(&model, crate::selection::PickerAction::Undo);
        assert_eq!(draft.region(), fixed);

        model.fe = FeModel {
            materials: vec![Material {
                name: "Steel".into(),
                density: None,
                elastic: Some(Elastic {
                    young: 210000.0,
                    poisson: 0.3,
                }),
                conductivity: None,
                specific_heat: None,
                expansion: None,
            }],
            sections: vec![Section {
                name: "Section-1".into(),
                material: "Steel".into(),
                region: Region::Parts(vec!["SOLID-1".into()]),
                thickness: 1.0,
            }],
            steps: vec![Step::new_static("Step-1")],
            ..FeModel::default()
        };
        let step = &mut model.fe.steps[0];
        step.boundary_conditions.push(BoundaryCondition {
            name: "Fix".into(),
            active: true,
            region: fixed.clone(),
            kind: BoundaryKind::Fixed,
        });
        step.loads.push(Load {
            name: "Pull".into(),
            active: true,
            region: pulled.clone(),
            kind: LoadKind::Pressure(-100.0),
        });

        let check = |model: &Model| {
            let mesh = &model.mesh;
            let nodes = fixed.nodes(mesh);
            assert!(fixed.missing_reference(mesh).is_none());
            assert!(coords(mesh, &nodes).iter().all(|c| c[0].abs() < 1e-9));
            let faces = pulled.faces(mesh);
            let on_end = Region::Faces(faces.clone()).nodes(mesh);
            assert!(
                coords(mesh, &on_end)
                    .iter()
                    .all(|c| (c[0] - 100.0).abs() < 1e-9)
            );
            (nodes.len(), faces.len())
        };
        let coarse = check(&model);
        let coarse_elongation = elongation(&model);

        // Finer, as after a first run, with the part's own meshing parameters.
        geometry.mesh_items.push(plx_model::MeshSetupItem {
            name: "Meshing_Parameters-1".into(),
            kind: plx_model::MeshSetupKind::MeshingParameters {
                parts: vec!["SOLID-1".into()],
                parameters: plx_model::MeshingParameters {
                    max_size: 3.0,
                    ..geometry.meshing
                },
            },
        });
        let part = plx_mesher::generate_part_mesh(&geometry, "SOLID-1").unwrap();
        let mesh = plx_mesher::merge_part(&model.mesh, part.mesh);
        model.set_mesh(mesh);
        let fine = check(&model);
        assert!(
            fine.0 > 2 * coarse.0 && fine.1 > 2 * coarse.1,
            "{coarse:?} -> {fine:?}"
        );
        let fine_elongation = elongation(&model);

        if let (Some(coarse), Some(fine)) = (coarse_elongation, fine_elongation) {
            // 100 MPa over 100 mm of steel stretch it by 0.048 mm; the hole adds a little.
            let plain = 100.0 / 210000.0 * 100.0;
            for value in [coarse, fine] {
                assert!(
                    value > plain && value < 1.3 * plain,
                    "{value} gegen {plain}"
                );
            }
            assert!((fine - coarse).abs() < 0.03 * fine, "{coarse} -> {fine}");
        }
    }

    /// The largest displacement along x CalculiX computes, if ccx is installed.
    fn elongation(model: &Model) -> Option<f32> {
        std::process::Command::new("ccx").arg("-v").output().ok()?;
        let inp = plx_io::inp::write_inp(&model.mesh, &model.fe, "Platte").unwrap();
        let dir = std::env::temp_dir().join(format!(
            "prepolix-remesh-{}-{}",
            std::process::id(),
            model.mesh.element_count()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("platte.inp"), inp).unwrap();
        let status = std::process::Command::new("ccx")
            .args(["-i", "platte"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(status.status.success(), "ccx: {status:?}");
        let results = read_frd(&dir.join("platte.frd")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let disp = results.increments.last().unwrap().field("DISP").unwrap();
        let values = &disp.component("U1").unwrap().values;
        Some(values.iter().copied().fold(f32::MIN, f32::max))
    }
}
