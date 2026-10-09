use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{DVec3, Vec3};
use plx_io::frd::{FrdImport, read_frd};
use plx_io::inp::{InpImport, read_inp};
use plx_mesh::{ElementId, FeMesh, NodeId, PartSkin, extract_part_skin};
use plx_mesher::{CadEntity, GeometryDisplay};
use plx_model::{FeModel, Geometry};
use plx_render::contour::normalize;
use plx_render::{RenderMesh, part_color, part_render_mesh, wireframe_edges};

use crate::results::ResultsView;

/// Angle between neighbouring faces above which their common edge counts as a feature edge
/// and the shading across it stays sharp.
const FEATURE_ANGLE_DEG: f64 = 30.0;
/// Within a surface patch, coarse meshes of curved surfaces can fold more than the feature
/// angle between neighbouring faces; shading still blends across such folds.
const SMOOTH_ANGLE_DEG: f64 = 60.0;

/// Extension of prepolix project files.
pub const PROJECT_EXTENSION: &str = "plx";

/// Colour of selected faces and nodes, PrePoMax's highlight red.
pub const HIGHLIGHT_COLOR: [f32; 3] = [1.0, 0.0, 0.0];

/// Summary of one part, computed once on load so the GUI never iterates large meshes.
pub struct PartInfo {
    pub name: String,
    pub color: [f32; 3],
    pub element_count: usize,
    pub node_count: usize,
    pub element_types: Vec<(String, usize)>,
    /// Bounding box relative to [`Model::origin`].
    pub bounds: Option<(Vec3, Vec3)>,
    pub visible: bool,
}

/// A loaded input file with everything the GUI shows about it.
pub struct Model {
    pub path: PathBuf,
    pub mesh: FeMesh,
    pub parts: Vec<PartInfo>,
    pub warnings: Vec<String>,
    pub skipped_keywords: BTreeMap<String, usize>,
    pub included_files: usize,
    pub load_time: Duration,
    /// Results read from an `.frd` file, with what the user currently looks at. A model
    /// with results is a results file: it lives in the Results workspace and has no FE model
    /// to set up, as in PrePoMax.
    pub results: Option<ResultsView>,
    /// The analysis set up on this mesh.
    pub fe: FeModel,
    /// CAD geometry the mesh is generated from.
    pub geometry: Option<Geometry>,
    /// The display of CAD geometry rather than a mesh: element edges are not drawn.
    is_geometry: bool,
    /// Faces and parts drawn in the highlight colour.
    pub highlight: Highlight,
    /// Centre of the mesh; render positions are relative to it.
    origin: DVec3,
    skins: Vec<PartSkin>,
}

/// What is shown selected: whole parts, element faces and nodes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Highlight {
    pub parts: HashSet<usize>,
    /// Element faces as (element, CalculiX face number).
    pub faces: HashSet<(ElementId, u8)>,
    /// Nodes, drawn as points over the scene.
    pub nodes: Vec<NodeId>,
}

/// A visible face under the mouse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub part: usize,
    /// Index into the part's skin faces.
    pub face: usize,
    /// Hit point relative to the model origin.
    pub point: Vec3,
}

/// Result of loading on a worker thread: the model plus one GPU-ready mesh per part of what
/// is shown first, the geometry if the model has no mesh yet.
pub struct LoadedModel {
    pub model: Model,
    pub render_meshes: Vec<RenderMesh>,
    /// Display of the model's CAD geometry.
    pub geometry_view: Option<Model>,
}

