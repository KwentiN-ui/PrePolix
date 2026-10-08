use egui::{Color32, PointerButton, Rect, Sense, Ui, pos2};
use egui_wgpu::RenderState;
use glam::Vec3;
use plx_render::wgpu::FilterMode;
use plx_render::{Camera, RenderMesh, ViewportRenderer};

const ZOOM_PER_SCROLL_POINT: f32 = 0.002;

/// The 3D view: owns the GPU renderer and the camera and maps mouse input to camera moves.
///
/// Left drag rotates, right or middle drag pans, the wheel zooms.
pub struct Viewport {
    render_state: RenderState,
    renderer: ViewportRenderer,
    texture: egui::TextureId,
    camera: Camera,
    bounds: (Vec3, Vec3),
}

impl Viewport {
    pub fn new(render_state: RenderState) -> Self {
        let mut renderer = ViewportRenderer::new(&render_state.device);
        let mesh = RenderMesh::demo_box(Vec3::new(2.0, 1.0, 0.5), [0.55, 0.70, 0.85]);
        renderer.set_mesh(&render_state.device, &mesh);
        let bounds = mesh
            .bounds()
            .unwrap_or((Vec3::splat(-1.0), Vec3::splat(1.0)));
        let mut camera = Camera::default();
        camera.fit(bounds.0, bounds.1);
        let texture = render_state.renderer.write().register_native_texture(
            &render_state.device,
            renderer.color_view(),
            FilterMode::Linear,
        );
        Self {
            render_state,
            renderer,
            texture,
            camera,
            bounds,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button("Einpassen").clicked() {
                self.camera.fit(self.bounds.0, self.bounds.1);
            }
            if ui.button("Isometrisch").clicked() {
                self.camera.set_isometric();
            }
            if ui.button("Vorne").clicked() {
                self.camera.set_front();
            }
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
            self.camera.fit(self.bounds.0, self.bounds.1);
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
            .render(device, &self.render_state.queue, &self.camera);
        ui.painter().image(
            self.texture,
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}
