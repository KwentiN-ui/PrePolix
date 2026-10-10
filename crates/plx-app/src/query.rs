//! PrePoMax's Query tool (Tools > Query): clicks in the 3D view write what was hit to the
//! output pane, as aligned lines like PrePoMax does, and leave an annotation in the view.
//! Nodes, elements and parts are queried with one click; distances, angles and circles
//! need two or three nodes.

use glam::{DVec3, Vec3};
use plx_mesh::NodeId;
use plx_model::Quantity;

use crate::model::{Hit, Model};
use crate::overlay::Marker;
use crate::results::{field_unit, format_value};
use crate::viewport::Preview;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryKind {
    Node,
    Element,
    Part,
    Assembly,
    BoundingBox,
    Distance,
    Angle,
    Circle,
}

const KINDS: [(QueryKind, &str); 8] = [
    (QueryKind::Node, "Vertex/Node"),
    (QueryKind::Element, "Facet/Element"),
    (QueryKind::Part, "Part"),
    (QueryKind::Assembly, "Assembly"),
    (QueryKind::BoundingBox, "Bounding box size"),
    (QueryKind::Distance, "Distance"),
    (QueryKind::Angle, "Angle"),
    (QueryKind::Circle, "Circle"),
];

impl QueryKind {
    /// How many nodes the query needs before it has something to say; 0 for queries that
    /// pick something else or nothing.
    fn nodes_needed(self) -> usize {
        match self {
            QueryKind::Node => 1,
            QueryKind::Distance => 2,
            QueryKind::Angle | QueryKind::Circle => 3,
            _ => 0,
        }
    }
}

/// What a query left in the 3D view. Nodes are kept by id, so the annotation follows the
/// shown deformation and stays where it belongs when another increment is shown.
#[derive(Clone, Debug, PartialEq)]
enum Annotation {
    Node(NodeId),
    Element(usize),
    Part(usize),
    Distance([NodeId; 2]),
    Angle([NodeId; 3]),
    Circle([NodeId; 3]),
    BoundingBox,
}

/// What the annotations draw: labelled points, lines and highlighted points.
#[derive(Default)]
pub struct QueryMarks {
    pub markers: Vec<Marker>,
    pub lines: Vec<Vec<Vec3>>,
    pub points: Vec<Vec3>,
}

pub enum QueryAction {
    Open,
    /// The annotations changed, so the overlay has to be rebuilt.
    Changed,
    Close,
}

pub struct QueryWindow {
    kind: QueryKind,
    /// Nodes picked so far for a distance, angle or circle.
    picked: Vec<NodeId>,
    annotations: Vec<Annotation>,
}

impl Default for QueryWindow {
    fn default() -> Self {
        Self {
            kind: QueryKind::Node,
            picked: Vec::new(),
            annotations: Vec::new(),
        }
    }
}