pub fn load(path: &Path) -> Result<LoadedModel, String> {
    let start = Instant::now();
    let extension = |e: &str| path.extension().is_some_and(|x| x.eq_ignore_ascii_case(e));
    if plx_mesher::is_cad_file(path) {
        let import = plx_mesher::import_cad(path).map_err(|e| e.to_string())?;
        let mut model = Model::new(path, FeMesh::default());
        model.geometry = Some(import.geometry);
        model.warnings = import.warnings;
        let view = Model::geometry_view(path, import.display);
        let render_meshes = view.render_meshes();
        model.load_time = start.elapsed();
        return Ok(LoadedModel {
            model,
            render_meshes,
            geometry_view: Some(view),
        });
    }
    let mut fe = FeModel::default();
    let mut geometry = None;
    let (mesh, mut warnings, skipped_keywords, included_files, increments) =
        if extension(PROJECT_EXTENSION) {
            let project = plx_io::project::read_project(path).map_err(|e| e.to_string())?;
            fe = project.model;
            geometry = project.geometry;
            (project.mesh, Vec::new(), BTreeMap::new(), 0, None)
        } else if extension("frd") {
            let FrdImport {
                mesh,
                increments,
                warnings,
                date,
                time,
                ..
            } = read_frd(path).map_err(|e| format!("{}: {e}", path.display()))?;
            (
                mesh,
                warnings,
                BTreeMap::new(),
                0,
                Some((increments, date, time)),
            )
        } else {
            let InpImport {
                mesh,
                warnings,
                skipped_keywords,
                files,
            } = read_inp(path).map_err(|e| e.to_string())?;
            let included = files.len().saturating_sub(1);
            (mesh, warnings, skipped_keywords, included, None)
        };
    if mesh.element_count() == 0 && geometry.is_none() {
        return Err(format!(
            "{} enthält keine darstellbaren Elemente",
            path.display()
        ));
    }
    // A project's geometry is shown on the Geometry tab; without Gmsh only its mesh is.
    let geometry_view = match &geometry {
        Some(geometry) => match plx_mesher::tessellate(geometry) {
            Ok(display) => Some(Model::geometry_view(path, display)),
            Err(error) => {
                warnings.push(format!("Geometrie kann nicht angezeigt werden: {error}"));
                None
            }
        },
        None => None,
    };
    let mut model = Model::new(path, mesh);
    model.fe = fe;
    model.geometry = geometry;
    model.warnings = warnings;
    model.skipped_keywords = skipped_keywords;
    model.included_files = included_files;
    model.results = increments.map(|(increments, date, time)| {
        let mut view = ResultsView::new(increments, model.mesh.bounds());
        view.date = date;
        view.time = time;
        view
    });
    // A project without a mesh yet opens on its geometry.
    let render_meshes = match &geometry_view {
        Some(view) if model.mesh.element_count() == 0 => view.render_meshes(),
        _ => model.render_meshes(),
    };
    for (part, render) in model.parts.iter_mut().zip(&render_meshes) {
        part.bounds = render.bounds();
    }
    model.load_time = start.elapsed();
    Ok(LoadedModel {
        model,
        render_meshes,
        geometry_view,
    })
}

impl Model {
    /// A model of the mesh with summaries and skins of its parts.
    pub fn new(path: &Path, mesh: FeMesh) -> Self {
        let skins = (mesh.parts.iter())
            .map(|part| extract_part_skin(&mesh, part, FEATURE_ANGLE_DEG))
            .collect();
        Self::with_skins(path, mesh, skins)
    }

    fn with_skins(path: &Path, mesh: FeMesh, skins: Vec<PartSkin>) -> Self {
        let origin = mesh.bounds().map_or(DVec3::ZERO, |(min, max)| {
            (DVec3::from(min) + DVec3::from(max)) * 0.5
        });
        let mut parts = Vec::with_capacity(mesh.parts.len());
        for (index, part) in mesh.parts.iter().enumerate() {
            let mut types: BTreeMap<&str, usize> = BTreeMap::new();
            let mut nodes = std::collections::HashSet::new();
            for element in part.elements.iter().filter_map(|&id| mesh.element(id)) {
                *types.entry(&element.type_name).or_default() += 1;
                nodes.extend(element.nodes.iter().copied());
            }
            parts.push(PartInfo {
                name: part.name.clone(),
                color: part_color(index),
                element_count: part.elements.len(),
                node_count: nodes.len(),
                element_types: types.into_iter().map(|(t, n)| (t.to_string(), n)).collect(),
                bounds: None,
                visible: true,
            });
        }
        let mut model = Self {
            path: path.to_path_buf(),
            mesh,
            parts,
            warnings: Vec::new(),
            skipped_keywords: BTreeMap::new(),
            included_files: 0,
            load_time: Duration::ZERO,
            results: None,
            fe: FeModel::default(),
            geometry: None,
            is_geometry: false,
            highlight: Highlight::default(),
            origin,
            skins,
        };
        let meshes = model.render_meshes();
        for (part, render) in model.parts.iter_mut().zip(&meshes) {
            part.bounds = render.bounds();
        }
        model
    }

