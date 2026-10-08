use std::collections::HashMap;

use crate::element::ElementFamily;
use crate::mesh::{FeMesh, Part};

/// A visible element face. Node references are indices into [`FeMesh::coords`].
#[derive(Clone, Debug, PartialEq)]
pub struct SkinFace {
    /// Index into [`FeMesh::elements`].
    pub element: usize,
    /// 0-based face number (S1 = 0).
    pub face: usize,
    pub corners: Vec<usize>,
    /// Midside nodes of quadratic faces, `mids[i]` between `corners[i]` and `corners[i + 1]`.
    pub mids: Vec<usize>,
    /// Smooth surface patch the face belongs to, numbered from 0 per part. Patches are
    /// bounded by feature edges, like the faces of the CAD geometry the mesh came from.
    pub region: usize,
}

/// An edge of the skin, optionally curved through a midside node.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinEdge {
    pub a: usize,
    pub b: usize,
    pub mid: Option<usize>,
    /// Edge on the outline: a free boundary, a junction of more than two faces, or a sharp
    /// crease between two different surface patches.
    pub feature: bool,
}

/// Everything needed to draw one part: outer faces, their edges and line elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartSkin {
    pub faces: Vec<SkinFace>,
    pub edges: Vec<SkinEdge>,
    /// Segments of line elements (beams, trusses) as node index pairs.
    pub lines: Vec<[usize; 2]>,
}

/// Extracts the visible surface of one part: faces of solid elements that no other solid
/// element of the part shares, every surface element, and all line elements.
///
/// Elements that reference undefined nodes are skipped.
pub fn extract_part_skin(mesh: &FeMesh, part: &Part, feature_angle_deg: f64) -> PartSkin {
    let mut skin = PartSkin::default();
    let mut solid_faces: HashMap<[usize; 4], Option<SkinFace>> = HashMap::new();

    for &element_id in &part.elements {
        let Some(element_index) = mesh.element_index(element_id) else {
            continue;
        };
        let element = &mesh.elements()[element_index];
        let Some(nodes) = element
            .nodes
            .iter()
            .map(|&id| mesh.node_index(id))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let shape = element.shape;
        if shape.family() == ElementFamily::Line {
            skin.lines.extend(
                shape
                    .line_segments()
                    .iter()
                    .map(|&[a, b]| [nodes[a], nodes[b]]),
            );
            continue;
        }
        for (face_index, topology) in shape.faces().iter().enumerate() {
            let face = SkinFace {
                element: element_index,
                face: face_index,
                corners: topology.corners.iter().map(|&i| nodes[i]).collect(),
                mids: if shape.is_quadratic() {
                    topology.mids.iter().map(|&i| nodes[i]).collect()
                } else {
                    Vec::new()
                },
                region: 0,
            };
            if shape.family() == ElementFamily::Surface {
                skin.faces.push(face);
                continue;
            }
            let mut key = [usize::MAX; 4];
            key[..face.corners.len()].copy_from_slice(&face.corners);
            key.sort_unstable();
            solid_faces
                .entry(key)
                .and_modify(|seen| *seen = None)
                .or_insert(Some(face));
        }
    }
    let mut outer: Vec<SkinFace> = solid_faces.into_values().flatten().collect();
    outer.sort_by_key(|f| (f.element, f.face));
    skin.faces.extend(outer);
    skin.edges = collect_edges(mesh.coords(), &mut skin.faces, feature_angle_deg);
    skin
}

struct EdgeAccumulator {
    a: usize,
    b: usize,
    mid: Option<usize>,
    faces: Vec<usize>,
}

/// Collects the skin edges and splits the faces into smooth patches.
///
/// Faces are joined into one patch across every edge they share at less than the feature
/// angle. A sharp edge only counts as a feature when it separates two patches: on coarse
/// meshes of curved surfaces single edges often exceed the angle, but such creases end
/// inside a patch and are mesh noise rather than geometry.
fn collect_edges(
    coords: &[[f64; 3]],
    faces: &mut [SkinFace],
    feature_angle_deg: f64,
) -> Vec<SkinEdge> {
    let cos_limit = feature_angle_deg.to_radians().cos();
    let normals: Vec<[f64; 3]> = faces
        .iter()
        .map(|f| face_normal(coords, &f.corners))
        .collect();
    let mut order = Vec::new();
    let mut edges: HashMap<(usize, usize), EdgeAccumulator> = HashMap::new();
    for (face_index, face) in faces.iter().enumerate() {
        let n = face.corners.len();
        for i in 0..n {
            let (a, b) = (face.corners[i], face.corners[(i + 1) % n]);
            let key = (a.min(b), a.max(b));
            edges
                .entry(key)
                .or_insert_with(|| {
                    order.push(key);
                    EdgeAccumulator {
                        a,
                        b,
                        mid: face.mids.get(i).copied(),
                        faces: Vec::with_capacity(2),
                    }
                })
                .faces
                .push(face_index);
        }
    }

    let smooth = |edge: &EdgeAccumulator| match edge.faces.as_slice() {
        &[f0, f1] => dot(normals[f0], normals[f1]) >= cos_limit,
        _ => false,
    };
    let mut patches = UnionFind::new(faces.len());
    for edge in edges.values().filter(|e| smooth(e)) {
        patches.union(edge.faces[0], edge.faces[1]);
    }
    merge_tiny_patches(coords, &edges, &normals, &mut patches);
    let mut region_of_root = HashMap::new();
    for (index, face) in faces.iter_mut().enumerate() {
        let next = region_of_root.len();
        face.region = *region_of_root.entry(patches.find(index)).or_insert(next);
    }

    order
        .into_iter()
        .map(|key| {
            let edge = &edges[&key];
            let feature = match edge.faces.as_slice() {
                &[f0, f1] => !smooth(edge) && faces[f0].region != faces[f1].region,
                _ => true,
            };
            SkinEdge {
                a: edge.a,
                b: edge.b,
                mid: edge.mid,
                feature,
            }
        })
        .collect()
}

