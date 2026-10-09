//! Geometry import and meshing with Gmsh, which brings OpenCASCADE for STEP, IGES and BREP.
//!
//! Gmsh is not linked but loaded at run time from its shared library (see [`gmsh`]), so
//! prepolix starts and works with meshes without it. Releases ship the library next to the
//! program; `scripts/fetch_gmsh.py` fetches it for development.

mod ffi;
pub mod gmsh;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use plx_mesh::{Element, ElementId, ElementShape, FeMesh, NodeId, Part, SurfaceDefinition};
use plx_model::{Algorithm2d, Algorithm3d, Geometry, MeshingParameters};

use gmsh::{Gmsh, with_gmsh};
pub use gmsh::{GmshError, LibraryInfo, loaded_library, set_library_path};

/// File extensions of the CAD formats that can be imported.
pub const CAD_EXTENSIONS: [&str; 5] = ["step", "stp", "iges", "igs", "brep"];

/// Gmsh's element type numbers.
const LINE2: i32 = 1;
const TRI3: i32 = 2;
const QUAD4: i32 = 3;
const TET4: i32 = 4;
const TRI6: i32 = 9;
const TET10: i32 = 11;
const QUAD8: i32 = 16;

/// Gmsh numbers the last two midside nodes of a quadratic tetrahedron the other way round
/// than CalculiX (edges 3-2, 3-1 against 2-4, 3-4 in 1-based CalculiX numbering).
const TET10_TO_CALCULIX: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 9, 8];

/// Whether a file is a CAD file by its extension.
pub fn is_cad_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| CAD_EXTENSIONS.iter().any(|c| e.eq_ignore_ascii_case(c)))
}

/// A face or edge of the CAD geometry, by Gmsh's tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CadEntity {
    Face(i32),
    Edge(i32),
}

/// A triangulation of the geometry for display: one part per solid, made of `S3` triangles
/// for its faces and `B31` lines for its edges.
#[derive(Clone, Debug, Default)]
pub struct GeometryDisplay {
    pub mesh: FeMesh,
    /// The CAD face or edge of each display element; element ids count from 1.
    pub entities: Vec<CadEntity>,
    pub solids: usize,
    pub faces: usize,
    pub edges: usize,
}

impl GeometryDisplay {
    pub fn entity(&self, element: ElementId) -> Option<CadEntity> {
        self.entities
            .get(usize::try_from(element).ok()?.checked_sub(1)?)
            .copied()
    }
}

/// An imported CAD file: the geometry for the project and its display.
#[derive(Debug)]
pub struct CadImport {
    pub geometry: Geometry,
    pub display: GeometryDisplay,
    /// What Gmsh warned about while reading.
    pub warnings: Vec<String>,
}

/// A mesh generated from the geometry.
#[derive(Debug)]
pub struct GeneratedMesh {
    pub mesh: FeMesh,
    pub warnings: Vec<String>,
}

/// Reads a STEP, IGES or BREP file. The mesh setup starts with PrePoMax's sizes for the
/// geometry's extent.
pub fn import_cad(path: &Path) -> Result<CadImport, GmshError> {
    let brep_file = TempFile::new("brep");
    let (diagonal, warnings) = with_gmsh(|gmsh| {
        let shapes = gmsh.import_shapes(path)?;
        if shapes.is_empty() {
            return Err(GmshError::Other(format!(
                "{} enthält keine Geometrie",
                path.display()
            )));
        }
        gmsh.write(&brep_file.0)?;
        let (min, max) = gmsh.bounding_box()?;
        let diagonal = (0..3)
            .map(|k| (max[k] - min[k]).powi(2))
            .sum::<f64>()
            .sqrt();
        Ok((diagonal, gmsh.warnings()?))
    })?;
    let brep = std::fs::read_to_string(&brep_file.0)
        .map_err(|e| GmshError::Other(format!("{}: {e}", brep_file.0.display())))?;
    let geometry = Geometry {
        source: path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        ),
        brep,
        meshing: MeshingParameters::for_diagonal(diagonal),
        mesh_items: Vec::new(),
        part_names: Vec::new(),
    };
    let display = tessellate(&geometry)?;
    Ok(CadImport {
        geometry,
        display,
        warnings,
    })
}

