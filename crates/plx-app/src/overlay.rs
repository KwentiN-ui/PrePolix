//! Annotations drawn over the 3D view in PrePoMax's layout: legend top left, information block
//! top right, scale bar bottom centre, axis triad bottom right, plus markers at the minimum and
//! maximum and a triad at the global origin. Legend, information block, scale bar and markers
//! can be dragged within the view.

use egui::{
    Align2, Color32, CursorIcon, FontId, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};
use glam::Vec3;
use plx_render::Camera;
use plx_render::contour::band_color;

use crate::results::{Legend, format_legend_value, format_value};

const MARGIN: f32 = 20.0;
const TEXT: Color32 = Color32::BLACK;
/// Legend band height; PrePoMax's boxes are a third wider than high.
const BAND_HEIGHT: f32 = 18.0;
const SCALE_BAR_WIDTH: f32 = 400.0;
const TRIAD_PICK_RADIUS: f32 = 9.0;
const SCALE_BAR_HEIGHT: f32 = 8.0;
const SCALE_BAR_FIELDS: usize = 5;
const STATUS_PADDING: f32 = 5.0;
const MARKER_PADDING: Vec2 = vec2(6.0, 3.0);

/// Everything drawn over the scene.
#[derive(Default)]
pub struct Overlay {
    pub legend: Option<Legend>,
    /// Lines of the information block in the top right corner; empty to hide it.
    pub status: Vec<String>,
    pub maximum: Option<Marker>,
    pub minimum: Option<Marker>,
    /// Global origin in render coordinates, where the global axis triad is drawn.
    pub global_origin: Option<Vec3>,
    pub show_scale_bar: bool,
    pub show_view_triad: bool,
    /// Selected nodes in render coordinates, drawn as highlighted points.
    pub nodes: Vec<Vec3>,
    /// Hot spot paths in render coordinates: the toe, then the read-out points.
    pub paths: Vec<Vec<Vec3>>,
}

/// Annotated point of the model, e.g. the node with the largest result value.
pub struct Marker {
    pub position: Vec3,
    pub text: String,
}

/// Where the user dragged the labels to, relative to their default places.
#[derive(Default)]
pub struct LabelOffsets {
    pub legend: Vec2,
    pub status: Vec2,
    pub scale_bar: Vec2,
    /// Box position relative to the marked point, once dragged.
    pub maximum: Option<Vec2>,
    pub minimum: Option<Vec2>,
}

fn font() -> FontId {
    FontId::proportional(14.0)
}

/// Screen position of a point in render coordinates.
pub fn project(camera: &Camera, rect: Rect, point: Vec3) -> Pos2 {
    let ndc = camera.view_proj(rect.aspect_ratio()).project_point3(point);
    pos2(
        rect.center().x + ndc.x * rect.width() * 0.5,
        rect.center().y - ndc.y * rect.height() * 0.5,
    )
}

/// Draws the annotations; returns the axis direction clicked in the corner triad, if any.
pub fn draw(
    ui: &Ui,
    rect: Rect,
    camera: &Camera,
    overlay: &Overlay,
    offsets: &mut LabelOffsets,
) -> Option<Vec3> {
    let painter = ui.painter_at(rect);
    let mut clicked_axis = None;
    if let Some(origin) = overlay.global_origin {
        let center = project(camera, rect, origin);
        if rect.contains(center) {
            triad(&painter, camera, center, 36.0, false);
        }
    }
    let view_proj = camera.view_proj(rect.aspect_ratio());
    for &node in &overlay.nodes {
        let ndc = view_proj.project_point3(node);
        let point = pos2(
            rect.center().x + ndc.x * rect.width() * 0.5,
            rect.center().y - ndc.y * rect.height() * 0.5,
        );
        painter.rect_filled(
            Rect::from_center_size(point, vec2(5.0, 5.0)),
            0.0,
            Color32::RED,
        );
    }
    for path in &overlay.paths {
        let points: Vec<Pos2> = path.iter().map(|&p| project(camera, rect, p)).collect();
        draw_path(&painter, &points);
    }
    if overlay.show_scale_bar {
        draw_scale_bar(ui, &painter, rect, camera, &mut offsets.scale_bar);
    }
    if overlay.show_view_triad {
        let corner = rect.right_bottom() - vec2(24.0 + 45.0, 24.0 + 45.0);
        triad(&painter, camera, corner, 45.0, true);
        clicked_axis = triad_buttons(ui, &painter, camera, corner, 45.0);
    }
    for (marker, offset, id) in [
        (&overlay.minimum, &mut offsets.minimum, "minimum label"),
        (&overlay.maximum, &mut offsets.maximum, "maximum label"),
    ] {
        if let Some(marker) = marker {
            let point = project(camera, rect, marker.position);
            draw_marker(ui, &painter, rect, point, &marker.text, offset, id);
        }
    }
    if let Some(legend) = &overlay.legend {
        draw_legend(ui, &painter, rect, legend, &mut offsets.legend);
    }
    if !overlay.status.is_empty() {
        draw_status(ui, &painter, rect, &overlay.status, &mut offsets.status);
    }
    clicked_axis
}

