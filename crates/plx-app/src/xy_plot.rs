//! Diagram of curves over a common x axis, PrePoMax's diagram view of a table selection: a
//! curve per y column, a legend, grid lines at round numbers and the value of the nearest
//! point under the mouse.

use egui::{Color32, Pos2, Rect, Stroke, Ui, pos2, vec2};

use crate::results::format_value;

/// Data of a diagram: x values and the y values of each named curve, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct XyData {
    pub title: String,
    pub x_label: String,
    pub x: Vec<f64>,
    pub curves: Vec<(String, Vec<f64>)>,
}

/// Distinct colours for the curves, in the order of the columns.
const COLORS: [Color32; 8] = [
    Color32::from_rgb(31, 119, 180),
    Color32::from_rgb(214, 39, 40),
    Color32::from_rgb(44, 160, 44),
    Color32::from_rgb(255, 127, 14),
    Color32::from_rgb(148, 103, 189),
    Color32::from_rgb(140, 86, 75),
    Color32::from_rgb(227, 119, 194),
    Color32::from_rgb(23, 190, 207),
];

pub fn color(index: usize) -> Color32 {
    COLORS[index % COLORS.len()]
}

/// Grid values at round steps (1, 2 or 5 times a power of ten) covering `min..=max`.
pub fn ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite()) || max <= min {
        return vec![min];
    }
    let raw = (max - min) / count.max(1) as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|f| f * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * magnitude);
    let first = (min / step).ceil() as i64;
    let last = (max / step).floor() as i64;
    (first..=last).map(|k| k as f64 * step).collect()
}

/// Range of the finite values with a margin; a single value gets a range around it.
fn range(values: impl Iterator<Item = f64>) -> Option<(f64, f64)> {
    let (min, max) = values
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(v), b.max(v))
        });
    if !(min.is_finite() && max.is_finite()) {
        return None;
    }
    if max - min <= 1e-12 * min.abs().max(max.abs()).max(1e-30) {
        let pad = min.abs().max(1.0) * 0.05;
        return Some((min - pad, max + pad));
    }
    Some((min, max))
}