/// Triangulates the geometry for display, finely enough that curved faces look round.
pub fn tessellate(geometry: &Geometry) -> Result<GeometryDisplay, GmshError> {
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        let (min, max) = gmsh.bounding_box()?;
        let diagonal = (0..3)
            .map(|k| (max[k] - min[k]).powi(2))
            .sum::<f64>()
            .sqrt();
        let options = MeshOptions {
            max_size: diagonal / 15.0,
            min_size: 0.0,
            elements_per_2pi: 24.0,
            order: 1,
            straight_midside_nodes: true,
            netgen: false,
            algorithms: Default::default(),
            quads: false,
        };
        options.apply(gmsh)?;
        // A face Gmsh cannot mesh is left out of the display rather than failing the import.
        if let Err(error) = gmsh.generate(2) {
            log::warn!("Darstellung der Geometrie unvollständig: {error}");
        }
        display_mesh(gmsh, geometry)
    })
}

/// Meshes every part of the geometry, one after the other as PrePoMax does.
pub fn generate_mesh(geometry: &Geometry) -> Result<GeneratedMesh, GmshError> {
    let mut mesh = FeMesh::default();
    let mut warnings = Vec::new();
    for name in part_names(geometry)? {
        let part = generate_part_mesh(geometry, &name)?;
        mesh = merge_part(&mesh, part.mesh);
        warnings.extend(part.warnings);
    }
    Ok(GeneratedMesh { mesh, warnings })
}

/// The names of the parts, one per solid and one per face outside the solids, as the
/// display and the meshes name them.
pub fn part_names(geometry: &Geometry) -> Result<Vec<String>, GmshError> {
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        Ok(parts(gmsh, geometry)?.into_iter().map(|(_, n)| n).collect())
    })
}

/// The parts of the geometry Gmsh read, as (dimension, tag) with the name: the solids, then
/// the faces outside them. Names the geometry keeps since a part was deleted take
/// precedence over those Gmsh gives.
fn parts(gmsh: &Gmsh, geometry: &Geometry) -> Result<Vec<(Entity, String)>, GmshError> {
    let volumes = gmsh.entities(3)?;
    let faces = free_faces(gmsh)?;
    let mut names = solid_names(gmsh, &volumes)?;
    names.extend(shell_names(gmsh, &faces)?);
    if geometry.part_names.len() == names.len() {
        names.clone_from(&geometry.part_names);
    }
    let entities = (volumes.iter().map(|&v| (3, v))).chain(faces.iter().map(|&f| (2, f)));
    Ok(entities.zip(names).collect())
}

/// Deletes a part of the geometry, PrePoMax's Delete of a geometry part; `None` once no
/// part is left. The other parts keep their names. Gmsh numbers the faces and edges anew
/// when it reads the smaller geometry, so the local mesh sizes are renumbered after where
/// the faces and edges lie; those of the deleted part are dropped.
pub fn delete_part(geometry: &Geometry, part: &str) -> Result<Option<Geometry>, GmshError> {
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    let smaller = TempFile::new("brep");
    let (names, old_boxes) = with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        let mut parts = parts(gmsh, geometry)?;
        let index = (parts.iter().position(|(_, n)| n == part))
            .ok_or_else(|| GmshError::Other(format!("Die Geometrie hat kein Part {part}")))?;
        let boxes = entity_boxes(gmsh)?;
        let (entity, _) = parts.remove(index);
        gmsh.remove(&[entity])?;
        gmsh.write(&smaller.0)?;
        Ok((parts.into_iter().map(|(_, n)| n).collect::<Vec<_>>(), boxes))
    })?;
    if names.is_empty() {
        return Ok(None);
    }
    let brep = std::fs::read_to_string(&smaller.0)
        .map_err(|e| GmshError::Other(format!("{}: {e}", smaller.0.display())))?;
    let new_boxes = with_gmsh(|gmsh| {
        gmsh.import_shapes(&smaller.0)?;
        entity_boxes(gmsh)
    })?;
    let renumber = |dim: i32, tags: &mut Vec<i32>| {
        *tags = (tags.iter())
            .filter_map(|tag| {
                let (_, old) = old_boxes.iter().find(|(e, _)| *e == (dim, *tag))?;
                (new_boxes.iter())
                    .find(|((d, _), new)| *d == dim && same_box(old, new))
                    .map(|((_, t), _)| *t)
            })
            .collect();
    };
    let mut mesh_items = geometry.mesh_items.clone();
    for item in &mut mesh_items {
        if let plx_model::MeshSetupKind::LocalMeshSize { faces, edges, .. } = &mut item.kind {
            renumber(2, faces);
            renumber(1, edges);
        }
    }
    Ok(Some(Geometry {
        brep,
        mesh_items,
        part_names: names,
        ..geometry.clone()
    }))
}