/// Makes the ends of the corner triad clickable: an axis tip, or the end of its faint negative
/// part, sets the view to look down that direction.
fn triad_buttons(
    ui: &Ui,
    painter: &Painter,
    camera: &Camera,
    center: Pos2,
    length: f32,
) -> Option<Vec3> {
    let screen = |axis: Vec3| vec2(axis.dot(camera.right()), -axis.dot(camera.up()));
    let mut ends = Vec::new();
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let v = screen(axis);
        // Axes pointing at the viewer collapse onto the centre and cannot be told apart.
        if v.length() < 0.2 {
            continue;
        }
        ends.push((axis, center + v * (length + 4.0)));
        ends.push((-axis, center - v * length * 2.0 / 3.0));
    }
    let pointer = ui.input(|i| i.pointer.hover_pos());
    // Nearest end under the pointer; ends towards the viewer win ties.
    let hovered = pointer.and_then(|pointer| {
        ends.iter()
            .filter(|(_, end)| end.distance(pointer) < TRIAD_PICK_RADIUS)
            .min_by(|a, b| {
                let key =
                    |(axis, end): &(Vec3, Pos2)| end.distance(pointer) + axis.dot(camera.forward());
                key(a).total_cmp(&key(b))
            })
            .copied()
    });
    let (axis, end) = hovered?;
    let area = Rect::from_center_size(end, Vec2::splat(2.0 * TRIAD_PICK_RADIUS));
    let response = ui.interact(area, ui.id().with("triad axis"), Sense::click());
    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    painter.circle_stroke(end, 7.0, Stroke::new(1.5, Color32::from_rgb(255, 160, 0)));
    response.clicked().then_some(axis)
}

/// A hot spot path: a line from the toe, a square on the toe and rings on the read-out
/// points.
fn draw_path(painter: &egui::Painter, points: &[Pos2]) {
    const COLOR: Color32 = Color32::from_rgb(0, 70, 200);
    let Some((&toe, readouts)) = points.split_first() else {
        return;
    };
    if let Some(&last) = readouts.last() {
        painter.line_segment([toe, last], egui::Stroke::new(1.5, COLOR));
    }
    painter.rect_filled(Rect::from_center_size(toe, vec2(6.0, 6.0)), 0.0, COLOR);
    for &point in readouts {
        painter.circle(point, 3.5, Color32::WHITE, egui::Stroke::new(1.5, COLOR));
    }
}

