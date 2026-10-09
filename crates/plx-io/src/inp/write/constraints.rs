//! Constraints written as CalculiX elements, the way PrePoMax's `CalculixFileWriter` splits
//! them: point and surface springs become `SPRING1` elements with one `*Spring` section per
//! node and direction, compression only supports become `GAPUNI` elements to fixed ground
//! nodes. The spring connection between two surfaces, which PrePoMax lacks, becomes `SPRING2`
//! elements from each node of the slave surface to a new node that `*Equation`s tie to the
//! closest point of the master surface.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use plx_mesh::{ElementId, FeMesh, NodeId};
use plx_model::{
    CompressionOnly, Constraint, FeModel, PointSpring, SurfaceSpring, SurfaceToSurfaceSpring,
};

use super::{Keyword, Members, Sets, WriteError, empty, number};

/// What the constraints add to the input file besides the mesh.
#[derive(Default)]
pub(super) struct Generated {
    /// New nodes, appended to the `*Node` block.
    pub nodes: Vec<(NodeId, [f64; 3])>,
    /// `*Element` blocks of spring and gap elements.
    pub elements: Vec<Keyword>,
    /// `*Spring` and `*Gap` sections.
    pub sections: Vec<Keyword>,
    /// `*Equation`s, written under the "Constraints" title after the ties.
    pub equations: Vec<Keyword>,
    /// Holds the ground nodes of the gap elements in every step.
    pub boundary: Option<Keyword>,
    /// PrePoMax's dummy plastic material that makes CalculiX solve gaps nonlinearly.
    pub material: Option<Keyword>,
}

/// The elements, sections and equations of the active springs and supports; ties are written
/// as `*Tie` by the caller.
pub(super) fn springs(sets: &mut Sets, model: &FeModel) -> Result<Generated, WriteError> {
    let mesh = sets.mesh;
    let mut writer = Writer {
        out: Generated::default(),
        next_node: mesh.node_ids().iter().max().map_or(1, |n| n + 1),
        next_element: mesh
            .elements()
            .iter()
            .map(|e| e.id)
            .max()
            .map_or(1, |e| e + 1),
        equations: Vec::new(),
        gaps: Vec::new(),
        ground: Vec::new(),
    };
    let mut nonlinear = false;
    for constraint in model.constraints.iter().filter(|c| c.active()) {
        match constraint {
            Constraint::PointSpring(spring) => writer.point_spring(sets, spring)?,
            Constraint::SurfaceSpring(spring) => writer.surface_spring(sets, spring)?,
            Constraint::CompressionOnly(support) => {
                writer.compression_only(sets, support)?;
                nonlinear |= support.nonlinear;
            }
            Constraint::SurfaceToSurfaceSpring(spring) => {
                writer.surface_to_surface(sets, spring)?
            }
            Constraint::Tie(_) => {}
        }
    }
    let mut out = writer.out;
    if !writer.gaps.is_empty() {
        let mut block = String::from("*Element, Type=GAPUNI\n");
        for (element, [a, b]) in &writer.gaps {
            let _ = writeln!(block, "{element}, {a}, {b}");
        }
        out.elements.push(Keyword::generated(block));
        let set = sets.unique("Internal_All_Compression_Only_Constraints_NodeSet");
        sets.node_sets.push((set.clone(), writer.ground));
        out.boundary = Some(Keyword::generated(format!(
            "** Name: Compression_Only_BC\n*Boundary\n{set}, 1, 1, 0\n{set}, 2, 2, 0\n{set}, 3, 3, 0\n"
        )));
    }
    if nonlinear {
        let name = sets.unique("Internal_compression_only-1");
        out.material = Some(Keyword::generated(format!(
            "*Material, Name={name}\n*Density\n1\n*Elastic\n1, 0\n*Plastic\n0, 0\n"
        )));
    }
    for equation in writer.equations {
        let mut text = format!("*Equation\n{}\n", equation.len());
        let terms: Vec<String> = (equation.iter())
            .map(|(node, dof, factor)| format!("{node}, {dof}, {}", number(*factor)))
            .collect();
        // Four terms (twelve entries) per line, as PrePoMax writes them.
        for chunk in terms.chunks(4) {
            text.push_str(&chunk.join(", "));
            text.push('\n');
        }
        out.equations.push(Keyword::generated(text));
    }
    Ok(out)
}