/// A Gmsh entity as (dimension, tag).
type Entity = (i32, i32);

type BoundingBox = ([f64; 3], [f64; 3]);

/// Bounding box of every face and edge, by (dimension, tag).
fn entity_boxes(gmsh: &Gmsh) -> Result<Vec<(Entity, BoundingBox)>, GmshError> {
    let mut boxes = Vec::new();
    for dim in [1, 2] {
        for tag in gmsh.entities(dim)? {
            boxes.push(((dim, tag), gmsh.entity_bounding_box(dim, tag)?));
        }
    }
    Ok(boxes)
}

/// Whether two bounding boxes are those of the same entity, read twice.
fn same_box(a: &BoundingBox, b: &BoundingBox) -> bool {
    let size = (0..3).map(|k| a.1[k] - a.0[k]).fold(0.0, f64::max);
    let tolerance = 1e-6 * size.max(1.0);
    (0..3).all(|k| (a.0[k] - b.0[k]).abs() <= tolerance && (a.1[k] - b.1[k]).abs() <= tolerance)
}

/// Faces that bound no solid, the shell parts of a geometry such as a 2D cross-section.
fn free_faces(gmsh: &Gmsh) -> Result<Vec<i32>, GmshError> {
    let mut in_solid = BTreeSet::new();
    for volume in gmsh.entities(3)? {
        in_solid.extend(gmsh.adjacencies(3, volume)?.1);
    }
    Ok((gmsh.entities(2)?.into_iter())
        .filter(|f| !in_solid.contains(f))
        .collect())
}

/// Meshes one part with tetrahedra after its meshing parameters, Gmsh algorithms and local
/// mesh sizes. The mesh holds the one part; its numbers count from 1, see [`merge_part`].
pub fn generate_part_mesh(geometry: &Geometry, part: &str) -> Result<GeneratedMesh, GmshError> {
    let setup = geometry.parameters(part);
    if !(setup.max_size > 0.0 && setup.min_size >= 0.0) {
        return Err(GmshError::Other(format!(
            "{part}: Die maximale Elementgröße muss größer als 0 sein"
        )));
    }
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        let volumes = gmsh.entities(3)?;
        let Some(volume) = (parts(gmsh, geometry)?.into_iter())
            .find(|((dim, _), n)| *dim == 3 && n == part)
            .map(|((_, tag), _)| tag)
        else {
            return shell_part_mesh(gmsh, geometry, part);
        };
        // The other solids go, so that only this one is meshed.
        let others: Vec<(i32, i32)> = (volumes.iter())
            .filter(|&&v| v != volume)
            .map(|&v| (3, v))
            .collect();
        if !others.is_empty() {
            gmsh.remove(&others)?;
        }
        let options = MeshOptions::of_part(geometry, part);
        options.apply(gmsh)?;
        let faces: BTreeSet<i32> = gmsh.adjacencies(3, volume)?.1.into_iter().collect();
        local_sizes(gmsh, geometry, &faces)?;
        gmsh.generate(3)?;
        let order = options.order;
        let mesh = solid_mesh(gmsh, order, &[(volume, part.to_string())])?;
        Ok(GeneratedMesh {
            mesh,
            warnings: gmsh.warnings()?,
        })
    })
}