/// Patches of at most this many faces are treated as mesh noise, not as faces of the geometry.
const MAX_NOISE_PATCH_FACES: usize = 4;

/// Patches only merge across borders folding less than this on average; steeper borders, such
/// as the 90° edges of a box meshed with one element per side, are real geometry.
const MAX_NOISE_FOLD_DEG: f64 = 60.0;

/// Joins tiny patches to the neighbour they share the longest gently folded border with. Coarse
/// meshes of curved surfaces otherwise break into small patches whose outlines clutter the view.
fn merge_tiny_patches(
    coords: &[[f64; 3]],
    edges: &HashMap<(usize, usize), EdgeAccumulator>,
    normals: &[[f64; 3]],
    patches: &mut UnionFind,
) {
    let face_count = patches.parent.len();
    let mut size = vec![0usize; face_count];
    for face in 0..face_count {
        size[patches.find(face)] += 1;
    }
    // Per pair of neighbouring patch roots: shared edge length and length-weighted fold angle.
    let mut borders: HashMap<(usize, usize), (f64, f64)> = HashMap::new();
    for edge in edges.values() {
        if let &[f0, f1] = edge.faces.as_slice() {
            let (r0, r1) = (patches.find(f0), patches.find(f1));
            if r0 != r1 {
                let (p, q) = (coords[edge.a], coords[edge.b]);
                let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                let length = dot(d, d).sqrt();
                let fold = dot(normals[f0], normals[f1]).clamp(-1.0, 1.0).acos();
                let border = borders.entry((r0.min(r1), r0.max(r1))).or_default();
                border.0 += length;
                border.1 += length * fold;
            }
        }
    }
    let max_fold = MAX_NOISE_FOLD_DEG.to_radians();
    borders.retain(|_, &mut (length, weighted_fold)| weighted_fold < max_fold * length);
    let mut tiny: Vec<usize> = (0..face_count)
        .filter(|&root| size[root] > 0 && size[root] <= MAX_NOISE_PATCH_FACES)
        .collect();
    tiny.sort_by_key(|&root| size[root]);
    for root in tiny {
        let root = patches.find(root);
        if size[root] > MAX_NOISE_PATCH_FACES {
            continue;
        }
        let mut best: Option<(usize, f64)> = None;
        for (&(a, b), &(length, _)) in &borders {
            let (a, b) = (patches.find(a), patches.find(b));
            let other = match (a == root, b == root) {
                (true, false) => b,
                (false, true) => a,
                _ => continue,
            };
            if best.is_none_or(|(_, l)| length > l) {
                best = Some((other, length));
            }
        }
        if let Some((other, _)) = best {
            let merged = size[root] + size[other];
            patches.union(root, other);
            size[patches.find(root)] = merged;
        }
    }
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.parent[a.max(b)] = a.min(b);
    }
}