    /// The CAD geometry for the Geometry tab: its faces shaded each on its own, outlined by
    /// its edges, without the triangles of the display mesh.
    pub fn geometry_view(path: &Path, display: GeometryDisplay) -> Self {
        let mesh = display.mesh;
        let skins = (mesh.parts.iter())
            .map(|part| {
                // Only the CAD edges, drawn as line elements, outline the geometry.
                let mut skin = extract_part_skin(&mesh, part, 180.0);
                for face in &mut skin.faces {
                    let id = mesh.elements()[face.element].id;
                    if let Some(CadEntity::Face(tag)) = display.entities.get(id as usize - 1) {
                        face.region = *tag as usize;
                    }
                }
                skin
            })
            .collect();
        let mut model = Self::with_skins(path, mesh, skins);
        model.is_geometry = true;
        model
    }

    /// Replaces the mesh, e.g. by a newly generated one; the FE model stays.
    pub fn set_mesh(&mut self, mesh: FeMesh) {
        let mut fresh = Self::new(&self.path, mesh);
        std::mem::swap(&mut self.mesh, &mut fresh.mesh);
        self.parts = fresh.parts;
        self.skins = fresh.skins;
        self.origin = fresh.origin;
        self.highlight = Highlight::default();
    }

    /// Whether this shows CAD geometry rather than a mesh.
    pub fn is_geometry(&self) -> bool {
        self.is_geometry
    }

    /// Whether this is a results file rather than an FE model.
    pub fn is_results(&self) -> bool {
        self.results.is_some()
    }

    /// GPU-ready meshes of all parts, deformed and coloured by the selected result if any.
    pub fn render_meshes(&self) -> Vec<RenderMesh> {
        let mut coords = std::borrow::Cow::Borrowed(self.mesh.coords());
        let mut scalars = None;
        let mut deformed = false;
        if let Some(view) = &self.results {
            let scale = (view.scale() * view.amplitude()) as f64;
            let displacements = view.current_increment().and_then(|i| i.displacements());
            if let (Some(displacements), true) = (displacements, scale != 0.0) {
                coords = std::borrow::Cow::Owned(
                    coords
                        .iter()
                        .zip(&displacements)
                        .map(|(p, d)| [0, 1, 2].map(|k| p[k] + scale * d[k] as f64))
                        .collect(),
                );
                deformed = view.show_undeformed;
            }
            if let (Some((_, component)), Some(legend)) = (view.current(), view.legend()) {
                let amplitude = view.amplitude();
                let values: Vec<f32> = component.values.iter().map(|v| v * amplitude).collect();
                scalars = Some(normalize(&values, legend.min, legend.max));
            }
        }
        self.mesh
            .parts
            .iter()
            .zip(&self.parts)
            .zip(&self.skins)
            .enumerate()
            .map(|(index, ((_, info), skin))| {
                let mut mesh = part_render_mesh(
                    &coords,
                    skin,
                    self.origin,
                    info.color,
                    SMOOTH_ANGLE_DEG,
                    scalars.as_deref(),
                );
                self.highlight_faces(index, skin, &mut mesh);
                if deformed {
                    mesh.wireframe_edges = wireframe_edges(self.mesh.coords(), skin, self.origin);
                }
                if self.is_geometry {
                    mesh.mesh_edges.clear();
                }
                mesh
            })
            .collect()
    }

    /// Recolours the vertices of highlighted faces; vertices are laid out face by face.
    fn highlight_faces(&self, part: usize, skin: &PartSkin, mesh: &mut RenderMesh) {
        if self.highlight.faces.is_empty() && !self.highlight.parts.contains(&part) {
            return;
        }
        let whole = self.highlight.parts.contains(&part);
        let elements = self.mesh.elements();
        let mut start = 0;
        for face in &skin.faces {
            let count = face.corners.len() + face.mids.len();
            let key = (elements[face.element].id, face.face as u8 + 1);
            if whole || self.highlight.faces.contains(&key) {
                for vertex in &mut mesh.vertices[start..start + count] {
                    vertex.color = HIGHLIGHT_COLOR;
                }
            }
            start += count;
        }
    }