/// Meshes a face outside the solids with triangles, or mostly quadrilaterals if its meshing
/// parameters ask for them, typed as shells (`S3`, `S6`, `S4`, `S8`); a 2D model gives them
/// its own types. The mesh holds the one part, numbered from 1.
fn shell_part_mesh(
    gmsh: &Gmsh,
    geometry: &Geometry,
    part: &str,
) -> Result<GeneratedMesh, GmshError> {
    let faces = free_faces(gmsh)?;
    let face = (parts(gmsh, geometry)?.into_iter())
        .find(|((dim, _), n)| *dim == 2 && n == part)
        .map(|((_, tag), _)| tag)
        .ok_or_else(|| GmshError::Other(format!("Die Geometrie hat kein Part {part}")))?;
    // Only the face stays, so that only it is meshed.
    let others: Vec<(i32, i32)> = (gmsh.entities(3)?.into_iter().map(|v| (3, v)))
        .chain(faces.iter().filter(|&&f| f != face).map(|&f| (2, f)))
        .collect();
    if !others.is_empty() {
        gmsh.remove(&others)?;
    }
    let options = MeshOptions::of_part(geometry, part);
    options.apply(gmsh)?;
    local_sizes(gmsh, geometry, &BTreeSet::from([face]))?;
    gmsh.generate(2)?;
    let coords = node_coords(gmsh)?;
    let types: [(i32, ElementShape, &str); 2] = if options.order == 2 {
        [
            (TRI6, ElementShape::Tri6, "S6"),
            (QUAD8, ElementShape::Quad8, "S8"),
        ]
    } else {
        [
            (TRI3, ElementShape::Tri3, "S3"),
            (QUAD4, ElementShape::Quad4, "S4"),
        ]
    };
    let mut mesh = FeMesh::default();
    let mut part_elements = Part {
        name: part.to_string(),
        elements: Vec::new(),
    };
    for (gmsh_type, shape, type_name) in types {
        let (_, nodes) = gmsh.elements(gmsh_type, face)?;
        // Gmsh numbers the nodes of triangles and quadrilaterals as CalculiX does.
        for element in nodes.chunks_exact(shape.node_count()) {
            let mut ids = Vec::with_capacity(element.len());
            for &tag in element {
                let position = coords
                    .get(&tag)
                    .ok_or_else(|| GmshError::Other(format!("Knoten {tag} fehlt")))?;
                let id = node_id(tag)?;
                mesh.set_node(id, *position);
                ids.push(id);
            }
            let id = ElementId::try_from(mesh.element_count() + 1)
                .map_err(|_| GmshError::Other("zu viele Elemente".into()))?;
            mesh.add_element(Element {
                id,
                type_name: type_name.into(),
                shape,
                nodes: ids,
            })
            .map_err(|e| GmshError::Other(e.to_string()))?;
            part_elements.elements.push(id);
        }
    }
    if part_elements.elements.is_empty() {
        return Err(GmshError::Other("Gmsh hat keine Elemente erzeugt".into()));
    }
    mesh.parts.push(part_elements);
    Ok(GeneratedMesh {
        mesh,
        warnings: gmsh.warnings()?,
    })
}

/// The local mesh sizes on the given faces and their edges as Gmsh size fields; Gmsh takes
/// the smallest of them and the other size limits.
fn local_sizes(gmsh: &Gmsh, geometry: &Geometry, faces: &BTreeSet<i32>) -> Result<(), GmshError> {
    let mut edges = BTreeSet::new();
    for &face in faces {
        edges.extend(gmsh.adjacencies(2, face)?.1);
    }
    let mut fields = Vec::new();
    for (local_faces, local_edges, size) in geometry.local_sizes() {
        let of_solid = |tags: &[i32], own: &BTreeSet<i32>| -> Vec<f64> {
            (tags.iter())
                .filter(|t| own.contains(t))
                .map(|&t| f64::from(t))
                .collect()
        };
        let (local_faces, local_edges) =
            (of_solid(local_faces, faces), of_solid(local_edges, &edges));
        if size.is_nan() || size <= 0.0 || (local_faces.is_empty() && local_edges.is_empty()) {
            continue;
        }
        let field = gmsh.add_field("Constant")?;
        if !local_faces.is_empty() {
            gmsh.set_field_numbers(field, "SurfacesList", &local_faces)?;
        }
        if !local_edges.is_empty() {
            gmsh.set_field_numbers(field, "CurvesList", &local_edges)?;
        }
        gmsh.set_field_number(field, "VIn", size)?;
        gmsh.set_field_number(field, "VOut", 1e22)?;
        gmsh.set_field_number(field, "IncludeBoundary", 1.0)?;
        fields.push(field);
    }
    let background = match fields.as_slice() {
        [] => return Ok(()),
        [field] => *field,
        _ => {
            let min = gmsh.add_field("Min")?;
            let list: Vec<f64> = fields.iter().map(|&f| f64::from(f)).collect();
            gmsh.set_field_numbers(min, "FieldsList", &list)?;
            min
        }
    };
    gmsh.set_background_field(background)
}

