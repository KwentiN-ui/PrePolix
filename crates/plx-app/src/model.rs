use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{DVec3, Vec3};
use plx_io::frd::{FrdImport, read_frd};
use plx_io::inp::{InpImport, read_inp};
use plx_mesh::{ElementId, FeMesh, NodeId, PartSkin, extract_part_skin};
use plx_model::FeModel;
use plx_render::contour::normalize;
use plx_render::{
    ClipPlane, RenderMesh, SectionCells, Vertex, lighten, part_color, part_render_mesh,
    section_mesh, wireframe_edges,
};

use crate::results::{Deformation, ResultsView};

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
    /// Hot spot values of a results file, evaluated with the hot spots of the FE model.
    pub hot_spots: Option<crate::hot_spots::Evaluation>,
    /// Faces and parts drawn in the highlight colour.
    pub highlight: Highlight,
    /// The section view plane in render coordinates; picking ignores what it cuts off.
    pub clip: Option<ClipPlane>,
    /// Centre of the mesh; render positions are relative to it.
    origin: DVec3,
    skins: Vec<PartSkin>,
    /// Elements of each part prepared for section cuts, built when first needed.
    section_cells: std::sync::OnceLock<Vec<SectionCells>>,
}

/// What is shown selected: whole parts, element faces and nodes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Highlight {
    pub parts: HashSet<usize>,
    /// Parts drawn with a red outline only, as PrePoMax shows a selected part.
    pub outlines: HashSet<usize>,
    /// Element faces as (element, CalculiX face number).
    pub faces: HashSet<(ElementId, u8)>,
    /// Nodes, drawn as points over the scene.
    pub nodes: Vec<NodeId>,
}

impl Highlight {
    /// A whole part, such as one selected in the tree or clicked in the 3D view.
    pub fn part(index: usize) -> Self {
        Self {
            outlines: HashSet::from([index]),
            ..Self::default()
        }
    }
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

/// Node coordinates as drawn, normalized contour values and whether the shape is deformed.
type ShownState<'a> = (std::borrow::Cow<'a, [[f64; 3]]>, Option<Vec<f32>>, bool);

/// Result of loading on a worker thread: the model plus one GPU-ready mesh per part.
pub struct LoadedModel {
    pub model: Model,
    pub render_meshes: Vec<RenderMesh>,
}

pub fn load(path: &Path) -> Result<LoadedModel, String> {
    let start = Instant::now();
    let extension = |e: &str| path.extension().is_some_and(|x| x.eq_ignore_ascii_case(e));
    let mut fe = FeModel::default();
    let (mesh, warnings, skipped_keywords, included_files, increments) =
        if extension(PROJECT_EXTENSION) {
            let project = plx_io::project::read_project(path).map_err(|e| e.to_string())?;
            fe = project.model;
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
    if mesh.element_count() == 0 {
        return Err(format!(
            "{} enthält keine darstellbaren Elemente",
            path.display()
        ));
    }
    let origin = mesh.bounds().map_or(DVec3::ZERO, |(min, max)| {
        (DVec3::from(min) + DVec3::from(max)) * 0.5
    });

    let mut parts = Vec::with_capacity(mesh.parts.len());
    let mut skins = Vec::with_capacity(mesh.parts.len());
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
        skins.push(extract_part_skin(&mesh, part, FEATURE_ANGLE_DEG));
    }
    let results = increments.map(|(increments, date, time)| {
        let mut view = ResultsView::new(increments, mesh.bounds());
        // Several parts usually mean contact, where an exaggerated deformation shows parts
        // penetrating each other; true scale is less confusing there.
        if mesh.parts.len() > 1 {
            view.deformation = Deformation::TrueScale;
        }
        view.date = date;
        view.time = time;
        view
    });
    let mut model = Model {
        path: path.to_path_buf(),
        mesh,
        parts,
        warnings,
        skipped_keywords,
        included_files,
        load_time: Duration::ZERO,
        results,
        fe,
        hot_spots: None,
        highlight: Highlight::default(),
        clip: None,
        origin,
        skins,
        section_cells: Default::default(),
    };
    let render_meshes = model.render_meshes();
    for (part, render) in model.parts.iter_mut().zip(&render_meshes) {
        part.bounds = render.bounds();
    }
    model.load_time = start.elapsed();
    Ok(LoadedModel {
        model,
        render_meshes,
    })
}