    /// All element faces on the surface of the parts.
    pub fn skin_faces(&self) -> impl Iterator<Item = (ElementId, u8)> + '_ {
        let elements = self.mesh.elements();
        self.skins
            .iter()
            .flat_map(|skin| &skin.faces)
            .map(|f| (elements[f.element].id, f.face as u8 + 1))
    }

    /// All element faces on the surface with the ids of their corner nodes.
    pub fn skin_faces_with_corners(
        &self,
    ) -> impl Iterator<Item = ((ElementId, u8), Vec<NodeId>)> + '_ {
        let (elements, ids) = (self.mesh.elements(), self.mesh.node_ids());
        self.skins
            .iter()
            .flat_map(|skin| &skin.faces)
            .map(move |f| {
                let corners = f.corners.iter().map(|&n| ids[n]).collect();
                ((elements[f.element].id, f.face as u8 + 1), corners)
            })
    }

    pub fn skin(&self, part: usize) -> &PartSkin {
        &self.skins[part]
    }

    /// Undeformed position of a node relative to the model origin, where picking happens.
    pub fn render_position(&self, index: usize) -> Vec3 {
        (DVec3::from(self.mesh.coords()[index]) - self.origin).as_vec3()
    }

    /// All nodes of the visible parts.
    pub fn visible_nodes(&self) -> BTreeSet<NodeId> {
        let elements = self.mesh.elements();
        self.parts
            .iter()
            .zip(&self.mesh.parts)
            .filter(|(info, _)| info.visible)
            .flat_map(|(_, part)| &part.elements)
            .filter_map(|&id| self.mesh.element_index(id))
            .flat_map(|index| elements[index].nodes.iter().copied())
            .collect()
    }

    /// All surface faces of the visible parts.
    pub fn visible_faces(&self) -> BTreeSet<(ElementId, u8)> {
        let elements = self.mesh.elements();
        self.skins
            .iter()
            .zip(&self.parts)
            .filter(|(_, info)| info.visible)
            .flat_map(|(skin, _)| &skin.faces)
            .map(|f| (elements[f.element].id, f.face as u8 + 1))
            .collect()
    }

    /// The nearest visible face hit by a ray, both relative to the model origin.
    pub fn pick(&self, origin: Vec3, direction: Vec3) -> Option<Hit> {
        let coords = self.mesh.coords();
        let position = |node: usize| (DVec3::from(coords[node]) - self.origin).as_vec3();
        let mut best: Option<(f32, Hit)> = None;
        for (part, skin) in self.skins.iter().enumerate() {
            if !self.parts[part].visible {
                continue;
            }
            for (index, face) in skin.faces.iter().enumerate() {
                let a = position(face.corners[0]);
                for pair in face.corners[1..].windows(2) {
                    let (b, c) = (position(pair[0]), position(pair[1]));
                    let Some(t) = ray_triangle(origin, direction, a, b, c) else {
                        continue;
                    };
                    if best.is_none_or(|(nearest, _)| t < nearest) {
                        let point = origin + direction * t;
                        best = Some((
                            t,
                            Hit {
                                part,
                                face: index,
                                point,
                            },
                        ));
                    }
                }
            }
        }
        best.map(|(_, hit)| hit)
    }

    /// The node of the hit face nearest to the hit point.
    pub fn hit_node(&self, hit: &Hit) -> NodeId {
        let face = &self.skins[hit.part].faces[hit.face];
        let coords = self.mesh.coords();
        let distance = |node: usize| {
            ((DVec3::from(coords[node]) - self.origin).as_vec3() - hit.point).length_squared()
        };
        let nearest = face
            .corners
            .iter()
            .chain(&face.mids)
            .copied()
            .min_by(|&a, &b| distance(a).total_cmp(&distance(b)))
            .unwrap_or(face.corners[0]);
        self.mesh.node_ids()[nearest]
    }

    /// Where a node is drawn, relative to the model origin, including the shown deformation.
    pub fn node_position(&self, index: usize) -> Option<Vec3> {
        let mut p = DVec3::from(*self.mesh.coords().get(index)?);
        if let Some(view) = &self.results {
            let scale = (view.scale() * view.amplitude()) as f64;
            let displacement = view
                .current_increment()
                .and_then(|i| i.field("DISP"))
                .map(|f| {
                    ["U1", "U2", "U3"].map(|n| f.component(n).map_or(0.0, |c| c.values[index]))
                });
            if let (Some(d), true) = (displacement, scale != 0.0) {
                p += scale * DVec3::new(d[0] as f64, d[1] as f64, d[2] as f64);
            }
        }
        Some((p - self.origin).as_vec3())
    }

    /// The global origin in render coordinates.
    pub fn global_origin(&self) -> Vec3 {
        (-self.origin).as_vec3()
    }

    /// Whether the model was opened from or saved to a project file, so saving needs no
    /// file dialog.
    pub fn is_project(&self) -> bool {
        self.path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION))
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// Bounding box of all visible parts, relative to the model origin.
    pub fn visible_bounds(&self) -> Option<(Vec3, Vec3)> {
        self.parts
            .iter()
            .filter(|p| p.visible)
            .filter_map(|p| p.bounds)
            .reduce(|(min_a, max_a), (min_b, max_b)| (min_a.min(min_b), max_a.max(max_b)))
    }
}

