//! Symbols of boundary conditions and loads drawn over the 3D view, as PrePoMax draws them
//! for the step chosen to show: arrows for forces at the loaded nodes, pressure arrows
//! pointing onto the faces, one arrow of the resultant force for a surface traction, and
//! cones (translations) and plates (rotations) for held degrees of freedom. Heat loads and
//! temperatures, which have no direction, are balls at the nodes; heat flowing through a
//! surface is an arrow onto it (into the part) or off it.
//!
//! Symbols keep their size on screen like PrePoMax's glyphs (PrePoMax symbol size 50) and lie
//! on top of the model. They are 3D shapes in pixel units, drawn as the outlines of their
//! convex parts.

use std::collections::HashSet;

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, vec2};
use glam::{DVec3, Vec3};
use plx_mesh::{ElementId, NodeId};
use plx_model::{BoundaryKind, LoadKind, Region};
use plx_render::Camera;

use crate::model::Model;

/// Unit length of the support symbols in pixels; PrePoMax's default symbol size 50 draws a
/// fixed support about this large.
const SUPPORT_SIZE: f32 = 20.0;
/// Arrow length in pixels, longer than PrePoMax's so that the arrows read at a glance.
const ARROW_SIZE: f32 = 40.0;
/// PrePoMax's default colours: loads royal blue, boundary conditions lime, selection red.
const LOAD_COLOR: Color32 = Color32::from_rgb(65, 105, 225);
const BOUNDARY_COLOR: Color32 = Color32::from_rgb(0, 255, 0);
const SELECTED_COLOR: Color32 = Color32::from_rgb(255, 0, 0);
/// Splits of the region's extent when spreading arrows, as PrePoMax's spatial sampling.
const DIVISIONS: f64 = 6.0;
/// Sides of the polygons standing in for circles.
const CIRCLE_SEGMENTS: usize = 16;

/// What a symbol stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Load(LoadKind),
    Boundary(BoundaryKind),
}

/// A boundary condition or load to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub region: Region,
    /// Selected in the tree or edited in a dialog: drawn in the highlight colour.
    pub selected: bool,
}

/// One symbol: its shape at a point of the model.
#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    /// Point in render coordinates.
    pub position: Vec3,
    /// Unit direction the shape points along.
    pub direction: Vec3,
    pub shape: SymbolShape,
    pub color: Color32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SymbolShape {
    /// Arrow starting at the point.
    Arrow,
    /// Arrow ending at the point, such as pressure pushing onto a face.
    ArrowOnto,
    /// Cone with its tip at the point: a held translation.
    Cone,
    /// Rod with a plate: a held rotation.
    RotationLock,
    /// Ball around the point: a temperature or heat at a node.
    Ball,
}

