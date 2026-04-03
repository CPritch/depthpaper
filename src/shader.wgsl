// Depthpaper fullscreen quad shader

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle from vertex index — no vertex buffers needed.
// Three vertices cover the entire clip space with a single oversized triangle.
@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var tex_sampler: sampler;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;

struct Uniforms {
    cursor_offset: vec2<f32>,
    intensity: f32,
    _pad: f32,
};
@group(0) @binding(3) var<uniform> u: Uniforms;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // --- parallax ---
    // let depth = textureSampleLevel(depth_tex, tex_sampler, in.uv, 0.0).r;
    //
    // Bleed margin: remap UVs from [0,1] to a slightly inset range so
    // displacement pulls real pixels from outside the visible viewport.
    // let margin = 0.025; // half of 5% zoom
    // let base_uv = mix(vec2(margin), vec2(1.0 - margin), in.uv);
    // let displaced = base_uv - u.cursor_offset * depth * u.intensity;
    // return textureSample(color_tex, tex_sampler, displaced);

    // simple passthrough
    return textureSample(color_tex, tex_sampler, in.uv);
}