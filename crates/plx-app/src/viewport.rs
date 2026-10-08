use egui::{Color32, PointerButton, Rect, Sense, Ui, pos2};
use egui_wgpu::RenderState;
use glam::Vec3;
use plx_render::wgpu::FilterMode;
use plx_render::{Camera, DisplayOptions, RenderMesh, StandardView, ViewportRenderer};

use crate::overlay::{self, Overlay};

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
    /// Legend, information block and markers drawn over the scene.
    pub overlay: Overlay,
}

/// Camera requests from toolbar, menu or tree, applied by the owner of the model bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewCommand {
    Fit,
    View(StandardView),
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
            overlay: Overlay::default(),
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
            ViewCommand::View(view) => self.camera.set_view(view),
        }
    }

    /// Draws the scene with its annotations; returns a camera command the user asked for.
    pub fn ui(&mut self, ui: &mut Ui) -> Option<ViewCommand> {
        let mut command = None;
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
        let painter = ui.painter_at(rect);
        painter.image(
            self.texture,
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        overlay::draw(&painter, rect, &self.camera, &self.overlay);
        command
    }
}
