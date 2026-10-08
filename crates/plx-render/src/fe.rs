use std::collections::HashMap;

use glam::DVec3;
use plx_mesh::{PartSkin, SkinFace, face_normal};

use crate::contour::NO_VALUE;

use crate::mesh::{RenderMesh, Vertex};

const EDGE_COLOR: [f32; 3] = [0.0, 0.0, 0.0];
/// Opacities of PrePoMax's edge actors; line vertices carry them in [`Vertex::scalar`].
pub const FEATURE_EDGE_OPACITY: f32 = 0.7;
pub const MESH_EDGE_OPACITY: f32 = 0.4;
pub const WIREFRAME_OPACITY: f32 = 0.5;

/// PrePoMax's default part colours (System.Drawing names), cycled by part index.
const PART_COLORS_SRGB: [[u8; 3]; 10] = [
    [245, 245, 220], // Beige
    [143, 188, 143], // DarkSeaGreen
    [240, 230, 140], // Khaki
    [70, 130, 180],  // SteelBlue
    [222, 184, 135], // BurlyWood
    [176, 196, 222], // LightSteelBlue
    [255, 228, 225], // MistyRose
    [233, 150, 122], // DarkSalmon
    [189, 183, 107], // DarkKhaki
    [255, 222, 173], // NavajoWhite
];

/// sRGB colour of the n-th part; the viewport shades in display space like VTK.
pub fn part_color(index: usize) -> [f32; 3] {
    PART_COLORS_SRGB[index % PART_COLORS_SRGB.len()].map(|c| c as f32 / 255.0)
}

/// Builds the GPU-ready geometry of one part from node coordinates (undeformed or deformed).
/// Positions are shifted by `-origin`, so that large coordinates keep their precision in `f32`.
/// `scalars` holds one normalized contour value per node (see [`crate::contour::normalize`]).
///
/// Faces of the same surface patch that meet at less than `smooth_angle_deg` share averaged
/// vertex normals, so curved surfaces look round; across feature edges the shading stays flat.
pub fn part_render_mesh(
    coords: &[[f64; 3]],
    skin: &PartSkin,
    origin: DVec3,
    color: [f32; 3],
    smooth_angle_deg: f64,
    scalars: Option<&[f32]>,
) -> RenderMesh {
    let scalar = |node: usize| scalars.map_or(NO_VALUE, |s| s[node]);
    let position = |node: usize| (DVec3::from(coords[node]) - origin).as_vec3().to_array();
    let mut render = RenderMesh::default();
    let normals = smooth_vertex_normals(coords, skin, smooth_angle_deg);

    for (face, face_normals) in skin.faces.iter().zip(normals) {
        let base = render.vertices.len() as u32;
        for (&node, normal) in face.corners.iter().chain(&face.mids).zip(face_normals) {
            render.vertices.push(Vertex {
                position: position(node),
                normal,
                color,
                scalar: scalar(node),
            });
        }
        render
            .triangles
            .extend(face_triangles(face).iter().map(|&local| base + local));
    }

    let line_vertex = |node: usize, color: [f32; 3], opacity: f32| Vertex {
        position: position(node),
        normal: [0.0; 3],
        color,
        scalar: opacity,
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
            let opacity = if edge.feature {
                FEATURE_EDGE_OPACITY
            } else {
                MESH_EDGE_OPACITY
            };
            target.extend([
                line_vertex(a, EDGE_COLOR, opacity),
                line_vertex(b, EDGE_COLOR, opacity),
            ]);
        }
    }
    let line_color = color.map(|c| c * 0.5);
    for &[a, b] in &skin.lines {
        render.feature_edges.extend([
            line_vertex(a, line_color, 1.0),
            line_vertex(b, line_color, 1.0),
        ]);
    }
    render
}

/// Feature edges of a part as translucent black lines, PrePoMax's wireframe of the undeformed
/// shape behind deformed results.
pub fn wireframe_edges(coords: &[[f64; 3]], skin: &PartSkin, origin: DVec3) -> Vec<Vertex> {
    let vertex = |node: usize| Vertex {
        position: (DVec3::from(coords[node]) - origin).as_vec3().to_array(),
        normal: [0.0; 3],
        color: EDGE_COLOR,
        scalar: WIREFRAME_OPACITY,
    };
    let mut lines = Vec::new();
    for edge in skin.edges.iter().filter(|e| e.feature) {
        let nodes: &[usize] = match edge.mid {
            Some(mid) => &[edge.a, mid, mid, edge.b],
            None => &[edge.a, edge.b],
        };
        lines.extend(nodes.iter().map(|&n| vertex(n)));
    }
    for &[a, b] in &skin.lines {
        lines.extend([vertex(a), vertex(b)]);
    }
    lines
}

