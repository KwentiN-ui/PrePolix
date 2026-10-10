//! Project files (`.plx`): the mesh and the FE model in RON, written by serde.

use std::path::{Path, PathBuf};

use plx_mesh::FeMesh;
use plx_model::{FeModel, Geometry, PROJECT_FORMAT, Project};

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: not a valid project file: {message}")]
    Format { path: PathBuf, message: String },
    #[error("{path} comes from a newer prepolix version (format {format})")]
    Newer { path: PathBuf, format: u32 },
}

/// Writes the project; a crash while saving leaves an existing file intact.
pub fn save_project(
    path: &Path,
    geometry: Option<&Geometry>,
    mesh: &FeMesh,
    model: &FeModel,
) -> Result<(), ProjectError> {
    let project = Project {
        format: PROJECT_FORMAT,
        geometry: geometry.cloned(),
        mesh: mesh.clone(),
        model: model.clone(),
    };
    let text = ron::to_string(&project).map_err(|e| ProjectError::Format {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    let io = |source| ProjectError::Io {
        path: path.to_owned(),
        source,
    };
    let temporary = path.with_extension("plx.tmp");
    std::fs::write(&temporary, text).map_err(io)?;
    std::fs::rename(&temporary, path).map_err(io)
}

pub fn read_project(path: &Path) -> Result<Project, ProjectError> {
    let text = std::fs::read_to_string(path).map_err(|source| ProjectError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut project: Project = ron::from_str(&text).map_err(|e| ProjectError::Format {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    if project.format > PROJECT_FORMAT {
        return Err(ProjectError::Newer {
            path: path.to_owned(),
            format: project.format,
        });
    }
    project.model.migrate();
    Ok(project)
}

#[cfg(test)]
mod tests {
    use plx_model::{
        Algorithm2d, Algorithm3d, Constraint, ContactPair, Friction, InteractionProperty, Material,
        MeshSetupItem, MeshSetupKind, MeshingParameters, Region, Step, SurfaceBehavior,
        SurfaceInteraction, Tie, UserKeyword,
    };

    use super::*;
    use crate::inp::read_inp;

    #[test]
    fn projects_read_back_what_was_saved() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mesh = read_inp(&root.join("testdata/block_c3d20r.inp"))
            .unwrap()
            .mesh;
        let model = FeModel {
            properties: Default::default(),
            materials: vec![Material {
                name: "Steel".into(),
                density: Some(7.85e-9),
                elastic: None,
                ..Default::default()
            }],
            sections: Vec::new(),
            steps: vec![Step::new_static("Step-1"), Step::new_coupled("Step-2")],
            user_keywords: vec![UserKeyword {
                position: vec![14, 0],
                text: "*Amplitude, Name=A\n0, 0, 1, 1".into(),
                active: false,
            }],
            reference_points: vec![plx_model::ReferencePoint::new("RP-1", [1.0, 2.0, 3.0])],
            coordinate_systems: vec![plx_model::CoordinateSystem {
                kind: plx_model::CoordinateSystemKind::Cylindrical,
                ..plx_model::CoordinateSystem::new("Coordinate_System-1")
            }],
            planes: vec![plx_model::Plane {
                source: plx_model::PlaneSource::ThreePoints {
                    points: [
                        plx_model::PointRef::ReferencePoint("RP-1".into()),
                        plx_model::PointRef::Coordinates([1.0, 0.0, 0.0]),
                        plx_model::PointRef::Coordinates([0.0, 1.0, 0.0]),
                    ],
                },
                ..plx_model::Plane::new("Plane-1")
            }],
            result_planes: vec![plx_model::ResultPlane {
                name: "Plane_Result-1".into(),
                plane: "Plane-1".into(),
            }],
            result_paths: vec![plx_model::ResultPath {
                start: plx_model::PointRef::ReferencePoint("RP-1".into()),
                ..plx_model::ResultPath::new("Path-1")
            }],
            constraints: Vec::new(),
            ties: vec![Tie {
                master: Region::Faces(vec![(1, 2)]),
                slave: Region::Surface("TOP".into()),
                ..Tie::new("Tie-1")
            }],
            node_ties: vec![plx_model::NodeTie {
                region: Region::Nodes(vec![1, 2]),
                rotations: false,
                ..plx_model::NodeTie::new("Node_Tie-1")
            }],
            surface_interactions: vec![SurfaceInteraction {
                name: "Surface_Interaction-1".into(),
                properties: vec![
                    InteractionProperty::SurfaceBehavior(SurfaceBehavior::Tabular(vec![
                        [0.0, 0.0],
                        [1e5, 1.0],
                    ])),
                    InteractionProperty::Friction(Friction::default()),
                ],
            }],
            contact_pairs: vec![ContactPair {
                adjustment_size: Some(0.01),
                master_color: [255, 0, 0],
                ..ContactPair::new("Contact_Pair-1", "Surface_Interaction-1")
            }],
            amplitudes: vec![plx_model::Amplitude {
                time_span: plx_model::AmplitudeTime::Total,
                points: vec![[0.0, 0.0], [1.0, 2.5]],
                ..plx_model::Amplitude::new("Amplitude-1")
            }],
            initial_conditions: vec![plx_model::InitialCondition {
                name: "Initial_Temperature-1".into(),
                active: true,
                region: Region::Parts(vec!["A".into()]),
                kind: plx_model::InitialConditionKind::Temperature(20.0),
            }],
        };
        let dir = std::env::temp_dir().join(format!("plx-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("block.plx");
        let geometry = Geometry {
            source: "block.step".into(),
            brep: "DBRep_DrawableShape\n\"quoted\" lines\n".into(),
            meshing: MeshingParameters::for_diagonal(100.0),
            mesh_items: vec![
                MeshSetupItem {
                    name: "Local_Mesh_Size-1".into(),
                    kind: MeshSetupKind::LocalMeshSize {
                        faces: vec![3, 4],
                        edges: vec![7],
                        size: 0.5,
                    },
                },
                MeshSetupItem {
                    name: "Tetrahedral_Gmsh-1".into(),
                    kind: MeshSetupKind::TetrahedralGmsh {
                        parts: vec!["SOLID-1".into()],
                        algorithm_2d: Algorithm2d::Delaunay,
                        algorithm_3d: Algorithm3d::Hxt,
                    },
                },
            ],
            part_names: vec!["SOLID-1".into()],
        };
        save_project(&path, Some(&geometry), &mesh, &model).unwrap();
        let project = read_project(&path).unwrap();
        assert_eq!(project.model, model);
        assert_eq!(project.geometry, Some(geometry));
        assert_eq!(project.mesh.elements(), mesh.elements());
        assert_eq!(project.mesh.coords(), mesh.coords());
        assert_eq!(project.mesh.node_sets, mesh.node_sets);

        let newer = std::fs::read_to_string(&path).unwrap().replacen(
            &format!("format:{PROJECT_FORMAT}"),
            "format:99",
            1,
        );
        std::fs::write(&path, newer).unwrap();
        assert!(matches!(
            read_project(&path),
            Err(ProjectError::Newer { format: 99, .. })
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn steps_of_older_projects_are_active() {
        let mut step = Step::new_static("Step-1");
        step.active = false;
        let text = ron::to_string(&step).unwrap();
        assert!(text.contains("active:false,"), "{text}");
        let older: Step = ron::from_str(&text.replace("active:false,", "")).unwrap();
        assert!(older.active);
    }

    #[test]
    fn ties_of_older_projects_move_to_the_contact_pairs() {
        // Ties and node ties were constraints; a project saved then still opens with them.
        let mut tie = plx_model::NodeTie::new("Node_Tie-1");
        tie.region = Region::Nodes(vec![3, 7]);
        let older = FeModel {
            constraints: vec![
                Constraint::Tie(Tie::new("Tie-1")),
                Constraint::NodeTie(tie.clone()),
            ],
            ..FeModel::default()
        };
        let mut model: FeModel = ron::from_str(&ron::to_string(&older).unwrap()).unwrap();
        model.migrate();
        assert_eq!(model.constraints, []);
        assert_eq!(model.ties, [Tie::new("Tie-1")]);
        assert_eq!(model.node_ties, [tie]);
    }

    #[test]
    fn hot_spots_of_older_projects_are_skipped() {
        // Hot spots were part of the FE model for a while; they are defined on the results
        // now, and projects saved with them still open.
        let text = ron::to_string(&FeModel::default()).unwrap();
        let hot_spots = "hot_spots:[(name:\"Hot_Spot-1\",toe:Nodes([3,7]),\
            direction:(1.0,0.0,0.0),thickness:10.0,extrapolation:Custom([2.0,6.0]),\
            component:Perpendicular)],";
        let older = text.replacen("user_keywords:", &format!("{hot_spots}user_keywords:"), 1);
        assert_ne!(older, text);
        let model: FeModel = ron::from_str(&older).unwrap();
        assert_eq!(model, FeModel::default());
    }
}
