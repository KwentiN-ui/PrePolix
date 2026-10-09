use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::contour::{MAX_LEVELS, band_colors};
use crate::mesh::{RenderMesh, Vertex};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLE_COUNT: u32 = 4;
/// How far edges are pulled towards the viewer, in pixels. Tied to the pixel size rather than
/// the model size, so that edges on the back of thin walls stay hidden when zoomed in.
const EDGE_OFFSET_PX: f32 = 1.5;
/// Width of [`RenderMesh::wide_edges`] in pixels, as PrePoMax draws a selected part's outline.
pub const WIDE_EDGE_PX: f32 = 3.0;

/// PrePoMax's background gradient: Gainsboro at the top, WhiteSmoke at the bottom.
const BACKGROUND_TOP: [f32; 4] = [220.0 / 255.0, 220.0 / 255.0, 220.0 / 255.0, 1.0];
const BACKGROUND_BOTTOM: [f32; 4] = [245.0 / 255.0, 245.0 / 255.0, 245.0 / 255.0, 1.0];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    lights: [[f32; 4]; 3],
    view_dir: [f32; 4],
    background_top: [f32; 4],
    background_bottom: [f32; 4],
    /// x: edge depth offset in normalized depth units; y, z: target size in pixels; w: width
    /// of wide edges in pixels.
    edge: [f32; 4],
    /// x: number of contour bands, 0 when surfaces show their part colour.
    contour: [f32; 4],
    palette: [[f32; 4]; MAX_LEVELS as usize],
}

/// A vertex or index buffer with its element count; `None` when there is nothing to draw.
struct GpuBuffer {
    buffer: wgpu::Buffer,
    count: u32,
}

struct GpuMesh {
    vertices: Option<GpuBuffer>,
    triangles: Option<GpuBuffer>,
    feature_edges: Option<GpuBuffer>,
    mesh_edges: Option<GpuBuffer>,
    wireframe_edges: Option<GpuBuffer>,
    wide_edges: Option<GpuBuffer>,
    visible: bool,
}

/// What the viewport draws besides the shaded surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayOptions {
    pub mesh_edges: bool,
    /// Colour surfaces by their normalized vertex scalars in this many bands.
    pub contour_levels: Option<u32>,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            mesh_edges: true,
            contour_levels: None,
        }
    }
}

struct Targets {
    width: u32,
    height: u32,
    msaa_view: wgpu::TextureView,
    resolve_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

/// Renders the 3D scene into an offscreen texture that the GUI shows as an image.
pub struct ViewportRenderer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    background_pipeline: wgpu::RenderPipeline,
    surface_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    wide_edge_pipeline: wgpu::RenderPipeline,
    parts: Vec<GpuMesh>,
    targets: Targets,
}