struct Writer {
    out: Generated,
    next_node: NodeId,
    next_element: ElementId,
    /// Linear equations as (node, degree of freedom, factor) terms, the dependent term first.
    equations: Vec<Vec<(NodeId, u8, f64)>>,
    /// Gap elements with their ground and surface node.
    gaps: Vec<(ElementId, [NodeId; 2])>,
    ground: Vec<NodeId>,
}

impl Writer {
    fn node(&mut self, coords: [f64; 3]) -> NodeId {
        let id = self.next_node;
        self.next_node += 1;
        self.out.nodes.push((id, coords));
        id
    }

    fn element(&mut self) -> ElementId {
        let id = self.next_element;
        self.next_element += 1;
        id
    }

    /// Springs of one constraint and one element block with all of them, like PrePoMax's
    /// point springs. Each group of springs shares the stiffness, an element set and a
    /// `*Spring` section per direction; it is (name, nodes of each spring, stiffness).
    fn springs(
        &mut self,
        sets: &mut Sets,
        name: &str,
        groups: Vec<(String, Vec<Vec<NodeId>>, [f64; 3])>,
    ) {
        let two_nodes = (groups.first())
            .and_then(|(_, springs, _)| springs.first())
            .is_some_and(|nodes| nodes.len() == 2);
        let mut block = String::new();
        for (group, springs, stiffness) in groups {
            for (dof, k) in (1..).zip(stiffness).filter(|(_, k)| *k != 0.0) {
                let mut elements = Vec::new();
                for nodes in &springs {
                    let element = self.element();
                    elements.push(element);
                    let nodes: Vec<String> = nodes.iter().map(u32::to_string).collect();
                    let _ = writeln!(block, "{element}, {}", nodes.join(", "));
                }
                let set = sets.unique(&format!("{group}_DOF_{dof}"));
                sets.element_sets
                    .push((set.clone(), Members::Ids(elements)));
                let dofs = if two_nodes {
                    format!("{dof}, {dof}")
                } else {
                    dof.to_string()
                };
                self.out.sections.push(Keyword::generated(format!(
                    "*Spring, Elset={set}\n{dofs}\n{}\n",
                    real(k)
                )));
            }
        }
        if block.is_empty() {
            return;
        }
        let kind = if two_nodes { "SPRING2" } else { "SPRING1" };
        let set = sets.unique(&format!("{name}_All"));
        self.out.elements.push(Keyword::generated(format!(
            "*Element, Type={kind}, Elset={set}\n{block}"
        )));
    }

    fn point_spring(&mut self, sets: &mut Sets, spring: &PointSpring) -> Result<(), WriteError> {
        let nodes = spring.region.nodes(sets.mesh);
        if nodes.is_empty() {
            return Err(empty(&spring.name, "Knoten"));
        }
        let name = super::name(&spring.name);
        let springs = nodes.into_iter().map(|node| vec![node]).collect();
        self.springs(sets, &name, vec![(name.clone(), springs, spring.stiffness)]);
        Ok(())
    }

    fn surface_spring(
        &mut self,
        sets: &mut Sets,
        spring: &SurfaceSpring,
    ) -> Result<(), WriteError> {
        let (weights, area) = node_areas(sets.mesh, &spring.region.faces(sets.mesh), false);
        if weights.is_empty() {
            return Err(empty(&spring.name, "Elementflächen"));
        }
        let scale = if spring.per_area { 1.0 } else { 1.0 / area };
        let name = super::name(&spring.name);
        let springs = (weights.iter())
            .filter(|(_, w)| **w != 0.0)
            .map(|(&node, w)| {
                let k = spring.stiffness.map(|k| k * w * scale);
                (format!("{name}_{node}"), vec![vec![node]], k)
            })
            .collect();
        self.springs(sets, &name, springs);
        Ok(())
    }

