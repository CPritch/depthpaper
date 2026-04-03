// Depthpaper fullscreen quad shader

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle from vertex index (3 vertices, no buffers needed)
// Covers [-1, -1] to [1, 1] with UVs [0, 0] to [1, 1]
@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    // Generates a full-screen triangle:
    //   idx 0 -> (-1, -1)  uv (0, 1)
    //   idx 1 -> ( 3, -1)  uv (2, 1)
    //   idx 2 -> (-1,  3)  uv (0, -1)
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    // Flip Y for UV (Vulkan clip space Y is top-down)
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;

struct Uniforms {
    cursor_offset: vec2<f32>,  // normalized cursor offset from center
    intensity: f32,            // parallax strength
    _pad: f32,
};
@group(0) @binding(3) var<uniform> u: Uniforms;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // simple passthrough
    // TODO: add:
    //   let depth = textureSample(depth_tex, tex_sampler, in.uv).r;
    //   let displaced = in.uv - u.cursor_offset * depth * u.intensity;
    //   return textureSample(color_tex, tex_sampler, displaced);

    return textureSample(color_tex, tex_sampler, in.uv);
}