impl QueryWindow {
    /// The window with the list of queries. Choosing Assembly or Bounding box writes the
    /// values at once, the other queries wait for clicks in the 3D view.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        model: Option<&Model>,
        output: &mut Vec<String>,
    ) -> QueryAction {
        let mut action = QueryAction::Open;
        let mut open = true;
        egui::Window::new("Query")
            .id(egui::Id::new("query"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(ctx.content_rect().left_top() + egui::vec2(300.0, 90.0))
            .show(ctx, |ui| {
                let before = self.kind;
                for (kind, label) in KINDS {
                    ui.selectable_value(&mut self.kind, kind, label);
                }
                if self.kind != before {
                    self.picked.clear();
                    if let Some(model) = model {
                        match self.kind {
                            QueryKind::Assembly => {
                                output.extend(assembly_lines(model));
                                action = QueryAction::Changed;
                            }
                            QueryKind::BoundingBox => {
                                output.extend(bounding_box_lines(model));
                                self.annotations.push(Annotation::BoundingBox);
                                action = QueryAction::Changed;
                            }
                            _ => {}
                        }
                    }
                }
                ui.add_space(4.0);
                let hint = match self.kind.nodes_needed() {
                    0 if matches!(self.kind, QueryKind::Element | QueryKind::Part) => {
                        "Click an item in the 3D view.".to_string()
                    }
                    0 => "The values are written to the output pane.".to_string(),
                    1 => "Click a node in the 3D view.".to_string(),
                    n => format!(
                        "Click {n} nodes in the 3D view ({} picked).",
                        self.picked.len()
                    ),
                };
                ui.weak(hint);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Close").clicked() {
                        action = QueryAction::Close;
                    }
                    if ui.button("Clear").clicked() {
                        self.annotations.clear();
                        self.picked.clear();
                        action = QueryAction::Changed;
                    }
                });
            });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = QueryAction::Close;
        }
        action
    }

    /// A click in the 3D view: writes the queried values and adds the annotation. Returns
    /// whether the overlay has to be rebuilt.
    pub fn click(&mut self, model: &Model, hit: Option<&Hit>, output: &mut Vec<String>) -> bool {
        let Some(hit) = hit else {
            return false;
        };
        match self.kind {
            QueryKind::Element => {
                let Some(element) = model.hit_element(hit) else {
                    return false;
                };
                output.extend(element_lines(model, element));
                self.annotations.push(Annotation::Element(element));
            }
            QueryKind::Part => {
                output.extend(part_lines(model, hit.part));
                self.annotations.push(Annotation::Part(hit.part));
            }
            QueryKind::Assembly | QueryKind::BoundingBox => return false,
            QueryKind::Node | QueryKind::Distance | QueryKind::Angle | QueryKind::Circle => {
                let node = model.hit_node(hit);
                self.picked.push(node);
                if self.picked.len() < self.kind.nodes_needed() {
                    return true;
                }
                let picked = std::mem::take(&mut self.picked);
                match self.kind {
                    QueryKind::Node => {
                        output.extend(node_lines(model, node));
                        self.annotations.push(Annotation::Node(node));
                    }
                    QueryKind::Distance => {
                        let nodes = [picked[0], picked[1]];
                        output.extend(distance_lines(model, nodes));
                        self.annotations.push(Annotation::Distance(nodes));
                    }
                    QueryKind::Angle => {
                        let nodes = [picked[0], picked[1], picked[2]];
                        output.extend(angle_lines(model, nodes));
                        self.annotations.push(Annotation::Angle(nodes));
                    }
                    _ => {
                        let nodes = [picked[0], picked[1], picked[2]];
                        output.extend(circle_lines(model, nodes));
                        self.annotations.push(Annotation::Circle(nodes));
                    }
                }
            }
        }
        true
    }

    /// What a click would query: the nearest node, or the edges of the hit face.
    pub fn preview(&self, model: &Model, hit: &Hit) -> Preview {
        match self.kind {
            QueryKind::Assembly | QueryKind::BoundingBox => Preview::default(),
            QueryKind::Element | QueryKind::Part => Preview {
                lines: model.hit_outline(hit),
                points: Vec::new(),
            },
            _ => {
                let node = model.hit_node(hit);
                let index = model.mesh.node_index(node);
                Preview {
                    lines: Vec::new(),
                    points: index
                        .and_then(|i| model.node_position(i))
                        .into_iter()
                        .collect(),
                }
            }
        }
    }

    /// The annotations and the nodes picked so far, in render coordinates of the shown
    /// model.
    pub fn marks(&self, model: &Model) -> QueryMarks {
        let mut marks = QueryMarks::default();
        let position = |id: NodeId| model.node_position(model.mesh.node_index(id)?);
        let shown = |id: NodeId| Some(DVec3::from(model.shown_node(id)?));
        let units = model.units();
        let length = units.unit(Quantity::Length);
        marks.points = self.picked.iter().filter_map(|&id| position(id)).collect();
        for annotation in &self.annotations {
            match annotation {
                Annotation::Node(id) => {
                    let Some(p) = position(*id) else { continue };
                    let mut text = format!("Node id: {id}");
                    if let Some((value, unit)) = field_value(model, *id) {
                        text.push_str(&format!("\nValue: {value} {unit}"));
                    }
                    marks.points.push(p);
                    marks.markers.push(Marker { position: p, text });
                }
                Annotation::Element(index) => {
                    let Some(element) = model.mesh.elements().get(*index) else {
                        continue;
                    };
                    let outline = model.element_outline(*index);
                    let Some(center) = centroid(&outline) else {
                        continue;
                    };
                    marks.lines.extend(outline.iter().map(|&[a, b]| vec![a, b]));
                    marks.markers.push(Marker {
                        position: center,
                        text: format!("Element id: {}\n{}", element.id, element.type_name),
                    });
                }
                Annotation::Part(index) => {
                    let Some(part) = model.parts.get(*index) else {
                        continue;
                    };
                    let Some((min, max)) = part.bounds else {
                        continue;
                    };
                    marks.markers.push(Marker {
                        position: (min + max) * 0.5,
                        text: format!("Part: {}", part.name),
                    });
                }
                Annotation::Distance([a, b]) => {
                    let (Some(pa), Some(pb)) = (position(*a), position(*b)) else {
                        continue;
                    };
                    let distance = match (shown(*a), shown(*b)) {
                        (Some(a), Some(b)) => a.distance(b),
                        _ => continue,
                    };
                    marks.points.extend([pa, pb]);
                    marks.lines.push(vec![pa, pb]);
                    marks.markers.push(Marker {
                        position: (pa + pb) * 0.5,
                        text: format!("Distance: {} {length}", number(distance)),
                    });
                }
                Annotation::Angle([a, b, c]) => {
                    let (Some(pa), Some(pb), Some(pc)) = (position(*a), position(*b), position(*c))
                    else {
                        continue;
                    };
                    let angle = match (shown(*a), shown(*b), shown(*c)) {
                        (Some(a), Some(b), Some(c)) => angle_between(a, b, c),
                        _ => continue,
                    };
                    marks.points.extend([pa, pb, pc]);
                    marks.lines.push(vec![pa, pb, pc]);
                    marks.markers.push(Marker {
                        position: pb,
                        text: format!("Angle: {} °", number(angle)),
                    });
                }
                Annotation::Circle([a, b, c]) => {
                    let (Some(pa), Some(pb), Some(pc)) = (position(*a), position(*b), position(*c))
                    else {
                        continue;
                    };
                    let Some((center, radius, axis)) = (match (shown(*a), shown(*b), shown(*c)) {
                        (Some(a), Some(b), Some(c)) => circle_through(a, b, c),
                        _ => None,
                    }) else {
                        continue;
                    };
                    marks.points.extend([pa, pb, pc]);
                    marks
                        .lines
                        .push(circle_polyline(model, center, radius, axis));
                    marks.markers.push(Marker {
                        position: model.to_render(center.to_array()),
                        text: format!("Radius: {} {length}", number(radius)),
                    });
                }
                Annotation::BoundingBox => {
                    let Some((min, max)) = model.visible_bounds() else {
                        continue;
                    };
                    let corner = |i: usize| {
                        Vec3::new(
                            if i & 1 == 0 { min.x } else { max.x },
                            if i & 2 == 0 { min.y } else { max.y },
                            if i & 4 == 0 { min.z } else { max.z },
                        )
                    };
                    for (a, b) in [
                        (0, 1),
                        (1, 3),
                        (3, 2),
                        (2, 0),
                        (4, 5),
                        (5, 7),
                        (7, 6),
                        (6, 4),
                        (0, 4),
                        (1, 5),
                        (2, 6),
                        (3, 7),
                    ] {
                        marks.lines.push(vec![corner(a), corner(b)]);
                    }
                    let size = (max - min).as_dvec3();
                    marks.markers.push(Marker {
                        position: (min + max) * 0.5,
                        text: format!(
                            "Size: {} x {} x {} {length}",
                            number(size.x),
                            number(size.y),
                            number(size.z)
                        ),
                    });
                }
            }
        }
        marks
    }
}

