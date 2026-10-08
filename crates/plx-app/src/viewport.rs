use egui::{Color32, PointerButton, Rect, Sense, Ui, pos2};
use egui_wgpu::RenderState;
use glam::Vec3;
use plx_render::contour::band_color;
use plx_render::wgpu::FilterMode;
use plx_render::{Camera, DisplayOptions, RenderMesh, ViewportRenderer};

use crate::results::{Legend, format_legend_value};

const ZOOM_PER_SCROLL_POINT: f32 = 0.002;

/// The 3D view: owns the GPU renderer and the camera and maps mouse input to camera moves.
///
/// Left drag rotates, right or middle drag pans, the wheel zooms.
pub struct Viewport {
    render_state: RenderState,
    renderer: ViewportRenderer,
    texture: egui::TextureId,
    camera: Camera,
    pub options: DisplayOptions,
    /// Colour legend drawn over the scene while a result is shown.
    pub legend: Option<Legend>,
}

/// Camera requests from toolbar, menu or tree, applied by the owner of the model bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewCommand {
    Fit,
    Isometric,
    Front,
}

impl Viewport {
    pub fn new(render_state: RenderState) -> Self {
        let renderer = ViewportRenderer::new(&render_state.device);
        let texture = render_state.renderer.write().register_native_texture(
            &render_state.device,
            renderer.color_view(),
            FilterMode::Linear,
        );
        Self {
            render_state,
            renderer,
            texture,
            camera: Camera::default(),
            options: DisplayOptions::default(),
            legend: None,
        }
    }

    pub fn set_parts(&mut self, parts: &[RenderMesh]) {
        self.renderer.set_parts(&self.render_state.device, parts);
    }

    pub fn set_part_visible(&mut self, index: usize, visible: bool) {
        self.renderer.set_part_visible(index, visible);
    }

    pub fn apply(&mut self, command: ViewCommand, bounds: Option<(Vec3, Vec3)>) {
        match command {
            ViewCommand::Fit => {
                if let Some((min, max)) = bounds {
                    self.camera.fit(min, max);
                }
            }
            ViewCommand::Isometric => self.camera.set_isometric(),
            ViewCommand::Front => self.camera.set_front(),
        }
    }

    /// Draws the toolbar and the scene; returns a camera command the user asked for.
    pub fn ui(&mut self, ui: &mut Ui) -> Option<ViewCommand> {
        let mut command = None;
        ui.horizontal(|ui| {
            if ui.button("Einpassen").clicked() {
                command = Some(ViewCommand::Fit);
            }
            if ui.button("Isometrisch").clicked() {
                command = Some(ViewCommand::Isometric);
            }
            if ui.button("Vorne").clicked() {
                command = Some(ViewCommand::Front);
            }
            ui.separator();
            ui.checkbox(&mut self.options.mesh_edges, "Netz");
        });

        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let delta = response.drag_delta();
        if response.dragged_by(PointerButton::Primary) {
            self.camera.orbit(delta.x, delta.y);
        } else if response.dragged_by(PointerButton::Secondary)
            || response.dragged_by(PointerButton::Middle)
        {
            self.camera
                .pan(delta.x, delta.y, rect.width(), rect.height());
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom((-scroll * ZOOM_PER_SCROLL_POINT).exp());
            }
        }
        if response.double_clicked() {
            command = Some(ViewCommand::Fit);
        }

        let pixels_per_point = ui.ctx().pixels_per_point();
        let width = (rect.width() * pixels_per_point).round() as u32;
        let height = (rect.height() * pixels_per_point).round() as u32;
        let device = &self.render_state.device;
        if self.renderer.resize(device, width, height) {
            self.render_state
                .renderer
                .write()
                .update_egui_texture_from_wgpu_texture(
                    device,
                    self.renderer.color_view(),
                    FilterMode::Linear,
                    self.texture,
                );
        }
        self.renderer
            .render(device, &self.render_state.queue, &self.camera, self.options);
        ui.painter().image(
            self.texture,
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        if let (Some(legend), Some(_)) = (&self.legend, self.options.contour_levels) {
            draw_legend(ui, rect, legend);
        }
        command
    }
}

const LEGEND_MARGIN: f32 = 12.0;
const LEGEND_BAND: egui::Vec2 = egui::vec2(24.0, 18.0);

/// PrePoMax-style legend in the top left corner: title, then one box per band with the
/// band limits as labels, maximum on top.
fn draw_legend(ui: &Ui, rect: Rect, legend: &Legend) {
    let painter = ui.painter_at(rect);
    let font = egui::FontId::proportional(13.0);
    let text_color = Color32::BLACK;
    let mut top = rect.top() + LEGEND_MARGIN;
    let left = rect.left() + LEGEND_MARGIN;
    let title = painter.text(
        pos2(left, top),
        egui::Align2::LEFT_TOP,
        &legend.title,
        font.clone(),
        text_color,
    );
    top = title.bottom() + 6.0;
    let levels = legend.levels;
    for i in 0..levels {
        let band = levels - 1 - i;
        let [r, g, b] = band_color(band, levels).map(|c| (c * 255.0).round() as u8);
        let cell = Rect::from_min_size(pos2(left, top + i as f32 * LEGEND_BAND.y), LEGEND_BAND);
        painter.rect_filled(cell, 0.0, Color32::from_rgb(r, g, b));
        painter.rect_stroke(
            cell,
            0.0,
            egui::Stroke::new(1.0, Color32::from_gray(60)),
            egui::StrokeKind::Inside,
        );
    }
    for i in 0..=levels {
        let t = 1.0 - i as f32 / levels as f32;
        let value = legend.min + t * (legend.max - legend.min);
        painter.text(
            pos2(left + LEGEND_BAND.x + 6.0, top + i as f32 * LEGEND_BAND.y),
            egui::Align2::LEFT_CENTER,
            format_legend_value(value),
            font.clone(),
            text_color,
        );
    }
}