/// Puts a newly meshed part into a mesh in place of the part of the same name. The other
/// parts keep their node and element numbers; the new part is numbered after the highest
/// numbers the mesh had, so that nothing that referred to the old part points into the new
/// one.
pub fn merge_part(mesh: &FeMesh, part_mesh: FeMesh) -> FeMesh {
    let replaced: Vec<String> = part_mesh.parts.iter().map(|p| p.name.clone()).collect();
    let is_replaced = |p: &Part| replaced.iter().any(|r| r.eq_ignore_ascii_case(&p.name));
    let removed: BTreeSet<ElementId> = (mesh.parts.iter())
        .filter(|p| is_replaced(p))
        .flat_map(|p| p.elements.iter().copied())
        .collect();
    let node_offset = mesh.node_ids().iter().copied().max().unwrap_or(0);
    let element_offset = mesh.elements().iter().map(|e| e.id).max().unwrap_or(0);

    let mut merged = FeMesh::default();
    let kept: Vec<&Element> = (mesh.elements().iter())
        .filter(|e| !removed.contains(&e.id))
        .collect();
    let used: BTreeSet<NodeId> = kept.iter().flat_map(|e| e.nodes.iter().copied()).collect();
    let removed_nodes: BTreeSet<NodeId> = (mesh.elements().iter())
        .filter(|e| removed.contains(&e.id))
        .flat_map(|e| e.nodes.iter().copied())
        .filter(|n| !used.contains(n))
        .collect();
    for (&id, &coords) in mesh.node_ids().iter().zip(mesh.coords()) {
        if !removed_nodes.contains(&id) {
            merged.set_node(id, coords);
        }
    }
    for element in kept {
        // The elements come from a valid mesh.
        let _ = merged.add_element(element.clone());
    }
    for (&id, &coords) in part_mesh.node_ids().iter().zip(part_mesh.coords()) {
        merged.set_node(id + node_offset, coords);
    }
    for element in part_mesh.elements() {
        let _ = merged.add_element(Element {
            id: element.id + element_offset,
            nodes: element.nodes.iter().map(|n| n + node_offset).collect(),
            ..element.clone()
        });
    }
    let mut new_parts: Vec<Part> = (part_mesh.parts.into_iter())
        .map(|p| Part {
            elements: p.elements.iter().map(|e| e + element_offset).collect(),
            ..p
        })
        .collect();
    for part in &mesh.parts {
        if !is_replaced(part) {
            merged.parts.push(part.clone());
        } else if let Some(index) =
            (new_parts.iter()).position(|p| p.name.eq_ignore_ascii_case(&part.name))
        {
            merged.parts.push(new_parts.remove(index));
        }
    }
    merged.parts.extend(new_parts);
    merged.node_sets = (mesh.node_sets.iter())
        .map(|(name, nodes)| {
            let nodes = nodes.iter().copied().filter(|n| !removed_nodes.contains(n));
            (name.clone(), nodes.collect())
        })
        .collect();
    merged.element_sets = (mesh.element_sets.iter())
        .map(|(name, elements)| {
            let elements = elements.iter().copied().filter(|e| !removed.contains(e));
            (name.clone(), elements.collect())
        })
        .collect();
    merged.surfaces = (mesh.surfaces.iter())
        .map(|(name, surface)| {
            let surface = match surface {
                SurfaceDefinition::ElementFaces(faces) => SurfaceDefinition::ElementFaces(
                    faces
                        .iter()
                        .copied()
                        .filter(|(e, _)| !removed.contains(e))
                        .collect(),
                ),
                SurfaceDefinition::Nodes(nodes) => SurfaceDefinition::Nodes(
                    nodes
                        .iter()
                        .copied()
                        .filter(|n| !removed_nodes.contains(n))
                        .collect(),
                ),
            };
            (name.clone(), surface)
        })
        .collect();
    merged
}

