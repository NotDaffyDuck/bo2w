// Full-screen colour grade driven by script: x = hue turn (radians), y = gamma,
// z = exposure (stops), w = saturation.
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> grade: vec4<f32>;

struct Corner {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tc: vec2<f32>,
}

// One oversized triangle that covers the screen.
@vertex
fn vertex(@builtin(vertex_index) corner: u32) -> Corner {
    let tc = vec2<f32>(f32((corner << 1u) & 2u), f32(corner & 2u));
    let ndc = vec2<f32>(tc.x * 2.0 - 1.0, 1.0 - tc.y * 2.0);
    return Corner(vec4<f32>(ndc, 0.0, 1.0), tc);
}

// YIQ chroma back to RGB for a given luma.
fn from_yiq(y: f32, i: f32, q: f32) -> vec3<f32> {
    return vec3<f32>(y + 0.956 * i + 0.621 * q, y - 0.272 * i - 0.647 * q, y - 1.106 * i + 1.703 * q);
}

@fragment
fn fragment(at: Corner) -> @location(0) vec4<f32> {
    let texel = textureSample(scene, scene_sampler, at.tc);
    let luma = dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let in_phase = dot(texel.rgb, vec3<f32>(0.596, -0.274, -0.322));
    let quadrature = dot(texel.rgb, vec3<f32>(0.211, -0.523, 0.312));

    // Turn the chroma vector by the hue angle and add only the difference, so a zero turn
    // returns the texel untouched.
    let turned_i = in_phase * cos(grade.x) - quadrature * sin(grade.x);
    let turned_q = in_phase * sin(grade.x) + quadrature * cos(grade.x);
    let hue_shifted = texel.rgb + from_yiq(luma, turned_i, turned_q) - from_yiq(luma, in_phase, quadrature);

    let saturated = mix(vec3<f32>(luma), hue_shifted, grade.w);
    let exposed = max(saturated * exp2(grade.z), vec3<f32>(0.0));
    return vec4<f32>(pow(exposed, vec3<f32>(1.0 / grade.y)), texel.a);
}