    fn compression_only(
        &mut self,
        sets: &mut Sets,
        support: &CompressionOnly,
    ) -> Result<(), WriteError> {
        let mesh = sets.mesh;
        let faces = support.region.faces(mesh);
        let (weights, area) = node_areas(mesh, &faces, false);
        if weights.is_empty() {
            return Err(empty(&support.name, "Elementflächen"));
        }
        // Normal of each node: the mean of the outward normals of its faces.
        let mut normals: BTreeMap<NodeId, [f64; 3]> = BTreeMap::new();
        for &(element, face) in &faces {
            let Some(normal) = outward_normal(mesh, element, face) else {
                continue;
            };
            for node in face_node_ids(mesh, element, face) {
                let sum = normals.entry(node).or_default();
                for k in 0..3 {
                    sum[k] += normal[k];
                }
            }
        }
        let stiffness = support
            .spring_stiffness
            .unwrap_or(CompressionOnly::DEFAULT_STIFFNESS)
            / area;
        let force = support
            .tensile_force
            .unwrap_or(CompressionOnly::DEFAULT_TENSILE_FORCE)
            / area;
        let name = super::name(&support.name);
        for (count, (&node, &w)) in (1..).zip(weights.iter().filter(|(_, w)| **w != 0.0)) {
            let (Some(coords), Some(normal)) = (mesh.node(node), normals.get(&node)) else {
                continue;
            };
            // Like PrePoMax the gap points into the material, from the ground node outside
            // the surface to the surface node, so that it closes when the surface moves
            // towards the ground.
            let normal = normalized(*normal).map(|v| -v);
            let ground = self.node(std::array::from_fn(|k| {
                coords[k] - support.offset * normal[k]
            }));
            self.ground.push(ground);
            let element = self.element();
            self.gaps.push((element, [ground, node]));
            let set = sets.unique(&format!("{name}_ElementSet-{count}"));
            sets.element_sets
                .push((set.clone(), Members::Ids(vec![element])));
            let properties =
                if support.spring_stiffness.is_some() || support.tensile_force.is_some() {
                    format!(", , {}, {}", number(stiffness * w), number(force * w))
                } else {
                    String::new()
                };
            self.out.sections.push(Keyword::generated(format!(
                "*Gap, Elset={set}\n{}, {}, {}, {}{properties}\n",
                number(support.clearance),
                number(normal[0]),
                number(normal[1]),
                number(normal[2])
            )));
        }
        Ok(())
    }

    fn surface_to_surface(
        &mut self,
        sets: &mut Sets,
        spring: &SurfaceToSurfaceSpring,
    ) -> Result<(), WriteError> {
        let mesh = sets.mesh;
        let (weights, area) = node_areas(mesh, &spring.slave.faces(mesh), false);
        if weights.is_empty() {
            return Err(empty(&spring.name, "Elementflächen auf der Slave-Seite"));
        }
        let target = Target::new(mesh, &spring.master.faces(mesh));
        if target.faces.is_empty() {
            return Err(empty(&spring.name, "Elementflächen auf der Master-Seite"));
        }
        let scale = if spring.per_area { 1.0 } else { 1.0 / area };
        let name = super::name(&spring.name);
        let mut springs = Vec::new();
        for (&node, &w) in weights.iter().filter(|(_, w)| **w != 0.0) {
            let Some(coords) = mesh.node(node) else {
                continue;
            };
            let Some((point, terms)) = target.closest(coords) else {
                continue;
            };
            // A node on both surfaces needs no spring; a node on top of a target node is
            // connected to it directly.
            let other = match terms.as_slice() {
                [(other, _)] if *other == node => continue,
                [(other, _)] => *other,
                _ => {
                    let other = self.node(point);
                    for dof in 1..=3 {
                        let mut equation = vec![(other, dof, 1.0)];
                        equation.extend(terms.iter().map(|&(n, f)| (n, dof, -f)));
                        self.equations.push(equation);
                    }
                    other
                }
            };
            let k = spring.stiffness.map(|k| k * w * scale);
            springs.push((format!("{name}_{node}"), vec![vec![node, other]], k));
        }
        self.springs(sets, &name, springs);
        Ok(())
    }
}

