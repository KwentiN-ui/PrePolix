use serde::{Deserialize, Serialize};

/// CAD geometry imported from a STEP, IGES or BREP file, the source of the mesh, like the
/// geometry parts of PrePoMax.
///
/// The shapes are kept in OpenCASCADE's BREP format, so that a project still meshes after the
/// original file moved or changed. Each solid is a part; faces and edges are referred to by
/// the tags Gmsh gives them when it reads the BREP, which stay the same between reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    /// Name of the imported file, for display.
    pub source: String,
    /// The shapes as an OpenCASCADE BREP file.
    pub brep: String,
    /// Meshing parameters of the parts that no Meshing Parameters item covers.
    #[serde(alias = "mesh_setup")]
    pub meshing: MeshingParameters,
    /// PrePoMax's mesh setup items.
    #[serde(default)]
    pub mesh_items: Vec<MeshSetupItem>,
}

impl Geometry {
    /// The meshing parameters of a part: of the last Meshing Parameters item naming it, else
    /// the defaults.
    pub fn parameters(&self, part: &str) -> MeshingParameters {
        (self.mesh_items.iter().rev())
            .find_map(|item| match &item.kind {
                MeshSetupKind::MeshingParameters { parts, parameters } if names(parts, part) => {
                    Some(*parameters)
                }
                _ => None,
            })
            .unwrap_or(self.meshing)
    }

    /// Gmsh's algorithms for a part: of the last Tetrahedral Gmsh item naming it, else the
    /// defaults.
    pub fn algorithms(&self, part: &str) -> (Algorithm2d, Algorithm3d) {
        (self.mesh_items.iter().rev())
            .find_map(|item| match &item.kind {
                MeshSetupKind::TetrahedralGmsh {
                    parts,
                    algorithm_2d,
                    algorithm_3d,
                } if names(parts, part) => Some((*algorithm_2d, *algorithm_3d)),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The local mesh sizes as (faces, edges, size).
    pub fn local_sizes(&self) -> impl Iterator<Item = (&[i32], &[i32], f64)> {
        self.mesh_items.iter().filter_map(|item| match &item.kind {
            MeshSetupKind::LocalMeshSize { faces, edges, size } => {
                Some((faces.as_slice(), edges.as_slice(), *size))
            }
            _ => None,
        })
    }
}

fn names(parts: &[String], part: &str) -> bool {
    parts.iter().any(|p| p.eq_ignore_ascii_case(part))
}

/// How a part is meshed with tetrahedra, PrePoMax's meshing parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshingParameters {
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

impl Default for MeshingParameters {
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

impl MeshingParameters {
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

/// An item of PrePoMax's mesh setup, refining how parts are meshed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshSetupItem {
    pub name: String,
    pub kind: MeshSetupKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MeshSetupKind {
    /// Own meshing parameters for some parts.
    MeshingParameters {
        parts: Vec<String>,
        parameters: MeshingParameters,
    },
    /// A smaller element size on faces and edges.
    LocalMeshSize {
        faces: Vec<i32>,
        edges: Vec<i32>,
        size: f64,
    },
    /// Gmsh's meshing algorithms for some parts.
    TetrahedralGmsh {
        parts: Vec<String>,
        algorithm_2d: Algorithm2d,
        algorithm_3d: Algorithm3d,
    },
}

impl MeshSetupKind {
    /// PrePoMax's name of the item type, also the prefix of new items' names.
    pub fn type_name(&self) -> &'static str {
        match self {
            MeshSetupKind::MeshingParameters { .. } => "Meshing_Parameters",
            MeshSetupKind::LocalMeshSize { .. } => "Local_Mesh_Size",
            MeshSetupKind::TetrahedralGmsh { .. } => "Tetrahedral_Gmsh",
        }
    }
}

/// Gmsh's algorithm for meshing the faces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algorithm2d {
    MeshAdapt,
    Automatic,
    Delaunay,
    #[default]
    FrontalDelaunay,
}

/// Gmsh's algorithm for meshing the volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algorithm3d {
    #[default]
    Delaunay,
    Frontal,
    Hxt,
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
        let setup = MeshingParameters::for_diagonal(173.2);
        assert!((setup.max_size - 9.0).abs() < 1e-9, "{}", setup.max_size);
        assert!((setup.min_size - 0.2).abs() < 1e-9, "{}", setup.min_size);
        assert_eq!(round_size(0.0), 0.0);
    }

    #[test]
    fn items_override_the_defaults_of_their_parts() {
        let fine = MeshingParameters {
            max_size: 1.0,
            ..MeshingParameters::default()
        };
        let mut geometry = Geometry {
            source: "a.step".into(),
            brep: String::new(),
            meshing: MeshingParameters::for_diagonal(100.0),
            mesh_items: vec![
                MeshSetupItem {
                    name: "Meshing_Parameters-1".into(),
                    kind: MeshSetupKind::MeshingParameters {
                        parts: vec!["SOLID-1".into()],
                        parameters: fine,
                    },
                },
                MeshSetupItem {
                    name: "Tetrahedral_Gmsh-1".into(),
                    kind: MeshSetupKind::TetrahedralGmsh {
                        parts: vec!["SOLID-2".into()],
                        algorithm_2d: Algorithm2d::Delaunay,
                        algorithm_3d: Algorithm3d::Hxt,
                    },
                },
                MeshSetupItem {
                    name: "Local_Mesh_Size-1".into(),
                    kind: MeshSetupKind::LocalMeshSize {
                        faces: vec![3],
                        edges: vec![],
                        size: 0.5,
                    },
                },
            ],
        };
        assert_eq!(geometry.parameters("solid-1"), fine);
        assert_eq!(geometry.parameters("SOLID-2").max_size, 5.0);
        assert_eq!(
            geometry.algorithms("SOLID-2"),
            (Algorithm2d::Delaunay, Algorithm3d::Hxt)
        );
        assert_eq!(geometry.algorithms("SOLID-1"), Default::default());
        let local: Vec<_> = geometry.local_sizes().collect();
        assert_eq!(local, vec![(&[3][..], &[][..], 0.5)]);
        geometry.mesh_items.clear();
        assert_eq!(geometry.parameters("SOLID-1").max_size, 5.0);
    }

    #[test]
    fn projects_of_the_first_geometry_version_still_read() {
        let text = r#"(source: "a.step", brep: "", mesh_setup: (max_size: 3.0))"#;
        let geometry: Geometry = ron::from_str(text).unwrap();
        assert_eq!(geometry.meshing.max_size, 3.0);
        assert!(geometry.mesh_items.is_empty());
    }
}
