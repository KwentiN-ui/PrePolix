//! Toolbar icons drawn with the egui painter, so that no icon font or emoji is needed.

use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use plx_render::StandardView as ViewIcon;

use crate::style::{HIGHLIGHT, HOVER_FILL, PRESSED_FILL};

const BUTTON: Vec2 = vec2(24.0, 22.0);
const ICON: f32 = 16.0;
const OUTLINE: Color32 = Color32::from_rgb(70, 70, 70);
const FACE: Color32 = Color32::from_rgb(232, 232, 232);
const ACCENT: Color32 = Color32::from_rgb(70, 140, 215);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    New,
    Open,
    Save,
    Fit,
    /// Standard view; the cube face looked at is highlighted.
    View(ViewIcon),
    /// Vertical view: an arrow standing on the ground line.
    Vertical,
    FeatureEdges,
    MeshEdges,
    First,
    Previous,
    Next,
    Last,
    /// Play triangle; also opens the animation.
    Animate,
    Pause,
    /// Arrow of the material library's copy and move buttons, pointing in this direction
    /// (unit vector, y down).
    Arrow(Vec2),
    /// Camera: screenshot of the 3D view.
    Screenshot,
}

/// Flat toolbar button in the Windows style: frame only while hovered or checked.
pub fn button(ui: &mut Ui, icon: Icon, tooltip: &str, enabled: bool, checked: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(BUTTON, sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = enabled && response.hovered();
        if checked || hovered {
            let fill = if checked || response.is_pointer_button_down_on() {
                PRESSED_FILL
            } else {
                HOVER_FILL
            };
            painter.rect(
                rect.shrink(1.0),
                0.0,
                fill,
                Stroke::new(1.0, HIGHLIGHT),
                egui::StrokeKind::Inside,
            );
        }
        let area = Rect::from_center_size(rect.center(), Vec2::splat(ICON));
        let mut shapes = Vec::new();
        paint(&mut shapes, icon, area);
        if !enabled {
            for shape in &mut shapes {
                fade(shape);
            }
        }
        painter.extend(shapes);
    }
    response.on_hover_text(tooltip)
}

/// Dialog button with a frame like a text button, showing an icon.
pub fn dialog_button(ui: &mut Ui, icon: Icon, tooltip: &str, enabled: bool) -> Response {
    let response = ui
        .add_enabled(enabled, egui::Button::new("").min_size(vec2(26.0, 24.0)))
        .on_hover_text(tooltip);
    if ui.is_rect_visible(response.rect) {
        let area = Rect::from_center_size(response.rect.center(), Vec2::splat(ICON));
        let mut shapes = Vec::new();
        paint(&mut shapes, icon, area);
        if !enabled {
            for shape in &mut shapes {
                fade(shape);
            }
        }
        ui.painter().extend(shapes);
    }
    response
}

fn fade(shape: &mut Shape) {
    let grey = |c: Color32| {
        let l = ((c.r() as u32 + c.g() as u32 + c.b() as u32) / 3) as u8;
        Color32::from_rgba_unmultiplied(l, l, l, c.a() / 2)
    };
    match shape {
        Shape::Path(path) => {
            path.fill = grey(path.fill);
            if let egui::epaint::PathStroke {
                color: egui::epaint::ColorMode::Solid(c),
                ..
            } = &mut path.stroke
            {
                *c = grey(*c);
            }
        }
        Shape::LineSegment { stroke, .. } => stroke.color = grey(stroke.color),
        Shape::Rect(r) => {
            r.fill = grey(r.fill);
            r.stroke.color = grey(r.stroke.color);
        }
        _ => {}
    }
}

fn line(shapes: &mut Vec<Shape>, points: &[Pos2], width: f32, color: Color32) {
    shapes.push(Shape::line(points.to_vec(), Stroke::new(width, color)));
}

fn polygon(shapes: &mut Vec<Shape>, points: Vec<Pos2>, fill: Color32, stroke: Color32) {
    shapes.push(Shape::convex_polygon(
        points,
        fill,
        Stroke::new(1.0, stroke),
    ));
}