/// A number with a decimal point: CalculiX takes an integer on the line after the degrees of
/// freedom of `*Spring` for more data and reports the card as empty, so PrePoMax writes
/// stiffnesses this way.
fn real(value: f64) -> String {
    let text = number(value);
    if text.contains(['.', 'E', 'e']) || !value.is_finite() {
        text
    } else {
        format!("{text}.")
    }
}

/// Share of the area of the faces that each node takes and the total area, PrePoMax's
/// `GetDistributedNodalValuesFromSurface`. Within a face the share follows the equivalent
/// nodal forces of a constant pressure: equal parts on linear faces, only midside nodes on
/// quadratic triangles, and -1/12 per corner and 1/3 per midside node on quadratic
/// quadrilaterals.
pub(super) fn node_areas(
    mesh: &FeMesh,
    faces: &[(ElementId, u8)],
    axisymmetric: bool,
) -> (BTreeMap<NodeId, f64>, f64) {
    let mut weights: BTreeMap<NodeId, f64> = BTreeMap::new();
    let mut total_area = 0.0;
    for &(element, face) in faces {
        let Some(element) = mesh.element(element) else {
            continue;
        };
        let Some(topology) = (face as usize)
            .checked_sub(1)
            .and_then(|f| element.faces().get(f))
        else {
            continue;
        };
        let node = |local: usize| element.nodes.get(local).copied();
        let point = |local: usize| node(local).and_then(|id| mesh.node(id));
        let corners: Vec<[f64; 3]> = topology.corners.iter().filter_map(|&l| point(l)).collect();
        if corners.len() != topology.corners.len() {
            continue;
        }
        let quadratic = element.shape.is_quadratic() && !topology.mids.is_empty();
        // Edges of 2D elements: their length, or for axisymmetric ones length times radius.
        if let [a, b] = corners[..] {
            let ends = [topology.corners[0], topology.corners[1]];
            let mid = topology.mids.first().copied().filter(|_| quadratic);
            let (nodes, edge) = match mid.and_then(|m| Some((m, point(m)?))) {
                Some((m, pm)) => (
                    vec![ends[0], ends[1], m],
                    super::edge_weights(&[a, b, pm], axisymmetric),
                ),
                None => (ends.to_vec(), super::edge_weights(&[a, b], axisymmetric)),
            };
            total_area += edge.iter().sum::<f64>();
            for (local, weight) in nodes.into_iter().zip(edge) {
                if let Some(id) = node(local) {
                    *weights.entry(id).or_default() += weight;
                }
            }
            continue;
        }
        let area = super::polygon_area(&corners);
        total_area += area;
        let (corner_weight, mid_weight) = match (corners.len(), quadratic) {
            (3, true) => (0.0, 1.0 / 3.0),
            (4, true) => (-1.0 / 12.0, 1.0 / 3.0),
            (n, _) => (1.0 / n as f64, 0.0),
        };
        for &local in topology.corners {
            if let Some(id) = node(local) {
                *weights.entry(id).or_default() += area * corner_weight;
            }
        }
        if quadratic {
            for &local in topology.mids {
                if let Some(id) = node(local) {
                    *weights.entry(id).or_default() += area * mid_weight;
                }
            }
        }
    }
    (weights, total_area)
}

