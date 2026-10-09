use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    /// Surfaces: result value normalized to 0..1 for contour plots, negative where there is
    /// none. Lines: opacity.
    pub scalar: f32,
}

/// Triangulated surface ready for upload, with edges as line lists (two vertices per segment).
#[derive(Clone, Debug, Default)]
pub struct RenderMesh {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<u32>,
    /// Outline of the part, always drawn; also holds line elements such as beams.
    pub feature_edges: Vec<Vertex>,
    /// Every element edge on the surface, drawn when the mesh is shown.
    pub mesh_edges: Vec<Vertex>,
    /// Outline of the undeformed shape, drawn behind deformed results.
    pub wireframe_edges: Vec<Vertex>,
    /// Lines drawn [`WIDE_EDGE_PX`](crate::renderer::WIDE_EDGE_PX) wide over everything
    /// else, such as the outline of a selected part.
    pub wide_edges: Vec<Vertex>,
}

impl RenderMesh {
    /// Adds another mesh to this one, e.g. a transformed copy of a part.
    pub fn append(&mut self, other: RenderMesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.triangles
            .extend(other.triangles.into_iter().map(|index| base + index));
        self.feature_edges.extend(other.feature_edges);
        self.mesh_edges.extend(other.mesh_edges);
        self.wireframe_edges.extend(other.wireframe_edges);
        self.wide_edges.extend(other.wide_edges);
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut positions = self
            .vertices
            .iter()
            .chain(&self.feature_edges)
            .map(|v| Vec3::from(v.position));
        let first = positions.next()?;
        Some(positions.fold((first, first), |(min, max), p| (min.min(p), max.max(p))))
    }
}
