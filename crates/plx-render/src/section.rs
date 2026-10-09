//! Section view: the scene is clipped at a plane, and where the plane cuts through solid
//! elements a section face is drawn, element by element, like PrePoMax's section caps.

use std::collections::HashMap;

use glam::{DVec3, Vec3};
use plx_mesh::{ElementFamily, ElementShape, FeMesh, Part};

use crate::contour::NO_VALUE;
use crate::fe::{FEATURE_EDGE_OPACITY, MESH_EDGE_OPACITY};
use crate::mesh::{RenderMesh, Vertex};

/// The half-space kept by a section view: points with `normal · (p - point) >= 0` stay
/// visible, the side the normal points away from is cut off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlane {
    pub point: Vec3,
    /// Unit normal, pointing into the kept half.
    pub normal: Vec3,
}

impl ClipPlane {
    /// Signed distance of a point from the plane; negative on the removed side.
    pub fn distance(&self, point: Vec3) -> f32 {
        self.normal.dot(point - self.point)
    }
}

/// An element of a part reduced to what a cut needs: its shape and the indices of its
/// corner nodes. Midside nodes are ignored, the cut of a quadratic element is drawn straight.
#[derive(Clone, Copy, Debug)]
struct Cell {
    shape: ElementShape,
    corners: [u32; 8],
}

/// The solid and surface elements of a part, prepared once for repeated cuts.
#[derive(Clone, Debug, Default)]
pub struct SectionCells {
    cells: Vec<Cell>,
}

