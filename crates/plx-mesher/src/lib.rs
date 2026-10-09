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

pub use plx_mesh::CadEntity;
use plx_mesh::{CadMap, Element, ElementId, ElementShape, FeMesh, NodeId, Part, SurfaceDefinition};
use plx_model::{Algorithm2d, Algorithm3d, Geometry, MeshingParameters, UnitSystem};

use gmsh::{Gmsh, with_gmsh};
pub use gmsh::{GmshError, LibraryInfo, loaded_library, set_library_path};

/// File extensions of the CAD formats that can be imported.
pub const CAD_EXTENSIONS: [&str; 5] = ["step", "stp", "iges", "igs", "brep"];

/// Gmsh's element type numbers.
const LINE2: i32 = 1;
const LINE3: i32 = 8;
const TRI3: i32 = 2;
const QUAD4: i32 = 3;
const TET4: i32 = 4;
const TRI6: i32 = 9;
const TET10: i32 = 11;
const POINT: i32 = 15;
const QUAD8: i32 = 16;

/// Gmsh numbers the last two midside nodes of a quadratic tetrahedron the other way round
/// than CalculiX (edges 3-2, 3-1 against 2-4, 3-4 in 1-based CalculiX numbering).
const TET10_TO_CALCULIX: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 9, 8];

/// Whether a file is a CAD file by its extension.
pub fn is_cad_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| CAD_EXTENSIONS.iter().any(|c| e.eq_ignore_ascii_case(c)))
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

/// Reads a STEP, IGES or BREP file. STEP and IGES files are converted to the length unit of
/// the model's unit system, millimetres without one; BREP files have no unit. The mesh setup
/// starts with PrePoMax's sizes for the geometry's extent.
pub fn import_cad(path: &Path, units: UnitSystem) -> Result<CadImport, GmshError> {
    import_cad_files(&[path.to_path_buf()], units)
}

