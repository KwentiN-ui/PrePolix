struct Globals {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    background_top: vec4<f32>,
    background_bottom: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// The target is a plain UNORM texture that the GUI shows as is, so colors are sRGB encoded here.
fn encode_srgb(linear: vec3<f32>) -> vec4<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return vec4<f32>(select(high, low, c <= vec3<f32>(0.0031308)), 1.0);
}

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
    return encode_srgb(mix(globals.background_bottom, globals.background_top, clamp(in.t, 0.0, 1.0)).rgb);
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
    let shade = 0.45 + 0.55 * diffuse;
    return encode_srgb(in.color * shade);
}

struct EdgeOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_edge(in: SurfaceIn) -> EdgeOut {
    var out: EdgeOut;
    out.position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_edge(in: EdgeOut) -> @location(0) vec4<f32> {
    return encode_srgb(in.color);
}