/// Symbols of the items, on the visible parts of the model.
pub fn build(model: &Model, items: &[Item]) -> Vec<Symbol> {
    let visible = Visible::new(model);
    let mut symbols = Vec::new();
    for item in items {
        let color = match (&item.kind, item.selected) {
            (_, true) => SELECTED_COLOR,
            (Kind::Load(_), false) => LOAD_COLOR,
            (Kind::Boundary(_), false) => BOUNDARY_COLOR,
        };
        let mut add = |position: DVec3, direction: DVec3, shape: SymbolShape| {
            if let Some(direction) = direction.try_normalize() {
                symbols.push(Symbol {
                    position: (position - model.origin()).as_vec3(),
                    direction: direction.as_vec3(),
                    shape,
                    color,
                });
            }
        };
        match item.kind {
            Kind::Load(LoadKind::ConcentratedForce(force)) => {
                let points = node_points(model, &item.region, &visible);
                for index in sample(&points) {
                    add(points[index], DVec3::from(force), SymbolShape::Arrow);
                }
            }
            Kind::Load(LoadKind::Pressure(pressure)) => {
                let faces = face_geometry(model, &item.region, &visible);
                let centers: Vec<DVec3> = faces.iter().map(|f| f.center).collect();
                for index in sample(&centers) {
                    let face = &faces[index];
                    if pressure >= 0.0 {
                        add(face.center, -face.normal, SymbolShape::ArrowOnto);
                    } else {
                        add(face.center, face.normal, SymbolShape::Arrow);
                    }
                }
            }
            Kind::Load(LoadKind::SurfaceTraction(force)) => {
                if let Some(center) = region_center(model, &item.region, &visible) {
                    add(center, DVec3::from(force), SymbolShape::Arrow);
                }
            }
            Kind::Load(LoadKind::PreTension {
                value, direction, ..
            }) => {
                // The preload pulls the cut together: arrows onto the faces, out of them
                // for a negative value.
                let faces = face_geometry(model, &item.region, &visible);
                let centers: Vec<DVec3> = faces.iter().map(|f| f.center).collect();
                for index in sample(&centers) {
                    let face = &faces[index];
                    let along = direction.map_or(face.normal, DVec3::from);
                    if value >= 0.0 {
                        add(face.center, -along, SymbolShape::ArrowOnto);
                    } else {
                        add(face.center, along, SymbolShape::Arrow);
                    }
                }
            }
            // Gravity as an arrow at the centre of the loaded elements, as in PrePoMax.
            Kind::Load(LoadKind::Gravity(acceleration)) => {
                if let Some(center) = region_center(model, &item.region, &visible) {
                    add(center, DVec3::from(acceleration), SymbolShape::Arrow);
                }
            }
            // The rotation axis through the loaded elements: the point of the axis closest
            // to their centre, with the direction of the axis.
            Kind::Load(LoadKind::Centrifugal { point, axis, .. }) => {
                if let Some(center) = region_center(model, &item.region, &visible)
                    && let Some(direction) = DVec3::from(axis).try_normalize()
                {
                    let point = DVec3::from(point);
                    let foot = point + (center - point).dot(direction) * direction;
                    add(foot, direction, SymbolShape::RotationLock);
                }
            }
            Kind::Load(
                LoadKind::SurfaceFlux(_) | LoadKind::Film { .. } | LoadKind::Radiation { .. },
            ) => {
                let faces = face_geometry(model, &item.region, &visible);
                let centers: Vec<DVec3> = faces.iter().map(|f| f.center).collect();
                let into = matches!(item.kind, Kind::Load(LoadKind::SurfaceFlux(q)) if q >= 0.0);
                for index in sample(&centers) {
                    let face = &faces[index];
                    if into {
                        add(face.center, -face.normal, SymbolShape::ArrowOnto);
                    } else {
                        add(face.center, face.normal, SymbolShape::Arrow);
                    }
                }
            }
            Kind::Load(LoadKind::ConcentratedFlux(_) | LoadKind::BodyFlux(_))
            | Kind::Boundary(BoundaryKind::Temperature(_)) => {
                let points = node_points(model, &item.region, &visible);
                for index in sample(&points) {
                    add(points[index], DVec3::Z, SymbolShape::Ball);
                }
            }
            Kind::Boundary(kind) => {
                let held: [bool; 6] = match kind {
                    BoundaryKind::Fixed => [true; 6],
                    BoundaryKind::Displacement(values) => values.map(|v| v.is_some()),
                    BoundaryKind::Temperature(_) => [false; 6],
                    BoundaryKind::Submodel { dofs, .. } => dofs,
                };
                if let Some(center) = region_center(model, &item.region, &visible) {
                    for (axis, &held) in held.iter().enumerate() {
                        let shape = if axis < 3 {
                            SymbolShape::Cone
                        } else {
                            SymbolShape::RotationLock
                        };
                        if held {
                            add(center, DVec3::AXES[axis % 3], shape);
                        }
                    }
                }
            }
        }
    }
    symbols
}

/// The parts shown; symbols on hidden parts are left out as in PrePoMax.
struct Visible {
    /// Elements of the hidden parts; empty when all parts are shown.
    hidden_elements: HashSet<ElementId>,
    /// Nodes of the shown parts, only when some part is hidden.
    nodes: Option<HashSet<NodeId>>,
}