/// Draws the diagram into the available space of `ui`.
pub fn show(ui: &mut Ui, data: &XyData) {
    let size = ui.available_size().max(vec2(360.0, 220.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let font = egui::FontId::proportional(12.0);
    let text = Color32::from_gray(40);
    let (Some((x0, x1)), Some((y0, y1))) = (
        range(data.x.iter().copied()),
        range(data.curves.iter().flat_map(|(_, y)| y.iter().copied())),
    ) else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No plottable values",
            font,
            text,
        );
        return;
    };
    let area = Rect::from_min_max(rect.min + vec2(72.0, 12.0), rect.max - vec2(16.0, 40.0));
    let to_screen = |x: f64, y: f64| {
        pos2(
            area.left() + ((x - x0) / (x1 - x0)) as f32 * area.width(),
            area.bottom() - ((y - y0) / (y1 - y0)) as f32 * area.height(),
        )
    };
    let grid = Stroke::new(1.0, Color32::from_gray(225));
    let x_count = (area.width() / 90.0).max(2.0) as usize;
    let y_count = (area.height() / 50.0).max(2.0) as usize;
    for y in ticks(y0, y1, y_count) {
        let p = to_screen(x0, y);
        painter.hline(area.x_range(), p.y, grid);
        painter.text(
            pos2(area.left() - 6.0, p.y),
            egui::Align2::RIGHT_CENTER,
            format_value(y as f32),
            font.clone(),
            text,
        );
    }
    for x in ticks(x0, x1, x_count) {
        let p = to_screen(x, y0);
        painter.vline(p.x, area.y_range(), grid);
        painter.text(
            pos2(p.x, area.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            format_value(x as f32),
            font.clone(),
            text,
        );
    }
    painter.rect_stroke(
        area,
        0.0,
        Stroke::new(1.0, Color32::from_gray(80)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        pos2(area.center().x, rect.bottom() - 4.0),
        egui::Align2::CENTER_BOTTOM,
        &data.x_label,
        font.clone(),
        text,
    );
    let markers = data.x.len() <= 60;
    for (index, (_, values)) in data.curves.iter().enumerate() {
        let color = color(index);
        // Rows without a value break the curve.
        let mut segment: Vec<Pos2> = Vec::new();
        let flush = |segment: &mut Vec<Pos2>| {
            if segment.len() > 1 {
                painter.add(egui::Shape::line(
                    std::mem::take(segment),
                    Stroke::new(2.0, color),
                ));
            }
            segment.clear();
        };
        for (&x, &y) in data.x.iter().zip(values) {
            if x.is_finite() && y.is_finite() {
                let p = to_screen(x, y);
                segment.push(p);
                if markers {
                    painter.circle_filled(p, 2.5, color);
                }
            } else {
                flush(&mut segment);
            }
        }
        flush(&mut segment);
    }
    legend(&painter, area, data, &font);
    if let Some(hover) = response.hover_pos().filter(|p| area.contains(*p)) {
        let nearest = (data.curves.iter().enumerate())
            .flat_map(|(c, (_, values))| {
                (data.x.iter().zip(values))
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .map(move |(&x, &y)| (c, x, y))
            })
            .min_by(|a, b| {
                let d = |x: f64, y: f64| to_screen(x, y).distance_sq(hover);
                d(a.1, a.2).total_cmp(&d(b.1, b.2))
            });
        if let Some((curve, x, y)) = nearest {
            let p = to_screen(x, y);
            painter.circle_stroke(p, 4.5, Stroke::new(1.5, Color32::BLACK));
            let label = format!(
                "{}\n{}: {}\n{}",
                data.curves[curve].0,
                data.x_label,
                format_value(x as f32),
                format_value(y as f32)
            );
            let galley = painter.layout_no_wrap(label, font, Color32::BLACK);
            let mut at = p + vec2(8.0, -8.0 - galley.size().y);
            at.x = at.x.min(area.right() - galley.size().x - 4.0);
            at.y = at.y.max(area.top() + 2.0);
            let frame = Rect::from_min_size(at, galley.size()).expand(3.0);
            painter.rect_filled(frame, 2.0, Color32::from_white_alpha(230));
            painter.rect_stroke(
                frame,
                2.0,
                Stroke::new(1.0, Color32::from_gray(150)),
                egui::StrokeKind::Inside,
            );
            painter.galley(at, galley, Color32::BLACK);
        }
    }
}

/// Names of the curves in the upper right corner of the plot area.
fn legend(painter: &egui::Painter, area: Rect, data: &XyData, font: &egui::FontId) {
    let shown = data.curves.len().min(12);
    if shown == 0 {
        return;
    }
    let galleys: Vec<_> = (data.curves.iter().take(shown))
        .map(|(name, _)| painter.layout_no_wrap(name.clone(), font.clone(), Color32::BLACK))
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 30.0;
    let line = 16.0;
    let extra = (data.curves.len() > shown) as usize;
    let height = line * (shown + extra) as f32 + 6.0;
    let frame = Rect::from_min_size(
        pos2(area.right() - width - 8.0, area.top() + 8.0),
        vec2(width, height),
    );
    painter.rect_filled(frame, 2.0, Color32::from_white_alpha(230));
    painter.rect_stroke(
        frame,
        2.0,
        Stroke::new(1.0, Color32::from_gray(160)),
        egui::StrokeKind::Inside,
    );
    for (index, galley) in galleys.into_iter().enumerate() {
        let y = frame.top() + 3.0 + line * index as f32 + line / 2.0;
        painter.line_segment(
            [pos2(frame.left() + 5.0, y), pos2(frame.left() + 21.0, y)],
            Stroke::new(2.0, color(index)),
        );
        painter.galley(
            pos2(frame.left() + 26.0, y - galley.size().y / 2.0),
            galley,
            Color32::BLACK,
        );
    }
    if extra > 0 {
        let y = frame.top() + 3.0 + line * shown as f32 + line / 2.0;
        painter.text(
            pos2(frame.left() + 26.0, y),
            egui::Align2::LEFT_CENTER,
            format!("… {} more", data.curves.len() - shown),
            font.clone(),
            Color32::from_gray(80),
        );
    }
}

/// The diagram in its own window, PrePoMax's diagram view; returns false when it is closed.
pub fn window(ctx: &egui::Context, data: &XyData) -> bool {
    let mut open = true;
    egui::Window::new(format!("Plot: {}", data.title))
        .id(egui::Id::new("history plot"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([640.0, 420.0])
        .pivot(egui::Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + vec2(-24.0, 96.0))
        .min_size([360.0, 240.0])
        .show(ctx, |ui| show(ui, data));
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_fall_on_round_numbers() {
        assert_eq!(
            ticks(0.0, 1.0, 5),
            [0.0, 0.2, 0.4, 0.6000000000000001, 0.8, 1.0]
        );
        assert_eq!(ticks(-3.0, 17.0, 4), [0.0, 5.0, 10.0, 15.0]);
        assert_eq!(ticks(2.0, 2.0, 4), [2.0]);
    }

    #[test]
    fn a_constant_curve_gets_a_range() {
        let (min, max) = range([5.0, 5.0, f64::NAN].into_iter()).unwrap();
        assert!(min < 5.0 && max > 5.0);
        assert!(range([f64::NAN].into_iter()).is_none());
    }
}