/// Places a draggable label: its default position plus the user's offset, kept inside the
/// view. Dragging moves the offset.
fn drag_label(ui: &Ui, rect: Rect, id: &str, default: Pos2, size: Vec2, offset: &mut Vec2) -> Rect {
    let frame = Rect::from_min_size(default + *offset, size);
    let response = ui.interact(frame, ui.id().with(id), Sense::drag());
    if response.dragged() {
        *offset += response.drag_delta();
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    let mut min = default + *offset;
    min.x = min
        .x
        .clamp(rect.left(), (rect.right() - size.x).max(rect.left()));
    min.y = min
        .y
        .clamp(rect.top(), (rect.bottom() - size.y).max(rect.top()));
    *offset = min - default;
    Rect::from_min_size(min, size)
}

/// Title lines, then one box per band with the band limits as labels, maximum on top.
fn draw_legend(ui: &Ui, painter: &Painter, rect: Rect, legend: &Legend, offset: &mut Vec2) {
    let titles: Vec<_> = legend
        .title
        .lines()
        .map(|l| painter.layout_no_wrap(l.to_string(), font(), TEXT))
        .collect();
    let levels = legend.levels;
    let values: Vec<_> = (0..=levels)
        .map(|i| {
            let t = 1.0 - i as f32 / levels as f32;
            let value = legend.min + t * (legend.max - legend.min);
            painter.layout_no_wrap(format_legend_value(value), font(), TEXT)
        })
        .collect();
    let band = vec2(BAND_HEIGHT / 0.75, BAND_HEIGHT);
    let label_height = values.first().map_or(0.0, |g| g.size().y);
    let title_height: f32 = titles.iter().map(|g| g.size().y + 2.0).sum::<f32>() + 6.0;
    let width = titles
        .iter()
        .map(|g| g.size().x)
        .chain(values.iter().map(|g| band.x + 8.0 + g.size().x))
        .fold(0.0, f32::max);
    let size = vec2(
        width,
        title_height + levels as f32 * band.y + label_height * 0.5,
    );
    let default = rect.left_top() + vec2(MARGIN, MARGIN);
    let frame = drag_label(ui, rect, "legend", default, size, offset);

    let mut top = frame.top();
    for galley in titles {
        let height = galley.size().y;
        painter.galley(pos2(frame.left(), top), galley, TEXT);
        top += height + 2.0;
    }
    top += 6.0;
    for i in 0..levels {
        let [r, g, b] = band_color(levels - 1 - i, levels).map(|c| (c * 255.0).round() as u8);
        let cell = Rect::from_min_size(pos2(frame.left(), top + i as f32 * band.y), band);
        painter.rect(
            cell,
            0.0,
            Color32::from_rgb(r, g, b),
            Stroke::new(1.0, TEXT),
            StrokeKind::Middle,
        );
    }
    for (i, galley) in values.into_iter().enumerate() {
        let y = top + i as f32 * band.y - galley.size().y * 0.5;
        painter.galley(pos2(frame.left() + band.x + 8.0, y), galley, TEXT);
    }
}

/// Framed text block, by default in the top right corner.
fn draw_status(ui: &Ui, painter: &Painter, rect: Rect, lines: &[String], offset: &mut Vec2) {
    let galleys: Vec<_> = lines
        .iter()
        .map(|l| painter.layout_no_wrap(l.clone(), font(), TEXT))
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let line_height = galleys.first().map_or(0.0, |g| g.size().y) + 2.0;
    let size = vec2(width, line_height * galleys.len() as f32)
        + vec2(2.0 * STATUS_PADDING, STATUS_PADDING);
    let default = pos2(rect.right() - MARGIN - size.x, rect.top() + MARGIN);
    let frame = drag_label(ui, rect, "status", default, size, offset);
    painter.rect_stroke(frame, 0.0, Stroke::new(1.0, TEXT), StrokeKind::Inside);
    for (i, galley) in galleys.into_iter().enumerate() {
        let pos = frame.min
            + vec2(
                STATUS_PADDING,
                STATUS_PADDING * 0.5 + i as f32 * line_height,
            );
        painter.galley(pos, galley, TEXT);
    }
}

/// Bar of five alternating fields whose total length is a round number of model units, by
/// default centred at the bottom; dragging it moves the bar together with its labels.
fn draw_scale_bar(ui: &Ui, painter: &Painter, rect: Rect, camera: &Camera, offset: &mut Vec2) {
    let world_per_point = camera.pixel_size(rect.width(), rect.height());
    let Some((length, width)) = scale_bar_length(world_per_point) else {
        return;
    };
    let labels: Vec<_> = (0..=SCALE_BAR_FIELDS)
        .map(|i| {
            let value = length * i as f32 / SCALE_BAR_FIELDS as f32;
            painter.layout_no_wrap(format_value(value), font(), TEXT)
        })
        .collect();
    // The outer labels are centred on the bar ends and stick out by half their width.
    let overhang =
        |galley: Option<&std::sync::Arc<egui::Galley>>| galley.map_or(0.0, |g| g.size().x * 0.5);
    let (left_overhang, right_overhang) = (overhang(labels.first()), overhang(labels.last()));
    let label_height = labels.first().map_or(0.0, |g| g.size().y);
    let size = vec2(
        left_overhang + width + right_overhang,
        label_height + 3.0 + SCALE_BAR_HEIGHT,
    );
    let default = pos2(
        rect.center().x - width * 0.5 - left_overhang,
        rect.bottom() - MARGIN - size.y,
    );
    let frame = drag_label(ui, rect, "scale bar", default, size, offset);
    let left = frame.left() + left_overhang;
    let top = frame.bottom() - SCALE_BAR_HEIGHT;
    let field = width / SCALE_BAR_FIELDS as f32;
    for i in 0..SCALE_BAR_FIELDS {
        let cell = Rect::from_min_size(
            pos2(left + i as f32 * field, top),
            vec2(field, SCALE_BAR_HEIGHT),
        );
        let fill = if i % 2 == 0 { TEXT } else { Color32::WHITE };
        painter.rect(cell, 0.0, fill, Stroke::new(1.0, TEXT), StrokeKind::Middle);
    }
    for (i, galley) in labels.into_iter().enumerate() {
        let x = left + i as f32 * field - galley.size().x * 0.5;
        painter.galley(pos2(x, frame.top()), galley, TEXT);
    }
}

/// Round length near [`SCALE_BAR_WIDTH`] points (steps of 0.2 of a power of ten) and its width
/// in points.
pub fn scale_bar_length(world_per_point: f32) -> Option<(f32, f32)> {
    let nominal = SCALE_BAR_WIDTH * world_per_point;
    if !(nominal.is_finite() && nominal > 0.0) {
        return None;
    }
    let power = 10f32.powf(nominal.log10().floor());
    let length = (5.0 * nominal / power).round() / 5.0 * power;
    Some((length, length / world_per_point))
}

/// Framed label next to a model point, with an arrow pointing at it. The box follows the point
/// when the view moves; dragging it changes where it sits relative to the point.
fn draw_marker(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    point: Pos2,
    text: &str,
    offset: &mut Option<Vec2>,
    id: &str,
) {
    if !rect.contains(point) {
        return;
    }
    let galley = painter.layout_no_wrap(text.to_string(), font(), TEXT);
    let size = galley.size() + 2.0 * MARKER_PADDING;
    // Above right of the point, or mirrored where the view ends.
    let automatic = || {
        let dx = if point.x + 60.0 + size.x < rect.right() {
            60.0
        } else {
            -60.0 - size.x
        };
        let dy = if point.y - 50.0 - size.y > rect.top() {
            -50.0 - size.y
        } else {
            50.0
        };
        vec2(dx, dy)
    };
    let mut relative = offset.unwrap_or_else(automatic);
    let before = relative;
    let frame = drag_label(ui, rect, id, point, size, &mut relative);
    if relative != before {
        *offset = Some(relative);
    }
    let anchor = pos2(
        point.x.clamp(frame.left(), frame.right()),
        point.y.clamp(frame.top(), frame.bottom()),
    );
    if anchor.distance(point) > 1.0 {
        painter.line_segment([anchor, point], Stroke::new(1.0, TEXT));
        let direction = (point - anchor).normalized();
        let normal = direction.rot90();
        painter.add(egui::Shape::convex_polygon(
            vec![
                point,
                point - direction * 9.0 + normal * 3.0,
                point - direction * 9.0 - normal * 3.0,
            ],
            TEXT,
            Stroke::NONE,
        ));
    }
    painter.rect(
        frame,
        0.0,
        Color32::WHITE,
        Stroke::new(1.0, TEXT),
        StrokeKind::Inside,
    );
    painter.galley(frame.min + MARKER_PADDING, galley, TEXT);
}

/// Axes X, Y, Z as arrows in screen space, drawn back to front; `full` adds PrePoMax's centre
/// sphere and the shorter, faint negative axes.
fn triad(painter: &Painter, camera: &Camera, center: Pos2, length: f32, full: bool) {
    let axes = [
        (Vec3::X, "X", Color32::from_rgb(255, 0, 0)),
        (Vec3::Y, "Y", Color32::from_rgb(0, 230, 0)),
        (Vec3::Z, "Z", Color32::from_rgb(0, 0, 255)),
    ];
    let screen = |axis: Vec3| vec2(axis.dot(camera.right()), -axis.dot(camera.up()));
    if full {
        let grey = Color32::from_rgba_unmultiplied(90, 90, 90, 128);
        for (axis, _, _) in axes {
            let end = center - screen(axis) * length * 2.0 / 3.0;
            painter.line_segment([center, end], Stroke::new(2.0, grey));
        }
    }
    let mut order: Vec<_> = axes.iter().collect();
    // Farthest first; forward points into the screen.
    order.sort_by(|a, b| {
        let depth = |axis: Vec3| axis.dot(camera.forward());
        depth(b.0).total_cmp(&depth(a.0))
    });
    let cone = length * 14.0 / 45.0;
    for &(axis, label, color) in order {
        let v = screen(axis);
        let tip = center + v * length;
        let base = center + v * (length - cone);
        painter.line_segment([center, base], Stroke::new(2.5, color));
        let normal = v.normalized().rot90() * 3.5;
        if v.length() > 0.05 {
            painter.add(egui::Shape::convex_polygon(
                vec![tip, base + normal, base - normal],
                color,
                Stroke::NONE,
            ));
        }
        let label_pos = center + v * length + v.normalized() * 9.0;
        painter.text(label_pos, Align2::CENTER_CENTER, label, font(), TEXT);
    }
    if full {
        painter.circle_filled(center, 3.5, Color32::from_rgb(200, 200, 200));
        painter.circle_stroke(center, 3.5, Stroke::new(0.5, Color32::from_gray(120)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_bar_rounds_to_fifths_of_a_decade() {
        // 400 points at 0.37 units each are 148 units: rounded to 140.
        let (length, width) = scale_bar_length(0.37).unwrap();
        assert!((length - 140.0).abs() < 1e-3, "{length}");
        assert!((width - 140.0 / 0.37).abs() < 1e-2);
        assert!(scale_bar_length(0.0).is_none());
    }
}
