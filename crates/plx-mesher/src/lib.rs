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

use plx_mesh::{Element, ElementId, ElementShape, FeMesh, NodeId, Part};
use plx_model::{Geometry, MeshSetup};

use gmsh::{Gmsh, with_gmsh};
pub use gmsh::{GmshError, LibraryInfo, loaded_library, set_library_path};

/// File extensions of the CAD formats that can be imported.
pub const CAD_EXTENSIONS: [&str; 5] = ["step", "stp", "iges", "igs", "brep"];

/// Gmsh's element type numbers.
const LINE2: i32 = 1;
const TRI3: i32 = 2;
const TET4: i32 = 4;
const TET10: i32 = 11;

/// Gmsh numbers the last two midside nodes of a quadratic tetrahedron the other way round
/// than CalculiX (edges 3-2, 3-1 against 2-4, 3-4 in 1-based CalculiX numbering).
const TET10_TO_CALCULIX: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 9, 8];

/// Whether a file is a CAD file by its extension.
pub fn is_cad_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| CAD_EXTENSIONS.iter().any(|c| e.eq_ignore_ascii_case(c)))
}

/// A face or edge of the CAD geometry, by Gmsh's tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
        mesh_setup: MeshSetup::for_diagonal(diagonal),
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
        set_mesh_options(gmsh, diagonal / 15.0, 0.0, 24.0, 1, true, false)?;
        // A face Gmsh cannot mesh is left out of the display rather than failing the import.
        if let Err(error) = gmsh.generate(2) {
            log::warn!("Darstellung der Geometrie unvollständig: {error}");
        }
        display_mesh(gmsh)
    })
}

/// Meshes the solids of the geometry with tetrahedra, one part per solid.
pub fn generate_mesh(geometry: &Geometry) -> Result<GeneratedMesh, GmshError> {
    let setup = &geometry.mesh_setup;
    if !(setup.max_size > 0.0 && setup.min_size >= 0.0) {
        return Err(GmshError::Other(
            "Die maximale Elementgröße muss größer als 0 sein".into(),
        ));
    }
    let file = TempFile::with_contents("brep", &geometry.brep)?;
    with_gmsh(|gmsh| {
        gmsh.import_shapes(&file.0)?;
        let curvature = if setup.elements_per_curvature > 0.0 {
            (std::f64::consts::TAU * setup.elements_per_curvature).round()
        } else {
            0.0
        };
        let order = if setup.second_order { 2 } else { 1 };
        set_mesh_options(
            gmsh,
            setup.max_size,
            setup.min_size.min(setup.max_size),
            curvature,
            order,
            !setup.midside_nodes_on_geometry,
            setup.optimize,
        )?;
        gmsh.generate(3)?;
        let mesh = solid_mesh(gmsh, order)?;
        Ok(GeneratedMesh {
            mesh,
            warnings: gmsh.warnings()?,
        })
    })
}

/// Loads Gmsh and meshes a cube, for the self test in the settings.
pub fn self_test() -> Result<(LibraryInfo, usize), GmshError> {
    with_gmsh(|gmsh| {
        gmsh.add_box([0.0; 3], [10.0; 3])?;
        set_mesh_options(gmsh, 2.5, 0.0, 0.0, 2, true, false)?;
        gmsh.generate(3)?;
        let mesh = solid_mesh(gmsh, 2)?;
        Ok((gmsh.info().clone(), mesh.element_count()))
    })
}

/// Sets every meshing option prepolix uses; Gmsh keeps options between uses.
fn set_mesh_options(
    gmsh: &Gmsh,
    max_size: f64,
    min_size: f64,
    elements_per_2pi: f64,
    order: i32,
    straight_midside_nodes: bool,
    netgen: bool,
) -> Result<(), GmshError> {
    for (name, value) in [
        ("Mesh.MeshSizeMax", max_size),
        ("Mesh.MeshSizeMin", min_size),
        ("Mesh.MeshSizeFromCurvature", elements_per_2pi),
        ("Mesh.MeshSizeFromPoints", 1.0),
        ("Mesh.MeshSizeExtendFromBoundary", 1.0),
        ("Mesh.ElementOrder", f64::from(order)),
        (
            "Mesh.SecondOrderLinear",
            f64::from(u8::from(straight_midside_nodes)),
        ),
        ("Mesh.HighOrderOptimize", 0.0),
        // Frontal-Delaunay on faces, Delaunay in volumes: Gmsh's robust defaults.
        ("Mesh.Algorithm", 6.0),
        ("Mesh.Algorithm3D", 1.0),
        ("Mesh.Optimize", 1.0),
        ("Mesh.OptimizeNetgen", f64::from(u8::from(netgen))),
        ("Mesh.RecombineAll", 0.0),
    ] {
        gmsh.set_number(name, value)?;
    }
    Ok(())
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

/// The tetrahedra of all solids, one part per solid.
fn solid_mesh(gmsh: &Gmsh, order: i32) -> Result<FeMesh, GmshError> {
    let volumes = gmsh.entities(3)?;
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
    let names = solid_names(gmsh, &volumes)?;
    let mut mesh = FeMesh::default();
    let mut next_id: ElementId = 1;
    for (&volume, name) in volumes.iter().zip(names) {
        let (_, nodes) = gmsh.elements(gmsh_type, volume)?;
        let mut part = Part {
            name,
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
fn display_mesh(gmsh: &Gmsh) -> Result<GeometryDisplay, GmshError> {
    let coords = node_coords(gmsh)?;
    let volumes = gmsh.entities(3)?;
    let surfaces = gmsh.entities(2)?;
    let curves = gmsh.entities(1)?;
    let mut groups: Vec<(String, Vec<i32>)> = Vec::new();
    let mut in_solid = BTreeSet::new();
    for (&volume, name) in volumes.iter().zip(solid_names(gmsh, &volumes)?) {
        let (_, faces) = gmsh.adjacencies(3, volume)?;
        in_solid.extend(faces.iter().copied());
        groups.push((name, faces));
    }
    let free: Vec<i32> = surfaces
        .iter()
        .copied()
        .filter(|s| !in_solid.contains(s))
        .collect();
    if !free.is_empty() {
        groups.push(("SURFACES".into(), free));
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
        solids: volumes.len(),
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
