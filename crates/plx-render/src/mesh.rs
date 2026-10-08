use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

/// Triangulated surface ready for upload, with separate line indices for drawn edges.
#[derive(Clone, Debug, Default)]
pub struct RenderMesh {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<u32>,
    pub edges: Vec<u32>,
}

impl RenderMesh {
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut positions = self.vertices.iter().map(|v| Vec3::from(v.position));
        let first = positions.next()?;
        Some(positions.fold((first, first), |(min, max), p| (min.min(p), max.max(p))))
    }

    /// Unit-sized box with flat-shaded faces, used as a placeholder model.
    pub fn demo_box(size: Vec3, color: [f32; 3]) -> Self {
        let h = size * 0.5;
        let faces: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ];
        let mut mesh = Self::default();
        for (normal, u, v) in faces {
            let base = mesh.vertices.len() as u32;
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let position = (normal + u * su + v * sv) * h;
                mesh.vertices.push(Vertex {
                    position: position.into(),
                    normal: normal.into(),
                    color,
                });
            }
            mesh.triangles
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            mesh.edges.extend([
                base,
                base + 1,
                base + 1,
                base + 2,
                base + 2,
                base + 3,
                base + 3,
                base,
            ]);
        }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_box_is_consistent() {
        let mesh = RenderMesh::demo_box(Vec3::new(2.0, 1.0, 4.0), [1.0, 1.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.triangles.len(), 36);
        assert!(
            mesh.triangles
                .iter()
                .chain(&mesh.edges)
                .all(|&i| (i as usize) < mesh.vertices.len())
        );
        let (min, max) = mesh.bounds().unwrap();
        assert_eq!(min, Vec3::new(-1.0, -0.5, -2.0));
        assert_eq!(max, Vec3::new(1.0, 0.5, 2.0));
    }

    #[test]
    fn demo_box_triangles_face_outward() {
        let mesh = RenderMesh::demo_box(Vec3::ONE, [1.0, 1.0, 1.0]);
        for tri in mesh.triangles.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(mesh.vertices[tri[k] as usize].position));
            let normal = Vec3::from(mesh.vertices[tri[0] as usize].normal);
            assert!((b - a).cross(c - a).dot(normal) > 0.0);
        }
    }

    #[test]
    fn empty_mesh_has_no_bounds() {
        assert!(RenderMesh::default().bounds().is_none());
    }
}