impl Visible {
    fn new(model: &Model) -> Self {
        if model.parts.iter().all(|p| p.visible) {
            return Self {
                hidden_elements: HashSet::new(),
                nodes: None,
            };
        }
        let hidden_elements = (model.parts.iter().zip(&model.mesh.parts))
            .filter(|(info, _)| !info.visible)
            .flat_map(|(_, part)| part.elements.iter().copied())
            .collect();
        Self {
            hidden_elements,
            nodes: Some(model.visible_nodes().into_iter().collect()),
        }
    }

    fn node(&self, id: NodeId) -> bool {
        self.nodes.as_ref().is_none_or(|nodes| nodes.contains(&id))
    }

    fn element(&self, id: ElementId) -> bool {
        !self.hidden_elements.contains(&id)
    }
}

fn node_points(model: &Model, region: &Region, visible: &Visible) -> Vec<DVec3> {
    let coords = model.exploded_coords();
    region
        .nodes(&model.mesh)
        .into_iter()
        .filter(|&id| visible.node(id))
        .filter_map(|id| model.mesh.node_index(id))
        .map(|index| DVec3::from(coords[index]))
        .collect()
}

struct FaceGeometry {
    center: DVec3,
    /// Unit normal pointing out of the element.
    normal: DVec3,
    area: f64,
}

/// Centre, outward normal and area of each element face of the region.
fn face_geometry(model: &Model, region: &Region, visible: &Visible) -> Vec<FaceGeometry> {
    let mesh = &model.mesh;
    let coords = model.exploded_coords();
    let position = |id: NodeId| mesh.node_index(id).map(|i| DVec3::from(coords[i]));
    let mut faces = Vec::new();
    for (element_id, face) in region.faces(mesh) {
        if !visible.element(element_id) {
            continue;
        }
        let Some(element) = mesh.element(element_id) else {
            continue;
        };
        let Some(topology) =
            (usize::from(face).checked_sub(1)).and_then(|f| element.faces().get(f))
        else {
            continue;
        };
        let corners: Option<Vec<DVec3>> = (topology.corners.iter())
            .map(|&local| element.nodes.get(local).copied().and_then(position))
            .collect();
        let Some(corners) = corners.filter(|c| c.len() >= 2) else {
            continue;
        };
        let center = corners.iter().sum::<DVec3>() / corners.len() as f64;
        let nodes: Vec<DVec3> = element.nodes.iter().filter_map(|&n| position(n)).collect();
        let centroid = nodes.iter().sum::<DVec3>() / nodes.len().max(1) as f64;
        let (area, normal) = if let [a, b] = corners[..] {
            // An edge of a 2D element: its length, and the normal in the element's plane.
            let along = b - a;
            let away = center - centroid;
            (
                along.length(),
                away - along * away.dot(along) / along.length_squared(),
            )
        } else {
            // Newell's method: twice the area along the normal, for any planar-ish polygon.
            let mut normal = DVec3::ZERO;
            for (i, a) in corners.iter().enumerate() {
                normal += a.cross(corners[(i + 1) % corners.len()]);
            }
            (normal.length() / 2.0, normal)
        };
        let Some(mut normal) = normal.try_normalize() else {
            continue;
        };
        if normal.dot(center - centroid) < 0.0 {
            normal = -normal;
        }
        faces.push(FaceGeometry {
            center,
            normal,
            area,
        });
    }
    faces
}

/// Area-weighted centre of the region's faces, or the mean of its nodes when it has none.
fn region_center(model: &Model, region: &Region, visible: &Visible) -> Option<DVec3> {
    let faces = face_geometry(model, region, visible);
    let area: f64 = faces.iter().map(|f| f.area).sum();
    if area > 0.0 {
        return Some(faces.iter().map(|f| f.center * f.area).sum::<DVec3>() / area);
    }
    let points = node_points(model, region, visible);
    (!points.is_empty()).then(|| points.iter().sum::<DVec3>() / points.len() as f64)
}