/// A value with six significant digits, PrePoMax's default number format of the query.
fn number(value: f64) -> String {
    let value: f64 = format!("{value:.5e}").parse().unwrap_or(value);
    if value != 0.0 && !(1e-4..1e7).contains(&value.abs()) {
        format!("{value:e}")
    } else {
        format!("{value}")
    }
}

/// PrePoMax's layout of a query line: the item, its unit in brackets, the names of the
/// values and the values, in columns.
fn row(item: &str, unit: &str, names: &str, values: &[String]) -> String {
    let unit = if unit.is_empty() {
        "[/]".to_string()
    } else {
        format!("[{unit}]")
    };
    let mut line = format!("{item:<16}{unit:>8}{names:>16}");
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            line.push(',');
        }
        line.push_str(&format!("{value:>16}"));
    }
    line
}

fn xyz(p: DVec3) -> Vec<String> {
    vec![number(p.x), number(p.y), number(p.z)]
}

/// The value of the shown field at a node and its unit, on the Results tab.
fn field_value(model: &Model, node: NodeId) -> Option<(String, String)> {
    let view = model.results.as_ref()?;
    let (field, component) = view.current()?;
    let index = model.mesh.node_index(node)?;
    let value = *component.values.get(index)?;
    let unit = field_unit(&field.name, &component.name, view.units).unwrap_or_default();
    Some((format_value(value), unit))
}