/// Nodes of an element face: corners, then midside nodes of quadratic elements.
fn face_node_ids(mesh: &FeMesh, element: ElementId, face: u8) -> Vec<NodeId> {
    let Some(element) = mesh.element(element) else {
        return Vec::new();
    };
    let Some(topology) = (face as usize)
        .checked_sub(1)
        .and_then(|f| element.faces().get(f))
    else {
        return Vec::new();
    };
    let mids: &[usize] = if element.shape.is_quadratic() {
        topology.mids
    } else {
        &[]
    };
    (topology.corners.iter().chain(mids))
        .filter_map(|&l| element.nodes.get(l).copied())
        .collect()
}

/// Unit normal of an element face pointing out of a solid element; for a shell, the normal
/// of its first face.
fn outward_normal(mesh: &FeMesh, element: ElementId, face: u8) -> Option<[f64; 3]> {
    let e = mesh.element(element)?;
    let topology = e.faces().get(usize::from(face).checked_sub(1)?)?;
    let corners: Vec<[f64; 3]> = (topology.corners.iter())
        .map(|&l| e.nodes.get(l).and_then(|&n| mesh.node(n)))
        .collect::<Option<_>>()?;
    let mut normal = [0.0; 3];
    for (i, a) in corners.iter().enumerate() {
        let b = corners[(i + 1) % corners.len()];
        normal[0] += a[1] * b[2] - a[2] * b[1];
        normal[1] += a[2] * b[0] - a[0] * b[2];
        normal[2] += a[0] * b[1] - a[1] * b[0];
    }
    if e.shape.family() == plx_mesh::ElementFamily::Solid {
        let all: Vec<[f64; 3]> = e.nodes.iter().filter_map(|&n| mesh.node(n)).collect();
        let centre = |points: &[[f64; 3]]| -> [f64; 3] {
            std::array::from_fn(|k| points.iter().map(|p| p[k]).sum::<f64>() / points.len() as f64)
        };
        let (face_centre, element_centre) = (centre(&corners), centre(&all));
        let outward: f64 = (0..3)
            .map(|k| normal[k] * (face_centre[k] - element_centre[k]))
            .sum();
        if outward < 0.0 {
            normal = normal.map(|v| -v);
        }
    }
    let length = dot(normal, normal).sqrt();
    (length > 0.0).then(|| normal.map(|v| v / length))
}

