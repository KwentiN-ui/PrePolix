use glam::DVec3;
use plx_mesh::{FeMesh, PartSkin, SkinFace, face_normal};

use crate::mesh::{RenderMesh, Vertex};

const FEATURE_EDGE_COLOR: [f32; 3] = [0.0, 0.0, 0.0];
const MESH_EDGE_COLOR: [f32; 3] = [0.08, 0.08, 0.1];

/// Default part colors in the spirit of PrePoMax, as sRGB.
const PART_COLORS_SRGB: [[u8; 3]; 8] = [
    [148, 182, 214],
    [214, 170, 120],
    [150, 200, 150],
    [210, 150, 170],
    [190, 180, 220],
    [220, 210, 140],
    [140, 200, 200],
    [200, 160, 140],
];

/// Linear RGB color for the n-th part.
pub fn part_color(index: usize) -> [f32; 3] {
    PART_COLORS_SRGB[index % PART_COLORS_SRGB.len()].map(|c| srgb_to_linear(c as f32 / 255.0))
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Builds the GPU-ready geometry of one part. Positions are shifted by `-origin`, so that
/// large coordinates keep their precision in `f32`.
pub fn part_render_mesh(
    mesh: &FeMesh,
    skin: &PartSkin,
    origin: DVec3,
    color: [f32; 3],
) -> RenderMesh {
    let coords = mesh.coords();
    let position = |node: usize| (DVec3::from(coords[node]) - origin).as_vec3().to_array();
    let mut render = RenderMesh::default();

    for face in &skin.faces {
        let normal = face_normal(coords, &face.corners).map(|c| c as f32);
        let base = render.vertices.len() as u32;
        for &node in face.corners.iter().chain(&face.mids) {
            render.vertices.push(Vertex {
                position: position(node),
                normal,
                color,
            });
        }
        render
            .triangles
            .extend(face_triangles(face).iter().map(|&local| base + local));
    }

    let line_vertex = |node: usize, color: [f32; 3]| Vertex {
        position: position(node),
        normal: [0.0; 3],
        color,
    };
    for edge in &skin.edges {
        let segments: &[[usize; 2]] = match edge.mid {
            Some(mid) => &[[edge.a, mid], [mid, edge.b]],
            None => &[[edge.a, edge.b]],
        };
        for &[a, b] in segments {
            let target = if edge.feature {
                &mut render.feature_edges
            } else {
                &mut render.mesh_edges
            };
            let color = if edge.feature {
                FEATURE_EDGE_COLOR
            } else {
                MESH_EDGE_COLOR
            };
            target.extend([line_vertex(a, color), line_vertex(b, color)]);
        }
    }
    let line_color = color.map(|c| c * 0.5);
    for &[a, b] in &skin.lines {
        render
            .feature_edges
            .extend([line_vertex(a, line_color), line_vertex(b, line_color)]);
    }
    render
}

/// Triangles of a face as indices into its corner-then-mid vertex list.
fn face_triangles(face: &SkinFace) -> &'static [u32] {
    match (face.corners.len(), face.mids.is_empty()) {
        (3, true) => &[0, 1, 2],
        (4, true) => &[0, 1, 2, 0, 2, 3],
        // Corners 0..3, mids 3..6 (m0 between c0 and c1, …).
        (3, false) => &[0, 3, 5, 3, 1, 4, 5, 4, 2, 3, 4, 5],
        // Corners 0..4, mids 4..8.
        (4, false) => &[0, 4, 7, 4, 1, 5, 5, 2, 6, 6, 3, 7, 4, 5, 7, 5, 6, 7],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_mesh::{Element, ElementShape, Part, extract_part_skin};

    fn single_element(shape: ElementShape, coords: &[[f64; 3]]) -> FeMesh {
        let mut mesh = FeMesh::default();
        for (i, c) in coords.iter().enumerate() {
            mesh.set_node(i as u32 + 1, *c);
        }
        mesh.add_element(Element {
            id: 1,
            type_name: format!("{shape:?}"),
            shape,
            nodes: (1..=coords.len() as u32).collect(),
        })
        .unwrap();
        mesh.parts.push(Part {
            name: "P".into(),
            elements: vec![1],
        });
        mesh
    }

    fn triangle_area_sum(render: &RenderMesh) -> f32 {
        render
            .triangles
            .chunks(3)
            .map(|t| {
                let [a, b, c] =
                    [0, 1, 2].map(|k| glam::Vec3::from(render.vertices[t[k] as usize].position));
                (b - a).cross(c - a).length() * 0.5
            })
            .sum()
    }

    #[test]
    fn quadratic_quad_is_split_without_gaps() {
        let coords = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [1.0, 2.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let mesh = single_element(ElementShape::Quad8, &coords);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        let render = part_render_mesh(&mesh, &skin, DVec3::ZERO, [1.0; 3]);
        assert_eq!(render.triangles.len(), 6 * 3);
        assert!((triangle_area_sum(&render) - 4.0).abs() < 1e-5);
        // Four curved boundary edges of two segments each, all on the outline.
        assert_eq!(render.feature_edges.len(), 4 * 2 * 2);
        assert!(render.mesh_edges.is_empty());
    }

    #[test]
    fn quadratic_tet_surface_area_is_preserved() {
        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let mid = |a: usize, b: usize| [0, 1, 2].map(|k| (corners[a][k] + corners[b][k]) / 2.0);
        let mut coords = corners.to_vec();
        coords.extend([
            mid(0, 1),
            mid(1, 2),
            mid(2, 0),
            mid(0, 3),
            mid(1, 3),
            mid(2, 3),
        ]);
        let mesh = single_element(ElementShape::Tet10, &coords);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        let render = part_render_mesh(&mesh, &skin, DVec3::ZERO, [1.0; 3]);
        let expected = 3.0 * 0.5 + 3.0_f32.sqrt() / 2.0;
        assert!((triangle_area_sum(&render) - expected).abs() < 1e-5);
    }

    #[test]
    fn positions_are_relative_to_origin() {
        let coords = [[1000.0, 0.0, 0.0], [1001.0, 0.0, 0.0], [1000.0, 1.0, 0.0]];
        let mesh = single_element(ElementShape::Tri3, &coords);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        let render = part_render_mesh(&mesh, &skin, DVec3::new(1000.0, 0.0, 0.0), [1.0; 3]);
        assert_eq!(render.vertices[1].position, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn part_colors_cycle() {
        assert_eq!(part_color(0), part_color(PART_COLORS_SRGB.len()));
        assert!(part_color(0).iter().all(|c| (0.0..=1.0).contains(c)));
    }
}