impl Model {
    /// Whether this is a results file rather than an FE model.
    pub fn is_results(&self) -> bool {
        self.results.is_some()
    }

    /// Node coordinates as drawn: with the shown deformation of results.
    pub fn shown_coords(&self) -> std::borrow::Cow<'_, [[f64; 3]]> {
        let coords = self.mesh.coords();
        let Some(view) = &self.results else {
            return std::borrow::Cow::Borrowed(coords);
        };
        let scale = (view.scale() * view.amplitude()) as f64;
        match view.current_increment().and_then(|i| i.displacements()) {
            Some(displacements) if scale != 0.0 => std::borrow::Cow::Owned(
                coords
                    .iter()
                    .zip(&displacements)
                    .map(|(p, d)| [0, 1, 2].map(|k| p[k] + scale * d[k] as f64))
                    .collect(),
            ),
            _ => std::borrow::Cow::Borrowed(coords),
        }
    }

    /// Node coordinates as drawn and the normalized contour values, if a result is shown;
    /// the flag tells whether the shape is deformed and the undeformed outline is wanted.
    fn shown_state(&self) -> ShownState<'_> {
        let coords = self.shown_coords();
        let mut scalars = None;
        let mut deformed = false;
        if let Some(view) = &self.results {
            if matches!(coords, std::borrow::Cow::Owned(_)) {
                deformed = view.show_undeformed;
            }
            if let (Some((_, component)), Some(legend)) = (view.current(), view.legend()) {
                let amplitude = view.value_amplitude();
                let values: Vec<f32> = component.values.iter().map(|v| v * amplitude).collect();
                scalars = Some(normalize(&values, legend.min, legend.max));
            }
        }
        (coords, scalars, deformed)
    }

    /// GPU-ready meshes of all parts, deformed and coloured by the selected result if any.
    pub fn render_meshes(&self) -> Vec<RenderMesh> {
        let (coords, scalars, deformed) = self.shown_state();
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
                mesh
            })
            .collect()
    }

    /// Section faces of all parts where a plane in model coordinates cuts them, deformed and
    /// coloured like the parts.
    pub fn section_meshes(
        &self,
        point: DVec3,
        normal: DVec3,
        lighten_colors: bool,
    ) -> Vec<RenderMesh> {
        let cells = self.section_cells.get_or_init(|| {
            self.mesh
                .parts
                .iter()
                .map(|part| SectionCells::new(&self.mesh, part))
                .collect()
        });
        let (coords, scalars, _) = self.shown_state();
        cells
            .iter()
            .zip(&self.parts)
            .map(|(cells, info)| {
                let color = if lighten_colors {
                    lighten(info.color)
                } else {
                    info.color
                };
                section_mesh(
                    cells,
                    &coords,
                    self.origin,
                    point,
                    normal,
                    color,
                    scalars.as_deref(),
                )
            })
            .collect()
    }

    /// The model origin in global coordinates; render positions are relative to it.
    pub fn origin(&self) -> DVec3 {
        self.origin
    }

    /// Recolours the vertices of highlighted faces; vertices are laid out face by face.
    fn highlight_faces(&self, part: usize, skin: &PartSkin, mesh: &mut RenderMesh) {
        if self.highlight.outlines.contains(&part) {
            mesh.wide_edges = (mesh.feature_edges.iter())
                .map(|&vertex| Vertex {
                    color: HIGHLIGHT_COLOR,
                    ..vertex
                })
                .collect();
        }
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

    /// Position of a node relative to the model origin as drawn, where picking happens.
    pub fn render_position(&self, index: usize) -> Vec3 {
        self.node_position(index).unwrap_or(Vec3::ZERO)
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

    /// Renames a part as CalculiX needs it: upper case, unique among parts, sets and
    /// surfaces. Sections, boundary conditions and loads on the part follow.
    pub fn rename_part(&mut self, index: usize, name: &str) -> Result<(), String> {
        let name = name.trim().to_ascii_uppercase();
        if name.is_empty() {
            return Err("Der Name darf nicht leer sein.".into());
        }
        if name.len() > 80 {
            return Err("Der Name darf höchstens 80 Zeichen lang sein.".into());
        }
        if !(name.chars()).all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err("Erlaubt sind Buchstaben ohne Umlaute, Ziffern, _ und -.".into());
        }
        let current = self
            .parts
            .get(index)
            .ok_or("Das Part gibt es nicht mehr.")?;
        if current.name == name {
            return Ok(());
        }
        if self.mesh.name_in_use(&name) {
            return Err(format!("Der Name {name} ist bereits vergeben."));
        }
        if let Some(old) = self.mesh.rename_part(index, &name) {
            self.fe.rename_part(&old, &name);
        }
        self.parts[index].name = name;
        Ok(())
    }

    /// The nearest visible face hit by a ray, both relative to the model origin.
    pub fn pick(&self, origin: Vec3, direction: Vec3) -> Option<Hit> {
        let coords = self.shown_coords();
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
                    let point = origin + direction * t;
                    if self.clip.is_some_and(|clip| clip.distance(point) < 0.0) {
                        continue;
                    }
                    if best.is_none_or(|(nearest, _)| t < nearest) {
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
        let coords = self.shown_coords();
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

    /// A point given in model coordinates, in render coordinates.
    pub fn to_render(&self, point: [f64; 3]) -> Vec3 {
        (DVec3::from(point) - self.origin).as_vec3()
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
    fn renamed_parts_keep_their_references() {
        let mut model = load(&testdata("platte_mit_stuetzen.inp")).unwrap().model;
        let (first, second) = (model.parts[0].name.clone(), model.parts[1].name.clone());
        model.fe.sections.push(plx_model::Section {
            name: "Section-1".into(),
            material: "Steel".into(),
            region: plx_model::Region::Parts(vec![first.clone()]),
        });
        assert!(model.rename_part(0, "").is_err());
        assert!(model.rename_part(0, "zwei wörter").is_err());
        assert!(model.rename_part(0, &second.to_lowercase()).is_err());
        assert_eq!(model.rename_part(0, " Platte-1 "), Ok(()));
        assert_eq!(model.parts[0].name, "PLATTE-1");
        assert_eq!(model.mesh.parts[0].name, "PLATTE-1");
        assert!(model.mesh.element_sets.contains_key("PLATTE-1"));
        assert!(!model.mesh.element_sets.contains_key(&first));
        assert_eq!(
            model.fe.sections[0].region,
            plx_model::Region::Parts(vec!["PLATTE-1".into()])
        );
        assert_eq!(model.rename_part(0, "platte-1"), Ok(()));
    }

    #[test]
    fn projects_open_with_their_fe_model() {
        let mut model = load(&testdata("platte_mit_stuetzen.inp")).unwrap().model;
        assert!(!model.is_project());
        model.fe.steps.push(plx_model::Step::new_static("Step-1"));
        let dir = std::env::temp_dir().join(format!("prepolix-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("platte.plx");
        plx_io::project::save_project(&path, &model.mesh, &model.fe).unwrap();
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
    fn multi_part_results_open_in_true_scale() {
        let single = load(&testdata("kragbalken_c3d8.frd")).unwrap().model;
        assert_eq!(
            single.results.unwrap().deformation,
            Deformation::Automatic(1.0)
        );
        // Give the first element another material, which makes it a second part.
        let text = std::fs::read_to_string(testdata("kragbalken_c3d8.frd")).unwrap();
        let text = text.replacen(
            " -1         1    1    0    1",
            " -1         1    1    0    2",
            1,
        );
        let path = std::env::temp_dir().join("prepolix_test_zwei_parts.frd");
        std::fs::write(&path, text).unwrap();
        let multi = load(&path).unwrap().model;
        std::fs::remove_file(&path).ok();
        assert_eq!(multi.parts.len(), 2);
        assert_eq!(multi.results.unwrap().deformation, Deformation::TrueScale);
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

    #[test]
    fn file_without_elements_is_rejected() {
        assert!(load(&testdata("platte_knoten.inp")).is_err());
    }
}