impl ViewportRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("viewport.wgsl"));
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport globals"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32],
        };
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology: wgpu::PrimitiveTopology,
                        depth: wgpu::DepthStencilState,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState {
                    count: SAMPLE_COUNT,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let depth_state =
            |write: bool, compare: wgpu::CompareFunction, bias: wgpu::DepthBiasState| {
                wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias,
                }
            };
        let background_pipeline = pipeline(
            "viewport background",
            "vs_background",
            "fs_background",
            &[],
            wgpu::PrimitiveTopology::TriangleList,
            depth_state(false, wgpu::CompareFunction::Always, Default::default()),
            None,
        );
        let surface_pipeline = pipeline(
            "viewport surfaces",
            "vs_surface",
            "fs_surface",
            &[Some(vertex_layout.clone())],
            wgpu::PrimitiveTopology::TriangleList,
            depth_state(
                true,
                wgpu::CompareFunction::Less,
                wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.0,
                    clamp: 0.0,
                },
            ),
            None,
        );
        let edge_pipeline = pipeline(
            "viewport edges",
            "vs_edge",
            "fs_edge",
            &[Some(vertex_layout)],
            wgpu::PrimitiveTopology::LineList,
            depth_state(false, wgpu::CompareFunction::LessEqual, Default::default()),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        // Wide lines are quads: one instance per segment reads both end vertices of the line
        // list, the vertex shader spreads them across the screen.
        let vertex_size = std::mem::size_of::<Vertex>() as u64;
        let segment_layout = wgpu::VertexBufferLayout {
            array_stride: 2 * vertex_size,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: std::mem::offset_of!(Vertex, color) as u64,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: vertex_size,
                    shader_location: 2,
                },
            ],
        };
        let wide_edge_pipeline = pipeline(
            "viewport wide edges",
            "vs_wide_edge",
            "fs_edge",
            &[Some(segment_layout)],
            wgpu::PrimitiveTopology::TriangleList,
            depth_state(false, wgpu::CompareFunction::LessEqual, Default::default()),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        Self {
            globals,
            bind_group,
            background_pipeline,
            surface_pipeline,
            edge_pipeline,
            wide_edge_pipeline,
            parts: Vec::new(),
            targets: Targets::new(device, 1, 1),
        }
    }

    /// Replaces the scene with one mesh per part; all parts start visible.
    pub fn set_parts(&mut self, device: &wgpu::Device, parts: &[RenderMesh]) {
        let buffer = |label: &str, contents: &[u8], count: usize, usage: wgpu::BufferUsages| {
            (count > 0).then(|| GpuBuffer {
                buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                }),
                count: count as u32,
            })
        };
        let vertex = wgpu::BufferUsages::VERTEX;
        self.parts = parts
            .iter()
            .map(|mesh| GpuMesh {
                vertices: buffer(
                    "part vertices",
                    bytemuck::cast_slice(&mesh.vertices),
                    mesh.vertices.len(),
                    vertex,
                ),
                triangles: buffer(
                    "part triangles",
                    bytemuck::cast_slice(&mesh.triangles),
                    mesh.triangles.len(),
                    wgpu::BufferUsages::INDEX,
                ),
                feature_edges: buffer(
                    "part feature edges",
                    bytemuck::cast_slice(&mesh.feature_edges),
                    mesh.feature_edges.len(),
                    vertex,
                ),
                mesh_edges: buffer(
                    "part mesh edges",
                    bytemuck::cast_slice(&mesh.mesh_edges),
                    mesh.mesh_edges.len(),
                    vertex,
                ),
                wireframe_edges: buffer(
                    "part wireframe edges",
                    bytemuck::cast_slice(&mesh.wireframe_edges),
                    mesh.wireframe_edges.len(),
                    vertex,
                ),
                wide_edges: buffer(
                    "part wide edges",
                    bytemuck::cast_slice(&mesh.wide_edges),
                    mesh.wide_edges.len(),
                    vertex,
                ),
                visible: true,
            })
            .collect();
    }

    pub fn set_part_visible(&mut self, index: usize, visible: bool) {
        if let Some(part) = self.parts.get_mut(index) {
            part.visible = visible;
        }
    }

    /// Recreates the render targets if the size changed; returns true when it did.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if self.targets.width == width && self.targets.height == height {
            return false;
        }
        self.targets = Targets::new(device, width, height);
        true
    }

    /// The resolved color texture, to be registered with the GUI.
    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.targets.resolve_view
    }

    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &Camera,
        options: DisplayOptions,
    ) {
        let aspect = self.targets.width as f32 / self.targets.height as f32;
        let globals = Globals {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            lights: camera.light_directions().map(|l| l.extend(0.0).into()),
            view_dir: (-camera.forward()).extend(0.0).into(),
            background_top: BACKGROUND_TOP,
            background_bottom: BACKGROUND_BOTTOM,
            edge: [
                EDGE_OFFSET_PX
                    * camera.pixel_size(self.targets.width as f32, self.targets.height as f32)
                    / camera.depth_range(),
                self.targets.width as f32,
                self.targets.height as f32,
                WIDE_EDGE_PX,
            ],
            contour: [
                options
                    .contour_levels
                    .map_or(0.0, |n| n.clamp(2, MAX_LEVELS) as f32),
                0.0,
                0.0,
                0.0,
            ],
            palette: palette(options.contour_levels),
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&self.targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.background_pipeline);
            pass.draw(0..3, 0..1);
            let visible = || self.parts.iter().filter(|p| p.visible);
            pass.set_pipeline(&self.surface_pipeline);
            for part in visible() {
                if let (Some(vertices), Some(triangles)) = (&part.vertices, &part.triangles) {
                    pass.set_vertex_buffer(0, vertices.buffer.slice(..));
                    pass.set_index_buffer(triangles.buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..triangles.count, 0, 0..1);
                }
            }
            pass.set_pipeline(&self.edge_pipeline);
            for part in visible() {
                let mesh_edges = part.mesh_edges.as_ref().filter(|_| options.mesh_edges);
                let lines = [
                    part.wireframe_edges.as_ref(),
                    mesh_edges,
                    part.feature_edges.as_ref(),
                ];
                for lines in lines.into_iter().flatten() {
                    pass.set_vertex_buffer(0, lines.buffer.slice(..));
                    pass.draw(0..lines.count, 0..1);
                }
            }
            pass.set_pipeline(&self.wide_edge_pipeline);
            for lines in visible().filter_map(|p| p.wide_edges.as_ref()) {
                pass.set_vertex_buffer(0, lines.buffer.slice(..));
                pass.draw(0..6, 0..lines.count / 2);
            }
        }
        queue.submit([encoder.finish()]);
    }
}

fn palette(levels: Option<u32>) -> [[f32; 4]; MAX_LEVELS as usize] {
    let mut palette = [[0.0; 4]; MAX_LEVELS as usize];
    if let Some(levels) = levels {
        for (slot, [r, g, b]) in palette.iter_mut().zip(band_colors(levels)) {
            *slot = [r, g, b, 1.0];
        }
    }
    palette
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = |label: &str, format, sample_count, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        Self {
            width,
            height,
            msaa_view: texture(
                "viewport color msaa",
                COLOR_FORMAT,
                SAMPLE_COUNT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            resolve_view: texture(
                "viewport color",
                COLOR_FORMAT,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
            depth_view: texture(
                "viewport depth",
                DEPTH_FORMAT,
                SAMPLE_COUNT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
        }
    }
}