/// Distance along the ray to a triangle (Möller-Trumbore), seen from either side.
fn ray_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (ab, ac) = (b - a, c - a);
    let p = direction.cross(ac);
    let det = ab.dot(p);
    if det.abs() < f32::EPSILON * ab.length() * ac.length() {
        return None;
    }
    let s = origin - a;
    let u = s.dot(p) / det;
    let q = s.cross(ab);
    let v = direction.dot(q) / det;
    let t = ac.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn testdata(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata")
            .join(name)
    }

    #[test]
    fn loads_testdata_with_part_summaries() {
        let loaded = load(&testdata("platte_mit_stuetzen.inp")).unwrap();
        let model = &loaded.model;
        assert_eq!(model.included_files, 1);
        assert_eq!(model.parts.len(), 2);
        assert_eq!(model.parts[0].element_types, [("S4R".to_string(), 16)]);
        assert_eq!(model.parts[0].node_count, 25);
        assert_eq!(model.parts[1].node_count, 4);
        assert_eq!(loaded.render_meshes.len(), 2);
        let (min, max) = model.visible_bounds().unwrap();
        assert_eq!(max - min, Vec3::new(20.0, 20.0, 15.0));
    }

    #[test]
    fn projects_open_with_their_fe_model() {
        let mut model = load(&testdata("platte_mit_stuetzen.inp")).unwrap().model;
        assert!(!model.is_project());
        model.fe.steps.push(plx_model::Step::new_static("Step-1"));
        let dir = std::env::temp_dir().join(format!("prepolix-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("platte.plx");
        plx_io::project::save_project(&path, None, &model.mesh, &model.fe).unwrap();
        let loaded = load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(loaded.model.is_project());
        assert!(!loaded.model.is_results());
        assert_eq!(loaded.model.fe, model.fe);
        assert_eq!(loaded.model.parts.len(), 2);
    }

    #[test]
    fn hidden_parts_do_not_count_for_fitting() {
        let mut model = load(&testdata("platte_mit_stuetzen.inp")).unwrap().model;
        model.parts[1].visible = false;
        let (min, max) = model.visible_bounds().unwrap();
        assert_eq!(max.z - min.z, 0.0);
    }

    #[test]
    fn loads_frd_results_with_deformation() {
        let loaded = load(&testdata("kragbalken_c3d8.frd")).unwrap();
        let model = &loaded.model;
        assert_eq!(model.parts[0].name, "STEEL");
        let view = model.results.as_ref().unwrap();
        assert_eq!(view.current().unwrap().1.name, "ALL");
        assert!(view.scale() > 1.0);
        // The beam bends downwards, so the deformed bounds reach below the undeformed ones.
        let undeformed_min_z = -5.0;
        let (min, _) = model.visible_bounds().unwrap();
        assert!(min.z < undeformed_min_z, "{min}");
        assert!(
            loaded.render_meshes[0]
                .vertices
                .iter()
                .all(|v| v.scalar >= 0.0)
        );
    }

    #[test]
    fn picking_finds_faces_and_nodes() {
        let model = load(&testdata("kragbalken_c3d8.inp")).unwrap().model;
        // The beam spans 0..100 x 0..10 x 0..10; look straight down at x = 95, y = 5.
        let origin = Vec3::new(95.0, 5.0, 50.0) - model.origin.as_vec3();
        let hit = model.pick(origin, Vec3::NEG_Z).unwrap();
        assert!((hit.point.z + model.origin.z as f32 - 10.0).abs() < 1e-4);
        let node = model.mesh.node(model.hit_node(&hit)).unwrap();
        assert_eq!(node[2], 10.0);
        assert!(model.pick(origin, Vec3::Z).is_none());
    }

    /// Whether Gmsh can be used; tests that need it pass with a note otherwise, unless
    /// `PREPOLIX_REQUIRE_GMSH` is set.
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

    #[test]
    fn step_files_open_as_geometry_and_mesh_into_the_project() {
        if !gmsh_available() {
            return;
        }
        let loaded = load(&testdata("zwei_bloecke.step")).unwrap();
        let view = loaded.geometry_view.unwrap();
        assert!(view.is_geometry());
        assert_eq!(view.parts.len(), 2);
        // The display shows the CAD edges but no triangle edges, and each CAD face is its
        // own patch: a block has six.
        assert_eq!(loaded.render_meshes.len(), 2);
        assert!(loaded.render_meshes[0].mesh_edges.is_empty());
        assert!(!loaded.render_meshes[0].feature_edges.is_empty());
        let patches: BTreeSet<usize> = view.skin(0).faces.iter().map(|f| f.region).collect();
        assert_eq!(patches.len(), 6);

        let mut model = loaded.model;
        assert_eq!(model.mesh.element_count(), 0);
        let geometry = model.geometry.clone().unwrap();
        let mesh = plx_mesher::generate_mesh(&geometry).unwrap().mesh;
        model.set_mesh(mesh);
        assert_eq!(model.parts.len(), 2);
        assert!(model.visible_bounds().is_some());

        // The project keeps geometry and mesh; it opens on its mesh with the geometry beside.
        let dir = std::env::temp_dir().join(format!("prepolix-geometry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bloecke.plx");
        plx_io::project::save_project(&path, model.geometry.as_ref(), &model.mesh, &model.fe)
            .unwrap();
        let reopened = load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(reopened.model.geometry, Some(geometry));
        assert_eq!(
            reopened.model.mesh.element_count(),
            model.mesh.element_count()
        );
        assert!(reopened.geometry_view.is_some());
        assert!(!reopened.render_meshes[0].mesh_edges.is_empty());
    }

    /// Meshes the plate with a hole, sets up a tension test and checks CalculiX's stress.
    #[test]
    fn generated_meshes_solve_with_calculix() {
        if !gmsh_available()
            || std::process::Command::new("ccx")
                .arg("-v")
                .output()
                .is_err()
        {
            eprintln!("Gmsh oder ccx fehlt, Test übersprungen");
            return;
        }
        use plx_model::{
            BoundaryCondition, BoundaryKind, Elastic, Material, Region, Section, Step,
        };
        let mut model = load(&testdata("platte_mit_loch.step")).unwrap().model;
        let mut geometry = model.geometry.clone().unwrap();
        geometry.mesh_setup.max_size = 4.0;
        model.set_mesh(plx_mesher::generate_mesh(&geometry).unwrap().mesh);
        let nodes_at = |x: f64| -> Vec<NodeId> {
            (model.mesh.node_ids().iter().zip(model.mesh.coords()))
                .filter(|(_, c)| (c[0] - x).abs() < 1e-6)
                .map(|(id, _)| *id)
                .collect()
        };
        let (fixed, pulled) = (nodes_at(0.0), nodes_at(100.0));
        let mut step = Step::new_static("Step-1");
        step.boundary_conditions.push(BoundaryCondition {
            name: "Fix".into(),
            region: Region::Nodes(fixed),
            kind: BoundaryKind::Fixed,
        });
        step.boundary_conditions.push(BoundaryCondition {
            name: "Pull".into(),
            region: Region::Nodes(pulled),
            kind: BoundaryKind::Displacement([Some(0.1), None, None, None, None, None]),
        });
        model.fe = FeModel {
            materials: vec![Material {
                name: "Steel".into(),
                density: None,
                elastic: Some(Elastic {
                    young: 210000.0,
                    poisson: 0.3,
                }),
            }],
            sections: vec![Section {
                name: "Section-1".into(),
                material: "Steel".into(),
                region: Region::Parts(vec!["SOLID-1".into()]),
            }],
            steps: vec![step],
            user_keywords: Vec::new(),
        };
        let inp = plx_io::inp::write_inp(&model.mesh, &model.fe, "Platte").unwrap();
        let dir = std::env::temp_dir().join(format!("prepolix-ccx-{}", std::process::id()));
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
        // A strain of 0.1 / 100 gives about 210 MPa in the net section away from the hole;
        // the hole raises the peak, CalculiX's extrapolation keeps it below a few times that.
        let stress = results.increments.last().unwrap().field("STRESS").unwrap();
        let sxx = &stress.component("S11").unwrap().values;
        let max = sxx.iter().copied().fold(f32::MIN, f32::max);
        assert!(max > 300.0 && max < 1500.0, "größte Spannung {max}");
    }

    #[test]
    fn file_without_elements_is_rejected() {
        assert!(load(&testdata("platte_knoten.inp")).is_err());
    }
}
