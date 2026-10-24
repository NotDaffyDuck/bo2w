// HUD quads. Corners arrive in screen pixels and reach clip space through the 2D
// scale (xy) and offset (zw); colours arrive as BGRA bytes and tint the image.

struct Screen2d {
    scale_offset: vec4<f32>,
}

@group(0) @binding(0) var<uniform> screen: Screen2d;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;

struct Corner {
    @location(0) pos: vec4<f32>,
    @location(1) bgra: vec4<f32>,
    @location(2) uv: vec2<f32>,
}

struct Shaded {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
}

@vertex
fn quad_vertex(corner: Corner) -> Shaded {
    var out: Shaded;
    out.clip = vec4<f32>(corner.pos.xy * screen.scale_offset.xy + screen.scale_offset.zw, 0.0, 1.0);
    out.uv = corner.uv;
    out.tint = corner.bgra.zyxw;
    return out;
}

@fragment
fn quad_fragment(px: Shaded) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, px.uv) * px.tint;
}
