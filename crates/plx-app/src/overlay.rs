//! Annotations drawn over the 3D view in PrePoMax's layout: legend top left, information block
//! top right, scale bar bottom centre, axis triad bottom right, plus the maximum marker and a
//! triad at the global origin.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use glam::Vec3;
use plx_render::Camera;
use plx_render::contour::band_color;

use crate::results::{Legend, format_legend_value, format_value};

const MARGIN: f32 = 20.0;
const TEXT: Color32 = Color32::BLACK;
/// Legend band height; PrePoMax's boxes are a third wider than high.
const BAND_HEIGHT: f32 = 18.0;
const SCALE_BAR_WIDTH: f32 = 400.0;
const SCALE_BAR_HEIGHT: f32 = 8.0;
const SCALE_BAR_FIELDS: usize = 5;

/// Everything drawn over the scene besides the triads.
#[derive(Default)]
pub struct Overlay {
    pub legend: Option<Legend>,
    /// Lines of the information block in the top right corner.
    pub status: Vec<String>,
    pub maximum: Option<Marker>,
    /// Global origin in render coordinates, where the global axis triad is drawn.
    pub global_origin: Option<Vec3>,
}

/// Annotated point of the model, e.g. the node with the largest result value.
pub struct Marker {
    pub position: Vec3,
    pub text: String,
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

pub fn draw(painter: &Painter, rect: Rect, camera: &Camera, overlay: &Overlay) {
    if let Some(origin) = overlay.global_origin {
        let center = project(camera, rect, origin);
        if rect.contains(center) {
            triad(painter, camera, center, 36.0, false);
        }
    }
    if let Some(marker) = &overlay.maximum {
        draw_marker(
            painter,
            rect,
            project(camera, rect, marker.position),
            &marker.text,
        );
    }
    if let Some(legend) = &overlay.legend {
        draw_legend(painter, rect, legend);
    }
    if !overlay.status.is_empty() {
        draw_status(painter, rect, &overlay.status);
    }
    if overlay.global_origin.is_some() {
        draw_scale_bar(painter, rect, camera);
    }
    let corner = rect.right_bottom() - vec2(24.0 + 45.0, 24.0 + 45.0);
    triad(painter, camera, corner, 45.0, true);
}

/// Title lines, then one box per band with the band limits as labels, maximum on top.
fn draw_legend(painter: &Painter, rect: Rect, legend: &Legend) {
    let left = rect.left() + MARGIN;
    let mut top = rect.top() + MARGIN;
    for line in legend.title.lines() {
        let r = painter.text(pos2(left, top), Align2::LEFT_TOP, line, font(), TEXT);
        top = r.bottom() + 2.0;
    }
    top += 6.0;
    let levels = legend.levels;
    let band = vec2(BAND_HEIGHT / 0.75, BAND_HEIGHT);
    for i in 0..levels {
        let [r, g, b] = band_color(levels - 1 - i, levels).map(|c| (c * 255.0).round() as u8);
        let cell = Rect::from_min_size(pos2(left, top + i as f32 * band.y), band);
        painter.rect(
            cell,
            0.0,
            Color32::from_rgb(r, g, b),
            Stroke::new(1.0, TEXT),
            StrokeKind::Middle,
        );
    }
    for i in 0..=levels {
        let t = 1.0 - i as f32 / levels as f32;
        let value = legend.min + t * (legend.max - legend.min);
        painter.text(
            pos2(left + band.x + 8.0, top + i as f32 * band.y),
            Align2::LEFT_CENTER,
            format_legend_value(value),
            font(),
            TEXT,
        );
    }
}

/// Framed text block in the top right corner.
fn draw_status(painter: &Painter, rect: Rect, lines: &[String]) {
    let galleys: Vec<_> = lines
        .iter()
        .map(|l| painter.layout_no_wrap(l.clone(), font(), TEXT))
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let line_height = galleys.first().map_or(0.0, |g| g.size().y) + 2.0;
    let padding = 5.0;
    let size = vec2(width, line_height * galleys.len() as f32) + vec2(2.0 * padding, padding);
    let frame = Rect::from_min_size(
        pos2(rect.right() - MARGIN - size.x, rect.top() + MARGIN),
        size,
    );
    painter.rect_stroke(frame, 0.0, Stroke::new(1.0, TEXT), StrokeKind::Inside);
    for (i, galley) in galleys.into_iter().enumerate() {
        let pos = frame.min + vec2(padding, padding * 0.5 + i as f32 * line_height);
        painter.galley(pos, galley, TEXT);
    }
}

/// Bar of five alternating fields whose total length is a round number of model units.
fn draw_scale_bar(painter: &Painter, rect: Rect, camera: &Camera) {
    let world_per_point = camera.pixel_size(rect.width(), rect.height());
    let Some((length, width)) = scale_bar_length(world_per_point) else {
        return;
    };
    let left = rect.center().x - width * 0.5;
    let top = rect.bottom() - MARGIN - SCALE_BAR_HEIGHT;
    let field = width / SCALE_BAR_FIELDS as f32;
    for i in 0..SCALE_BAR_FIELDS {
        let cell = Rect::from_min_size(
            pos2(left + i as f32 * field, top),
            vec2(field, SCALE_BAR_HEIGHT),
        );
        let fill = if i % 2 == 0 { TEXT } else { Color32::WHITE };
        painter.rect(cell, 0.0, fill, Stroke::new(1.0, TEXT), StrokeKind::Middle);
    }
    for i in 0..=SCALE_BAR_FIELDS {
        let value = length * i as f32 / SCALE_BAR_FIELDS as f32;
        painter.text(
            pos2(left + i as f32 * field, top - 3.0),
            Align2::CENTER_BOTTOM,
            format_value(value),
            font(),
            TEXT,
        );
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

/// Framed label next to a model point, with a line pointing at it.
fn draw_marker(painter: &Painter, rect: Rect, point: Pos2, text: &str) {
    if !rect.contains(point) {
        return;
    }
    let galley = painter.layout_no_wrap(text.to_string(), font(), TEXT);
    let padding = vec2(6.0, 3.0);
    let size = galley.size() + 2.0 * padding;
    // Above right of the point, or mirrored where the view ends.
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
    let frame = Rect::from_min_size(point + vec2(dx, dy), size);
    let anchor = pos2(
        point.x.clamp(frame.left(), frame.right()),
        point.y.clamp(frame.top(), frame.bottom()),
    );
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
    painter.rect(
        frame,
        0.0,
        Color32::WHITE,
        Stroke::new(1.0, TEXT),
        StrokeKind::Inside,
    );
    painter.galley(frame.min + padding, galley, TEXT);
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
