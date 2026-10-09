use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{DAffine3, DVec3, Vec3};
use plx_io::frd::{FrdImport, read_frd};
use plx_io::inp::{InpImport, read_inp};
use plx_mesh::{ElementId, FeMesh, NodeId, PartSkin, extract_part_skin};
use plx_mesher::{CadEntity, GeometryDisplay};
use plx_model::{FeModel, Geometry};
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

/// Colour of the second region of an item, such as the slave surface of a contact pair:
/// PrePoMax's secondary highlight colour, violet.
pub const SECONDARY_HIGHLIGHT_COLOR: [f32; 3] = [238.0 / 255.0, 130.0 / 255.0, 238.0 / 255.0];

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
    /// For the display of CAD geometry, the CAD face or edge of each element; element ids
    /// count from 1.
    cad_entities: Vec<CadEntity>,
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
    /// Element faces in the secondary highlight colour, e.g. slave surfaces.
    pub secondary_faces: HashSet<(ElementId, u8)>,
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
        // Several parts usually mean contact, where an exaggerated deformation shows parts
        // penetrating each other; true scale is less confusing there.
        if model.mesh.parts.len() > 1 {
            view.deformation = Deformation::TrueScale;
        }
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
            cad_entities: Vec::new(),
            hot_spots: None,
            highlight: Highlight::default(),
            clip: None,
            origin,
            skins,
            section_cells: Default::default(),
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
        model.cad_entities = display.entities;
        model
    }

    /// The CAD face or edge an element of the geometry display shows.
    pub fn cad_entity(&self, element: ElementId) -> Option<CadEntity> {
        let index = usize::try_from(element).ok()?.checked_sub(1)?;
        self.cad_entities.get(index).copied()
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

    /// Node coordinates as drawn: with the shown deformation of results.
    pub fn shown_coords(&self) -> std::borrow::Cow<'_, [[f64; 3]]> {
        let coords = self.mesh.coords();
        let Some(view) = &self.results else {
            return std::borrow::Cow::Borrowed(coords);
        };
        let scale = (view.scale() * view.amplitude()) as f64;
        match view.shown_displacements() {
            Some(displacements) if scale != 0.0 => std::borrow::Cow::Owned(
                coords
                    .iter()
                    .zip(displacements.iter())
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
            if let (Some(values), Some(legend)) = (view.shown_values(), view.legend()) {
                scalars = Some(normalize(&values, legend.min, legend.max));
            }
        }
        (coords, scalars, deformed)
    }

    /// The transformed copies of the results ([`ResultsView::transformations`]): per copy
    /// its transformation and its normalized contour values.
    fn transformed_copies(&self) -> Vec<(DAffine3, Option<Vec<f32>>)> {
        let Some(view) = &self.results else {
            return Vec::new();
        };
        let legend = view.legend();
        (view.instances().into_iter().skip(1))
            .map(|instance| {
                let scalars = (view.shown_values_on(&instance))
                    .zip(legend.as_ref())
                    .map(|(values, legend)| normalize(&values, legend.min, legend.max));
                (instance, scalars)
            })
            .collect()
    }

    /// GPU-ready meshes of all parts, deformed and coloured by the selected result if any,
    /// with the transformed copies of the results.
    pub fn render_meshes(&self) -> Vec<RenderMesh> {
        let (coords, scalars, deformed) = self.shown_state();
        let mut meshes = self.part_meshes(&coords, scalars.as_deref(), deformed);
        for (instance, scalars) in self.transformed_copies() {
            let moved = transform_coords(&coords, &instance);
            let undeformed = deformed.then(|| transform_coords(self.mesh.coords(), &instance));
            for ((mesh, info), skin) in meshes.iter_mut().zip(&self.parts).zip(&self.skins) {
                let mut copy = part_render_mesh(
                    &moved,
                    skin,
                    self.origin,
                    info.color,
                    SMOOTH_ANGLE_DEG,
                    scalars.as_deref(),
                );
                if let Some(undeformed) = &undeformed {
                    copy.wireframe_edges = wireframe_edges(undeformed, skin, self.origin);
                }
                mesh.append(copy);
            }
        }
        meshes
    }

    fn part_meshes(
        &self,
        coords: &[[f64; 3]],
        scalars: Option<&[f32]>,
        deformed: bool,
    ) -> Vec<RenderMesh> {
        self.mesh
            .parts
            .iter()
            .zip(&self.parts)
            .zip(&self.skins)
            .enumerate()
            .map(|(index, ((_, info), skin))| {
                let mut mesh = part_render_mesh(
                    coords,
                    skin,
                    self.origin,
                    info.color,
                    SMOOTH_ANGLE_DEG,
                    scalars,
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
        let color = |info: &PartInfo| {
            if lighten_colors {
                lighten(info.color)
            } else {
                info.color
            }
        };
        let mut meshes: Vec<RenderMesh> = cells
            .iter()
            .zip(&self.parts)
            .map(|(cells, info)| {
                let color = color(info);
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
            .collect();
        // The copies are cut by the same plane.
        for (instance, scalars) in self.transformed_copies() {
            let moved = transform_coords(&coords, &instance);
            for ((mesh, cells), info) in meshes.iter_mut().zip(cells).zip(&self.parts) {
                let color = color(info);
                let scalars = scalars.as_deref();
                mesh.append(section_mesh(
                    cells,
                    &moved,
                    self.origin,
                    point,
                    normal,
                    color,
                    scalars,
                ));
            }
        }
        meshes
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
        if self.highlight.faces.is_empty()
            && self.highlight.secondary_faces.is_empty()
            && !self.highlight.parts.contains(&part)
        {
            return;
        }
        let whole = self.highlight.parts.contains(&part);
        let elements = self.mesh.elements();
        let mut start = 0;
        for face in &skin.faces {
            let count = face.corners.len() + face.mids.len();
            let key = (elements[face.element].id, face.face as u8 + 1);
            let color = if whole || self.highlight.faces.contains(&key) {
                Some(HIGHLIGHT_COLOR)
            } else {
                (self.highlight.secondary_faces.contains(&key)).then_some(SECONDARY_HIGHLIGHT_COLOR)
            };
            if let Some(color) = color {
                for vertex in &mut mesh.vertices[start..start + count] {
                    vertex.color = color;
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

    /// Skins of all parts, in the order of the mesh's parts.
    pub fn skins(&self) -> &[PartSkin] {
        &self.skins
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
            let displacement = view.shown_displacement(index);
            if let (Some(d), true) = (displacement, scale != 0.0) {
                p += scale * DVec3::new(d[0] as f64, d[1] as f64, d[2] as f64);
            }
        }
        Some((p - self.origin).as_vec3())
    }

    /// Where a node is drawn on the given item of the results ([`ResultsView::instances`]).
    pub fn node_position_on(&self, index: usize, item: usize) -> Option<Vec3> {
        let position = self.node_position(index)?;
        if item == 0 {
            return Some(position);
        }
        let instance = *self.results.as_ref()?.instances().get(item)?;
        let global = position.as_dvec3() + self.origin;
        Some((instance.transform_point3(global) - self.origin).as_vec3())
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

fn transform_coords(coords: &[[f64; 3]], instance: &DAffine3) -> Vec<[f64; 3]> {
    (coords.iter())
        .map(|&p| instance.transform_point3(DVec3::from(p)).to_array())
        .collect()
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
    fn transformed_copies_are_drawn_with_mirrored_values() {
        use plx_results::transformation::{SymmetryPlane, Transformation};
        let mut model = load(&testdata("kragbalken_c3d8.frd")).unwrap().model;
        let single = model.render_meshes()[0].vertices.len();
        let view = model.results.as_mut().unwrap();
        let fields = &view.current_increment().unwrap().fields;
        let field = fields.iter().position(|f| f.name == "DISP").unwrap();
        let components = &fields[field].components;
        let component = components.iter().position(|c| c.name == "U3").unwrap();
        (view.field, view.component) = (field, component);
        let (min, max) = view.legend().map(|l| (l.min, l.max)).unwrap();
        view.transformations = vec![Transformation::symmetry(SymmetryPlane::Z)];
        // Mirrored at z, the beam bending down shows a copy bending up.
        let legend = view.legend().unwrap();
        assert_eq!((legend.min, legend.max), (min.min(-max), max.max(-min)));
        assert_eq!(view.maximum().map(|m| (m.1, m.2)), Some((-min, 1)));
        assert_eq!(model.render_meshes()[0].vertices.len(), 2 * single);
        let node = model.mesh.coords().len() - 1;
        let (a, b) = (
            model.node_position_on(node, 0).unwrap(),
            model.node_position_on(node, 1).unwrap(),
        );
        let global_z = |p: Vec3| p.z as f64 + model.origin().z;
        assert!((global_z(a) + global_z(b)).abs() < 1e-3, "{a} {b}");
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
        geometry.meshing.max_size = 4.0;
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
            active: true,
            region: Region::Nodes(fixed),
            kind: BoundaryKind::Fixed,
        });
        step.boundary_conditions.push(BoundaryCondition {
            name: "Pull".into(),
            active: true,
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
            ..FeModel::default()
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
