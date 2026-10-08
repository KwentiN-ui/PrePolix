use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{DVec3, Vec3};
use plx_io::frd::{FrdImport, read_frd};
use plx_io::inp::{InpImport, read_inp};
use plx_mesh::{FeMesh, PartSkin, extract_part_skin};
use plx_render::contour::normalize;
use plx_render::{RenderMesh, part_color, part_render_mesh, wireframe_edges};

use crate::results::ResultsView;

/// Angle between neighbouring faces above which their common edge counts as a feature edge
/// and the shading across it stays sharp.
const FEATURE_ANGLE_DEG: f64 = 30.0;
/// Within a surface patch, coarse meshes of curved surfaces can fold more than the feature
/// angle between neighbouring faces; shading still blends across such folds.
const SMOOTH_ANGLE_DEG: f64 = 60.0;

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
    /// Results read from an `.frd` file, with what the user currently looks at.
    pub results: Option<ResultsView>,
    /// Centre of the mesh; render positions are relative to it.
    origin: DVec3,
    skins: Vec<PartSkin>,
}

/// Result of loading on a worker thread: the model plus one GPU-ready mesh per part.
pub struct LoadedModel {
    pub model: Model,
    pub render_meshes: Vec<RenderMesh>,
}

pub fn load(path: &Path) -> Result<LoadedModel, String> {
    let start = Instant::now();
    let is_frd = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("frd"));
    let (mesh, warnings, skipped_keywords, included_files, increments) = if is_frd {
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
        origin,
        skins,
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
            .map(|((_, info), skin)| {
                let mut mesh = part_render_mesh(
                    &coords,
                    skin,
                    self.origin,
                    info.color,
                    SMOOTH_ANGLE_DEG,
                    scalars.as_deref(),
                );
                if deformed {
                    mesh.wireframe_edges = wireframe_edges(self.mesh.coords(), skin, self.origin);
                }
                mesh
            })
            .collect()
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
    fn file_without_elements_is_rejected() {
        assert!(load(&testdata("platte_knoten.inp")).is_err());
    }
}