/// Deletes a part of a mesh with its elements and the nodes no other part uses, PrePoMax's
/// Delete of a mesh part. Sets and surfaces lose them too; the other parts keep their
/// numbers.
pub fn delete_mesh_part(mesh: &FeMesh, part: &str) -> FeMesh {
    let mut empty = FeMesh::default();
    empty.parts.push(Part {
        name: part.to_string(),
        elements: Vec::new(),
    });
    let mut smaller = merge_part(mesh, empty);
    smaller.parts.retain(|p| !p.name.eq_ignore_ascii_case(part));
    smaller
}

/// Loads Gmsh and meshes a cube, for the self test in the settings.
pub fn self_test() -> Result<(LibraryInfo, usize), GmshError> {
    with_gmsh(|gmsh| {
        gmsh.add_box([0.0; 3], [10.0; 3])?;
        let options = MeshOptions {
            max_size: 2.5,
            min_size: 0.0,
            elements_per_2pi: 0.0,
            order: 2,
            straight_midside_nodes: true,
            netgen: false,
            algorithms: Default::default(),
            quads: false,
        };
        options.apply(gmsh)?;
        gmsh.generate(3)?;
        let volumes = gmsh.entities(3)?;
        let named: Vec<(i32, String)> =
            volumes.iter().map(|&v| (v, format!("SOLID-{v}"))).collect();
        let mesh = solid_mesh(gmsh, 2, &named)?;
        Ok((gmsh.info().clone(), mesh.element_count()))
    })
}

/// The meshing options prepolix sets.
struct MeshOptions {
    max_size: f64,
    min_size: f64,
    elements_per_2pi: f64,
    order: i32,
    straight_midside_nodes: bool,
    netgen: bool,
    algorithms: (Algorithm2d, Algorithm3d),
    /// Recombines triangles into quadrilaterals where Gmsh can (faces only).
    quads: bool,
}

impl MeshOptions {
    /// The options of a part after its meshing parameters and Gmsh algorithms.
    fn of_part(geometry: &Geometry, part: &str) -> Self {
        let setup = geometry.parameters(part);
        let curvature = if setup.elements_per_curvature > 0.0 {
            (std::f64::consts::TAU * setup.elements_per_curvature).round()
        } else {
            0.0
        };
        Self {
            max_size: setup.max_size,
            min_size: setup.min_size.min(setup.max_size),
            elements_per_2pi: curvature,
            order: if setup.second_order { 2 } else { 1 },
            straight_midside_nodes: !setup.midside_nodes_on_geometry,
            netgen: setup.optimize,
            algorithms: geometry.algorithms(part),
            quads: setup.quad_dominated,
        }
    }

    /// Sets every option; Gmsh keeps options between uses.
    fn apply(&self, gmsh: &Gmsh) -> Result<(), GmshError> {
        let (algorithm_2d, algorithm_3d) = self.algorithms;
        // Gmsh's numbers of the algorithms.
        let algorithm_2d = match algorithm_2d {
            Algorithm2d::MeshAdapt => 1.0,
            Algorithm2d::Automatic => 2.0,
            Algorithm2d::Delaunay => 5.0,
            Algorithm2d::FrontalDelaunay => 6.0,
        };
        let algorithm_3d = match algorithm_3d {
            Algorithm3d::Delaunay => 1.0,
            Algorithm3d::Frontal => 4.0,
            Algorithm3d::Hxt => 10.0,
        };
        for (name, value) in [
            ("Mesh.MeshSizeMax", self.max_size),
            ("Mesh.MeshSizeMin", self.min_size),
            ("Mesh.MeshSizeFromCurvature", self.elements_per_2pi),
            // Gmsh gives the points of a shape sizes of its own when it reads it or removes
            // other shapes; the sizes come from the parameters and size fields only.
            ("Mesh.MeshSizeFromPoints", 0.0),
            ("Mesh.MeshSizeExtendFromBoundary", 1.0),
            ("Mesh.ElementOrder", f64::from(self.order)),
            (
                "Mesh.SecondOrderLinear",
                f64::from(u8::from(self.straight_midside_nodes)),
            ),
            ("Mesh.HighOrderOptimize", 0.0),
            ("Mesh.Algorithm", algorithm_2d),
            ("Mesh.Algorithm3D", algorithm_3d),
            ("Mesh.Optimize", 1.0),
            ("Mesh.OptimizeNetgen", f64::from(u8::from(self.netgen))),
            ("Mesh.RecombineAll", f64::from(u8::from(self.quads))),
            // Quadratic quadrilaterals with 8 nodes, as CalculiX has them.
            ("Mesh.SecondOrderIncomplete", 1.0),
        ] {
            gmsh.set_number(name, value)?;
        }
        Ok(())
    }
}

