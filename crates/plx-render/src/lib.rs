//! wgpu-Renderer für FE-Netze und Ergebnisse.

mod camera;
pub mod contour;
mod fe;
mod mesh;
mod renderer;
mod section;

pub use camera::{Camera, StandardView};
pub use fe::{part_color, part_render_mesh, wireframe_edges};
pub use mesh::{RenderMesh, Vertex};
pub use renderer::{COLOR_FORMAT, DisplayOptions, ViewportRenderer};
pub use section::{
    ClipPlane, SectionCells, SectionValues, clip_plane, lighten, section_mesh, section_values,
};
pub use wgpu;
