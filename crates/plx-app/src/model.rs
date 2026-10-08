use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{DVec3, Vec3};
use plx_io::inp::{InpImport, read_inp};
use plx_mesh::{FeMesh, extract_part_skin};
use plx_render::{RenderMesh, part_color, part_render_mesh};

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
}

/// Result of loading on a worker thread: the model plus one GPU-ready mesh per part.
pub struct LoadedModel {
    pub model: Model,
    pub render_meshes: Vec<RenderMesh>,
}

pub fn load(path: &Path) -> Result<LoadedModel, String> {
    let start = Instant::now();
    let InpImport {
        mesh,
        warnings,
        skipped_keywords,
        files,
    } = read_inp(path).map_err(|e| e.to_string())?;
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
    let mut render_meshes = Vec::with_capacity(mesh.parts.len());
    for (index, part) in mesh.parts.iter().enumerate() {
        let color = part_color(index);
        let skin = extract_part_skin(&mesh, part, FEATURE_ANGLE_DEG);
        let render = part_render_mesh(&mesh, &skin, origin, color, SMOOTH_ANGLE_DEG);
        let mut types: BTreeMap<&str, usize> = BTreeMap::new();
        let mut nodes = std::collections::HashSet::new();
        for element in part.elements.iter().filter_map(|&id| mesh.element(id)) {
            *types.entry(&element.type_name).or_default() += 1;
            nodes.extend(element.nodes.iter().copied());
        }
        parts.push(PartInfo {
            name: part.name.clone(),
            color,
            element_count: part.elements.len(),
            node_count: nodes.len(),
            element_types: types.into_iter().map(|(t, n)| (t.to_string(), n)).collect(),
            bounds: render.bounds(),
            visible: true,
        });
        render_meshes.push(render);
    }
    Ok(LoadedModel {
        model: Model {
            path: path.to_path_buf(),
            mesh,
            parts,
            warnings,
            skipped_keywords,
            included_files: files.len().saturating_sub(1),
            load_time: start.elapsed(),
        },
        render_meshes,
    })
}

impl Model {
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
    fn file_without_elements_is_rejected() {
        assert!(load(&testdata("platte_knoten.inp")).is_err());
    }
}