/// Per face, one normal for each of its corner and mid nodes: the area-weighted average of the
/// normals of all faces of the same patch at that node that deviate from this face by less
/// than the angle.
fn smooth_vertex_normals(
    coords: &[[f64; 3]],
    skin: &PartSkin,
    smooth_angle_deg: f64,
) -> Vec<Vec<[f32; 3]>> {
    let cos_limit = smooth_angle_deg.to_radians().cos();
    let face_normals: Vec<DVec3> = skin
        .faces
        .iter()
        .map(|f| DVec3::from(face_normal(coords, &f.corners)))
        .collect();
    let weights: Vec<f64> = skin
        .faces
        .iter()
        .map(|f| polygon_area(coords, &f.corners))
        .collect();
    let mut faces_at_node: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, face) in skin.faces.iter().enumerate() {
        for &node in face.corners.iter().chain(&face.mids) {
            faces_at_node.entry(node).or_default().push(index);
        }
    }
    skin.faces
        .iter()
        .zip(&face_normals)
        .map(|(face, &own)| {
            face.corners
                .iter()
                .chain(&face.mids)
                .map(|node| {
                    let sum: DVec3 = faces_at_node[node]
                        .iter()
                        .filter(|&&other| {
                            skin.faces[other].region == face.region
                                && own.dot(face_normals[other]) >= cos_limit
                        })
                        .map(|&other| face_normals[other] * weights[other])
                        .sum();
                    sum.try_normalize().unwrap_or(own).as_vec3().to_array()
                })
                .collect()
        })
        .collect()
}

fn polygon_area(coords: &[[f64; 3]], corners: &[usize]) -> f64 {
    let p0 = DVec3::from(coords[corners[0]]);
    (1..corners.len() - 1)
        .map(|i| {
            let (a, b) = (
                DVec3::from(coords[corners[i]]),
                DVec3::from(coords[corners[i + 1]]),
            );
            (a - p0).cross(b - p0).length() * 0.5
        })
        .sum::<f64>()
        .max(f64::MIN_POSITIVE)
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
    use plx_mesh::{Element, ElementShape, FeMesh, Part, extract_part_skin};

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
        let render = part_render_mesh(mesh.coords(), &skin, DVec3::ZERO, [1.0; 3], 30.0, None);
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
        let render = part_render_mesh(mesh.coords(), &skin, DVec3::ZERO, [1.0; 3], 30.0, None);
        let expected = 3.0 * 0.5 + 3.0_f32.sqrt() / 2.0;
        assert!((triangle_area_sum(&render) - expected).abs() < 1e-5);
    }

    #[test]
    fn positions_are_relative_to_origin() {
        let coords = [[1000.0, 0.0, 0.0], [1001.0, 0.0, 0.0], [1000.0, 1.0, 0.0]];
        let mesh = single_element(ElementShape::Tri3, &coords);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        let render = part_render_mesh(
            mesh.coords(),
            &skin,
            DVec3::new(1000.0, 0.0, 0.0),
            [1.0; 3],
            30.0,
            None,
        );
        assert_eq!(render.vertices[1].position, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn normals_are_smoothed_across_shallow_edges_only() {
        // Two triangles folded by 20° along x and a third folded by 90°, all sharing node 0.
        let fold = 20f64.to_radians();
        let coords = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -fold.cos(), fold.sin()],
            [0.0, 0.0, 1.0],
        ];
        let mut mesh = FeMesh::default();
        for (i, c) in coords.iter().enumerate() {
            mesh.set_node(i as u32 + 1, *c);
        }
        for (id, nodes) in [(1, vec![1, 2, 3]), (2, vec![1, 4, 2]), (3, vec![1, 3, 5])] {
            mesh.add_element(Element {
                id,
                type_name: "S3".into(),
                shape: ElementShape::Tri3,
                nodes,
            })
            .unwrap();
        }
        mesh.parts.push(Part {
            name: "P".into(),
            elements: vec![1, 2, 3],
        });
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        let render = part_render_mesh(mesh.coords(), &skin, DVec3::ZERO, [1.0; 3], 30.0, None);
        let normal_at_origin = |face: usize| glam::Vec3::from(render.vertices[face * 3].normal);
        // The flat face and the 20° face blend, the 90° face keeps its own normal.
        let blended = normal_at_origin(0);
        assert!(
            (blended.angle_between(glam::Vec3::Z).to_degrees() - 10.0).abs() < 1e-3,
            "{blended}"
        );
        assert!((normal_at_origin(1) - blended).length() < 1e-6);
        assert!((normal_at_origin(2) - glam::Vec3::X).length() < 1e-6);
    }

    #[test]
    fn part_colors_cycle() {
        assert_eq!(part_color(0), part_color(PART_COLORS_SRGB.len()));
        assert!(part_color(0).iter().all(|c| (0.0..=1.0).contains(c)));
    }
}
