// Depthpaper fullscreen quad shader — depth-based parallax displacement

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle from vertex index — no vertex buffers needed.
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
    // Load depth via textureLoad (R32Float is non-filterable, can't use sampler)
    let depth_size = textureDimensions(depth_tex);
    let depth_coord = vec2<u32>(in.uv * vec2<f32>(depth_size));
    let depth = textureLoad(depth_tex, depth_coord, 0).r;

    // Bleed margin: zoom the texture slightly (5% total) so parallax
    // displacement pulls real pixels from outside the visible viewport
    // instead of stretching edge pixels.
    let margin = 0.025;
    let base_uv = mix(vec2<f32>(margin), vec2<f32>(1.0 - margin), in.uv);

    // Displace: cursor moves right (+x) → foreground shifts left (-x)
    let displaced_uv = base_uv - (u.cursor_offset * depth * u.intensity);

    return textureSample(color_tex, tex_sampler, displaced_uv);
}