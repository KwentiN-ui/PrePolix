use glam::camera::rh::{proj::directx, view::look_at_mat4};
use glam::{Mat4, Quat, Vec3};

/// Orthographic camera orbiting a target point, as used in CAE viewports.
///
/// `rotation` maps camera space (x right, y up, looking along -z) to world space.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub rotation: Quat,
    pub scene_radius: f32,
}

const ORBIT_RADIANS_PER_PIXEL: f32 = 0.008;
const MIN_HALF_EXTENT: f32 = 1e-6;

impl Default for Camera {
    fn default() -> Self {
        let mut camera = Self {
            target: Vec3::ZERO,
            distance: 1.0,
            rotation: Quat::IDENTITY,
            scene_radius: 1.0,
        };
        camera.set_isometric();
        camera
    }
}

impl Camera {
    pub fn eye(&self) -> Vec3 {
        self.target + self.rotation * Vec3::Z * self.distance
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    /// Half of the visible extent along the shorter viewport side, in world units.
    pub fn half_extent(&self) -> f32 {
        (self.distance * 0.5).max(MIN_HALF_EXTENT)
    }

    /// Direction the light travels in world space: a headlight slightly above and left of the eye.
    pub fn light_direction(&self) -> Vec3 {
        self.rotation * Vec3::new(0.25, -0.35, -1.0).normalize()
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let aspect = aspect.max(1e-6);
        let (half_width, half_height) = if aspect >= 1.0 {
            (self.half_extent() * aspect, self.half_extent())
        } else {
            (self.half_extent(), self.half_extent() / aspect)
        };
        let depth = self.scene_radius.max(self.distance) * 4.0;
        let view = look_at_mat4(self.eye(), self.target, self.up());
        let proj = directx::orthographic(
            -half_width,
            half_width,
            -half_height,
            half_height,
            self.distance - depth,
            self.distance + depth,
        );
        proj * view
    }

    /// Rotates the view around the target by a mouse drag in pixels.
    pub fn orbit(&mut self, dx_px: f32, dy_px: f32) {
        let yaw = Quat::from_axis_angle(self.up(), -dx_px * ORBIT_RADIANS_PER_PIXEL);
        let pitch = Quat::from_axis_angle(self.right(), -dy_px * ORBIT_RADIANS_PER_PIXEL);
        self.rotation = (yaw * pitch * self.rotation).normalize();
    }

    /// Moves the target so that the scene follows a mouse drag in pixels.
    pub fn pan(&mut self, dx_px: f32, dy_px: f32, viewport_width_px: f32, viewport_height_px: f32) {
        let shorter_side_px = viewport_width_px.min(viewport_height_px).max(1.0);
        let world_per_pixel = 2.0 * self.half_extent() / shorter_side_px;
        self.target -= self.right() * dx_px * world_per_pixel;
        self.target += self.up() * dy_px * world_per_pixel;
    }

    /// Scales the visible region; factors below 1 zoom in.
    pub fn zoom(&mut self, factor: f32) {
        self.distance =
            (self.distance * factor).clamp(self.scene_radius * 1e-4, self.scene_radius * 1e3);
    }

    /// Centers the view on a bounding box and makes it fill the viewport.
    pub fn fit(&mut self, min: Vec3, max: Vec3) {
        self.target = (min + max) * 0.5;
        self.scene_radius = ((max - min).length() * 0.5).max(MIN_HALF_EXTENT);
        self.distance = self.scene_radius * 2.2;
    }

    pub fn set_front(&mut self) {
        self.rotation = Quat::IDENTITY;
    }

    pub fn set_isometric(&mut self) {
        let yaw = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let pitch = Quat::from_rotation_x(-(1.0_f32 / 2.0_f32.sqrt()).atan());
        self.rotation = yaw * pitch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn project(camera: &Camera, point: Vec3) -> Vec3 {
        project_with_aspect(camera, point, 1.0)
    }

    fn project_with_aspect(camera: &Camera, point: Vec3, aspect: f32) -> Vec3 {
        camera.view_proj(aspect).project_point3(point)
    }

    #[test]
    fn target_projects_to_viewport_center() {
        let mut camera = Camera::default();
        camera.fit(Vec3::new(1.0, 2.0, 3.0), Vec3::new(5.0, 4.0, 9.0));
        let ndc = project(&camera, camera.target);
        assert!(ndc.x.abs() < EPS && ndc.y.abs() < EPS);
        assert!((0.0..=1.0).contains(&ndc.z));
    }

    #[test]
    fn fitted_box_is_inside_view_volume() {
        let mut camera = Camera::default();
        let (min, max) = (Vec3::splat(-1.0), Vec3::new(3.0, 1.0, 2.0));
        camera.fit(min, max);
        for (step, aspect) in [0.3, 0.8, 1.0, 1.7, 3.0]
            .into_iter()
            .cycle()
            .take(10)
            .enumerate()
        {
            camera.orbit(37.0 * step as f32, -21.0);
            for corner in 0..8 {
                let point = Vec3::new(
                    if corner & 1 == 0 { min.x } else { max.x },
                    if corner & 2 == 0 { min.y } else { max.y },
                    if corner & 4 == 0 { min.z } else { max.z },
                );
                let ndc = project_with_aspect(&camera, point, aspect);
                assert!(ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0, "{ndc}");
                assert!((0.0..=1.0).contains(&ndc.z), "{ndc}");
            }
        }
    }

    #[test]
    fn orbit_keeps_distance_and_orthonormal_axes() {
        let mut camera = Camera::default();
        let distance_before = (camera.eye() - camera.target).length();
        camera.orbit(120.0, 45.0);
        assert!(((camera.eye() - camera.target).length() - distance_before).abs() < EPS);
        assert!(camera.right().dot(camera.up()).abs() < EPS);
        assert!(camera.forward().dot(camera.up()).abs() < EPS);
    }

    #[test]
    fn pan_moves_scene_with_the_mouse() {
        let mut camera = Camera::default();
        let anchor = Vec3::ZERO;
        let before = project(&camera, anchor);
        camera.pan(50.0, 0.0, 500.0, 500.0);
        let after = project(&camera, anchor);
        assert!(
            (after.x - before.x - 0.2).abs() < EPS,
            "{before} -> {after}"
        );
        assert!((after.y - before.y).abs() < EPS);
    }

    #[test]
    fn pan_uses_shorter_viewport_side() {
        let mut camera = Camera::default();
        let aspect = 0.5;
        let before = project_with_aspect(&camera, Vec3::ZERO, aspect);
        camera.pan(25.0, 0.0, 250.0, 500.0);
        let after = project_with_aspect(&camera, Vec3::ZERO, aspect);
        assert!(
            (after.x - before.x - 0.2).abs() < EPS,
            "{before} -> {after}"
        );
    }

    #[test]
    fn zoom_in_magnifies() {
        let mut camera = Camera::default();
        let point = camera.target + camera.right();
        let before = project(&camera, point).x;
        camera.zoom(0.5);
        let after = project(&camera, point).x;
        assert!((after - 2.0 * before).abs() < EPS);
    }
}