/// Indices of points spread evenly over their extent: clusters are halved across their
/// largest extent until none spans more than a sixth of the whole, then the point nearest
/// each cluster's mean stands for it. A simpler take on PrePoMax's spatial point sampler.
fn sample(points: &[DVec3]) -> Vec<usize> {
    let span = |indices: &[usize]| {
        let (min, max) = indices.iter().fold(
            (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)),
            |(min, max), &i| (min.min(points[i]), max.max(points[i])),
        );
        (min, max - min)
    };
    let all: Vec<usize> = (0..points.len()).collect();
    if all.is_empty() {
        return all;
    }
    let limit = span(&all).1.max_element() / DIVISIONS;
    let mut pending = vec![all];
    let mut done = Vec::new();
    while let Some(cluster) = pending.pop() {
        let (min, extent) = span(&cluster);
        let largest = extent.max_element();
        if cluster.len() <= 1 || largest <= limit {
            done.push(cluster);
            continue;
        }
        let axis = if extent.x == largest {
            0
        } else if extent.y == largest {
            1
        } else {
            2
        };
        let middle = min[axis] + largest / 2.0;
        let (low, high): (Vec<usize>, Vec<usize>) =
            cluster.iter().partition(|&&i| points[i][axis] < middle);
        if low.is_empty() || high.is_empty() {
            done.push(cluster);
            continue;
        }
        pending.push(low);
        pending.push(high);
    }
    let mut picked: Vec<usize> = done
        .iter()
        .map(|cluster| {
            let mean = cluster.iter().map(|&i| points[i]).sum::<DVec3>() / cluster.len() as f64;
            *cluster
                .iter()
                .min_by(|&&a, &&b| {
                    (points[a].distance_squared(mean)).total_cmp(&points[b].distance_squared(mean))
                })
                .expect("clusters are not empty")
        })
        .collect();
    picked.sort_unstable();
    picked
}

/// Draws the symbols, farthest first.
pub fn draw(painter: &Painter, rect: Rect, camera: &Camera, symbols: &[Symbol]) {
    let (right, up, forward) = (camera.right(), camera.up(), camera.forward());
    // Offsets in pixels on screen for a vector in pixel units of the scene.
    let screen = |v: Vec3| vec2(v.dot(right), -v.dot(up));
    let mut order: Vec<&Symbol> = symbols.iter().collect();
    order.sort_by(|a, b| (b.position.dot(forward)).total_cmp(&a.position.dot(forward)));
    for symbol in order {
        let anchor = crate::overlay::project(camera, rect, symbol.position);
        if !rect.expand(2.0 * ARROW_SIZE).contains(anchor) {
            continue;
        }
        let mut solids = solids(symbol.shape, symbol.direction);
        solids.sort_by(|a, b| (b.depth.dot(forward)).total_cmp(&a.depth.dot(forward)));
        for solid in solids {
            let points: Vec<Pos2> = solid.points.iter().map(|&p| anchor + screen(p)).collect();
            let hull = convex_hull(points);
            if hull.len() >= 3 {
                painter.add(Shape::convex_polygon(
                    hull,
                    symbol.color,
                    Stroke::new(1.0, edge(symbol.color)),
                ));
            }
        }
    }
}

/// Dark edges, like PrePoMax's symbol edges; they keep red symbols visible on the red
/// highlight of the selected region.
fn edge(color: Color32) -> Color32 {
    Color32::from_rgb(color.r() / 2, color.g() / 2, color.b() / 2)
}

/// A convex part of a symbol in pixel units relative to its point: its outline is the hull
/// of the points.
struct Solid {
    points: Vec<Vec3>,
    /// A point inside, to draw the nearer parts last.
    depth: Vec3,
}

