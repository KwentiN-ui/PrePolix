//! FE-Netz: Knoten, Elemente, Sets, Surfaces, Geometrie-Topologie.

mod element;
mod fast_map;
mod mesh;
mod skin;

pub use element::{ElementFamily, ElementShape, FaceTopology};
pub use mesh::{Element, ElementId, FeMesh, MeshError, NodeId, Part, SurfaceDefinition};
pub use skin::{PartSkin, SkinEdge, SkinFace, extract_part_skin, face_normal};
