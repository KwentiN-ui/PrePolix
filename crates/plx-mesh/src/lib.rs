//! FE-Netz: Knoten, Elemente, Sets, Surfaces, Geometrie-Topologie.

mod contact_search;
mod element;
mod fast_map;
mod jacobian;
mod mesh;
mod skin;

pub use contact_search::{
    GroupBy, MasterSlaveItem, SearchParameters, SurfaceId, find_contact_pairs, surface_faces,
};
pub use element::{ElementFamily, ElementShape, FaceTopology};
pub use mesh::{Element, ElementId, FeMesh, MeshError, NodeId, Part, SurfaceDefinition};
pub use skin::{PartSkin, SkinEdge, SkinFace, extract_part_skin, face_normal};