impl SectionCells {
    pub fn new(mesh: &FeMesh, part: &Part) -> Self {
        let cells = part
            .elements
            .iter()
            .filter_map(|&id| mesh.element(id))
            .filter(|e| e.shape.family() != ElementFamily::Line)
            .filter_map(|element| {
                let mut corners = [0; 8];
                for (slot, &id) in corners
                    .iter_mut()
                    .zip(&element.nodes[..corner_count(element.shape)])
                {
                    *slot = mesh.node_index(id)? as u32;
                }
                Some(Cell {
                    shape: element.shape,
                    corners,
                })
            })
            .collect();
        Self { cells }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

fn corner_count(shape: ElementShape) -> usize {
    match shape {
        ElementShape::Line2 | ElementShape::Line3 => 2,
        ElementShape::Tri3 | ElementShape::Tri6 => 3,
        ElementShape::Quad4 | ElementShape::Quad8 | ElementShape::Tet4 | ElementShape::Tet10 => 4,
        ElementShape::Wedge6 | ElementShape::Wedge15 => 6,
        ElementShape::Hex8 | ElementShape::Hex20 => 8,
    }
}

/// PrePoMax's "lighten section colors": brightness raised and saturation lowered by 75 %.
pub fn lighten(color: [f32; 3]) -> [f32; 3] {
    const FACTOR: f32 = 0.75;
    let max = color.iter().copied().fold(0.0, f32::max);
    let min = color.iter().copied().fold(1.0, f32::min);
    let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
    let brightness = max + (1.0 - max) * FACTOR;
    let saturation = saturation * (1.0 - FACTOR);
    // Same hue: scale the colour's position between its minimum and maximum.
    color.map(|c| {
        let t = if max > min {
            (c - min) / (max - min)
        } else {
            1.0
        };
        brightness * (1.0 - saturation * (1.0 - t))
    })
}

/// Where a plane cuts the elements of a part: one polygon per cut solid element, outlined by
/// its element edges, and the cut lines of surface elements.
///
/// `point` and `normal` are in model coordinates; positions are shifted by `-origin` like
/// [`crate::part_render_mesh`]. `scalars` holds normalized contour values per node.
pub fn section_mesh(
    cells: &SectionCells,
    coords: &[[f64; 3]],
    origin: DVec3,
    point: DVec3,
    normal: DVec3,
    color: [f32; 3],
    scalars: Option<&[f32]>,
) -> RenderMesh {
    let normal = normal.normalize_or_zero();
    let mut render = RenderMesh::default();
    if normal == DVec3::ZERO {
        return render;
    }
    // In-plane axes to order the corners of each section polygon.
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let face_normal = (-normal).as_vec3().to_array();
    let scalar = |node: u32| scalars.map_or(NO_VALUE, |s| s[node as usize]);
    // Cut segments of element faces, keyed by the face's sorted corner nodes: a face shared
    // by two elements is cut twice, once from each side.
    let mut segments: HashMap<[u32; 4], (usize, [CutPoint; 2])> = HashMap::new();
    let mut shell_lines = Vec::new();

    for cell in &cells.cells {
        let count = corner_count(cell.shape);
        let corners = &cell.corners[..count];
        let mut distance = [0.0; 8];
        let (mut below, mut above) = (false, false);
        for (d, &node) in distance.iter_mut().zip(corners) {
            *d = normal.dot(DVec3::from(coords[node as usize]) - point);
            if *d < 0.0 {
                below = true;
            } else {
                above = true;
            }
        }
        if !(below && above) {
            continue;
        }
        let cut = |a: usize, b: usize| -> Option<CutPoint> {
            let (da, db) = (distance[a], distance[b]);
            if (da < 0.0) == (db < 0.0) {
                return None;
            }
            let t = da / (da - db);
            let (pa, pb) = (
                DVec3::from(coords[corners[a] as usize]),
                DVec3::from(coords[corners[b] as usize]),
            );
            let (sa, sb) = (scalar(corners[a]), scalar(corners[b]));
            Some(CutPoint {
                position: pa + (pb - pa) * t,
                scalar: if sa < 0.0 || sb < 0.0 {
                    NO_VALUE
                } else {
                    sa + (sb - sa) * t as f32
                },
                edge: (a.min(b), a.max(b)),
            })
        };

        let mut polygon: Vec<CutPoint> = Vec::with_capacity(6);
        for face in cell.shape.faces() {
            let n = face.corners.len();
            let mut points = (0..n).filter_map(|k| cut(face.corners[k], face.corners[(k + 1) % n]));
            let (Some(a), Some(b)) = (points.next(), points.next()) else {
                continue;
            };
            if cell.shape.family() == ElementFamily::Surface {
                shell_lines.push([a, b]);
                continue;
            }
            let mut key = [u32::MAX; 4];
            for (slot, &local) in key.iter_mut().zip(face.corners) {
                *slot = corners[local];
            }
            key.sort_unstable();
            segments
                .entry(key)
                .and_modify(|(count, _)| *count += 1)
                .or_insert((1, [a, b]));
            for p in [a, b] {
                if !polygon.iter().any(|q| q.edge == p.edge) {
                    polygon.push(p);
                }
            }
        }
        if polygon.len() < 3 {
            continue;
        }
        // The cut of a convex cell is a convex polygon: sorting its corners by angle around
        // their centre puts them in order.
        let centre = polygon.iter().map(|p| p.position).sum::<DVec3>() / polygon.len() as f64;
        let angle = |p: &CutPoint| {
            let d = p.position - centre;
            d.dot(v).atan2(d.dot(u))
        };
        polygon.sort_by(|a, b| angle(a).total_cmp(&angle(b)));
        let base = render.vertices.len() as u32;
        render.vertices.extend(polygon.iter().map(|p| Vertex {
            position: (p.position - origin).as_vec3().to_array(),
            normal: face_normal,
            color,
            scalar: p.scalar,
        }));
        for k in 1..polygon.len() as u32 - 1 {
            render.triangles.extend([base, base + k, base + k + 1]);
        }
    }

    let line_vertex = |p: &CutPoint, opacity: f32| Vertex {
        position: (p.position - origin).as_vec3().to_array(),
        normal: [0.0; 3],
        color: [0.0; 3],
        scalar: opacity,
    };
    for (count, [a, b]) in segments.values() {
        // A face cut only once lies on the part's surface: the outline of the section.
        if *count == 1 {
            render.feature_edges.extend([
                line_vertex(a, FEATURE_EDGE_OPACITY),
                line_vertex(b, FEATURE_EDGE_OPACITY),
            ]);
        } else {
            render.mesh_edges.extend([
                line_vertex(a, MESH_EDGE_OPACITY),
                line_vertex(b, MESH_EDGE_OPACITY),
            ]);
        }
    }
    let shell_color = color.map(|c| c * 0.5);
    for [a, b] in &shell_lines {
        for p in [a, b] {
            render.feature_edges.push(Vertex {
                color: shell_color,
                ..line_vertex(p, 1.0)
            });
        }
    }
    render
}

/// A point where the plane crosses an element edge.
#[derive(Clone, Copy, Debug)]
struct CutPoint {
    position: DVec3,
    scalar: f32,
    /// The crossed edge as local corner indices, to find the same point from both faces.
    edge: (usize, usize),
}

/// Unit normal and a point of a plane in render coordinates, for the GPU and for picking.
pub fn clip_plane(point: DVec3, normal: DVec3, origin: DVec3) -> ClipPlane {
    ClipPlane {
        point: (point - origin).as_vec3(),
        normal: normal.normalize_or_zero().as_vec3(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_mesh::Element;

    /// A unit cube of one hexahedron, or of `n` stacked ones along x.
    fn cubes(n: u32) -> FeMesh {
        let mut mesh = FeMesh::default();
        for i in 0..=n {
            for (k, [y, z]) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
                .into_iter()
                .enumerate()
            {
                mesh.set_node(i * 4 + k as u32 + 1, [i as f64, y, z]);
            }
        }
        for i in 0..n {
            let a = i * 4 + 1;
            let b = a + 4;
            // Hex: bottom face 1-2-3-4, top face 5-6-7-8; here "bottom" is the face at x = i.
            mesh.add_element(Element {
                id: i + 1,
                type_name: "C3D8".into(),
                shape: ElementShape::Hex8,
                nodes: vec![a, a + 1, a + 2, a + 3, b, b + 1, b + 2, b + 3],
            })
            .unwrap();
        }
        mesh.parts.push(Part {
            name: "CUBES".into(),
            elements: (1..=n).collect(),
        });
        mesh
    }

    fn area(mesh: &RenderMesh) -> f32 {
        mesh.triangles
            .chunks(3)
            .map(|t| {
                let p = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.vertices[i as usize].position));
                (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5
            })
            .sum()
    }

    #[test]
    fn cut_through_cubes_covers_the_section() {
        let mesh = cubes(3);
        let cells = SectionCells::new(&mesh, &mesh.parts[0]);
        // Lengthwise through the middle: three unit squares, outlined by eight unit edges
        // around the outside and two element edges inside.
        let section = section_mesh(
            &cells,
            mesh.coords(),
            DVec3::ZERO,
            DVec3::new(0.0, 0.5, 0.0),
            DVec3::Y,
            [1.0; 3],
            None,
        );
        assert!((area(&section) - 3.0).abs() < 1e-5, "{}", area(&section));
        assert_eq!(section.feature_edges.len(), 2 * 8);
        assert_eq!(section.mesh_edges.len(), 2 * 2);
        assert!(
            section
                .vertices
                .iter()
                .all(|v| (v.position[1] - 0.5).abs() < 1e-6)
        );
    }

    #[test]
    fn oblique_cut_has_the_projected_area() {
        let mesh = cubes(1);
        let cells = SectionCells::new(&mesh, &mesh.parts[0]);
        let normal = DVec3::new(1.0, 1.0, 0.0);
        let section = section_mesh(
            &cells,
            mesh.coords(),
            DVec3::ZERO,
            DVec3::splat(0.5),
            normal,
            [1.0; 3],
            None,
        );
        // The diagonal plane cuts a 1 × sqrt(2) rectangle.
        assert!((area(&section) - 2.0_f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn planes_beside_the_part_cut_nothing() {
        let mesh = cubes(2);
        let cells = SectionCells::new(&mesh, &mesh.parts[0]);
        let section = section_mesh(
            &cells,
            mesh.coords(),
            DVec3::ZERO,
            DVec3::new(5.0, 0.0, 0.0),
            DVec3::X,
            [1.0; 3],
            None,
        );
        assert!(section.vertices.is_empty() && section.feature_edges.is_empty());
    }

    #[test]
    fn section_values_are_interpolated() {
        let mesh = cubes(1);
        let cells = SectionCells::new(&mesh, &mesh.parts[0]);
        // 0 on the face at x = 0, 1 on the face at x = 1.
        let scalars: Vec<f32> = mesh.coords().iter().map(|p| p[0] as f32).collect();
        let section = section_mesh(
            &cells,
            mesh.coords(),
            DVec3::ZERO,
            DVec3::new(0.25, 0.0, 0.0),
            DVec3::X,
            [1.0; 3],
            Some(&scalars),
        );
        assert!(!section.vertices.is_empty());
        assert!(
            section
                .vertices
                .iter()
                .all(|v| (v.scalar - 0.25).abs() < 1e-6)
        );
    }

    #[test]
    fn lightening_keeps_grey_grey_and_brightens() {
        let grey = lighten([0.4, 0.4, 0.4]);
        assert!((grey[0] - 0.85).abs() < 1e-6 && grey[0] == grey[1] && grey[1] == grey[2]);
        let blue = lighten([70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0]);
        assert!(blue[2] > 0.9 && blue[0] < blue[2]);
    }
}