/// Node coordinates of the generated mesh by Gmsh node tag.
fn node_coords(gmsh: &Gmsh) -> Result<HashMap<usize, [f64; 3]>, GmshError> {
    let (tags, coords) = gmsh.nodes()?;
    Ok(tags
        .into_iter()
        .zip(coords.as_chunks::<3>().0)
        .map(|(tag, &c)| (tag, c))
        .collect())
}

fn node_id(tag: usize) -> Result<NodeId, GmshError> {
    NodeId::try_from(tag).map_err(|_| GmshError::Other("zu viele Knoten".into()))
}

/// The tetrahedra of the solids given with their part names, one part per solid.
fn solid_mesh(gmsh: &Gmsh, order: i32, volumes: &[(i32, String)]) -> Result<FeMesh, GmshError> {
    if volumes.is_empty() {
        return Err(GmshError::Other(
            "Die Geometrie enthält keine Volumenkörper".into(),
        ));
    }
    let coords = node_coords(gmsh)?;
    let (gmsh_type, shape, type_name) = if order == 2 {
        (TET10, ElementShape::Tet10, "C3D10")
    } else {
        (TET4, ElementShape::Tet4, "C3D4")
    };
    let mut mesh = FeMesh::default();
    let mut next_id: ElementId = 1;
    for (volume, name) in volumes {
        let (_, nodes) = gmsh.elements(gmsh_type, *volume)?;
        let mut part = Part {
            name: name.clone(),
            elements: Vec::with_capacity(nodes.len() / shape.node_count()),
        };
        for tet in nodes.chunks_exact(shape.node_count()) {
            let mut ids = Vec::with_capacity(tet.len());
            for k in 0..tet.len() {
                let tag = if order == 2 {
                    tet[TET10_TO_CALCULIX[k]]
                } else {
                    tet[k]
                };
                let position = coords
                    .get(&tag)
                    .ok_or_else(|| GmshError::Other(format!("Knoten {tag} fehlt")))?;
                let id = node_id(tag)?;
                mesh.set_node(id, *position);
                ids.push(id);
            }
            mesh.add_element(Element {
                id: next_id,
                type_name: type_name.into(),
                shape,
                nodes: ids,
            })
            .map_err(|e| GmshError::Other(e.to_string()))?;
            part.elements.push(next_id);
            next_id += 1;
        }
        if !part.elements.is_empty() {
            mesh.parts.push(part);
        }
    }
    if mesh.element_count() == 0 {
        return Err(GmshError::Other("Gmsh hat keine Elemente erzeugt".into()));
    }
    Ok(mesh)
}

/// Part names from the names in the CAD file, else "SOLID-n"; upper case and unique, as
/// CalculiX names are case-insensitive.
fn solid_names(gmsh: &Gmsh, volumes: &[i32]) -> Result<Vec<String>, GmshError> {
    let mut names = Vec::with_capacity(volumes.len());
    let mut used = BTreeSet::new();
    for (index, &volume) in volumes.iter().enumerate() {
        let label = gmsh.entity_name(3, volume)?;
        let base = calculix_name(label.rsplit('/').next().unwrap_or_default())
            .unwrap_or_else(|| format!("SOLID-{}", index + 1));
        let mut name = base.clone();
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{base}-{n}");
            n += 1;
        }
        names.push(name);
    }
    Ok(names)
}

/// Names of the shell parts like [`solid_names`], "SHELL-n" where the file names none.
fn shell_names(gmsh: &Gmsh, faces: &[i32]) -> Result<Vec<String>, GmshError> {
    let mut names = Vec::with_capacity(faces.len());
    let mut used = BTreeSet::new();
    for (index, &face) in faces.iter().enumerate() {
        let label = gmsh.entity_name(2, face)?;
        let base = calculix_name(label.rsplit('/').next().unwrap_or_default())
            .unwrap_or_else(|| format!("SHELL-{}", index + 1));
        let mut name = base.clone();
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{base}-{n}");
            n += 1;
        }
        names.push(name);
    }
    Ok(names)
}

