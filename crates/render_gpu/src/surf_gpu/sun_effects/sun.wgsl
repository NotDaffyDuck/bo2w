// Sun glare. A one-texel history texture carries (visible, flare, blind, glare) between frames;
// the sprite, flare and blind passes read it back. Group 0 is shared by every entry point.

struct SunUniforms {
    sun_clip: vec4<f32>,    // sun position in clip space
    screen: vec4<f32>,      // width, height, sprite size, flare size
    flare_fade: vec4<f32>,  // flare alpha, dt ms, flare fade-in ms, flare fade-out ms
    blind_fade: vec4<f32>,  // blind goal, blind fade-in ms, blind fade-out ms, glare goal
    glare_fade: vec4<f32>,  // glare fade-in ms, glare fade-out ms, camera cut, behind camera
}

struct FullscreenVert {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tc: vec2<f32>,
}

struct QuadVert {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tc: vec2<f32>,
    @location(1) alpha: f32,
}

@group(0) @binding(0) var<uniform> u: SunUniforms;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var history_in: texture_2d<f32>;
@group(0) @binding(3) var sprite_tex: texture_2d<f32>;
@group(0) @binding(4) var linear_samp: sampler;
@group(0) @binding(5) var scene_colour: texture_2d<f32>;

const PROBE_REACH: f32 = 8.0;
const PROBE_GRID: i32 = 4;
const DEPTH_NEAR: f32 = 0.002;

// Move `cur` toward `goal`, taking `rise_ms` to climb and `fall_ms` to drop (0 = instant).
fn slew(cur: f32, goal: f32, rise_ms: f32, fall_ms: f32, dt_ms: f32) -> f32 {
    if goal > cur {
        if rise_ms <= 0.0 {
            return goal;
        }
        return min(goal, cur + dt_ms / rise_ms);
    }
    if goal < cur {
        if fall_ms <= 0.0 {
            return goal;
        }
        return max(goal, cur - dt_ms / fall_ms);
    }
    return cur;
}

fn sun_screen_uv() -> vec2<f32> {
    let ndc = u.sun_clip.xy / max(u.sun_clip.w, 1.0e-6);
    return vec2<f32>(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
}

// Fraction of a 4x4 patch of depth samples around the sun that sees open sky.
fn sky_fraction() -> f32 {
    if u.glare_fade.z > 0.5 || u.glare_fade.w > 0.5 {
        return 0.0;
    }
    let tc = sun_screen_uv();
    if tc.x < 0.0 || tc.x > 1.0 || tc.y < 0.0 || tc.y > 1.0 {
        return 0.0;
    }
    let extent = u.screen.xy;
    let origin = tc * extent;
    let spacing = vec2<f32>(PROBE_REACH, PROBE_REACH) / 1.5;
    var open = 0.0;
    var taken = 0.0;
    for (var row = 0; row < PROBE_GRID; row = row + 1) {
        for (var col = 0; col < PROBE_GRID; col = col + 1) {
            let at = origin + vec2<f32>(f32(col) - 1.5, f32(row) - 1.5) * spacing;
            if at.x < 0.0 || at.y < 0.0 || at.x >= extent.x || at.y >= extent.y {
                continue;
            }
            taken = taken + 1.0;
            if textureLoad(scene_depth, vec2<i32>(at), 0) <= DEPTH_NEAR {
                open = open + 1.0;
            }
        }
    }
    if taken <= 0.0 {
        return 0.0;
    }
    return open / taken;
}

fn fullscreen_triangle(index: u32) -> FullscreenVert {
    var v: FullscreenVert;
    let tc = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    v.clip_pos = vec4<f32>(tc.x * 2.0 - 1.0, 1.0 - tc.y * 2.0, 0.0, 1.0);
    v.tc = tc;
    return v;
}

fn current_history() -> vec4<f32> {
    return textureLoad(history_in, vec2<i32>(0, 0), 0);
}

@vertex
fn vs_history(@builtin(vertex_index) index: u32) -> FullscreenVert {
    return fullscreen_triangle(index);
}

@fragment
fn fs_history(_in: FullscreenVert) -> @location(0) vec4<f32> {
    var open = sky_fraction();
    var before = current_history();
    if u.glare_fade.z > 0.5 {
        before = vec4<f32>(0.0);
        open = 0.0;
    }
    let dt = max(u.flare_fade.y, 0.0);
    let flare = slew(before.g, open, u.flare_fade.z, u.flare_fade.w, dt);
    let blind = slew(before.b, open * u.blind_fade.x, u.blind_fade.y, u.blind_fade.z, dt);
    let glare = slew(before.a, open * u.blind_fade.w, u.glare_fade.x, u.glare_fade.y, dt);
    return vec4<f32>(open, flare, blind, glare);
}

@vertex
fn vs_quad(@builtin(vertex_index) index: u32) -> QuadVert {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[index];
#ifdef SUN_FLARE
    let span = u.screen.w;
#else
    let span = u.screen.z;
#endif
    var at = u.sun_clip;
    at.x = at.x + corner.x * at.w * (span / 640.0);
    at.y = at.y + corner.y * at.w * (span / 480.0);
    var v: QuadVert;
    v.clip_pos = at;
    v.tc = vec2<f32>(corner.x * 0.5 + 0.5, -corner.y * 0.5 + 0.5);
    v.alpha = u.flare_fade.x;
    return v;
}

@fragment
fn fs_sprite(v: QuadVert) -> @location(0) vec4<f32> {
    return textureSample(sprite_tex, linear_samp, v.tc) * current_history().r;
}

@fragment
fn fs_flare(v: QuadVert) -> @location(0) vec4<f32> {
    return textureSample(sprite_tex, linear_samp, v.tc) * (current_history().g * v.alpha);
}

@vertex
fn vs_grade(@builtin(vertex_index) index: u32) -> FullscreenVert {
    return fullscreen_triangle(index);
}

@fragment
fn fs_grade(v: FullscreenVert) -> @location(0) vec4<f32> {
    let history = current_history();
    let scene = textureSample(scene_colour, linear_samp, v.tc);
    let dimmed = scene.rgb * (1.0 - clamp(history.b, 0.0, 1.0));
    return vec4<f32>(dimmed + vec3<f32>(clamp(history.a, 0.0, 1.0)), scene.a);
}