fn paint(shapes: &mut Vec<Shape>, icon: Icon, r: Rect) {
    // Coordinates on a 16 × 16 grid.
    let p = |x: f32, y: f32| pos2(r.left() + x, r.top() + y);
    match icon {
        Icon::Vertical => {
            line(shapes, &[p(2.5, 14.0), p(13.5, 14.0)], 1.5, OUTLINE);
            line(shapes, &[p(8.0, 13.0), p(8.0, 5.0)], 2.0, ACCENT);
            polygon(
                shapes,
                vec![p(8.0, 1.0), p(11.5, 6.0), p(4.5, 6.0)],
                ACCENT,
                ACCENT,
            );
        }
        Icon::New => {
            polygon(
                shapes,
                vec![
                    p(3.5, 1.5),
                    p(10.0, 1.5),
                    p(13.5, 5.0),
                    p(13.5, 14.5),
                    p(3.5, 14.5),
                ],
                Color32::WHITE,
                OUTLINE,
            );
            line(
                shapes,
                &[p(10.0, 1.5), p(10.0, 5.0), p(13.5, 5.0)],
                1.0,
                OUTLINE,
            );
        }
        Icon::Open => {
            let back = Color32::from_rgb(230, 170, 40);
            let front = Color32::from_rgb(255, 206, 84);
            polygon(
                shapes,
                vec![
                    p(1.0, 3.0),
                    p(6.0, 3.0),
                    p(7.5, 4.5),
                    p(14.0, 4.5),
                    p(14.0, 13.5),
                    p(1.0, 13.5),
                ],
                back,
                Color32::from_rgb(170, 120, 20),
            );
            polygon(
                shapes,
                vec![p(3.5, 7.0), p(15.5, 7.0), p(13.5, 13.5), p(1.0, 13.5)],
                front,
                Color32::from_rgb(170, 120, 20),
            );
        }
        Icon::Save => {
            let blue = Color32::from_rgb(60, 110, 180);
            polygon(
                shapes,
                vec![
                    p(1.5, 1.5),
                    p(12.5, 1.5),
                    p(14.5, 3.5),
                    p(14.5, 14.5),
                    p(1.5, 14.5),
                ],
                blue,
                Color32::from_rgb(30, 60, 110),
            );
            shapes.push(Shape::rect_filled(
                Rect::from_min_max(p(4.0, 1.5), p(11.0, 6.0)),
                0.0,
                Color32::WHITE,
            ));
            shapes.push(Shape::rect_filled(
                Rect::from_min_max(p(8.5, 2.5), p(10.0, 5.0)),
                0.0,
                blue,
            ));
            shapes.push(Shape::rect_filled(
                Rect::from_min_max(p(3.5, 9.0), p(12.5, 14.5)),
                0.0,
                Color32::from_rgb(235, 235, 235),
            ));
        }
        Icon::Fit => {
            shapes.push(Shape::rect_filled(
                Rect::from_min_max(p(5.0, 5.0), p(11.0, 11.0)),
                0.0,
                ACCENT,
            ));
            for (corner, dx, dy) in [
                (p(1.5, 1.5), 1.0, 1.0),
                (p(14.5, 1.5), -1.0, 1.0),
                (p(1.5, 14.5), 1.0, -1.0),
                (p(14.5, 14.5), -1.0, -1.0),
            ] {
                line(
                    shapes,
                    &[
                        corner + vec2(4.0 * dx, 0.0),
                        corner,
                        corner + vec2(0.0, 4.0 * dy),
                    ],
                    1.5,
                    OUTLINE,
                );
            }
        }
        Icon::View(view) => cube(shapes, r, Some(view), false),
        Icon::FeatureEdges => cube(shapes, r, None, false),
        Icon::MeshEdges => cube(shapes, r, None, true),
        Icon::Animate => {
            let green = Color32::from_rgb(40, 150, 60);
            polygon(
                shapes,
                vec![p(4.0, 2.0), p(14.0, 8.0), p(4.0, 14.0)],
                green,
                Color32::from_rgb(20, 100, 35),
            );
        }
        Icon::Pause => {
            let color = Color32::from_rgb(50, 50, 50);
            for x in [4.0, 9.5] {
                shapes.push(Shape::rect_filled(
                    Rect::from_min_max(p(x, 3.0), p(x + 3.0, 13.0)),
                    0.0,
                    color,
                ));
            }
        }
        Icon::Arrow(direction) => {
            // Drawn pointing right, then turned into the direction.
            let c = r.center();
            let normal = direction.rot90();
            let at = |x: f32, y: f32| c + direction * x + normal * y;
            let color = Color32::from_rgb(50, 50, 50);
            polygon(
                shapes,
                vec![at(-6.0, -1.8), at(0.5, -1.8), at(0.5, 1.8), at(-6.0, 1.8)],
                color,
                color,
            );
            polygon(
                shapes,
                vec![at(0.0, -5.0), at(6.0, 0.0), at(0.0, 5.0)],
                color,
                color,
            );
        }
        Icon::Screenshot => {
            let body = Color32::from_rgb(85, 85, 90);
            polygon(
                shapes,
                vec![p(5.0, 4.5), p(6.5, 2.5), p(10.5, 2.5), p(12.0, 4.5)],
                body,
                body,
            );
            shapes.push(Shape::rect_filled(
                Rect::from_min_max(p(1.0, 4.0), p(15.0, 14.0)),
                1.5,
                body,
            ));
            shapes.push(Shape::rect_filled(
                Rect::from_min_max(p(2.5, 5.5), p(4.5, 6.5)),
                0.0,
                Color32::from_rgb(255, 200, 60),
            ));
            shapes.push(Shape::circle_filled(p(8.5, 9.0), 3.8, Color32::WHITE));
            shapes.push(Shape::circle_filled(p(8.5, 9.0), 2.6, ACCENT));
            shapes.push(Shape::circle_filled(p(7.6, 8.1), 0.8, Color32::WHITE));
        }
        Icon::First | Icon::Previous | Icon::Next | Icon::Last => {
            let forward = matches!(icon, Icon::Next | Icon::Last);
            let s = if forward { 1.0 } else { -1.0 };
            let c = r.center();
            let tri = |x0: f32| {
                vec![
                    c + vec2(s * (x0 - 3.5), -5.0),
                    c + vec2(s * (x0 + 3.5), 0.0),
                    c + vec2(s * (x0 - 3.5), 5.0),
                ]
            };
            let color = Color32::from_rgb(50, 50, 50);
            let double = matches!(icon, Icon::First | Icon::Last);
            if double {
                polygon(shapes, tri(-1.5), color, color);
                shapes.push(Shape::rect_filled(
                    Rect::from_center_size(c + vec2(s * 4.5, 0.0), vec2(2.0, 10.0)),
                    0.0,
                    color,
                ));
            } else {
                polygon(shapes, tri(0.0), color, color);
            }
        }
    }
}