fn node_lines(model: &Model, node: NodeId) -> Vec<String> {
    let Some(base) = model.mesh.node(node).map(DVec3::from) else {
        return Vec::new();
    };
    let length = model.units().unit(Quantity::Length);
    let item = if model.is_geometry() {
        "Vertex"
    } else {
        "Node"
    };
    let mut lines = vec![
        String::new(),
        row(item, "", "id:", &[node.to_string()]),
        row("Base", length, "x, y, z:", &xyz(base)),
    ];
    if let Some(view) = &model.results {
        let displacement = (view.current_increment())
            .and_then(|increment| increment.field("DISP"))
            .and_then(|field| {
                let index = model.mesh.node_index(node)?;
                let component = |name: &str| field.component(name).map_or(0.0, |c| c.values[index]);
                Some(DVec3::new(
                    component("U1") as f64,
                    component("U2") as f64,
                    component("U3") as f64,
                ))
            });
        if let Some(d) = displacement {
            lines.push(row("Deformed", length, "x, y, z:", &xyz(base + d)));
            lines.push(row("Displacement", length, "x, y, z:", &xyz(d)));
        }
        if let Some((value, unit)) = field_value(model, node) {
            let name = view.current().map_or(String::new(), |(field, component)| {
                format!("{}, {}:", field.name, component.name)
            });
            lines.push(row("Field value", &unit, &name, &[value]));
        }
    }
    lines.push(String::new());
    lines
}

fn element_lines(model: &Model, index: usize) -> Vec<String> {
    let Some(element) = model.mesh.elements().get(index) else {
        return Vec::new();
    };
    let item = if model.is_geometry() {
        "Facet"
    } else {
        "Element"
    };
    let mut lines = vec![
        String::new(),
        row(item, "", "id:", &[element.id.to_string()]),
    ];
    if !model.is_geometry() {
        lines.push(row(
            "Element type",
            "",
            ":",
            std::slice::from_ref(&element.type_name),
        ));
        let nodes: Vec<String> = element.nodes.iter().map(|n| n.to_string()).collect();
        lines.push(format!("{:<16}{:>8}  {}", "Nodes", "[/]", nodes.join(", ")));
    }
    lines.push(String::new());
    lines
}

fn element_type_lines(types: &[(String, usize)]) -> Vec<String> {
    (types.iter())
        .map(|(name, count)| format!("    {name:<24}{count:>10}"))
        .collect()
}

fn part_lines(model: &Model, index: usize) -> Vec<String> {
    let Some(part) = model.parts.get(index) else {
        return Vec::new();
    };
    let length = model.units().unit(Quantity::Length);
    let (elements, nodes) = if model.is_geometry() {
        ("Number of facets:", "Number of vertices:")
    } else {
        ("Number of elements:", "Number of nodes:")
    };
    let mut lines = vec![String::new(), format!("Part name: {}", part.name)];
    if let Some((min, max)) = part.bounds {
        let size = (max - min).as_dvec3();
        lines.push(row("Size", length, "x, y, z:", &xyz(size)));
    }
    lines.push(format!("{elements} {}", part.element_count));
    if !model.is_geometry() {
        lines.extend(element_type_lines(&part.element_types));
    }
    lines.push(format!("{nodes} {}", part.node_count));
    lines.push(String::new());
    lines
}