/// A name CalculiX accepts: letters, digits, `-` and `_`, upper case, at most 60 characters.
fn calculix_name(label: &str) -> Option<String> {
    let name: String = label
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let name = name.trim_matches('_').to_string();
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        .then_some(name)
}

/// Builds the display mesh from Gmsh's surface mesh: per solid its faces as triangles and its
/// edges as lines. Faces outside any solid form one more part.
fn display_mesh(gmsh: &Gmsh, geometry: &Geometry) -> Result<GeometryDisplay, GmshError> {
    let coords = node_coords(gmsh)?;
    let surfaces = gmsh.entities(2)?;
    let curves = gmsh.entities(1)?;
    let mut groups: Vec<(String, Vec<i32>)> = Vec::new();
    // Every face outside the solids is a shell part of its own, as for 2D models.
    for ((dim, tag), name) in parts(gmsh, geometry)? {
        let faces = match dim {
            3 => gmsh.adjacencies(3, tag)?.1,
            _ => vec![tag],
        };
        groups.push((name, faces));
    }

    // Triangles and edge segments of each entity, fetched once.
    let mut triangles: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for &surface in &surfaces {
        triangles.insert(surface, gmsh.elements(TRI3, surface)?.1);
    }
    let mut segments: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for &curve in &curves {
        segments.insert(curve, gmsh.elements(LINE2, curve)?.1);
    }

    let mut mesh = FeMesh::default();
    let mut entities = Vec::new();
    let mut add = |mesh: &mut FeMesh,
                   part: &mut Part,
                   entity: CadEntity,
                   nodes: &[usize]|
     -> Result<(), GmshError> {
        let (shape, type_name) = match entity {
            CadEntity::Face(_) => (ElementShape::Tri3, "S3"),
            CadEntity::Edge(_) => (ElementShape::Line2, "B31"),
        };
        let mut ids = Vec::with_capacity(nodes.len());
        for &tag in nodes {
            let id = node_id(tag)?;
            if let Some(position) = coords.get(&tag) {
                mesh.set_node(id, *position);
            }
            ids.push(id);
        }
        let id = ElementId::try_from(entities.len() + 1)
            .map_err(|_| GmshError::Other("zu viele Elemente".into()))?;
        entities.push(entity);
        part.elements.push(id);
        mesh.add_element(Element {
            id,
            type_name: type_name.into(),
            shape,
            nodes: ids,
        })
        .map_err(|e| GmshError::Other(e.to_string()))
    };
    for (name, faces) in groups {
        let mut part = Part {
            name,
            elements: Vec::new(),
        };
        let mut edges = BTreeSet::new();
        for &face in &faces {
            let nodes = triangles.get(&face).map_or(&[][..], Vec::as_slice);
            for triangle in nodes.as_chunks::<3>().0 {
                add(&mut mesh, &mut part, CadEntity::Face(face), triangle)?;
            }
            edges.extend(gmsh.adjacencies(2, face)?.1);
        }
        for edge in edges {
            let nodes = segments.get(&edge).map_or(&[][..], Vec::as_slice);
            for segment in nodes.as_chunks::<2>().0 {
                add(&mut mesh, &mut part, CadEntity::Edge(edge), segment)?;
            }
        }
        if !part.elements.is_empty() {
            mesh.parts.push(part);
        }
    }
    Ok(GeometryDisplay {
        mesh,
        entities,
        solids: gmsh.entities(3)?.len(),
        faces: surfaces.len(),
        edges: curves.len(),
    })
}

/// A file in the temporary directory, deleted when dropped; Gmsh reads and writes only files.
struct TempFile(PathBuf);

impl TempFile {
    fn new(extension: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "prepolix-gmsh-{}-{n}.{extension}",
            std::process::id()
        )))
    }

    fn with_contents(extension: &str, contents: &str) -> Result<Self, GmshError> {
        let file = Self::new(extension);
        std::fs::write(&file.0, contents)
            .map_err(|e| GmshError::Other(format!("{}: {e}", file.0.display())))?;
        Ok(file)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests;
