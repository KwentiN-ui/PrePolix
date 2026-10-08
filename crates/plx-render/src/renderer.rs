use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::mesh::{RenderMesh, Vertex};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLE_COUNT: u32 = 4;

/// Background gradient in the style of PrePoMax (linear RGB).
const BACKGROUND_TOP: [f32; 4] = [0.30, 0.42, 0.62, 1.0];
const BACKGROUND_BOTTOM: [f32; 4] = [0.92, 0.94, 0.97, 1.0];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
    background_top: [f32; 4],
    background_bottom: [f32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    triangles: wgpu::Buffer,
    triangle_count: u32,
    edges: wgpu::Buffer,
    edge_count: u32,
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
    mesh: Option<GpuMesh>,
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
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3],
        };
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology: wgpu::PrimitiveTopology,
                        depth: wgpu::DepthStencilState| {
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
                    targets: &[Some(COLOR_FORMAT.into())],
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
        );
        let edge_pipeline = pipeline(
            "viewport edges",
            "vs_edge",
            "fs_edge",
            &[Some(vertex_layout)],
            wgpu::PrimitiveTopology::LineList,
            depth_state(false, wgpu::CompareFunction::LessEqual, Default::default()),
        );

        Self {
            globals,
            bind_group,
            background_pipeline,
            surface_pipeline,
            edge_pipeline,
            mesh: None,
            targets: Targets::new(device, 1, 1),
        }
    }

    pub fn set_mesh(&mut self, device: &wgpu::Device, mesh: &RenderMesh) {
        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        self.mesh = Some(GpuMesh {
            vertices: buffer(
                "mesh vertices",
                bytemuck::cast_slice(&mesh.vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            triangles: buffer(
                "mesh triangles",
                bytemuck::cast_slice(&mesh.triangles),
                wgpu::BufferUsages::INDEX,
            ),
            triangle_count: mesh.triangles.len() as u32,
            edges: buffer(
                "mesh edges",
                bytemuck::cast_slice(&mesh.edges),
                wgpu::BufferUsages::INDEX,
            ),
            edge_count: mesh.edges.len() as u32,
        });
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

    pub fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, camera: &Camera) {
        let aspect = self.targets.width as f32 / self.targets.height as f32;
        let globals = Globals {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            light_dir: camera.light_direction().extend(0.0).into(),
            background_top: BACKGROUND_TOP,
            background_bottom: BACKGROUND_BOTTOM,
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
            if let Some(mesh) = &self.mesh {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_pipeline(&self.surface_pipeline);
                pass.set_index_buffer(mesh.triangles.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.triangle_count, 0, 0..1);
                pass.set_pipeline(&self.edge_pipeline);
                pass.set_index_buffer(mesh.edges.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.edge_count, 0, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }
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
