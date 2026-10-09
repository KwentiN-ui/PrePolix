struct Globals {
    view_proj: mat4x4<f32>,
    // Directions towards PrePoMax's three camera lights, in world space.
    lights: array<vec4<f32>, 3>,
    // Direction towards the viewer (orthographic, so the same everywhere).
    view_dir: vec4<f32>,
    background_top: vec4<f32>,
    background_bottom: vec4<f32>,
    // x: depth offset of edges towards the viewer, about one and a half pixels.
    edge: vec4<f32>,
    // x: number of contour bands, 0 to show part colours.
    contour: vec4<f32>,
    // Section view plane: unit normal in xyz, normal · point in w.
    clip: vec4<f32>,
    // x: 1 while the section view is on, y: tolerance behind the plane in world units.
    clip_options: vec4<f32>,
    palette: array<vec4<f32>, 24>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// Like VTK, all colours and lighting live in display (sRGB) space: the target is a plain UNORM
// texture that the GUI shows as is, so shaded colours are written without any encoding.

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
    return vec4<f32>(mix(globals.background_bottom, globals.background_top, clamp(in.t, 0.0, 1.0)).rgb, 1.0);
}

struct SurfaceIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) scalar: f32,
};

struct SurfaceOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) scalar: f32,
    @location(3) world: vec3<f32>,
};

// Everything on the cut-off side of the section plane is not drawn.
fn clipped(world: vec3<f32>) -> bool {
    return globals.clip_options.x > 0.0
        && dot(globals.clip.xyz, world) - globals.clip.w < -globals.clip_options.y;
}

// Colour without a result value: light grey, like PrePoMax's NaN colour.
const NO_VALUE_COLOR: vec3<f32> = vec3<f32>(0.75, 0.75, 0.75);

// PrePoMax's actor properties and light intensity.
const AMBIENT: f32 = 0.6;
const DIFFUSE: f32 = 0.6;
const SPECULAR: f32 = 0.6;
const SPECULAR_POWER: f32 = 100.0;
const LIGHT_INTENSITY: f32 = 0.4;

@vertex
fn vs_surface(in: SurfaceIn) -> SurfaceOut {
    var out: SurfaceOut;
    out.position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    out.scalar = in.scalar;
    out.world = in.position;
    return out;
}

// Discrete band of a normalized value; the top value 1.0 belongs to the last band.
fn contour_color(t: f32) -> vec3<f32> {
    let levels = globals.contour.x;
    if t < 0.0 {
        return NO_VALUE_COLOR;
    }
    let band = u32(clamp(floor(t * levels), 0.0, levels - 1.0));
    return globals.palette[band].rgb;
}

@fragment
fn fs_surface(in: SurfaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if clipped(in.world) {
        discard;
    }
    var color = in.color;
    if globals.contour.x > 0.0 {
        color = contour_color(in.scalar);
    }
    // Two-sided lighting: back faces (inside of open shells) are lit like front faces.
    var n = normalize(in.normal);
    let v = globals.view_dir.xyz;
    if dot(n, v) < 0.0 {
        n = -n;
    }
    var diffuse = 0.0;
    var specular = 0.0;
    for (var i = 0; i < 3; i++) {
        let l = globals.lights[i].xyz;
        diffuse += LIGHT_INTENSITY * max(dot(n, l), 0.0);
        let h = normalize(l + v);
        specular += LIGHT_INTENSITY * pow(max(dot(n, h), 0.0), SPECULAR_POWER);
    }
    let shaded = color * (AMBIENT + DIFFUSE * diffuse) + vec3<f32>(SPECULAR * specular);
    return vec4<f32>(clamp(shaded, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

struct EdgeOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
};

// Edges lie exactly on surface triangles; pulling them slightly towards the viewer keeps them from
// flickering in and out of the depth test (polygon offset alone is too weak for a float depth buffer).
// The offset is a fixed number of pixels, so it never reaches through thin walls.
@vertex
fn vs_edge(in: SurfaceIn) -> EdgeOut {
    var out: EdgeOut;
    out.position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.position.z -= globals.edge.x * out.position.w;
    // Line vertices carry their opacity in the scalar slot.
    out.color = vec4<f32>(in.color, in.scalar);
    out.world = in.position;
    return out;
}

@fragment
fn fs_edge(in: EdgeOut) -> @location(0) vec4<f32> {
    if clipped(in.world) {
        discard;
    }
    return in.color;
}
