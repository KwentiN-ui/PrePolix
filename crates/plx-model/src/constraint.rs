//! PrePoMax's spring and support constraints, and prepolix's spring connection between two
//! surfaces. Ties are in [`crate::contact`].
//!
//! Like everything in the model they store what the user defined. The spring and gap elements,
//! extra nodes and equations CalculiX needs for them are generated when the input file is
//! written.

use serde::{Deserialize, Serialize};

use crate::Region;

/// Nodes tied to each other, prepolix's node tie: the ends of beams or trusses meeting at a
/// point, which have nodes of their own since parts share no nodes. Every node of the region
/// moves with the first. Node ties are listed with the contact pairs, as the contact search
/// finds them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeTie {
    pub name: String,
    #[serde(default = "crate::active")]
    pub active: bool,
    pub region: Region,
    /// Ties the rotations too, a rigid joint between beams; a hinge otherwise. Nodes
    /// without rotations (trusses, solids) are tied in their translations only either way.
    pub rotations: bool,
}

impl NodeTie {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            region: Region::Nodes(Vec::new()),
            rotations: true,
        }
    }
}

/// Nodes that move as one rigid body with a reference point (`*RIGID BODY`), PrePoMax's rigid
/// body constraint. Boundary conditions and loads on the reference point drive the body:
/// its translations and rotations, forces and moments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigidBody {
    pub name: String,
    #[serde(default = "crate::active")]
    pub active: bool,
    pub region: Region,
    /// Name of the reference point ([`crate::ReferencePoint`]) the body is driven by.
    pub reference_point: String,
}

impl RigidBody {
    pub fn new(name: impl Into<String>, reference_point: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            active: true,
            region: Region::Faces(Vec::new()),
            reference_point: reference_point.into(),
        }
    }
}

/// Springs from every node of the region to ground (`SPRING1`), PrePoMax's point spring.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointSpring {
    pub name: String,
    #[serde(default = "crate::active")]
    pub active: bool,
    pub region: Region,
    /// K1..K3 of each node's spring in force per length; a zero adds no spring in that
    /// direction.
    pub stiffness: [f64; 3],
}

/// Springs from the nodes of a surface to ground, spread over the nodes by area, PrePoMax's
/// surface spring.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSpring {
    pub name: String,
    #[serde(default = "crate::active")]
    pub active: bool,
    pub region: Region,
    /// K1..K3 of the whole surface (force per length), or per area of it (force per volume)
    /// with `per_area`.
    pub stiffness: [f64; 3],
    pub per_area: bool,
}

/// Gap elements on a surface that only take pressure (`GAPUNI`), PrePoMax's compression only
/// support.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompressionOnly {
    pub name: String,
    #[serde(default = "crate::active")]
    pub active: bool,
    pub region: Region,
    /// Distance the surface can move away before the support takes load.
    pub clearance: f64,
    /// Stiffness of the closed gaps of the whole surface; `None` is PrePoMax's default.
    pub spring_stiffness: Option<f64>,
    /// Tensile force of the whole surface at negative infinity; `None` is PrePoMax's
    /// default.
    pub tensile_force: Option<f64>,
    /// Distance by which the ground node of each gap lies outside the surface.
    pub offset: f64,
    /// Solves the support nonlinearly; otherwise it is linearized in a linear step.
    pub nonlinear: bool,
}

impl CompressionOnly {
    /// PrePoMax's `GapSectionData.InitialSpringStiffness`.
    pub const DEFAULT_STIFFNESS: f64 = 1e12;
    /// PrePoMax's `GapSectionData.InitialTensileForceAtNegativeInfinity`.
    pub const DEFAULT_TENSILE_FORCE: f64 = 1e-3;
}

/// Springs between two surfaces instead of to ground; not in PrePoMax. Every node of the
/// slave surface is connected to the closest point of the master surface by springs in the
/// global directions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceToSurfaceSpring {
    pub name: String,
    #[serde(default = "crate::active")]
    pub active: bool,
    /// The surface the springs end on.
    pub master: Region,
    /// The surface whose nodes get the springs.
    pub slave: Region,
    /// K1..K3 of the whole connection, or per area of the slave surface with `per_area`.
    pub stiffness: [f64; 3],
    pub per_area: bool,
}
