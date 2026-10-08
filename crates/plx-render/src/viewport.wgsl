struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    background_top: vec4<f32>,
    background_bottom: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct BackgroundOut {
    @builtin(position) position: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn vs_background(@builtin(vertex_index) index: u32) -> BackgroundOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: BackgroundOut;
    out.position = vec4<f32>(uv * 2.0 - 1.0, 1.0, 1.0);
    out.t = uv.y * 0.5;
    return out;
}

@fragment
fn fs_background(in: BackgroundOut) -> @location(0) vec4<f32> {
    return mix(globals.background_bottom, globals.background_top, clamp(in.t, 0.0, 1.0));
}

struct SurfaceIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

struct SurfaceOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_surface(in: SurfaceIn) -> SurfaceOut {
    var out: SurfaceOut;
    out.position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    return out;
}

@fragment
fn fs_surface(in: SurfaceOut) -> @location(0) vec4<f32> {
    let diffuse = abs(dot(normalize(in.normal), -globals.light_dir.xyz));
    let shade = 0.3 + 0.7 * diffuse;
    return vec4<f32>(in.color * shade, 1.0);
}

@vertex
fn vs_edge(in: SurfaceIn) -> @builtin(position) vec4<f32> {
    return globals.view_proj * vec4<f32>(in.position, 1.0);
}

@fragment
fn fs_edge() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