/// Isometric unit cube; x to the lower right, z to the lower left, y up.
fn cube(shapes: &mut Vec<Shape>, r: Rect, view: Option<ViewIcon>, grid: bool) {
    let c = r.center() + vec2(0.0, 0.5);
    let s = 6.2;
    let project = |[x, y, z]: [f32; 3]| {
        // Centre the cube on the origin of the icon.
        let (x, y, z) = (x - 0.5, y - 0.5, z - 0.5);
        c + vec2((x - z) * 0.866 * s * 1.25, ((x + z) * 0.5 - y) * s * 1.25)
    };
    let corner = |i: usize| [(i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32];
    // Faces as corner indices (bits: x = 1, y = 2, z = 4), visible ones last.
    let faces: [(ViewIcon, [usize; 4], bool); 6] = [
        (ViewIcon::Back, [0, 1, 3, 2], false),
        (ViewIcon::Bottom, [0, 1, 5, 4], false),
        (ViewIcon::Left, [0, 2, 6, 4], false),
        (ViewIcon::Front, [4, 5, 7, 6], true),
        (ViewIcon::Top, [2, 3, 7, 6], true),
        (ViewIcon::Right, [1, 3, 7, 5], true),
    ];
    // A highlighted back face is seen through a wireframe of the front faces.
    let see_through = faces
        .iter()
        .any(|(face, _, visible)| !visible && view == Some(*face));
    let hidden_edge = Color32::from_rgb(150, 150, 150);
    for (face, corners, visible) in faces {
        let points: Vec<Pos2> = corners.iter().map(|&i| project(corner(i))).collect();
        let highlighted = view == Some(face) || (view == Some(ViewIcon::Isometric) && visible);
        if highlighted {
            polygon(shapes, points.clone(), ACCENT, Color32::TRANSPARENT);
        } else if visible && !see_through {
            polygon(shapes, points.clone(), FACE, Color32::TRANSPARENT);
        }
        let mut closed = points.clone();
        closed.push(points[0]);
        if visible {
            line(shapes, &closed, 1.0, OUTLINE);
        } else if highlighted {
            line(shapes, &closed, 1.0, hidden_edge);
        }
        if grid && visible {
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                let lerp = |a: Pos2, b: Pos2| a + (b - a) * t;
                line(
                    shapes,
                    &[lerp(points[0], points[1]), lerp(points[3], points[2])],
                    0.7,
                    OUTLINE,
                );
                line(
                    shapes,
                    &[lerp(points[0], points[3]), lerp(points[1], points[2])],
                    0.7,
                    OUTLINE,
                );
            }
        }
    }
}