/// Unit normal of a planar or slightly warped polygon (Newell's method).
pub fn face_normal(coords: &[[f64; 3]], corners: &[usize]) -> [f64; 3] {
    let mut normal = [0.0; 3];
    for i in 0..corners.len() {
        let p = coords[corners[i]];
        let q = coords[corners[(i + 1) % corners.len()]];
        normal[0] += (p[1] - q[1]) * (p[2] + q[2]);
        normal[1] += (p[2] - q[2]) * (p[0] + q[0]);
        normal[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    let length = dot(normal, normal).sqrt();
    if length > 0.0 {
        normal.map(|c| c / length)
    } else {
        normal
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementShape;
    use crate::mesh::Element;

    /// `nx` × 1 × 1 hex elements along x, each 1 × 1 × 1.
    fn hex_bar(nx: u32) -> FeMesh {
        let mut mesh = FeMesh::default();
        let id = |i: u32, j: u32, k: u32| 1 + i + (nx + 1) * (j + 2 * k);
        for k in 0..2 {
            for j in 0..2 {
                for i in 0..=nx {
                    mesh.set_node(id(i, j, k), [i as f64, j as f64, k as f64]);
                }
            }
        }
        for i in 0..nx {
            let nodes = vec![
                id(i, 0, 0),
                id(i + 1, 0, 0),
                id(i + 1, 1, 0),
                id(i, 1, 0),
                id(i, 0, 1),
                id(i + 1, 0, 1),
                id(i + 1, 1, 1),
                id(i, 1, 1),
            ];
            mesh.add_element(Element {
                id: i + 1,
                type_name: "C3D8".into(),
                shape: ElementShape::Hex8,
                nodes,
            })
            .unwrap();
        }
        mesh.parts.push(Part {
            name: "BAR".into(),
            elements: (1..=nx).collect(),
        });
        mesh
    }

    #[test]
    fn inner_faces_of_a_hex_bar_are_hidden() {
        let mesh = hex_bar(3);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        // 4 long sides × 3 elements + 2 end caps.
        assert_eq!(skin.faces.len(), 14);
        // 4 long corner lines × 3 segments and 4 rings × 4 edges; only the two inner rings are not features.
        assert_eq!(skin.edges.len(), 12 + 16);
        assert_eq!(skin.edges.iter().filter(|e| e.feature).count(), 12 + 8);
    }

    #[test]
    fn flat_shell_has_only_boundary_features() {
        let mut mesh = FeMesh::default();
        for (id, xy) in [
            (1, [0.0, 0.0]),
            (2, [1.0, 0.0]),
            (3, [2.0, 0.0]),
            (4, [0.0, 1.0]),
            (5, [1.0, 1.0]),
            (6, [2.0, 1.0]),
        ] {
            mesh.set_node(id, [xy[0], xy[1], 0.0]);
        }
        for (id, nodes) in [(1, vec![1, 2, 5, 4]), (2, vec![2, 3, 6, 5])] {
            mesh.add_element(Element {
                id,
                type_name: "S4".into(),
                shape: ElementShape::Quad4,
                nodes,
            })
            .unwrap();
        }
        mesh.parts.push(Part {
            name: "PLATE".into(),
            elements: vec![1, 2],
        });
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        assert_eq!(skin.faces.len(), 2);
        assert_eq!(skin.edges.len(), 7);
        assert_eq!(skin.edges.iter().filter(|e| e.feature).count(), 6);
    }

    /// 2 × 2 S3 triangle pairs around a centre node lifted by `height`.
    fn tent(height: f64) -> FeMesh {
        let mut mesh = FeMesh::default();
        for j in 0..3 {
            for i in 0..3 {
                let z = if (i, j) == (1, 1) { height } else { 0.0 };
                mesh.set_node(1 + i + 3 * j, [i as f64, j as f64, z]);
            }
        }
        let mut id = 0;
        for j in 0..2 {
            for i in 0..2 {
                let n = |di: u32, dj: u32| 1 + (i + di) + 3 * (j + dj);
                for nodes in [
                    vec![n(0, 0), n(1, 0), n(1, 1)],
                    vec![n(0, 0), n(1, 1), n(0, 1)],
                ] {
                    id += 1;
                    mesh.add_element(Element {
                        id,
                        type_name: "S3".into(),
                        shape: ElementShape::Tri3,
                        nodes,
                    })
                    .unwrap();
                }
            }
        }
        mesh.parts.push(Part {
            name: "TENT".into(),
            elements: (1..=id).collect(),
        });
        mesh
    }

    #[test]
    fn gentle_folds_of_a_coarse_surface_are_no_features() {
        // Neighbouring faces fold by 32–53°, but the surface has no real edges inside.
        let mesh = tent(0.8);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        assert!(skin.faces.iter().all(|f| f.region == 0));
        assert_eq!(skin.edges.iter().filter(|e| e.feature).count(), 8);
    }

    #[test]
    fn steep_folds_stay_features_even_on_tiny_patches() {
        let mesh = tent(3.0);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        assert!(skin.edges.iter().filter(|e| e.feature).count() > 8);
    }

    #[test]
    fn elements_with_missing_nodes_are_skipped() {
        let mut mesh = hex_bar(2);
        mesh.add_element(Element {
            id: 99,
            type_name: "C3D4".into(),
            shape: ElementShape::Tet4,
            nodes: vec![1, 2, 3, 1000],
        })
        .unwrap();
        mesh.parts[0].elements.push(99);
        let skin = extract_part_skin(&mesh, &mesh.parts[0], 30.0);
        assert_eq!(skin.faces.len(), 10);
    }

    #[test]
    fn newell_normal_of_unit_square() {
        let coords = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        assert_eq!(face_normal(&coords, &[0, 1, 2, 3]), [0.0, 0.0, 1.0]);
    }
}
