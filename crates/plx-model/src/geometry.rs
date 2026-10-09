use serde::{Deserialize, Serialize};

/// CAD geometry imported from a STEP, IGES or BREP file, the source of the mesh, like the
/// geometry parts of PrePoMax.
///
/// The shapes are kept in OpenCASCADE's BREP format, so that a project still meshes after the
/// original file moved or changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    /// Name of the imported file, for display.
    pub source: String,
    /// The shapes as an OpenCASCADE BREP file.
    pub brep: String,
    pub mesh_setup: MeshSetup,
}

/// How the geometry is meshed with tetrahedra, PrePoMax's meshing parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshSetup {
    pub max_size: f64,
    pub min_size: f64,
    /// Elements per curvature radius; curved faces get smaller elements.
    pub elements_per_curvature: f64,
    /// Quadratic tetrahedra (C3D10) instead of linear ones (C3D4).
    pub second_order: bool,
    /// Midside nodes moved onto curved geometry instead of the straight edge's middle.
    pub midside_nodes_on_geometry: bool,
    /// Improves element quality after meshing with Netgen's optimiser.
    pub optimize: bool,
}

impl Default for MeshSetup {
    fn default() -> Self {
        Self {
            max_size: 1000.0,
            min_size: 0.0,
            elements_per_curvature: 2.0,
            second_order: true,
            midside_nodes_on_geometry: false,
            optimize: true,
        }
    }
}

impl MeshSetup {
    /// PrePoMax's sizes for a new geometry: 5 % and 0.1 % of the bounding box diagonal,
    /// rounded to one significant digit.
    pub fn for_diagonal(diagonal: f64) -> Self {
        Self {
            max_size: round_size(0.05 * diagonal),
            min_size: round_size(0.001 * diagonal),
            ..Self::default()
        }
    }
}

/// Rounds to one significant digit, e.g. 0.0347 to 0.03.
fn round_size(size: f64) -> f64 {
    if !(size.is_finite() && size > 0.0) {
        return size;
    }
    let scale = 10f64.powf(size.log10().floor());
    (size / scale).round() * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_the_diagonal() {
        let setup = MeshSetup::for_diagonal(173.2);
        assert!((setup.max_size - 9.0).abs() < 1e-9, "{}", setup.max_size);
        assert!((setup.min_size - 0.2).abs() < 1e-9, "{}", setup.min_size);
        assert_eq!(round_size(0.0), 0.0);
    }
}