/// Reads several STEP, IGES or BREP files into one geometry, like PrePoMax's import of
/// several selected files: their parts sit side by side, numbered on from file to file.
pub fn import_cad_files(paths: &[PathBuf], units: UnitSystem) -> Result<CadImport, GmshError> {
    if paths.is_empty() {
        return Err(GmshError::Other("Keine Datei gewählt".into()));
    }
    let brep_file = TempFile::new("brep");
    let (diagonal, warnings) = with_gmsh(|gmsh| {
        read_files(gmsh, paths, units)?;
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
        source: file_names(paths),
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

/// Reads CAD files into the open Gmsh model, STEP and IGES in the length unit of `units`.
fn read_files(gmsh: &Gmsh, paths: &[PathBuf], units: UnitSystem) -> Result<(), GmshError> {
    let unit = match units {
        UnitSystem::MKgSC | UnitSystem::MTonSC => "M",
        UnitSystem::InLbSF => "INCH",
        UnitSystem::MmTonSC | UnitSystem::Unitless => "MM",
    };
    gmsh.set_string("Geometry.OCCTargetUnit", unit)?;
    let imported = paths.iter().try_for_each(|path| {
        let shapes = gmsh.import_shapes(path)?;
        if shapes.is_empty() {
            return Err(GmshError::Other(format!(
                "{} enthält keine Geometrie",
                path.display()
            )));
        }
        Ok(())
    });
    gmsh.set_string("Geometry.OCCTargetUnit", "MM")?;
    imported
}

/// The files' names, for [`Geometry::source`].
fn file_names(paths: &[PathBuf]) -> String {
    (paths.iter())
        .map(|path| {
            path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// CAD files added to a geometry by [`add_cad_files`].
#[derive(Debug)]
pub struct CadAddition {
    /// The geometry with the old and the new parts.
    pub import: CadImport,
    /// The new tag of every face, edge and vertex of the old geometry, by its old one.
    pub renumbered: BTreeMap<CadEntity, CadEntity>,
    /// Names of the new parts.
    pub added: Vec<String>,
}

/// Adds the parts of CAD files to a geometry, PrePoMax's import into a model that has
/// one, or into a model with a mesh but no geometry (`None`). The parts already there keep
/// their names, local mesh sizes and selections (through [`CadAddition::renumbered`]); the
/// new ones get names neither the geometry nor `taken`, e.g. the mesh parts of the model,
/// uses.
pub fn add_cad_files(
    geometry: Option<&Geometry>,
    paths: &[PathBuf],
    units: UnitSystem,
    taken: &[String],
) -> Result<CadAddition, GmshError> {
    let Some(geometry) = geometry else {
        let mut import = import_cad_files(paths, units)?;
        let taken = taken.iter().cloned().collect();
        let added = fresh_names(&default_part_kinds(&import.geometry)?, taken);
        import.geometry.part_names.clone_from(&added);
        return Ok(CadAddition {
            import,
            renumbered: BTreeMap::new(),
            added,
        });
    };
    if paths.is_empty() {
        return Err(GmshError::Other("Keine Datei gewählt".into()));
    }
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    let combined = TempFile::new("brep");
    let (old_parts, old_boxes, warnings) = with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        let parts = part_boxes(gmsh, parts(gmsh, geometry)?)?;
        let boxes = entity_boxes(gmsh)?;
        read_files(gmsh, paths, units)?;
        gmsh.write(&combined.0)?;
        Ok((parts, boxes, gmsh.warnings()?))
    })?;
    let brep = std::fs::read_to_string(&combined.0)
        .map_err(|e| GmshError::Other(format!("{}: {e}", combined.0.display())))?;
    let (new_parts, new_boxes) = with_gmsh(|gmsh| {
        gmsh.import_shapes(&combined.0)?;
        Ok((part_boxes(gmsh, default_parts(gmsh)?)?, entity_boxes(gmsh)?))
    })?;
    // The old parts are found again by where they lie, the new ones are numbered on.
    let mut used: BTreeSet<String> = (old_parts.iter().map(|(_, n, _)| n.clone()))
        .chain(taken.iter().cloned())
        .collect();
    let mut old_parts: Vec<_> = old_parts.into_iter().map(Some).collect();
    let mut names = Vec::with_capacity(new_parts.len());
    let mut added = Vec::new();
    for ((dim, _), _, bounds) in &new_parts {
        let found = old_parts.iter_mut().find(|old| {
            old.as_ref()
                .is_some_and(|((d, _), _, b)| d == dim && same_box(b, bounds))
        });
        if let Some((_, name, _)) = found.and_then(Option::take) {
            names.push(name);
            continue;
        }
        let name = fresh_name(*dim, &used);
        used.insert(name.clone());
        names.push(name.clone());
        added.push(name);
    }
    let mut mesh_items = geometry.mesh_items.clone();
    renumber_mesh_items(&mut mesh_items, &old_boxes, &new_boxes);
    let combined = Geometry {
        source: format!("{}, {}", geometry.source, file_names(paths)),
        brep,
        mesh_items,
        part_names: names,
        ..geometry.clone()
    };
    let display = tessellate(&combined)?;
    Ok(CadAddition {
        import: CadImport {
            geometry: combined,
            display,
            warnings,
        },
        renumbered: renumbering(&old_boxes, &new_boxes),
        added,
    })
}

/// The geometry enlarged by `factor` about the origin, for a model whose length unit
/// changes. Faces and edges keep their tags.
pub fn scale_geometry(geometry: &Geometry, factor: f64) -> Result<Geometry, GmshError> {
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    let scaled = TempFile::new("brep");
    with_gmsh(|gmsh| {
        gmsh.set_number("Geometry.OCCScaling", factor)?;
        let imported = gmsh.import_shapes(&file.0);
        gmsh.set_number("Geometry.OCCScaling", 1.0)?;
        imported?;
        gmsh.write(&scaled.0)
    })?;
    let brep = std::fs::read_to_string(&scaled.0)
        .map_err(|e| GmshError::Other(format!("{}: {e}", scaled.0.display())))?;
    Ok(Geometry {
        brep,
        ..geometry.clone()
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

/// The names of the parts, one per solid, one per face outside the solids and one per edge
/// outside the faces, as the display and the meshes name them.
pub fn part_names(geometry: &Geometry) -> Result<Vec<String>, GmshError> {
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        Ok(parts(gmsh, geometry)?.into_iter().map(|(_, n)| n).collect())
    })
}

/// The parts of the geometry Gmsh read, as (dimension, tag) with the name: the solids, then
/// the faces outside them, then the edges outside the faces. Names the geometry keeps since
/// a part was deleted take precedence over those Gmsh gives.
fn parts(gmsh: &Gmsh, geometry: &Geometry) -> Result<Vec<(Entity, String)>, GmshError> {
    let mut parts = default_parts(gmsh)?;
    if geometry.part_names.len() == parts.len() {
        for ((_, name), kept) in parts.iter_mut().zip(&geometry.part_names) {
            name.clone_from(kept);
        }
    }
    Ok(parts)
}

/// The parts Gmsh read, as [`parts`], with the names Gmsh gives.
fn default_parts(gmsh: &Gmsh) -> Result<Vec<(Entity, String)>, GmshError> {
    let volumes = gmsh.entities(3)?;
    let faces = free_faces(gmsh)?;
    let edges = free_edges(gmsh)?;
    let mut names = solid_names(gmsh, &volumes)?;
    names.extend(shell_names(gmsh, &faces)?);
    names.extend(line_names(gmsh, &edges)?);
    let entities = (volumes.iter().map(|&v| (3, v)))
        .chain(faces.iter().map(|&f| (2, f)))
        .chain(edges.iter().map(|&e| (1, e)));
    Ok(entities.zip(names).collect())
}

/// Edges that bound no face, the line parts of a geometry: wires of a STEP file that become
/// beams or trusses.
fn free_edges(gmsh: &Gmsh) -> Result<Vec<i32>, GmshError> {
    let mut in_face = BTreeSet::new();
    for face in gmsh.entities(2)? {
        in_face.extend(gmsh.adjacencies(2, face)?.1);
    }
    Ok((gmsh.entities(1)?.into_iter())
        .filter(|e| !in_face.contains(e))
        .collect())
}

/// Deletes a part of the geometry, PrePoMax's Delete of a geometry part; `None` once no
/// part is left. The other parts keep their names. Gmsh numbers the faces and edges anew
/// when it reads the smaller geometry, so the local mesh sizes are renumbered after where
/// the faces and edges lie; those of the deleted part are dropped.
pub fn delete_part(geometry: &Geometry, part: &str) -> Result<Option<Geometry>, GmshError> {
    delete_part_renumbered(geometry, part).map(|(geometry, _)| geometry)
}

/// [`delete_part`] with the new tag of every face, edge and vertex that stays, by its old
/// one, for the mesh and the selections on it.
pub fn delete_part_renumbered(
    geometry: &Geometry,
    part: &str,
) -> Result<(Option<Geometry>, BTreeMap<CadEntity, CadEntity>), GmshError> {
    delete_parts_renumbered(geometry, &[part.to_owned()])
}

/// [`delete_part_renumbered`] for several parts at once, as selected together: Gmsh
/// removes them all and numbers the rest anew once, so the renumbering maps the old tags
/// straight to the final ones.
pub fn delete_parts_renumbered(
    geometry: &Geometry,
    deleted: &[String],
) -> Result<(Option<Geometry>, BTreeMap<CadEntity, CadEntity>), GmshError> {
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    let smaller = TempFile::new("brep");
    let (names, old_boxes) = with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        let parts = parts(gmsh, geometry)?;
        if let Some(part) = (deleted.iter()).find(|d| !parts.iter().any(|(_, n)| n == *d)) {
            return Err(GmshError::Other(format!(
                "Die Geometrie hat kein Part {part}"
            )));
        }
        let boxes = entity_boxes(gmsh)?;
        let (gone, kept): (Vec<_>, Vec<_>) =
            (parts.into_iter()).partition(|(_, n)| deleted.contains(n));
        let entities: Vec<Entity> = gone.into_iter().map(|(entity, _)| entity).collect();
        gmsh.remove(&entities)?;
        gmsh.write(&smaller.0)?;
        Ok((kept.into_iter().map(|(_, n)| n).collect::<Vec<_>>(), boxes))
    })?;
    if names.is_empty() {
        return Ok((None, BTreeMap::new()));
    }
    let brep = std::fs::read_to_string(&smaller.0)
        .map_err(|e| GmshError::Other(format!("{}: {e}", smaller.0.display())))?;
    let new_boxes = with_gmsh(|gmsh| {
        gmsh.import_shapes(&smaller.0)?;
        entity_boxes(gmsh)
    })?;
    let mut mesh_items = geometry.mesh_items.clone();
    renumber_mesh_items(&mut mesh_items, &old_boxes, &new_boxes);
    let renumbered = renumbering(&old_boxes, &new_boxes);
    let smaller = Geometry {
        brep,
        mesh_items,
        part_names: names,
        ..geometry.clone()
    };
    Ok((Some(smaller), renumbered))
}

/// A Gmsh entity as (dimension, tag).
type Entity = (i32, i32);

type BoundingBox = ([f64; 3], [f64; 3]);

/// Bounding box of every face, edge and vertex, by (dimension, tag).
fn entity_boxes(gmsh: &Gmsh) -> Result<Vec<(Entity, BoundingBox)>, GmshError> {
    let mut boxes = Vec::new();
    for dim in [0, 1, 2] {
        for tag in gmsh.entities(dim)? {
            boxes.push(((dim, tag), gmsh.entity_bounding_box(dim, tag)?));
        }
    }
    Ok(boxes)
}

/// The first "SOLID-n", "SHELL-n" or "LINE-n" for a part of dimension `dim` not yet used.
fn fresh_name(dim: i32, used: &BTreeSet<String>) -> String {
    let prefix = match dim {
        3 => "SOLID",
        2 => "SHELL",
        _ => "LINE",
    };
    (1..)
        .map(|n| format!("{prefix}-{n}"))
        .find(|name| !used.contains(name))
        .unwrap_or_default()
}

/// Names for parts of the given dimensions that `used` does not have yet.
fn fresh_names(dims: &[i32], mut used: BTreeSet<String>) -> Vec<String> {
    (dims.iter())
        .map(|&dim| {
            let name = fresh_name(dim, &used);
            used.insert(name.clone());
            name
        })
        .collect()
}

/// The dimension of each part of the geometry, in the order of [`parts`].
fn default_part_kinds(geometry: &Geometry) -> Result<Vec<i32>, GmshError> {
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        Ok((default_parts(gmsh)?.into_iter())
            .map(|((dim, _), _)| dim)
            .collect())
    })
}

/// Bounding box of every part, as [`parts`] lists them.
fn part_boxes(
    gmsh: &Gmsh,
    parts: Vec<(Entity, String)>,
) -> Result<Vec<(Entity, String, BoundingBox)>, GmshError> {
    (parts.into_iter())
        .map(|((dim, tag), name)| Ok(((dim, tag), name, gmsh.entity_bounding_box(dim, tag)?)))
        .collect()
}

/// The new entity of every old one that lies where it did, after Gmsh numbered the faces,
/// edges and vertices anew.
fn renumbering(
    old_boxes: &[(Entity, BoundingBox)],
    new_boxes: &[(Entity, BoundingBox)],
) -> BTreeMap<CadEntity, CadEntity> {
    let entity = |(dim, tag): Entity| match dim {
        0 => CadEntity::Vertex(tag),
        1 => CadEntity::Edge(tag),
        _ => CadEntity::Face(tag),
    };
    (old_boxes.iter())
        .filter_map(|&((dim, tag), ref old)| {
            let (new, _) =
                (new_boxes.iter()).find(|((d, _), new)| *d == dim && same_box(old, new))?;
            Some((entity((dim, tag)), entity(*new)))
        })
        .collect()
}

/// Renumbers the faces and edges of the local mesh sizes after [`renumbering`]; those
/// without a new tag are dropped.
fn renumber_mesh_items(
    items: &mut [plx_model::MeshSetupItem],
    old_boxes: &[(Entity, BoundingBox)],
    new_boxes: &[(Entity, BoundingBox)],
) {
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
    for item in items {
        if let plx_model::MeshSetupKind::LocalMeshSize { faces, edges, .. } = &mut item.kind {
            renumber(2, faces);
            renumber(1, edges);
        }
    }
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
        let Some((dim, tag)) = (parts(gmsh, geometry)?.into_iter())
            .find(|(_, n)| n == part)
            .map(|(entity, _)| entity)
        else {
            return Err(GmshError::Other(format!(
                "Die Geometrie hat kein Part {part}"
            )));
        };
        let volume = match dim {
            3 => tag,
            2 => return shell_part_mesh(gmsh, geometry, part, tag),
            _ => return line_part_mesh(gmsh, geometry, part, tag),
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
    face: i32,
) -> Result<GeneratedMesh, GmshError> {
    let faces = free_faces(gmsh)?;
    // Only the face stays, so that only it is meshed.
    let others: Vec<(i32, i32)> = (gmsh.entities(3)?.into_iter().map(|v| (3, v)))
        .chain(faces.iter().filter(|&&f| f != face).map(|&f| (2, f)))
        .chain(free_edges(gmsh)?.into_iter().map(|e| (1, e)))
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
    mesh.cad = cad_map(gmsh, &mesh, &[face], options.order, true)?;
    Ok(GeneratedMesh {
        mesh,
        warnings: gmsh.warnings()?,
    })
}

/// Meshes an edge outside the faces with 2- or 3-node lines, typed as beams (`B31`, `B32`);
/// the section decides what CalculiX makes of them. The mesh holds the one part, numbered
/// from 1.
fn line_part_mesh(
    gmsh: &Gmsh,
    geometry: &Geometry,
    part: &str,
    edge: i32,
) -> Result<GeneratedMesh, GmshError> {
    let edges = free_edges(gmsh)?;
    // Only the edge stays, so that only it is meshed.
    let others: Vec<(i32, i32)> = (gmsh.entities(3)?.into_iter().map(|v| (3, v)))
        .chain(free_faces(gmsh)?.into_iter().map(|f| (2, f)))
        .chain(edges.iter().filter(|&&e| e != edge).map(|&e| (1, e)))
        .collect();
    if !others.is_empty() {
        gmsh.remove(&others)?;
    }
    let options = MeshOptions::of_part(geometry, part);
    options.apply(gmsh)?;
    for (_, local_edges, size) in geometry.local_sizes() {
        if local_edges.contains(&edge) && size.is_finite() && size > 0.0 {
            let field = gmsh.add_field("Constant")?;
            gmsh.set_field_number(field, "VIn", size)?;
            gmsh.set_field_number(field, "VOut", 1e22)?;
            gmsh.set_field_number(field, "IncludeBoundary", 1.0)?;
            gmsh.set_field_numbers(field, "CurvesList", &[f64::from(edge)])?;
            gmsh.set_background_field(field)?;
        }
    }
    gmsh.generate(1)?;
    let coords = node_coords(gmsh)?;
    let (gmsh_type, shape, type_name) = if options.order == 2 {
        (LINE3, ElementShape::Line3, "B32")
    } else {
        (LINE2, ElementShape::Line2, "B31")
    };
    let mut mesh = FeMesh::default();
    let mut part_elements = Part {
        name: part.to_string(),
        elements: Vec::new(),
    };
    let (_, nodes) = gmsh.elements(gmsh_type, edge)?;
    for element in nodes.chunks_exact(shape.node_count()) {
        // Gmsh lists the end nodes first and the midside node last; CalculiX wants the
        // midside node in the middle.
        let order: &[usize] = if shape == ElementShape::Line3 {
            &[0, 2, 1]
        } else {
            &[0, 1]
        };
        let mut ids = Vec::with_capacity(element.len());
        for &k in order {
            let tag = element[k];
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
    merged.cad = mesh.cad.without(&removed, &removed_nodes);
    merged
        .cad
        .extend(part_mesh.cad.offset(node_offset, element_offset));
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
    let mut faces = BTreeSet::new();
    for (volume, _) in volumes {
        faces.extend(gmsh.adjacencies(3, *volume)?.1);
    }
    let faces: Vec<i32> = faces.into_iter().collect();
    mesh.cad = cad_map(gmsh, &mesh, &faces, order, false)?;
    Ok(mesh)
}

/// Where the given CAD faces, their edges and vertices lie in a mesh made from Gmsh's nodes:
/// the faces of solid elements on the CAD faces, or for shells the elements themselves.
fn cad_map(
    gmsh: &Gmsh,
    mesh: &FeMesh,
    faces: &[i32],
    order: i32,
    shell: bool,
) -> Result<CadMap, GmshError> {
    let (surface_types, line_type): (&[i32], i32) = match (order, shell) {
        (2, false) => (&[TRI6], LINE3),
        (2, true) => (&[TRI6, QUAD8], LINE3),
        (_, false) => (&[TRI3], LINE2),
        (_, true) => (&[TRI3, QUAD4], LINE2),
    };
    // The solid elements' faces by their sorted corner nodes.
    let mut solid_faces: HashMap<Vec<NodeId>, (ElementId, u8)> = HashMap::new();
    if !shell {
        for element in mesh.elements() {
            for (k, face) in element.faces().iter().enumerate() {
                let mut corners: Vec<NodeId> =
                    face.corners.iter().map(|&i| element.nodes[i]).collect();
                corners.sort_unstable();
                solid_faces.insert(corners, (element.id, (k + 1) as u8));
            }
        }
    }
    let shell_faces: HashMap<Vec<NodeId>, ElementId> = if shell {
        (mesh.elements().iter())
            .map(|e| {
                let mut nodes = e.nodes.clone();
                nodes.sort_unstable();
                (nodes, e.id)
            })
            .collect()
    } else {
        HashMap::new()
    };
    let ids = |tags: &[usize]| -> Result<Vec<NodeId>, GmshError> {
        tags.iter().map(|&t| node_id(t)).collect()
    };
    let mut map = CadMap::default();
    let mut edges = BTreeSet::new();
    for &face in faces {
        let mut nodes = BTreeSet::new();
        let mut on_face = Vec::new();
        for &gmsh_type in surface_types {
            let n = match gmsh_type {
                TRI3 => 3,
                QUAD4 => 4,
                TRI6 => 6,
                _ => 8,
            };
            let (_, tags) = gmsh.elements(gmsh_type, face)?;
            for element in tags.chunks_exact(n) {
                let element = ids(element)?;
                nodes.extend(element.iter().copied());
                let found = if shell {
                    let mut sorted = element.clone();
                    sorted.sort_unstable();
                    shell_faces.get(&sorted).map(|&e| (e, 1))
                } else {
                    let mut corners = element[..3].to_vec();
                    corners.sort_unstable();
                    solid_faces.get(&corners).copied()
                };
                on_face.extend(found);
            }
        }
        map.nodes
            .insert(CadEntity::Face(face), nodes.into_iter().collect());
        map.faces.insert(face, on_face);
        edges.extend(gmsh.adjacencies(2, face)?.1);
    }
    let mut vertices = BTreeSet::new();
    for edge in edges {
        let (_, tags) = gmsh.elements(line_type, edge)?;
        let n = if line_type == LINE3 { 3 } else { 2 };
        let mut nodes = BTreeSet::new();
        let mut segments = Vec::new();
        for segment in tags.chunks_exact(n) {
            let segment = ids(segment)?;
            nodes.extend(segment.iter().copied());
            segments.push([segment[0], segment[1]]);
        }
        map.nodes
            .insert(CadEntity::Edge(edge), nodes.into_iter().collect());
        map.segments.insert(edge, segments);
        vertices.extend(gmsh.adjacencies(1, edge)?.1);
    }
    for vertex in vertices {
        let (_, tags) = gmsh.elements(POINT, vertex)?;
        let nodes = ids(&tags)?;
        if !nodes.is_empty() {
            map.nodes.insert(CadEntity::Vertex(vertex), nodes);
        }
    }
    Ok(map)
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

/// Names of the line parts like [`solid_names`], "LINE-n" where the file names none.
fn line_names(gmsh: &Gmsh, edges: &[i32]) -> Result<Vec<String>, GmshError> {
    let mut names = Vec::with_capacity(edges.len());
    let mut used = BTreeSet::new();
    for (index, &edge) in edges.iter().enumerate() {
        let label = gmsh.entity_name(1, edge)?;
        let base = calculix_name(label.rsplit('/').next().unwrap_or_default())
            .unwrap_or_else(|| format!("LINE-{}", index + 1));
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
/// edges as lines. Faces outside any solid form one more part each, and so do edges outside
/// any face.
fn display_mesh(gmsh: &Gmsh, geometry: &Geometry) -> Result<GeometryDisplay, GmshError> {
    let coords = node_coords(gmsh)?;
    let surfaces = gmsh.entities(2)?;
    let curves = gmsh.entities(1)?;
    // Parts as (name, faces, edges); the edges of the faces are added below. Every face
    // outside the solids is a shell part of its own, as for 2D models, every edge outside
    // the faces a line part.
    let mut groups: Vec<(String, Vec<i32>, Vec<i32>)> = Vec::new();
    for ((dim, tag), name) in parts(gmsh, geometry)? {
        let (faces, edges) = match dim {
            3 => (gmsh.adjacencies(3, tag)?.1, Vec::new()),
            2 => (vec![tag], Vec::new()),
            _ => (Vec::new(), vec![tag]),
        };
        groups.push((name, faces, edges));
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
            CadEntity::Edge(_) | CadEntity::Vertex(_) => (ElementShape::Line2, "B31"),
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
    for (name, faces, own_edges) in groups {
        let mut part = Part {
            name,
            elements: Vec::new(),
        };
        let mut edges: BTreeSet<i32> = own_edges.into_iter().collect();
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