fn normalized(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    if length > 0.0 {
        v.map(|x| x / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Nodes with their shape function value at a point.
type Terms = Vec<(NodeId, f64)>;

/// A face of the target surface: its corner coordinates, its nodes (corners, then midside
/// nodes on quadratic faces) and its bounding box.
struct TargetFace {
    corners: Vec<[f64; 3]>,
    nodes: Vec<NodeId>,
    min: [f64; 3],
    max: [f64; 3],
}

/// The master surface of a spring connection, searched for the closest point of each slave
/// node.
struct Target {
    faces: Vec<TargetFace>,
}

impl Target {
    fn new(mesh: &FeMesh, faces: &[(ElementId, u8)]) -> Self {
        let faces = faces
            .iter()
            .filter_map(|&(element, face)| {
                let e = mesh.element(element)?;
                let topology = e.faces().get(usize::from(face).checked_sub(1)?)?;
                let corners: Vec<[f64; 3]> = (topology.corners.iter())
                    .map(|&l| e.nodes.get(l).and_then(|&n| mesh.node(n)))
                    .collect::<Option<_>>()?;
                let nodes = face_node_ids(mesh, element, face);
                let min =
                    std::array::from_fn(|k| corners.iter().map(|c| c[k]).fold(f64::MAX, f64::min));
                let max =
                    std::array::from_fn(|k| corners.iter().map(|c| c[k]).fold(f64::MIN, f64::max));
                Some(TargetFace {
                    corners,
                    nodes,
                    min,
                    max,
                })
            })
            .collect();
        Self { faces }
    }

    /// Closest point of the surface to `p` and the shape function values of the face's
    /// nodes there, without the nodes whose value is zero.
    fn closest(&self, p: [f64; 3]) -> Option<([f64; 3], Terms)> {
        let mut best: Option<(f64, usize, [f64; 3])> = None;
        for (index, face) in self.faces.iter().enumerate() {
            // Distance to the bounding box is a lower bound of the distance to the face.
            let gap: f64 = (0..3)
                .map(|k| {
                    (face.min[k] - p[k])
                        .max(p[k] - face.max[k])
                        .max(0.0)
                        .powi(2)
                })
                .sum();
            if best.is_some_and(|(d, ..)| gap >= d) {
                continue;
            }
            let c = &face.corners;
            let triangles: &[[usize; 3]] = if c.len() == 4 {
                &[[0, 1, 2], [0, 2, 3]]
            } else {
                &[[0, 1, 2]]
            };
            for t in triangles {
                let q = closest_on_triangle(p, c[t[0]], c[t[1]], c[t[2]]);
                let d = dot(sub(q, p), sub(q, p));
                if best.is_none_or(|(b, ..)| d < b) {
                    best = Some((d, index, q));
                }
            }
        }
        let (_, index, point) = best?;
        let face = &self.faces[index];
        let values = shape_functions(&face.corners, face.nodes.len(), point);
        let terms: Vec<(NodeId, f64)> = (face.nodes.iter().copied())
            .zip(values)
            .filter(|(_, v)| v.abs() > 1e-9)
            .collect();
        // On a node all others vanish.
        match terms.as_slice() {
            [(node, v)] if (v - 1.0).abs() < 1e-9 => Some((point, vec![(*node, 1.0)])),
            _ => Some((point, terms)),
        }
    }
}

/// Closest point of triangle `abc` to `p` (Ericson, Real-Time Collision Detection 5.1.5).
fn closest_on_triangle(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let at = |s: f64, from: [f64; 3], d: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|k| from[k] + s * d[k])
    };
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return at(d1 / (d1 - d3), a, ab);
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return at(d2 / (d2 - d6), a, ac);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return at((d4 - d3) / ((d4 - d3) + (d5 - d6)), b, sub(c, b));
    }
    let denominator = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denominator, vc * denominator);
    std::array::from_fn(|k| a[k] + ab[k] * v + ac[k] * w)
}

/// Values of the shape functions of a face with `count` nodes (3, 6, 4 or 8, in CalculiX's
/// order) at a point on it. The local coordinates are found on the face spanned by the
/// corners.
fn shape_functions(corners: &[[f64; 3]], count: usize, p: [f64; 3]) -> Vec<f64> {
    if corners.len() == 3 {
        let [l1, l2, l3] = barycentric(p, corners[0], corners[1], corners[2]);
        if count == 6 {
            vec![
                l1 * (2.0 * l1 - 1.0),
                l2 * (2.0 * l2 - 1.0),
                l3 * (2.0 * l3 - 1.0),
                4.0 * l1 * l2,
                4.0 * l2 * l3,
                4.0 * l3 * l1,
            ]
        } else {
            vec![l1, l2, l3]
        }
    } else {
        let (xi, eta) = quad_coordinates(corners, p);
        let signs = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        if count == 8 {
            let mut values: Vec<f64> = signs
                .iter()
                .map(|(s, t)| 0.25 * (1.0 + s * xi) * (1.0 + t * eta) * (s * xi + t * eta - 1.0))
                .collect();
            values.extend([
                0.5 * (1.0 - xi * xi) * (1.0 - eta),
                0.5 * (1.0 + xi) * (1.0 - eta * eta),
                0.5 * (1.0 - xi * xi) * (1.0 + eta),
                0.5 * (1.0 - xi) * (1.0 - eta * eta),
            ]);
            values
        } else {
            (signs.iter())
                .map(|(s, t)| 0.25 * (1.0 + s * xi) * (1.0 + t * eta))
                .collect()
        }
    }
}