fn assembly_lines(model: &Model) -> Vec<String> {
    let (elements, nodes) = if model.is_geometry() {
        ("Number of facets:", "Number of vertices:")
    } else {
        ("Number of elements:", "Number of nodes:")
    };
    let mut lines = vec![
        String::new(),
        "Assembly".to_string(),
        format!("Number of parts: {}", model.parts.len()),
        format!("{elements} {}", model.mesh.element_count()),
    ];
    if !model.is_geometry() {
        let mut types: Vec<(String, usize)> = Vec::new();
        for (name, count) in model.parts.iter().flat_map(|p| p.element_types.iter()) {
            match types.iter_mut().find(|(n, _)| n == name) {
                Some((_, total)) => *total += count,
                None => types.push((name.clone(), *count)),
            }
        }
        lines.extend(element_type_lines(&types));
    }
    lines.push(format!("{nodes} {}", model.mesh.node_count()));
    lines.push(String::new());
    lines
}

fn bounding_box_lines(model: &Model) -> Vec<String> {
    let Some((min, max)) = model.visible_bounds() else {
        return vec!["Bounding box: nothing is visible".to_string()];
    };
    let (min, max) = (model.model_point(min), model.model_point(max));
    let length = model.units().unit(Quantity::Length);
    let mut lines = vec![String::new(), "Bounding box".to_string()];
    if let Some(view) = &model.results {
        lines.push(row(
            "Def. scale factor",
            "",
            "sf:",
            &[number((view.scale() * view.amplitude()) as f64)],
        ));
    }
    lines.push(row("Min", length, "x, y, z:", &xyz(min)));
    lines.push(row("Max", length, "x, y, z:", &xyz(max)));
    lines.push(row("Center", length, "x, y, z:", &xyz((min + max) * 0.5)));
    lines.push(row("Size", length, "x, y, z:", &xyz(max - min)));
    lines.push(String::new());
    lines
}

/// The base and, with results, the true-scale deformed position of a node.
fn base_and_deformed(model: &Model, node: NodeId) -> Option<(DVec3, Option<DVec3>)> {
    let base = DVec3::from(model.mesh.node(node)?);
    let deformed = model.results.as_ref().and_then(|view| {
        let increment = view.current_increment()?;
        let field = increment.field("DISP")?;
        let index = model.mesh.node_index(node)?;
        let component = |name: &str| field.component(name).map_or(0.0, |c| c.values[index]) as f64;
        Some(base + DVec3::new(component("U1"), component("U2"), component("U3")))
    });
    Some((base, deformed))
}

fn distance_lines(model: &Model, [a, b]: [NodeId; 2]) -> Vec<String> {
    let (Some((base_a, def_a)), Some((base_b, def_b))) =
        (base_and_deformed(model, a), base_and_deformed(model, b))
    else {
        return Vec::new();
    };
    let length = model.units().unit(Quantity::Length);
    let values = |d: DVec3| {
        let mut v = xyz(d);
        v.push(number(d.length()));
        v
    };
    let base = base_b - base_a;
    let mut lines = vec![
        String::new(),
        row("Distance", "", "id1, id2:", &[a.to_string(), b.to_string()]),
        row("Base", length, "dx, dy, dz, D:", &values(base)),
    ];
    if let (Some(def_a), Some(def_b)) = (def_a, def_b) {
        let deformed = def_b - def_a;
        lines.push(row("Deformed", length, "dx, dy, dz, D:", &values(deformed)));
        let mut delta = xyz(deformed - base);
        delta.push(number(deformed.length() - base.length()));
        lines.push(row("Delta", length, "dx, dy, dz, D:", &delta));
    }
    lines.push(String::new());
    lines
}

/// The angle at `b` between the lines to `a` and `c`, in degrees.
fn angle_between(a: DVec3, b: DVec3, c: DVec3) -> f64 {
    let (u, v) = ((a - b).normalize_or_zero(), (c - b).normalize_or_zero());
    u.dot(v).clamp(-1.0, 1.0).acos().to_degrees()
}

fn angle_lines(model: &Model, nodes: [NodeId; 3]) -> Vec<String> {
    let Some(points) = nodes
        .map(|n| base_and_deformed(model, n))
        .into_iter()
        .collect::<Option<Vec<_>>>()
    else {
        return Vec::new();
    };
    let base = angle_between(points[0].0, points[1].0, points[2].0);
    let ids = nodes.map(|n| n.to_string());
    let mut lines = vec![
        String::new(),
        row("Angle", "", "id1, id2, id3:", &ids),
        row("Base", "°", "phi:", &[number(base)]),
    ];
    if let (Some(a), Some(b), Some(c)) = (points[0].1, points[1].1, points[2].1) {
        let deformed = angle_between(a, b, c);
        lines.push(row("Deformed", "°", "phi:", &[number(deformed)]));
        lines.push(row("Delta", "°", "phi:", &[number(deformed - base)]));
    }
    lines.push(String::new());
    lines
}