/// The parts of a shape along a direction, after PrePoMax's VTK glyphs (unit length is the
/// symbol size): the arrow source with tip length 0.3, tip radius 0.1 and shaft radius 0.03,
/// shifted 0.05 off the point; the cone of height 1 and radius 0.5; the rod of length 1.3
/// with a 0.4 thick plate of width 0.9.
fn solids(shape: SymbolShape, direction: Vec3) -> Vec<Solid> {
    let size = match shape {
        SymbolShape::Arrow | SymbolShape::ArrowOnto => ARROW_SIZE,
        SymbolShape::Cone | SymbolShape::RotationLock | SymbolShape::Ball => SUPPORT_SIZE,
    };
    let axis = direction * size;
    let (u, v) = direction.any_orthonormal_pair();
    let disc = |at: f32, radius: f32| -> Vec<Vec3> {
        (0..CIRCLE_SEGMENTS)
            .map(|i| {
                let angle = i as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                axis * at + (u * angle.cos() + v * angle.sin()) * radius * size
            })
            .collect()
    };
    let cylinder = |from: f32, to: f32, radius: f32| Solid {
        points: [disc(from, radius), disc(to, radius)].concat(),
        depth: axis * (from + to) / 2.0,
    };
    let cone = |base: f32, tip: f32, radius: f32| Solid {
        points: [disc(base, radius), vec![axis * tip]].concat(),
        depth: axis * (base + tip) / 2.0,
    };
    match shape {
        SymbolShape::Arrow | SymbolShape::ArrowOnto => {
            let start = if shape == SymbolShape::Arrow {
                0.05
            } else {
                -1.05
            };
            vec![
                cylinder(start, start + 0.7, 0.03),
                cone(start + 0.7, start + 1.0, 0.1),
            ]
        }
        SymbolShape::Cone => vec![cone(-1.0, 0.0, 0.5)],
        SymbolShape::Ball => {
            // Three great circles; their outline is round from every side.
            let radius = 0.3 * size;
            let circle = |a: Vec3, b: Vec3| -> Vec<Vec3> {
                (0..CIRCLE_SEGMENTS)
                    .map(|i| {
                        let angle = i as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                        (a * angle.cos() + b * angle.sin()) * radius
                    })
                    .collect()
            };
            vec![Solid {
                points: [circle(u, v), circle(v, direction), circle(direction, u)].concat(),
                depth: Vec3::ZERO,
            }]
        }
        SymbolShape::RotationLock => {
            let mut plate = Vec::new();
            for along in [-1.35, -1.75] {
                for (a, b) in [(1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (-1.0, 1.0)] {
                    plate.push(axis * along + (u * a + v * b) * 0.45 * size);
                }
            }
            vec![
                cylinder(0.0, -1.3, 0.02),
                Solid {
                    points: plate,
                    depth: axis * -1.55,
                },
            ]
        }
    }
}

/// Convex hull in counter-clockwise order (Andrew's monotone chain).
fn convex_hull(mut points: Vec<Pos2>) -> Vec<Pos2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup_by(|a, b| (*a - *b).length_sq() < 1e-6);
    if points.len() < 3 {
        return points;
    }
    let cross = |o: Pos2, a: Pos2, b: Pos2| {
        let (oa, ob): (Vec2, Vec2) = (a - o, b - o);
        oa.x * ob.y - oa.y * ob.x
    };
    let mut hull: Vec<Pos2> = Vec::with_capacity(points.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &Pos2>> = if pass == 0 {
            Box::new(points.iter())
        } else {
            Box::new(points.iter().rev())
        };
        for &p in iter {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_drops_inner_points() {
        let points = vec![
            Pos2::new(0.0, 0.0),
            Pos2::new(2.0, 0.0),
            Pos2::new(1.0, 1.0),
            Pos2::new(2.0, 2.0),
            Pos2::new(0.0, 2.0),
        ];
        let hull = convex_hull(points);
        assert_eq!(hull.len(), 4);
        assert!(!hull.contains(&Pos2::new(1.0, 1.0)));
    }

    #[test]
    fn sampling_spreads_over_the_extent() {
        // 101 points on a line: about one per sixth of its length.
        let points: Vec<DVec3> = (0..=100).map(|i| DVec3::new(i as f64, 0.0, 0.0)).collect();
        let picked = sample(&points);
        assert!((6..=12).contains(&picked.len()), "{picked:?}");
        assert_eq!(sample(&points[..1]), vec![0]);
        assert!(sample(&[]).is_empty());
    }

    #[test]
    fn arrows_start_or_end_at_the_point() {
        let tip = |shape| {
            let parts = solids(shape, Vec3::X);
            let head = &parts[1].points;
            head[head.len() - 1].x / ARROW_SIZE
        };
        assert!((tip(SymbolShape::Arrow) - 1.05).abs() < 1e-5);
        assert!((tip(SymbolShape::ArrowOnto) + 0.05).abs() < 1e-5);
    }
}