fn barycentric(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let (v0, v1, v2) = (sub(b, a), sub(c, a), sub(p, a));
    let (d00, d01, d11) = (dot(v0, v0), dot(v0, v1), dot(v1, v1));
    let (d20, d21) = (dot(v2, v0), dot(v2, v1));
    let denominator = d00 * d11 - d01 * d01;
    if denominator == 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let v = (d11 * d20 - d01 * d21) / denominator;
    let w = (d00 * d21 - d01 * d20) / denominator;
    [1.0 - v - w, v, w]
}

/// Local coordinates (xi, eta) in [-1, 1] of a point on a bilinear quadrilateral, by
/// Gauss-Newton iteration.
fn quad_coordinates(c: &[[f64; 3]], p: [f64; 3]) -> (f64, f64) {
    let (mut xi, mut eta) = (0.0_f64, 0.0_f64);
    for _ in 0..20 {
        let n = [
            0.25 * (1.0 - xi) * (1.0 - eta),
            0.25 * (1.0 + xi) * (1.0 - eta),
            0.25 * (1.0 + xi) * (1.0 + eta),
            0.25 * (1.0 - xi) * (1.0 + eta),
        ];
        let dxi = [
            -0.25 * (1.0 - eta),
            0.25 * (1.0 - eta),
            0.25 * (1.0 + eta),
            -0.25 * (1.0 + eta),
        ];
        let deta = [
            -0.25 * (1.0 - xi),
            -0.25 * (1.0 + xi),
            0.25 * (1.0 + xi),
            0.25 * (1.0 - xi),
        ];
        let combine = |w: &[f64; 4]| -> [f64; 3] {
            std::array::from_fn(|k| (0..4).map(|i| w[i] * c[i][k]).sum())
        };
        let (x, t1, t2) = (combine(&n), combine(&dxi), combine(&deta));
        let r = sub(p, x);
        let (a11, a12, a22) = (dot(t1, t1), dot(t1, t2), dot(t2, t2));
        let (b1, b2) = (dot(t1, r), dot(t2, r));
        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-300 {
            break;
        }
        let dx = (a22 * b1 - a12 * b2) / det;
        let de = (a11 * b2 - a12 * b1) / det;
        xi = (xi + dx).clamp(-1.0, 1.0);
        eta = (eta + de).clamp(-1.0, 1.0);
        if dx.abs() + de.abs() < 1e-12 {
            break;
        }
    }
    (xi, eta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_functions_sum_to_one_and_hit_nodes() {
        let tri = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let quad = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        for (corners, count) in [(&tri[..], 3), (&tri[..], 6), (&quad[..], 4), (&quad[..], 8)] {
            let values = shape_functions(corners, count, [0.5, 0.3, 0.0]);
            assert_eq!(values.len(), count);
            assert!((values.iter().sum::<f64>() - 1.0).abs() < 1e-12);
            let at_corner = shape_functions(corners, count, corners[1]);
            assert!((at_corner[1] - 1.0).abs() < 1e-9, "{at_corner:?}");
        }
        // Midside node of the first edge of a quadratic quadrilateral.
        let mid = shape_functions(&quad, 8, [1.0, 0.0, 0.0]);
        assert!((mid[4] - 1.0).abs() < 1e-9, "{mid:?}");
    }

    #[test]
    fn closest_point_on_a_triangle() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let near = |p: [f64; 3], q: [f64; 3]| (0..3).all(|k| (p[k] - q[k]).abs() < 1e-12);
        assert!(near(
            closest_on_triangle([0.2, 0.2, 5.0], a, b, c),
            [0.2, 0.2, 0.0]
        ));
        assert!(near(closest_on_triangle([-1.0, -1.0, 0.0], a, b, c), a));
        assert!(near(
            closest_on_triangle([0.5, -2.0, 1.0], a, b, c),
            [0.5, 0.0, 0.0]
        ));
        assert!(near(
            closest_on_triangle([1.0, 1.0, 0.0], a, b, c),
            [0.5, 0.5, 0.0]
        ));
    }
}