/// Centre, radius and unit axis of the circle through three points; `None` on a line.
fn circle_through(a: DVec3, b: DVec3, c: DVec3) -> Option<(DVec3, f64, DVec3)> {
    let (u, v) = (b - a, c - a);
    let w = u.cross(v);
    let w2 = w.length_squared();
    if w2 < 1e-24 * (u.length_squared() * v.length_squared()).max(f64::MIN_POSITIVE) {
        return None;
    }
    let center =
        a + (v.length_squared() * w.cross(u) + u.length_squared() * v.cross(w)) / (2.0 * w2);
    Some((center, center.distance(a), w / w2.sqrt()))
}

fn circle_lines(model: &Model, nodes: [NodeId; 3]) -> Vec<String> {
    let Some(points) = nodes
        .map(|n| base_and_deformed(model, n))
        .into_iter()
        .collect::<Option<Vec<_>>>()
    else {
        return Vec::new();
    };
    let length = model.units().unit(Quantity::Length);
    let ids = nodes.map(|n| n.to_string());
    let mut lines = vec![String::new(), row("Circle", "", "id1, id2, id3:", &ids)];
    let describe = |label: &str, axis_label: &str, circle: Option<(DVec3, f64, DVec3)>| match circle
    {
        Some((center, radius, axis)) => {
            let mut values = xyz(center);
            values.push(number(radius));
            vec![
                row(label, length, "x, y, z, R:", &values),
                row(axis_label, "", "x, y, z:", &xyz(axis)),
            ]
        }
        None => vec![format!("{label}: the nodes lie on a line")],
    };
    lines.extend(describe(
        "Base",
        "Base axis",
        circle_through(points[0].0, points[1].0, points[2].0),
    ));
    if let (Some(a), Some(b), Some(c)) = (points[0].1, points[1].1, points[2].1) {
        lines.extend(describe(
            "Deformed",
            "Deformed axis",
            circle_through(a, b, c),
        ));
    }
    lines.push(String::new());
    lines
}

/// The circle as a closed polyline in render coordinates.
fn circle_polyline(model: &Model, center: DVec3, radius: f64, axis: DVec3) -> Vec<Vec3> {
    let helper = if axis.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let u = axis.cross(helper).normalize_or_zero();
    let v = axis.cross(u);
    (0..=72)
        .map(|i| {
            let t = i as f64 * std::f64::consts::TAU / 72.0;
            model.to_render((center + radius * (t.cos() * u + t.sin() * v)).to_array())
        })
        .collect()
}

fn centroid(lines: &[[Vec3; 2]]) -> Option<Vec3> {
    if lines.is_empty() {
        return None;
    }
    let sum: Vec3 = lines.iter().map(|[a, b]| *a + *b).sum();
    Some(sum / (2 * lines.len()) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circles_and_angles() {
        let (center, radius, axis) = circle_through(
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
        )
        .unwrap();
        assert!(center.length() < 1e-12);
        assert!((radius - 1.0).abs() < 1e-12);
        assert!((axis - DVec3::Z).length() < 1e-12);
        assert!(circle_through(DVec3::ZERO, DVec3::X, 2.0 * DVec3::X).is_none());
        let angle = angle_between(DVec3::X, DVec3::ZERO, DVec3::Y);
        assert!((angle - 90.0).abs() < 1e-12);
    }

    #[test]
    fn lines_are_aligned_like_prepomax() {
        let line = row("Base", "mm", "x, y, z:", &xyz(DVec3::new(1.0, 2.5, -3.0)));
        assert_eq!(
            line,
            "Base                [mm]        x, y, z:               1,             2.5,              -3"
        );
        assert_eq!(number(0.1234567891), "0.123457");
        assert_eq!(number(12345678.9), "1.23457e7");
        assert_eq!(number(0.0), "0");
    }
}
