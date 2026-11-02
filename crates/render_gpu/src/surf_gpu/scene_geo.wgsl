#import bevy_render::view::View

@group(0) @binding(0) var<uniform> cam: View;

// bo2zm: a world surface's colour map (only the textured world pipeline
// binds group 1).
@group(1) @binding(0) var colour_map: texture_2d<f32>;
@group(1) @binding(1) var colour_sampler: sampler;
// bo2zm: a layered surface's layer 1 and 2 colour maps.
@group(1) @binding(6) var colour_map1: texture_2d<f32>;
@group(1) @binding(7) var colour_map2: texture_2d<f32>;

// bo2zm: a T6 lightmap page and the world lighting. Lightmapped world
// surfaces bind all four; every other Black Ops II surface (unlit world
// surfaces, props) binds the lighting and lights alone (bindings 2, 3).
struct T6Lighting {
    // xyz: direction to the sun; w: exposure scale.
    sun_dir_exposure: vec4<f32>,
    // rgb: sun colour; w: the vision's saturation.
    sun_color: vec4<f32>,
    // The sky's rotation (x, y, z of skyBoxRotation) and brightness (w,
    // negative when z flips).
    sky: vec4<f32>,
    // The sky's skyColorParm x (above 0: the cube is rgb / a, else rgb^2)
    // and w (the share of the fog it takes).
    sky_parms: vec4<f32>,
}
// The map's primary lights, four vec4 each: origin and type (1 sun, 2 spot,
// 5 omni), colour and radius, direction and the cone's outer cosine, then
// dAttenuation and the inner cosine. Rows 256 and 257 (vec4 1024 on) are
// the fog's.
const T6_MAX_LIGHTS: u32 = 256u;
struct T6Lights {
    v: array<vec4<f32>, 1032>,
}
@group(2) @binding(0) var lightmap: texture_2d<f32>;
@group(2) @binding(1) var lightmap_sampler: sampler;
@group(2) @binding(2) var<uniform> t6: T6Lighting;
@group(2) @binding(3) var<uniform> t6_lights: T6Lights;
// bo2zm: the lights effects throw (muzzle flashes, explosions, impact
// flashes): a count, then per light its origin and radius, its colour.
struct T6DynLights {
    count: vec4<f32>,
    v: array<vec4<f32>, 32>,
    // bo2mp: the canopy wind: windDirection (the wind's direction on the
    // ground, its length), then the sixteen variantWindSprings (0, then
    // each axis's rustle in radians), as the game's client builds them.
    wind: array<vec4<f32>, 17>,
}
@group(2) @binding(4) var<uniform> t6_dyn: T6DynLights;
// bo2mp: Black Ops II's sun shadow partitions (near, far), read with a
// compare sampler: each one's clip-from-world, then (on, near texel, far
// texel, depth range), sizes in world units.
struct SunShadow {
    near_from_world: mat4x4<f32>,
    far_from_world: mat4x4<f32>,
    info: vec4<f32>,
}
@group(2) @binding(5) var sun_shadow_map: texture_depth_2d_array;
@group(2) @binding(6) var sun_shadow_sampler: sampler_comparison;
@group(2) @binding(7) var<uniform> sun_shadow: SunShadow;
// bo2mp: the placed models' own light grid sets (Black Ops II's static
// model lighting, `mc_lit_sm` / `mlv_lit_sm`, fxc disassembly): a model's
// 56-direction colours in a 4x4x4 block of a 3D texture (block handle - 1,
// 128 blocks a row) and its spherical harmonics, three vec4 a model.
@group(2) @binding(8) var model_light_map: texture_3d<f32>;
@group(2) @binding(9) var model_light_sampler: sampler;
@group(2) @binding(10) var<storage, read> model_light_sh: array<vec4<f32>>;

// bo2mp: a model's light toward n as the game reads it: from its block's
// middle out toward n (1.5 texels across, 0.375 deep, by n over its largest
// axis), filtered, (texel^2) * 32.
fn t6_model_cube(h: u32, n: vec3<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(model_light_map).xy);
    let b = h - 1u;
    let centre = vec3(
        (f32((b & 127u) * 4u) + 2.0) / size.x,
        (f32((b >> 7u) * 4u) + 2.0) / size.y,
        0.5,
    );
    let m = max(max(abs(n.x), abs(n.y)), max(abs(n.z), 0.000001));
    let at = centre + n * vec3(1.5 / size.x, 1.5 / size.y, 0.375) / m;
    let c = textureSampleLevel(model_light_map, model_light_sampler, at, 0.0).rgb;
    return c * c * 32.0;
}

// bo2mp: a model's spherical-harmonic light toward n (the game's gridSH):
// its luminance toward n times its colour, at least 0.1.
fn t6_model_sh(h: u32, n: vec3<f32>) -> vec3<f32> {
    let i = (h - 1u) * 3u;
    let v0 = model_light_sh[i];
    let v1 = model_light_sh[i + 1u];
    let v2 = model_light_sh[i + 2u];
    let lum = dot(v1, vec4(n, 1.0))
        + dot(v2, vec4(n.z * n.x, n.y * n.z, n.x * n.y, n.x * n.x - n.y * n.y))
        + v0.w * n.z * n.z;
    return max(lum * v0.xyz, vec3(0.1));
}

// bo2zm: Black Ops II's lit shaders add such lights ("glights", the lmap
// pixel shaders' combined_glight path) as colour * saturate(1 - d /
// radius)^2 * N.L, diffuse only.
fn dyn_lights(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3(0.0);
    let count = min(u32(t6_dyn.count.x + 0.5), 16u);
    for (var i = 0u; i < count; i++) {
        let a = t6_dyn.v[i * 2u];
        let c = t6_dyn.v[i * 2u + 1u];
        let d = a.xyz - p;
        let dist = max(length(d), 0.0001);
        let fall = saturate(1.0 - dist / max(a.w, 1.0));
        sum += c.rgb * (fall * fall) * saturate(dot(d / dist, n));
    }
    return sum;
}

#ifdef SOFT
// bo2zm: the scene depth, for soft effect edges.
@group(3) @binding(0) var soft_depth: texture_depth_2d;

// BO2's zfeather pixel shaders: how far the scene behind this pixel lies
// past it, times `inv` (1 / the fade distance), clamped to 0..1.
fn soft_fade(clip: vec4<f32>, world: vec3<f32>, inv: f32) -> f32 {
    let scene_d = textureLoad(soft_depth, vec2<i32>(clip.xy), 0);
    let uv = (clip.xy - cam.viewport.xy) / cam.viewport.zw;
    let ndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_d, 1.0);
    let scene_view = cam.view_from_clip * ndc;
    let scene_z = select(1e9, -scene_view.z / scene_view.w, scene_d > 0.0);
    let sprite_z = -(cam.view_from_world * vec4(world, 1.0)).z;
    return saturate((scene_z - sprite_z) * inv);
}
#endif

#ifdef SHINE
// bo2zm: a shine surface's normal and specular maps, and the reflection
// probes (their cube array and, per probe, its lightingSH: three vec4).
@group(1) @binding(8) var normal_map: texture_2d<f32>;
@group(1) @binding(9) var specular_map: texture_2d<f32>;
// bo2mp: an emissive flow surface's colour remap (rows above) and constants
// (the last row); a 1x1 stand-in for every other shine surface.
@group(1) @binding(10) var aux_map: texture_2d<f32>;
// bo2mp: a sea's shore picture (its control map is aux_map); a 1x1
// stand-in for every other shine surface.
@group(1) @binding(11) var shore_map: texture_2d<f32>;
// bo2mp: a sea's white-water noise and mask maps (1x1 stand-ins elsewhere).
@group(1) @binding(12) var foam_noise: texture_2d<f32>;
@group(1) @binding(13) var foam_mask: texture_2d<f32>;
struct T6Probes {
    v: array<vec4<f32>, 96>,
}
@group(3) @binding(0) var probe_cubes: texture_cube_array<f32>;
@group(3) @binding(1) var probe_sampler: sampler;
@group(3) @binding(2) var<uniform> probes: T6Probes;
#ifdef REFRACT
// bo2mp: refracting water (pools, ponds): the scene depth and a copy of the
// frame as the world pass left it (BO2's resolved scene and its depth).
@group(3) @binding(3) var scene_depth: texture_depth_2d;
@group(3) @binding(4) var scene_copy: texture_2d<f32>;

// The copy at framebuffer pixel `px`, back to the light BO2 squares from
// its resolve: the roll-off above 0.75 undone (bo2_final), still encoded.
fn scene_encoded(px: vec2<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(scene_copy));
    let r = textureSampleLevel(scene_copy, probe_sampler, clamp(px, vec2(0.5), dims - 0.5) / dims, 0.0).rgb;
    let unrolled = 0.75 - log2(max(1.0 - (r - 0.75) / 0.25, vec3(0.0001))) / 5.77078;
    return select(r, unrolled, r > vec3(0.75));
}

// The linear light behind framebuffer pixel `px` (encoded squared, over
// the exposure).
fn scene_light(px: vec2<f32>) -> vec3<f32> {
    let e = scene_encoded(px);
    return e * e / max(t6.sun_dir_exposure.w, 0.000001);
}

// How far the scene behind pixel `clip` lies past the water at `world`,
// times `inv`, clamped to 0..1 (BO2's water thickness: zNear / depth less
// the water's view depth).
fn scene_thickness(clip: vec4<f32>, world: vec3<f32>, inv: f32) -> f32 {
    let scene_d = textureLoad(scene_depth, vec2<i32>(clip.xy), 0);
    let uv = (clip.xy - cam.viewport.xy) / cam.viewport.zw;
    let ndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_d, 1.0);
    let scene_view = cam.view_from_clip * ndc;
    let scene_z = select(1e9, -scene_view.z / scene_view.w, scene_d > 0.0);
    let water_z = -(cam.view_from_world * vec4(world, 1.0)).z;
    return saturate((scene_z - water_z) * inv);
}

// The framebuffer pixel of world point `p` and how far a reflection ray
// that lands there has left the screen (BO2's pond: past 95%..97.5% of the
// half-screen it fades over the next 5%..2.5%, smoothed; 1 behind the eye).
fn ssr_hit(p: vec3<f32>, edge: f32, k: f32) -> vec3<f32> {
    let clip = cam.clip_from_world * vec4(p, 1.0);
    if (clip.w <= 0.0) {
        return vec3(0.0, 0.0, 1.0);
    }
    let ndc = clip.xy / clip.w;
    let f = saturate((max(abs(ndc.x), abs(ndc.y)) - edge) * k);
    let uv = ndc * vec2(0.5, -0.5) + 0.5;
    return vec3(cam.viewport.xy + uv * cam.viewport.zw, f * f * (3.0 - 2.0 * f));
}
#endif
#endif

struct WorldIn {
    @builtin(instance_index) instance: u32,
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    // bo2zm: xyz the tangent, w the binormal's sign.
    @location(2) tangent: vec4<f32>,
    @location(3) tint: vec4<f32>,
    @location(4) tex: vec2<f32>,
    @location(5) lightmap_uv: vec2<f32>,
#ifdef LAYERS
    // bo2zm: layer 1 and 2 texcoords (a second buffer).
    @location(6) layer_uv: vec4<f32>,
#endif
}

struct PropIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) tex: vec2<f32>,
    @location(6) model_col0: vec4<f32>,
    @location(7) model_col1: vec4<f32>,
    @location(8) model_col2: vec4<f32>,
    @location(9) model_col3: vec4<f32>,
}

struct Varyings {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) nrm: vec3<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) tex: vec2<f32>,
    @location(3) lightmap_uv: vec2<f32>,
    @location(4) world_position: vec3<f32>,
    // bo2zm: the surface's primary light (the world draw's instance index).
    @location(5) @interpolate(flat) light_index: u32,
    @location(6) layer_uv: vec4<f32>,
    // bo2zm: the tangent frame and reflection probe (shine).
    @location(7) tangent: vec3<f32>,
    @location(8) binormal: vec3<f32>,
    @location(9) @interpolate(flat) probe: u32,
}

@vertex
fn vertex(in: WorldIn) -> Varyings {
    var out: Varyings;
    out.clip_pos = cam.clip_from_world * vec4(in.pos, 1.0);
    out.nrm = in.nrm;
    out.tint = in.tint;
    out.tex = in.tex;
    out.lightmap_uv = in.lightmap_uv;
    out.world_position = in.pos;
    out.tangent = in.tangent.xyz;
    // As BO2's world vertex shaders: binormal = cross(normal, tangent)
    // times the sign of the vertex's binormal sign.
    out.binormal = cross(in.nrm, in.tangent.xyz) * sign(in.tangent.w);
#ifdef SHINE
    // A shine surface's instance index: primary light, then its probe.
    out.light_index = in.instance & 63u;
    out.probe = in.instance >> 6u;
#else
    out.light_index = in.instance;
    out.probe = 0u;
#endif
#ifdef LAYERS
    out.layer_uv = in.layer_uv;
#else
    out.layer_uv = vec4(0.0);
#endif
    return out;
}

@vertex
fn vertex_smodel(in: PropIn) -> Varyings {
    let model_mat = mat4x4<f32>(
        in.model_col0,
        in.model_col1,
        in.model_col2,
        in.model_col3,
    );
    var out: Varyings;
    let world = model_mat * vec4(in.pos, 1.0);
    out.clip_pos = cam.clip_from_world * world;
    out.nrm = (model_mat * vec4(in.nrm, 0.0)).xyz;
    out.tint = in.tint;
    out.tex = in.tex;
    out.lightmap_uv = vec2(0.0);
    out.world_position = world.xyz;
    out.light_index = 0u;
    out.layer_uv = vec4(0.0);
    out.tangent = vec3(1.0, 0.0, 0.0);
    out.binormal = vec3(0.0, 1.0, 0.0);
    out.probe = 0u;
    return out;
}

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    // Shaded by normal direction, with a 16x16 checker mixed in so stretched texture
    // coordinates show.
    let by_normal = abs(normalize(in.nrm)) * 0.72 + vec3(0.08, 0.02, 0.10);
    let checker = (i32(floor(in.tex.x * 16.0)) + i32(floor(in.tex.y * 16.0))) & 1;
    let checker_rgb = select(vec3(0.72, 0.22, 0.78), vec3(0.22, 0.72, 0.78), checker == 0);
    let lifted = max(in.tint.rgb, vec3<f32>(0.35));
    return vec4(mix(by_normal, checker_rgb, 0.28) * lifted, 1.0);
}

// bo2zm: the colour map's stored texel. The material images decode as sRGB;
// Black Ops II's shaders read the raw texel and square it themselves.
fn raw_texel(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3(0.0));
    let lo = x * 12.92;
    let hi = 1.055 * pow(x, vec3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, x <= vec3(0.0031308));
}

// bo2zm: Black Ops II's final image, after its surface shaders: the game's
// resolve rolls highlights off above 0.75 (hdr_bloom_apply: 0.75 + 0.25 *
// (1 - 2^(-5.77 * (x - 0.75)))), here, so the 8-bit frame keeps them. The
// final pass (bo2_bloom) undoes it, adds bloom, rolls off again and looks
// the frame up in the map's colour table (its vision file and lut image).
fn bo2_final(encoded: vec3<f32>) -> vec3<f32> {
    let rolled = 0.75 + 0.25 * (1.0 - exp2(-5.77078 * (encoded - 0.75)));
    return max(select(encoded, rolled, encoded > vec3(0.75)), vec3(0.0));
}

// bo2zm: what every lit and unlit Black Ops II shader writes: the colour
// times the exposure (`hdrControl0.x`), square-rooted; the colour target
// holds gamma-encoded values. Then the game's final image steps.
fn bo2_out(colour: vec3<f32>) -> vec3<f32> {
    return bo2_final(sqrt(max(colour * t6.sun_dir_exposure.w, vec3(0.0))));
}

// bo2zm A7: what an additive draw adds: the final image of its colour less
// that of black (the roll-off keeps black at zero; the grade is applied once
// to the frame by bo2_bloom).
fn bo2_added(encoded: vec3<f32>) -> vec3<f32> {
    return max(bo2_final(encoded) - bo2_final(vec3(0.0)), vec3(0.0));
}

// bo2zm: an unlit surface's output (additive ones add no black, above).
fn unlit_out(colour: vec3<f32>) -> vec3<f32> {
#ifdef ADD
    return bo2_added(sqrt(max(colour * t6.sun_dir_exposure.w, vec3(0.0))));
#else
    return bo2_out(colour);
#endif
}

// bo2zm: the map's fog as the game's shaders apply it (vertex shaders of
// the lit, unlit and sky techniques). Density halves every half height
// above the base height and doubles every half height below it, up to 100x
// (BO2's fog constants: x carries ln(density / max density) = ln(0.01), max
// density 100x; Die Rise's cloud deck under the rooftops). Along the ray
// from the eye its mean is 100 (F(x1) - F(x0)) / (x1 - x0) with x = -ln2 *
// (z - base) / half height + ln(0.01) and F(x) = e^x below zero, x + 1
// above. Fog = 1 - min(1, 2^-((ln2 * mean * d - start) / half distance)):
// with height fog on, BO2 scales the distance term by ln2 and the shader's
// last step is a base-2 exp. A half height under 1 is no height
// fog (the world fog's rows already raise theirs to 1, as BO2 does). Its
// colour and opacity blend toward the
// sun fog by t = saturate(y * dot(sun fog dir, view dir) + x); the colour
// moves toward the fog colour (HDR, before the exposure) by fog * opacity.
// `a`..`e` are light-table slot 257 (four vec4) and slot 256's first vec4.
fn fog_f(x: f32) -> f32 {
    return select(x + 1.0, exp(x), x < 0.0);
}

// BO2's ln2 on the distance term, only with height fog on.
fn fog_ln2(half_height: f32) -> f32 {
    return select(1.0, 0.6931472, half_height >= 1.0);
}

// The mean fog density (in units of the base density) from the eye to `pz`.
fn fog_mean(eye_z: f32, pz: f32, base: f32, half_height: f32) -> f32 {
    let k = select(0.0, -0.6931472 / half_height, half_height >= 1.0);
    let x0 = k * (eye_z - base) - 4.6051702;
    let x1 = k * (pz - base) - 4.6051702;
    let dx = x1 - x0;
    let flat_mean = select(1.0, exp(x0), x0 < 0.0);
    return 100.0 * select((fog_f(x1) - fog_f(x0)) / dx, flat_mean, abs(dx) < 0.0001);
}

fn fog_apply(
    colour: vec3<f32>,
    p: vec3<f32>,
    a: vec4<f32>,
    c: vec4<f32>,
    s: vec4<f32>,
    e: vec4<f32>,
    q: vec4<f32>,
) -> vec3<f32> {
    if (a.w < 0.5) {
        return colour;
    }
    let f = fog_at(p, a, c, s, e, q);
    return mix(colour, f.tint, f.fog * f.opacity);
}

// The fog toward `p`: its colour, amount and opacity.
struct FogAt {
    tint: vec3<f32>,
    fog: f32,
    opacity: f32,
}

fn fog_at(
    p: vec3<f32>,
    a: vec4<f32>,
    c: vec4<f32>,
    s: vec4<f32>,
    e: vec4<f32>,
    q: vec4<f32>,
) -> FogAt {
    let eye = cam.world_position;
    let d = distance(p, eye);
    let mean = fog_mean(eye.z, p.z, c.w, e.w) * fog_ln2(e.w);
    let fog = 1.0 - min(exp2(-(mean * d - a.x) / max(a.y, 1.0)), 1.0);
    let t = saturate(q.y * dot(e.xyz, (p - eye) / max(d, 0.0001)) + q.x);
    return FogAt(mix(c.rgb, s.rgb, t), fog, mix(a.z, s.w, t));
}

// bo2zm: the fog as Black Ops II's effect vertex shaders hand it on: the
// fog colour times the exposure, square-rooted (display space), and how
// much of the effect's own colour stays, (1 - fog * opacity)^2.
fn effect_fog(p: vec3<f32>) -> vec4<f32> {
    let a = t6_lights.v[1028u];
    let c = t6_lights.v[1029u];
    let s = t6_lights.v[1030u];
    let e = t6_lights.v[1031u];
    let q = t6_lights.v[1024u];
    if (a.w < 0.5) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let eye = cam.world_position;
    let d = distance(p, eye);
    let mean = fog_mean(eye.z, p.z, c.w, e.w) * fog_ln2(e.w);
    let fog = 1.0 - min(exp2(-(mean * d - a.x) / max(a.y, 1.0)), 1.0);
    let t = saturate(q.y * dot(e.xyz, (p - eye) / max(d, 0.0001)) + q.x);
    let tint = mix(c.rgb, s.rgb, t);
    let opacity = mix(a.z, s.w, t);
    let keep = 1.0 - fog * opacity;
    return vec4(sqrt(saturate(tint * t6.sun_dir_exposure.w)), keep * keep);
}

fn bo2_fog(colour: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    return fog_apply(
        colour,
        p,
        t6_lights.v[1028u],
        t6_lights.v[1029u],
        t6_lights.v[1030u],
        t6_lights.v[1031u],
        t6_lights.v[1024u],
    );
}

// bo2zm: the direct light of primary light `index` at `p` with normal `n`,
// before its baked visibility. The sun uses the world's sun; spot and omni
// lights fall off with dAttenuation / d^2 (full strength inside) times a
// smoothstep to zero at their radius, and spots fade between their outer
// and inner cone cosines (the beam runs along -direction). Structure from
// the game's lmap shaders; the parameter mapping is inferred.
// bo2zm: a primary light at a point: the direction to it and its colour
// there (attenuated, before N.L).
struct LightRay {
    dir: vec3<f32>,
    colour: vec3<f32>,
}

// bo2mp: one sun partition at a point (its clip position): four compare
// taps half a texel apart, the point moved toward the sun by a texel and a
// half plus half a unit so a lit side does not shadow itself.
fn sun_shadow_partition(layer: i32, clip: vec4<f32>, texel: f32) -> f32 {
    let uv = clip.xy * vec2(0.5, -0.5) + vec2(0.5);
    let depth = clip.z - (texel * 1.5 + 0.5) / sun_shadow.info.w;
    let o = 0.5 / 2048.0;
    var lifted = textureSampleCompareLevel(sun_shadow_map, sun_shadow_sampler, uv + vec2(-o, -o), layer, depth);
    lifted += textureSampleCompareLevel(sun_shadow_map, sun_shadow_sampler, uv + vec2(o, -o), layer, depth);
    lifted += textureSampleCompareLevel(sun_shadow_map, sun_shadow_sampler, uv + vec2(-o, o), layer, depth);
    lifted += textureSampleCompareLevel(sun_shadow_map, sun_shadow_sampler, uv + vec2(o, o), layer, depth);
    return lifted * 0.25;
}

// bo2mp: how much sun reaches `p` past the things that move (1 = all): the
// near partition where it covers `p`, else the far one, fading out at its
// edge; past both, all of it.
fn sun_shadow_at(p: vec3<f32>) -> f32 {
    if (sun_shadow.info.x < 0.5) {
        return 1.0;
    }
    let near = sun_shadow.near_from_world * vec4(p, 1.0);
    if (all(abs(near.xy) < vec2(0.96)) && near.z > 0.0 && near.z < 1.0) {
        return sun_shadow_partition(0, near, sun_shadow.info.y);
    }
    let far = sun_shadow.far_from_world * vec4(p, 1.0);
    let edge = max(abs(far.x), abs(far.y));
    if (edge >= 1.0 || far.z <= 0.0 || far.z >= 1.0) {
        return 1.0;
    }
    return mix(sun_shadow_partition(1, far, sun_shadow.info.z), 1.0, smoothstep(0.8, 1.0, edge));
}

fn primary_light_ray(index: u32, p: vec3<f32>) -> LightRay {
    var ray: LightRay;
    ray.dir = vec3(0.0, 0.0, 1.0);
    ray.colour = vec3(0.0);
    if (index >= T6_MAX_LIGHTS) {
        return ray;
    }
    let a = t6_lights.v[index * 4u];
    let b = t6_lights.v[index * 4u + 1u];
    let c = t6_lights.v[index * 4u + 2u];
    let d = t6_lights.v[index * 4u + 3u];
    let kind = u32(a.w + 0.5);
    if (kind == 1u) {
        ray.dir = t6.sun_dir_exposure.xyz;
        ray.colour = t6.sun_color.rgb * sun_shadow_at(p);
        return ray;
    }
    if (kind == 0u) {
        return ray;
    }
    let to_light = a.xyz - p;
    let d2 = max(dot(to_light, to_light), 1.0);
    let dist = sqrt(d2);
    let l = to_light / dist;
    var atten = saturate(d.x / d2);
    let fall = saturate(1.0 - dist / max(b.w, 1.0));
    atten *= fall * fall * (3.0 - 2.0 * fall);
    if (kind >= 2u && kind <= 4u) {
        atten *= smoothstep(c.w, max(d.y, c.w + 0.0001), dot(l, c.xyz));
    }
    ray.dir = l;
    ray.colour = b.rgb * atten;
    return ray;
}

fn primary_light(index: u32, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let ray = primary_light_ray(index, p);
    return ray.colour * saturate(dot(n, ray.dir));
}

#ifdef ANIM
// bo2mp: Black Ops II's animated screen (`sw4_3d_animated_unlit`, technique
// 4 pixel shader, fxc disassembly): the colour map is a grid of frames
// (Grid columns by rows) played at FPS by game time, each frame's cell
// scaled about its centre by Padding, sampled with the gradients scaled
// to match. The constants sit in layer 1's place: FPS, Grid, Padding, then
// hdrAmount squared.
fn anim_texel(uv: vec2<f32>) -> vec4<f32> {
    let k = textureLoad(colour_map1, vec2(0, 0), 0);
    let grid = max(k.yz, vec2(1.0));
    let frames = grid.x * grid.y;
    let f = floor(frames * fract(t6_dyn.count.y * k.x / frames));
    let cell = vec2(fract(f / grid.x) * grid.x, floor(f / grid.x));
    let at = (cell + (fract(uv) - 0.5) * k.w + 0.5) / grid;
    let g = k.w / grid;
    return textureSampleGrad(colour_map, colour_sampler, at, dpdx(uv) * g, dpdy(uv) * g);
}

// Its colour: (texel * hdrAmount)^2 (no vertex colour; the reflection the
// game adds by Reflection_Amount is left out), fogged, alpha one.
fn anim_colour(albedo: vec4<f32>, world_position: vec3<f32>) -> vec4<f32> {
    let k = textureLoad(colour_map1, vec2(1, 0), 0);
    let texel = raw_texel(albedo.rgb);
    return vec4(bo2_out(bo2_fog(texel * texel * k.x, world_position)), 1.0);
}

// bo2mp: Black Ops II's three-layer unlit (`sw4_3d_unlit_3layer`:
// Nightclub's starfield ceiling; technique 4 pixel shader, fxc
// disassembly), drawn as an animated screen whose table's texel 1 has y
// set. Layer i's uv: ((uv - Origin + Offset + parallax * Depth) * Scale)
// turned by Rotation.x * (1 + Rotation.y * time) turns, plus time * Scroll
// * Scale + Origin. Texels 2 + 3i.. hold [Scale, Offset], [Scroll, Origin],
// [Rotation, Alpha, Depth].
fn three_layer_uv(i: i32, uv: vec2<f32>, parallax: vec2<f32>) -> vec2<f32> {
    let a = textureLoad(colour_map1, vec2(2 + 3 * i, 0), 0);
    let b = textureLoad(colour_map1, vec2(3 + 3 * i, 0), 0);
    let c = textureLoad(colour_map1, vec2(4 + 3 * i, 0), 0);
    let t = t6_dyn.count.y;
    let p = (uv - b.zw + a.zw + parallax * c.w) * a.xy;
    let turn = c.x * (1.0 + c.y * t) * 6.283185;
    let s = sin(turn);
    let r = cos(turn);
    return vec2(r * p.x + s * p.y, r * p.y - s * p.x) + t * b.xy * a.xy + b.zw;
}

// The parallax is the eye's direction in the surface's tangent frame (here
// from the screen derivatives: u's and v's directions) over its facing,
// negated. Layer 3 (layer 2's map) is laid down by its alpha, then layer 2
// and layer 1 (the colour map) mixed over it by alpha times their Alpha,
// all times the summed alpha; then (colour * hdrAmount)^2, unfogged.
fn three_layer_colour(uv: vec2<f32>, world_position: vec3<f32>) -> vec4<f32> {
    let dp1 = dpdx(world_position);
    let dp2 = dpdy(world_position);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let n = normalize(cross(dp1, dp2));
    let p2 = cross(dp2, n);
    let p1 = cross(n, dp1);
    let tu = p2 * duv1.x + p1 * duv2.x;
    let tv = p2 * duv1.y + p1 * duv2.y;
    let to_eye = normalize(cam.world_position - world_position);
    let facing = max(abs(dot(to_eye, n)), 0.0001);
    let along = vec2(dot(to_eye, tu) * inverseSqrt(max(dot(tu, tu), 1e-20)), dot(to_eye, tv) * inverseSqrt(max(dot(tv, tv), 1e-20)));
    let parallax = -along / facing;
    let l1 = textureSample(colour_map, colour_sampler, three_layer_uv(0, uv, parallax));
    let l2 = textureSample(colour_map, colour_sampler, three_layer_uv(1, uv, parallax));
    let l3 = textureSample(colour_map2, colour_sampler, three_layer_uv(2, uv, parallax));
    let k = textureLoad(colour_map1, vec2(1, 0), 0);
    let alpha1 = textureLoad(colour_map1, vec2(4, 0), 0).z;
    let alpha2 = textureLoad(colour_map1, vec2(7, 0), 0).z;
    let alpha3 = textureLoad(colour_map1, vec2(10, 0), 0).z;
    var c = raw_texel(l3.rgb) * l3.a;
    let k2 = alpha2 * l2.a;
    c = c + k2 * (raw_texel(l2.rgb) - c);
    let k1 = alpha1 * l1.a;
    let a = saturate(saturate(l3.a * alpha3 + k2) + k1);
    c = (c + k1 * (raw_texel(l1.rgb) - c)) * a;
    return vec4(bo2_out(c * c * k.x), 1.0);
}
#endif

// bo2zm: world surfaces without a lightmap. Unlit ones as the game's unlit
// shaders draw them: (texel * vertex colour)^2 times the material's colour
// scale, then the BO2 output. A multiply decal scales what is behind by
// 1 + a * (texel * vertex colour - 1). Lit ones without a lightmap get a
// plain sun-and-sky light. Alpha feeds the blended variants.
@fragment
fn fragment_textured(in: Varyings) -> @location(0) vec4<f32> {
#ifdef ANIM
    let screen = anim_colour(anim_texel(in.tex), in.world_position);
    let layered = three_layer_colour(in.tex, in.world_position);
    return select(screen, layered, textureLoad(colour_map1, vec2(1, 0), 0).y > 0.5);
#else
    let albedo = textureSample(colour_map, colour_sampler, in.tex);
#ifdef ALPHA_TEST
    // The game's alpha-tested shaders discard below 128/255.
    if (albedo.a < 0.501961) {
        discard;
    }
#endif
    let texel = raw_texel(albedo.rgb);
#ifdef UNLIT
#ifdef MULTIPLY
    let a = albedo.a * in.tint.a;
    return vec4(mix(vec3(1.0), texel * in.tint.rgb, a), a);
#else
    let c = texel * in.tint.rgb;
#ifdef ADD
    // Additive ones take no fog (it would add the fog colour over the
    // whole surface: Socotra's sea foam went white far off).
    return vec4(bo2_out(c * c * f32(#{UNLIT_SCALE})), albedo.a * in.tint.a);
#else
    return vec4(
        unlit_out(unlit_fog(c * c * f32(#{UNLIT_SCALE}), in.world_position)),
        albedo.a * in.tint.a,
    );
#endif
#endif
#else
    let n = normalize(in.nrm);
    let sun = normalize(vec3(0.35, 0.25, 0.9));
    let light = 0.55 + 0.45 * max(dot(n, sun), 0.0);
    return vec4(texel * light, albedo.a);
#endif
#endif
}

#ifdef FLOW
// bo2mp: Black Ops II's emissive flow (`cod7emissiveflow`, the molten
// lava; its lit technique, fxc disassembly). The specular map holds its
// emissive masks and noise (hi.a, lo.a, noise.g, noise.a, packed by the
// assets lane), the auxiliary map its colour remap and, in the last
// row, its constants: diffuse uv control (scale, scroll), emissive uv
// scale and scroll, noise uv control, normal scale and bias, remap uv,
// diffuse filter (w: noise weight), emissive filter. The heat is the
// masks' product plus noise times the vertex blue; the remap's row is a
// smoothstep of noise plus the vertex red. The rock: the sun alone
// (Burley diffuse, a GGX lobe of alpha^2 0.2401, Schlick from 0.15), no
// lightmap; the glow: remap^2 times the emissive filter, plus remap^2
// times the diffuse filter by n.v. The normal map fades where the remap
// alpha does. No fog (the shader has none).
fn bo2_flow(
    uv: vec2<f32>,
    vcolour: vec4<f32>,
    vn: vec3<f32>,
    tangent: vec3<f32>,
    binormal: vec3<f32>,
    p: vec3<f32>,
) -> vec3<f32> {
    let crow = i32(textureDimensions(aux_map).y) - 1;
    let fc0 = textureLoad(aux_map, vec2<i32>(0, crow), 0);
    let fc1 = textureLoad(aux_map, vec2<i32>(1, crow), 0);
    let fc2 = textureLoad(aux_map, vec2<i32>(2, crow), 0);
    let fc3 = textureLoad(aux_map, vec2<i32>(3, crow), 0);
    let fc4 = textureLoad(aux_map, vec2<i32>(4, crow), 0);
    let fc5 = textureLoad(aux_map, vec2<i32>(5, crow), 0);
    let fc6 = textureLoad(aux_map, vec2<i32>(6, crow), 0);
    let fc7 = textureLoad(aux_map, vec2<i32>(7, crow), 0);
    let ft = t6_dyn.count.y;
    let fuv0 = uv * fc0.xy;
    let fuv = fuv0 + ft * fc0.zw;
    let fnoise = fuv.xyxy * fc3;
    let femit = fuv0.xyxy * fc1 + ft * fc2;
    let fhi = textureSample(specular_map, colour_sampler, femit.xy).r;
    let flo = textureSample(specular_map, colour_sampler, femit.zw).g;
    let fny = textureSample(specular_map, colour_sampler, fnoise.xy).b;
    let fna = textureSample(specular_map, colour_sampler, fnoise.zw).a;
    let fheat = fhi * flo + fny * vcolour.b * fc6.w;
    var frow = saturate((fna - 0.5) * 0.25 + vcolour.r);
    frow = frow * frow * (3.0 - 2.0 * frow);
    let fruv = vec2(fheat, frow) * fc5.xy + fc5.zw;
    let frows = f32(crow);
    let fremap = textureSample(aux_map, colour_sampler, vec2(fruv.x, fruv.y * frows / (frows + 1.0)));
    let falbedo = textureSample(colour_map, colour_sampler, fuv);
    let fnm = (textureSample(normal_map, colour_sampler, fuv).xy * fc4.x + fc4.y) * fremap.a;
    let fn_ = normalize(fnm.x * tangent + fnm.y * binormal + vn);
    let fv = normalize(p - cam.world_position);
    let fl = t6.sun_dir_exposure.xyz;
    let fh = normalize(fl - fv);
    let fndoth = saturate(dot(fn_, fh));
    let fdd = fndoth * fndoth * -0.7599 + 1.0;
    let fd = 0.2401 / max(fdd * fdd, 0.000001);
    let fhv = saturate(dot(fh, -fv));
    let fhl = saturate(dot(fh, fl));
    let fnv = saturate(dot(fn_, -fv));
    let fnl = saturate(dot(fn_, fl));
    let fq = 1.0 - fhl;
    let fspec = fd * (fq * fq * fq * fq * fq * 0.85 + 0.15) / max(fhv * 4.0, 0.0001) * 0.3184;
    let f90 = fhl * fhl * 0.98 - 0.5;
    let fqv = 1.0 - fnv;
    let fql = 1.0 - fnl;
    let fburley = (1.0 + f90 * fqv * fqv * fqv * fqv * fqv) * (1.0 + f90 * fql * fql * fql * fql * fql);
    let fbase = raw_texel(falbedo.rgb);
    let fr2 = fremap * fremap;
    let flit = fnl * (fbase * fbase * fburley + vec3(fspec)) * t6.sun_color.rgb;
    let fglow = fr2.rgb * fc6.rgb * fnv + fr2.rgb * fc7.rgb;
    return flit + fglow;
}
#endif

// bo2zm: an unlit surface's fog. Additive ones fade toward black by the
// fog's keep factor (going toward the fog colour would add it over the
// whole surface: Tranzit's 4000-unit fire glow cards lit the town's skyline
// and the power station's window in bright rectangles); the rest go toward
// the fog colour.
fn unlit_fog(colour: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
#ifdef ADD
    return colour * sqrt(effect_fog(p).w);
#else
    return bo2_fog(colour, p);
#endif
}

// bo2zm: Black Ops II's lit world colour, as its lightmap shaders compute it
// (pimp_shader_lmap_*, read from the game's own shader code): the colour
// map times the vertex colour, squared into linear; the lightmap's three
// pages at (u, v / 3 + page / 3): base light and directional light (rgb / a),
// light direction (rgb * 2 - 1) with the baked visibility of the surface's
// primary light in alpha; light is base, plus directional times N.dir, plus
// primary times that baked visibility; then the BO2 output. Normal maps,
// specular and fog are left out here.
@fragment
fn fragment_lightmapped(in: Varyings) -> @location(0) vec4<f32> {
    let albedo = textureSample(colour_map, colour_sampler, in.tex);
#ifdef ALPHA_TEST
    // The game's alpha-tested shaders discard below 128/255.
    if (albedo.a < 0.501961) {
        discard;
    }
#endif
    let texel = raw_texel(albedo.rgb);
#ifdef UNTINTED
    // Layered materials: the vertex colour is the layer weights (green
    // layer 1, blue layer 2), not a tint. As the game's layer shaders: a
    // blend layer mixes in by its alpha times its weight, a multiply layer
    // scales by 1 + weight * (layer - 1), a tested layer replaces the colour
    // where alpha times weight >= 0.5, an add layer adds layer * alpha *
    // weight; all on raw texels, then squared.
    var c = texel;
#ifdef LAYERS
#ifdef LAYER1_BLEND
    let l1 = textureSample(colour_map1, colour_sampler, in.layer_uv.xy);
    c = mix(c, raw_texel(l1.rgb), l1.a * in.tint.g);
#endif
#ifdef LAYER1_MULTIPLY
    let m1 = raw_texel(textureSample(colour_map1, colour_sampler, in.layer_uv.xy).rgb);
    c = c * (1.0 + in.tint.g * (m1 - 1.0));
#endif
#ifdef LAYER2_BLEND
    let l2 = textureSample(colour_map2, colour_sampler, in.layer_uv.zw);
    c = mix(c, raw_texel(l2.rgb), l2.a * in.tint.b);
#endif
#ifdef LAYER2_MULTIPLY
    let m2 = raw_texel(textureSample(colour_map2, colour_sampler, in.layer_uv.zw).rgb);
    c = c * (1.0 + in.tint.b * (m2 - 1.0));
#endif
    // bo2mp: a test layer (`t1c1`, the `lit_sm_*_t1c1n1` pixel shader):
    // the layer replaces the colour where its alpha times its weight is at
    // least one half (its normal map is not drawn).
#ifdef LAYER1_TEST
    let t1 = textureSample(colour_map1, colour_sampler, in.layer_uv.xy);
    c = select(c, raw_texel(t1.rgb), t1.a * in.tint.g >= 0.5);
#endif
    // bo2mp: an add layer (`aK`, the `lit_sm_*_a1c1` pixel shader): the
    // layer times its alpha times its weight, added before squaring.
#ifdef LAYER1_ADD
    let a1 = textureSample(colour_map1, colour_sampler, in.layer_uv.xy);
    c = c + raw_texel(a1.rgb) * a1.a * in.tint.g;
#endif
#ifdef LAYER2_ADD
    let a2 = textureSample(colour_map2, colour_sampler, in.layer_uv.zw);
    c = c + raw_texel(a2.rgb) * a2.a * in.tint.b;
#endif
#endif
    let base = c * c;
    let out_a = albedo.a;
#else
    let tinted = texel * in.tint.rgb;
    let base = tinted * tinted;
    // As BO2's lit world shaders write it: the colour map's alpha times
    // the vertex alpha (a decal's painted fade), then sqrt, as the colour.
    let out_a = sqrt(albedo.a * in.tint.a);
#endif
    let v = in.lightmap_uv.y / 3.0;
    let page0 = textureSample(lightmap, lightmap_sampler, vec2(in.lightmap_uv.x, v));
    let page1 = textureSample(lightmap, lightmap_sampler, vec2(in.lightmap_uv.x, v + 1.0 / 3.0));
    let page2 = textureSample(lightmap, lightmap_sampler, vec2(in.lightmap_uv.x, v + 2.0 / 3.0));
    let ambient = page0.rgb / (page0.a + 0.000001);
    let directional = page1.rgb / (page1.a + 0.000001);
    let dir = page2.rgb * 2.0 - 1.0;
#ifdef SHINE
#ifdef FLOW
    // bo2mp: Black Ops II's emissive flow (`bo2_flow`).
    return vec4(
        bo2_out(bo2_flow(in.tex, in.tint, normalize(in.nrm), in.tangent, in.binormal, in.world_position)),
        1.0,
    );
#endif
#ifdef WATER
    // bo2mp: Black Ops II's water (`cod7water` pools, `cod7watershore` seas
    // and rivers; their sun-shadow techniques, fxc disassembly). The colour
    // map holds the material's constants (assets lane `water_table`), the
    // normal and specular maps its two normal maps. Four normal samples
    // scrolled over the world's xy by game time make the water's normal
    // (as the game's, no tangent frame: water lies flat); fresnel (bias and
    // scale on (1 - n.v)^5) mixes its colour with the reflection probe
    // (the sea: its own reflection cube),
    // relit by the light here over the light the probe saw straight up;
    // the sun adds two highlight lobes. The game refracts the scene behind
    // by the water's opacity; so does REFRACT (pools and ponds), else the
    // water blends over it.
    let wl0 = textureLoad(colour_map, vec2<i32>(0, 0), 0);
    let wl1 = textureLoad(colour_map, vec2<i32>(1, 0), 0);
    let wl2 = textureLoad(colour_map, vec2<i32>(2, 0), 0);
    let wl3 = textureLoad(colour_map, vec2<i32>(3, 0), 0);
    let wns = textureLoad(colour_map, vec2<i32>(4, 0), 0);
    let wlw = textureLoad(colour_map, vec2<i32>(5, 0), 0);
    let wc0 = textureLoad(colour_map, vec2<i32>(6, 0), 0);
    let wc1 = textureLoad(colour_map, vec2<i32>(7, 0), 0);
    let wca = textureLoad(colour_map, vec2<i32>(8, 0), 0);
    let wcb = textureLoad(colour_map, vec2<i32>(9, 0), 0);
    let wsh = textureLoad(colour_map, vec2<i32>(10, 0), 0);
    let wt = t6_dyn.count.y;
    let wp = in.world_position.xy;
    let na = textureSample(normal_map, colour_sampler, wp * wl0.xy + wt * wl0.zw).xy * wns.x + wns.y;
    let nb = textureSample(normal_map, colour_sampler, wp * wl1.xy + wt * wl1.zw).xy * wns.x + wns.y;
    let nc = textureSample(specular_map, colour_sampler, wp * wl2.xy + wt * wl2.zw).xy * wns.z + wns.w;
    let nd = textureSample(specular_map, colour_sampler, wp * wl3.xy + wt * wl3.zw).xy * wns.z + wns.w;
    let wsum = na * wlw.x + nb * wlw.y + nc * wlw.z + nd * wlw.w;
    let wn = normalize(vec3(wsum, 1.0));
    let wv = normalize(in.world_position - cam.world_position);
    let wndotv = saturate(dot(wn, -wv));
    let wf = 1.0 - wndotv;
    let wfres = wc0.w * wf * wf * wf * wf * wf + wc0.z;
    var wray = primary_light_ray(in.light_index, in.world_position);
    let wamb = ambient + directional * 0.25;
    let wr = wv - 2.0 * dot(wv, wn) * wn;
    let wenv = textureSampleLevel(probe_cubes, probe_sampler, wr, i32(in.probe), 0.0);
    let wpi = min(in.probe, 31u) * 3u;
    let ws0 = probes.v[wpi];
    let ws1 = probes.v[wpi + 1u];
    let wprobe = max((ws1.z + ws1.w + ws0.w) * ws0.xyz, vec3(0.1));
    // The pool relights the probe by the light here over the probe's own;
    // the sea reflects its sky: its own cube where it has one (one of the
    // probe array's last slots, texel 11.w counted from the end; texels
    // 11..13), squared, faded to the horizon colour with distance and
    // scaled, as BO2's sea; else the probe's.
    let wenv_rgb = wenv.rgb / (wenv.a + 0.000001);
    let wcube_h = textureLoad(colour_map, vec2<i32>(11, 0), 0);
    let wcube_s = textureLoad(colour_map, vec2<i32>(12, 0), 0);
    let wcube_f = textureLoad(colour_map, vec2<i32>(13, 0), 0);
    let wlayer = i32(textureNumLayers(probe_cubes)) - max(i32(wcube_h.w + 0.5), 1);
    let wsky = textureSample(probe_cubes, probe_sampler, wr, wlayer).rgb;
    // bo2mp: Concert's pond (`cod7waterflowreflect`, kind 3; its technique
    // 4, fxc disassembly). Three normal samples (Lo, Lo, Hi) summed, scaled
    // twice by normalMapScale: one normal for the fresnel, one to reflect
    // by; its own cube at (x, y, |z|), squared, times probeScale; the body
    // the scatter colour mixed to the water colour by the fresnel, lit by
    // the lightmap (main page plus a quarter of the second, as the pool).
    // The game draws water = f * reflection + (1 - f) * mix(scene behind,
    // light * body, vertex alpha * probeScale.w); here as one blend over
    // the scene (REFRACT: as the game, below). Without it: no refraction offset of the scene behind, the
    // screen-space reflection rays (the cube is the shader's own answer
    // where they miss) and the depth soft edge.
    let wpk = wlw.x + wlw.y + wlw.z + wlw.w;
    let wpf = normalize(vec3(wsum * wsh.x + wpk * wsh.y, 1.0));
    let wpr = normalize(vec3(wsum * wsh.z + wpk * wsh.w, 1.0));
    let wpd = wv - 2.0 * dot(wv, wpr) * wpr;
    let wpcube = textureSample(probe_cubes, probe_sampler, vec3(wpd.xy, abs(wpd.z)), wlayer).rgb;
    let wpo = 1.0 - saturate(dot(wpf, -wv));
    let wpfres = saturate(wc0.w * wpo * wpo * wpo * wpo * wpo + wc0.z);
    let wpa = saturate(in.tint.a * wcube_s.w);
    let wpbody = mix(wca.rgb, wcb.rgb, wpfres) * wamb;
    let wpalpha = 1.0 - (1.0 - wpfres) * (1.0 - wpa);
    let wprgb = (wpfres * wpcube * wpcube * wcube_s.rgb + (1.0 - wpfres) * wpa * wpbody) / max(wpalpha, 0.0001);
    // The sea's control map and shore picture, sampled here in uniform
    // flow (texels 14 and 15; unused by every other water).
    let wsc = textureLoad(colour_map, vec2<i32>(15, 0), 0);
    let wsuv = textureLoad(colour_map, vec2<i32>(14, 0), 0);
    let wctl_s = textureSample(aux_map, colour_sampler, in.tex + wn.xy * wsc.x);
    let wsp = textureSample(shore_map, colour_sampler,
        wp * wsuv.xy + wsuv.zw + wn.xy * (wsc.y * sqrt(saturate(wctl_s.w)))).rgb;
    let wshore_on = textureDimensions(aux_map).x > 1u && textureDimensions(shore_map).x > 1u;
    // The sea's white water (texels 18..24): two noise uvs, each read at
    // two mip steps by the height under the eye and blended, multiplied
    // and normalised into four weights for four mask reads.
    let wnsc = textureLoad(colour_map, vec2<i32>(18, 0), 0);
    let wnsr = textureLoad(colour_map, vec2<i32>(19, 0), 0);
    let wmsc01 = textureLoad(colour_map, vec2<i32>(20, 0), 0);
    let wmsc23 = textureLoad(colour_map, vec2<i32>(21, 0), 0);
    let wmsr01 = textureLoad(colour_map, vec2<i32>(22, 0), 0);
    let wmsr23 = textureLoad(colour_map, vec2<i32>(23, 0), 0);
    let wuvn = wp.xyxy * wnsc + wt * wnsr;
    let wuvm01 = wp.xyxy * wmsc01 + wt * wmsr01;
    let wuvm23 = wp.xyxy * wmsc23 + wt * wmsr23;
    let wlod = max(log2(max((in.world_position.z - cam.world_position.z) * 0.02, 1e-30)), 0.0);
    let wlodi = floor(wlod);
    let wlodf = saturate(wlod - wlodi);
    let wk0 = exp2(-wlodi);
    let wk1 = exp2(-(wlodi + 1.0));
    let wna = mix(textureSample(foam_noise, colour_sampler, wuvn.xy * wk0),
        textureSample(foam_noise, colour_sampler, wuvn.xy * wk1), wlodf);
    let wnb = mix(textureSample(foam_noise, colour_sampler, wuvn.zw * wk0),
        textureSample(foam_noise, colour_sampler, wuvn.zw * wk1), wlodf);
    let wmask = vec4(
        textureSample(foam_mask, colour_sampler, wuvm01.xy).w,
        textureSample(foam_mask, colour_sampler, wuvm01.zw).y,
        textureSample(foam_mask, colour_sampler, wuvm23.xy).w,
        textureSample(foam_mask, colour_sampler, wuvm23.zw).y);
    let wfoam_on = wshore_on && textureDimensions(foam_noise).x > 1u && textureDimensions(foam_mask).x > 1u;
#ifdef REFRACT
    // bo2mp: the pond as BO2 draws it: three screen-space reflection rays
    // (texel 7: their lengths) read the frame, faded where they leave the
    // screen to its own cube; the scene behind offset by the reflecting
    // normal (texel 13.y pixels) times the thickness (13.x a unit), which
    // also fades the water in from its shoreline.
    if (wc1.w > 2.5) {
        let wq = in.world_position;
        let h0 = ssr_hit(wq + wpd * wc1.x, 0.95, 20.0);
        let h1 = ssr_hit(wq + wpd * wc1.y, 0.9625, 26.666666);
        let h2 = ssr_hit(wq + wpd * wc1.z, 0.975, 40.0);
        let wscube = wpcube * wpcube * wcube_s.rgb * ((h0.z + h1.z + h2.z) / 3.0);
        let wsr = (scene_encoded(h0.xy) * (1.0 - h0.z) + scene_encoded(h1.xy) * (1.0 - h1.z)
            + scene_encoded(h2.xy) * (1.0 - h2.z)) / 3.0;
        let wsrefl = wsr * wsr / max(t6.sun_dir_exposure.w, 0.000001) + wscube;
        let wthick = scene_thickness(in.clip_pos, wq, wcube_f.x);
        let wrefr = scene_light(in.clip_pos.xy + wpr.xy * (wcube_f.y * wthick));
        let wpw = wpfres * wsrefl + (1.0 - wpfres) * mix(wrefr, wpbody, wpa);
        return vec4(bo2_out(mix(wrefr, bo2_fog(wpw, wq), wthick)), 1.0);
    }
#endif
    if (wc1.w > 2.5) {
        return vec4(bo2_out(bo2_fog(wprgb, in.world_position)), wpalpha);
    }
    let wfade = min(exp(length(in.world_position - cam.world_position) * wcube_f.x + wcube_f.y), 1.0);
    let wown = mix(wcube_h.rgb, wsky * wsky, wfade) * wcube_s.rgb;
    let wrefl = select(select(wenv_rgb * wamb / wprobe, wenv_rgb, wc1.w > 0.5), wown, wcube_h.w > 0.5);
    let wbody = mix(wca, wcb, wndotv);
    let wsea = wc1.w > 0.5;
    // The sun: the pool's by its baked visibility; the sea's at least
    // three quarters (its shadow term plus 0.75); a sea whose shader takes
    // no shadow (texel 13.z), all of it, and the scene's sun whatever
    // primary light its surface names (the shader reads the per-scene sun
    // constants; Hijacked's distant sea names none).
    if (wsea && wcube_f.z > 0.5) {
        wray.dir = t6.sun_dir_exposure.xyz;
        wray.colour = t6.sun_color.rgb;
    }
    let wndotl = saturate(dot(wn, wray.dir));
    let wshadow = select(select(page2.a, saturate(page2.a + 0.75), wsea), 1.0, wcube_f.z > 0.5);
    let wsun = wray.colour * wshadow;
    var wsolid: vec3<f32>;
    var wadd: vec3<f32>;
    if (wsea) {
        // bo2mp: the sea's control map (aux_map, at the surface's own uv
        // pushed by the normal times texel 15.x) picks deep from shallow by
        // its green (body and scatter: base + delta * g, deltas texels 16
        // and 17) and water from shore by its alpha; the shore picture
        // (shore_map, at the world's xy by texel 14, pushed by the normal
        // times texel 15.y * sqrt(alpha)), squared, lit by the sun over the
        // view's slope plus the probe term, tinted by the body as the water
        // deepens. A sea with no control map (1x1) keeps g = 1, alpha = 1.
        var wscat = wca.rgb;
        var wdeep = wcb.rgb;
        var wctl = vec4(1.0);
        if (wshore_on) {
            wctl = wctl_s;
            let wbd = textureLoad(colour_map, vec2<i32>(16, 0), 0).rgb;
            let wsd = textureLoad(colour_map, vec2<i32>(17, 0), 0).rgb;
            wdeep = wcb.rgb + wbd * (wctl.y - 1.0);
            wscat = wca.rgb + wsd * (wctl.y - 1.0);
        }
        let wlit = wsun * (wndotl / max(wndotv + wndotl, 0.0001)) + wsh.rgb;
        var wcol = mix(wscat, wdeep, wfres);
        if (wfoam_on) {
            // BO2's white water: the noise weights pick a mask mix m; one
            // foam by the control map's red (2r, smoothed) times sqrt(m)
            // darkens the body by whiteWaterColor.w; one where m^1.5 * 0.65
            // plus red passes a quarter adds whiteWaterColor by m.
            let wwc = textureLoad(colour_map, vec2<i32>(24, 0), 0);
            var wnz = wna * wnb + 0.25;
            wnz = wnz / dot(wnz, vec4(1.0));
            let wm = saturate(dot(wnz, wmask));
            let wsa = smoothstep(0.0, 1.0, saturate(2.0 * wctl.x));
            let wsb = smoothstep(0.0, 1.0,
                saturate((saturate(pow(wm, 1.5) * 0.65 + wctl.x) - 0.25) * (4.0 / 3.0)));
            let wfa = wsa * sqrt(wm);
            let wfb = wsb * wm;
            wcol = mix(wcol, saturate(wcol * wwc.w), wfa);
            wcol = mix(wcol, saturate(wfb * wwc.rgb + wcol), wfb);
        }
        let wwater = mix(wcol * wlit, wrefl, wfres);
        if (wshore_on) {
            var wshore = wsp * wsp;
            wshore *= wsun * (wray.dir.z / max(wray.dir.z - wv.z, 0.0001)) + wsh.rgb;
            wshore = mix(wshore, wshore * wdeep, wctl.w);
            wsolid = mix(wshore, wwater, wctl.w);
        } else {
            wsolid = wwater;
        }
        wadd = vec3(0.0);
    } else {
        let wlight = wamb + wsun * wndotl + dyn_lights(in.world_position, wn);
        wsolid = wbody.rgb * (1.0 - wfres) * wlight;
        wadd = wrefl * wfres;
    }
    let wh = normalize(wray.dir - wv);
    let wndoth = max(saturate(dot(wn, wh)), 0.000001);
    let wlobe = exp2(log2(wndoth) * wc0.x) * wc1.x + exp2(log2(wndoth) * wc0.y) * wc1.y;
    let whl = 1.0 - saturate(dot(wh, wray.dir));
    let wfh = wc0.w * whl * whl * whl * whl * whl + wc0.z;
    let whv = max(saturate(dot(wh, -wv)) * 4.0, 0.0001);
    wadd += wsun * (wlobe * wfh * wndotl / whv * wc1.z);
    // The open sea (kind 2) never reads the vertex colour: its far quads
    // carry alpha 0 at their edges (Frostbite's distant sea).
    let wvalpha = select(in.tint.a, 1.0, wc1.w > 1.5);
    let wa = saturate(select(wbody.a, wca.a, wsea) * wvalpha);
#ifdef REFRACT
    // bo2mp: the pool as BO2 draws it: the scene behind offset by the normal
    // (texel 13.y pixels) times the root of the thickness (13.x a unit),
    // under the water by its opacity, fogged, faded in from the shoreline.
    if (!wsea) {
        let wthick = scene_thickness(in.clip_pos, in.world_position, wcube_f.x);
        let wrefr = scene_light(in.clip_pos.xy + wn.xy * (wcube_f.y * sqrt(wthick)));
        let wpool = (1.0 - wa) * wrefr + wa * wsolid + wadd;
        return vec4(bo2_out(mix(wrefr, bo2_fog(wpool, in.world_position), wthick)), 1.0);
    }
#endif
    let wout = wsolid + wadd / max(wa, 0.05);
    return vec4(bo2_out(bo2_fog(wout, in.world_position)), wa);
#endif
    // bo2zm: Black Ops II's lit world shader (`wpc_lit_sm_r0c0n0s0`, its
    // sun-shadow technique, from the fxc disassembly), with the lightmap's
    // baked sun visibility (page 2 alpha) for its shadow map.
    let vn = normalize(in.nrm);
    let nm = textureSample(normal_map, colour_sampler, in.tex).xy * 4.015748 - 2.015748;
    let n = normalize(nm.x * in.tangent + nm.y * in.binormal + vn);
    let spec_tex = textureSample(specular_map, colour_sampler, in.tex);
    let spec_raw = raw_texel(spec_tex.rgb);
    let spec = spec_raw * spec_raw;
    let gloss = spec_tex.a;
    let vdir = normalize(in.world_position - cam.world_position);
    var light = ambient + directional * saturate(dot(dir, n));
    // The reflection: the surface's probe by the reflected view, blurrier
    // the lower the gloss, weighted by a gloss-shaped fresnel and scaled by
    // the light here (vertex normal) over the light the probe saw (its SH
    // at the normal).
    let ndotv = saturate(dot(n, -vdir));
    let fk = gloss * vec4(1.041667, 0.475, 0.018229, 0.25) + vec4(0.0, 0.0, -0.015625, 0.75);
    var f = min(exp2(-9.28 * ndotv), fk.y);
    f = fk.x * f + fk.z;
    let env_weight = saturate(spec * (fk.w - f) + f);
    let r = vdir - 2.0 * dot(vdir, n) * n;
    let env = textureSampleLevel(probe_cubes, probe_sampler, r, i32(in.probe), 4.0 - 4.0 * gloss);
    let env_rgb = env.rgb / (env.a + 0.000001);
    let pi = min(in.probe, 31u) * 3u;
    let s0 = probes.v[pi];
    let s1 = probes.v[pi + 1u];
    let s2 = probes.v[pi + 2u];
    let sh = dot(s2, vec4(vn.z * vn.x, vn.y * vn.z, vn.x * vn.y, vn.x * vn.x - vn.y * vn.y))
        + dot(s1, vec4(vn, 1.0)) + s0.w * vn.z * vn.z;
    let probe_light = max(sh * s0.xyz, vec3(0.1));
    let here = ambient + directional * saturate(dot(dir, vn));
    let reflection = env_weight * env_rgb * here / probe_light;
    // The primary light: diffuse, and its highlight (normalised
    // Blinn-Phong by gloss, BO2's visibility and fresnel terms).
    let ray = primary_light_ray(in.light_index, in.world_position);
    let lifted = ray.colour * page2.a;
    let ndotl = saturate(dot(n, ray.dir));
    light += lifted * ndotl;
    light += dyn_lights(in.world_position, n);
    let h = normalize(ray.dir - vdir);
    let ndoth = saturate(dot(n, h));
    let ldoth = saturate(dot(h, ray.dir));
    let power = exp2(gloss * 13.0);
    let lobe = exp2(log2(max(ndoth, 0.000001)) * power) * (power * 0.125 + 0.25);
    let k = saturate(gloss + 0.545);
    let vis = ndotl / (ldoth * ldoth * k - k + 1.01);
    let fres = spec + (1.0 - spec) * exp2(-10.0 * ldoth);
    let highlight = fres * lobe * vis * lifted;
    let colour = base * light + reflection + highlight;
    return vec4(bo2_out(bo2_fog(colour, in.world_position)), out_a);
#else
    let n = normalize(in.nrm);
    var light = ambient + directional * saturate(dot(dir, n));
    light += primary_light(in.light_index, in.world_position, n) * page2.a;
    light += dyn_lights(in.world_position, n);
    return vec4(bo2_out(bo2_fog(base * light, in.world_position)), out_a);
#endif
}

// bo2zm: Black Ops II props. The instance carries its world-from-local
// columns, its light: rgb from the light grid (linear, already times the
// exposure, w = 0), or for an instance with baked vertex light w = the
// exposure, the vertex colour then holding that light ((c^2) * 32, as the
// game's mlv_* shaders read it); then its primary light and that light's
// visibility from the light grid.
struct PropVertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) tex: vec2<f32>,
    @location(6) model_col0: vec4<f32>,
    @location(7) model_col1: vec4<f32>,
    @location(8) model_col2: vec4<f32>,
    @location(9) model_col3: vec4<f32>,
    @location(10) light: vec4<f32>,
    @location(11) primary: vec4<f32>,
    // bo2mp: a swaying canopy's treeCanopySwayParms xyz (else zero).
    @location(12) sway: vec4<f32>,
}

struct PropOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) nrm: vec3<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) tex: vec2<f32>,
    @location(3) light: vec4<f32>,
    @location(4) world_position: vec3<f32>,
    @location(5) primary: vec4<f32>,
#ifdef FOLIAGE
    // bo2mp: a tree canopy's scatter colour (rgb) and sun visibility (w);
    // its colour tint (rgb) and the model's own vertex alpha (w), kept
    // apart from `tint`, which with baked vertex light carries that light.
    @location(6) canopy: vec4<f32>,
    @location(7) canopy_tint: vec4<f32>,
#endif
    // bo2mp: the placed model's own light grid set: its handle plus one
    // (instance slot 27), 0 for none.
    @location(8) @interpolate(flat) model_light: f32,
#ifdef DECAL
    // A projected decal's box from the world (columns).
    @location(9) @interpolate(flat) box0: vec4<f32>,
    @location(10) @interpolate(flat) box1: vec4<f32>,
    @location(11) @interpolate(flat) box2: vec4<f32>,
    @location(12) @interpolate(flat) box3: vec4<f32>,
#endif
#ifdef GLASS
    // codmp: the glass material's own amounts and map scales (instance
    // slots 24-26, see `glass_uv`; 0 = the riot shield's, all 1).
    @location(13) @interpolate(flat) glass: vec3<f32>,
#endif
}

#ifdef FLARE
// bo2mp: Black Ops II's lens flare (`sw4_3d_superflare_mp_nuketown_sun1`
// vertex and pixel shaders, fxc disassembly). One flare is 25 ghost quads,
// all at the effect's origin; the vertex red (times 255) picks the ghost.
// Each ghost takes its own sprite cell, and its size and colour from how far
// the origin sits from the screen centre (`edge`, the larger of |x| and |y|);
// the ring of small ghosts sits along the line from the origin through the
// centre. FlareIntensity and FlareScale are instance slots 16 and 17 (see the
// extraction). The quad is drawn over everything (depth 1, w 1, as the
// game's). Terms and constants as the game has them.
const FLARE_TABLE = array<vec4<f32>, 17>(
    vec4(0.878811, 0.997223, 0.179846, 0.217048),
    vec4(0.880245, 0.580889, 0.770531, 0.170843),
    vec4(0.703604, 0.999176, 0.480728, 0.280221),
    vec4(0.824610, 0.187964, 0.047456, 0.222877),
    vec4(0.169836, 0.637135, 0.334422, 0.002045),
    vec4(0.201270, 0.636952, 0.914853, 0.495102),
    vec4(0.895505, 0.915860, 0.108280, 0.040040),
    vec4(0.354900, 0.820887, 0.265328, 0.999207),
    vec4(0.639699, 0.322947, 0.973998, 0.004303),
    vec4(0.734153, 0.383312, 0.842708, 0.471297),
    vec4(0.166540, 0.0, 0.0, 0.669393),
    vec4(0.631855, 0.0, 0.0, 0.888211),
    vec4(0.246712, 0.0, 0.0, 0.647511),
    vec4(0.529954, 0.0, 0.0, 0.936430),
    vec4(0.101993, 0.0, 0.0, 0.0),
    vec4(0.477554, 0.0, 0.0, 0.0),
    vec4(0.511521, 0.0, 0.0, 0.0),
);

fn flare_smooth(t: f32) -> f32 {
    return t * t * (3.0 - 2.0 * t);
}

fn flare_vertex(in: PropVertexIn, world: vec4<f32>) -> PropOut {
    var out: PropOut;
    out.clip_pos = vec4(-1000.0);
    out.world_position = world.xyz;
    let g = floor(in.tint.x * 255.0);
    let clip = cam.clip_from_world * world;
    if (g >= 25.0 || clip.w < 0.0) {
        return out;
    }
    let i = u32(clamp(g, 0.0, 24.0));
    let fi = in.light.x;
    let fs = in.light.y;
    let aspect = cam.viewport.w / cam.viewport.z;
    let ndc = clip.xy / clip.w;
    let s = vec2(ndc.x, -ndc.y);
    let edge = max(abs(s.x), abs(s.y));

    var uv = in.tex * 0.25;
    var centre = s;
    var size = vec2(0.0);
    var colour = vec3(0.0);
    var channel = vec4(0.0, 1.0, 0.0, 0.0);
    var grime = vec2(1.0, 0.0);
    var angle = 0.0;
    var whole = 0.0;
    if (i < 2u) {
        // The two crossed streaks through the origin.
        uv = in.tex * 0.25 + vec2(0.25, 0.5);
        let k = min(pow(flare_smooth(min(edge * 0.4, 1.0)), 0.7), 1.0) * 2.6 + 0.4;
        size = vec2(aspect * k * 0.9 * fs, k * 0.3 * fs);
        let fade = flare_smooth(min(edge * 2.0, 1.0));
        colour = vec3((1.0 - min(fade * fade, 1.0)) * 0.75);
        channel = vec4(0.0);
        angle = select(45.0, -45.0, i == 0u);
        whole = 1.0;
    } else if (i < 4u) {
        // The two rings: one opposite the origin (1.2 times out), one on it.
        uv = in.tex * 0.25 + vec2(0.25, 0.0);
        if (i == 2u) {
            centre = vec2(ndc.x * -1.2, ndc.y * 1.2);
        }
        let grow = flare_smooth(min(edge, 1.0));
        let r = 1.5 * fs * min(pow(grow, 5.0), 1.0);
        size = vec2(aspect * r, r);
        let a = flare_smooth(saturate((edge - 0.75) * 10.0));
        let b = flare_smooth(saturate((edge - 0.85) * 6.666667));
        let tint = select(vec3(2.835, 2.799396, 2.025), vec3(2.835, 2.829438, 2.32875), i == 2u);
        colour = min(pow(a, 5.0), 1.0) * (1.0 - min(b, 1.0)) * tint;
        grime = vec2(0.333, 0.0);
    } else if (i < 21u) {
        // The small ghosts along the line through the centre: ten warm, then
        // seven blue.
        let fade_in = min(flare_smooth(min(edge * 2.0, 1.0)), 1.0);
        let fade_out = 1.0 - min(flare_smooth(saturate((edge - 0.8) * 5.0)), 1.0);
        let grow = min(flare_smooth(min(edge * 4.0, 1.0)), 1.0);
        var p = 0.0;
        var r = 0.0;
        if (i < 14u) {
            let row = FLARE_TABLE[i - 4u];
            uv = in.tex * 0.125 + vec2(0.375, 0.125);
            p = (row.x * -2.05 + 0.8) * 0.5 + 0.5;
            r = (row.y * 0.18 + 0.02) * fs * grow;
            colour = (row.z * 0.29 + 0.01) * vec3(1.0, 0.984301, 0.964286) * fade_in * fade_out;
        } else {
            let j = i - 14u;
            uv = in.tex * 0.125 + vec2(0.125, 0.125);
            p = (FLARE_TABLE[j].w * -3.5 + 0.5) * 0.5 + 0.5;
            r = (FLARE_TABLE[j + 7u].w * 0.1 + 0.1) * fs * grow;
            colour = (FLARE_TABLE[j + 10u].x * 0.3 + 0.3) * vec3(0.571429, 0.844584, 1.0) * fade_in * fade_out;
        }
        centre = vec2(2.0 * p * ndc.x, -2.0 * p * ndc.y) - s;
        size = vec2(aspect * r, r);
        channel = vec4(1.0, 0.0, 0.0, 0.0);
    } else if (i == 21u) {
        // The flat streak, widest at the centre.
        uv = in.tex * 0.25 + vec2(0.25, 0.0);
        let t = min(flare_smooth(min(abs(s.x) * 1.333333, 1.0)), 1.0);
        size = vec2(aspect * (1.0 - t) * 2.5 * fs, 0.05 * fs);
        colour = (1.0 - 0.6 * t) * vec3(0.346204, 0.265402, 0.352632);
    } else if (i == 22u) {
        // The flat streak near the screen's sides.
        uv = in.tex * 0.25 + vec2(0.25, 0.0);
        let t = min(flare_smooth(min(abs(s.x), 1.0)), 1.0);
        size = vec2(aspect * t * 4.5 * fs, 0.1 * fs);
        colour = min(flare_smooth(saturate((abs(s.x) - 0.7) * 5.0)), 1.0)
            * (1.0 - min(flare_smooth(saturate((edge - 0.9) * 10.0)), 1.0))
            * vec3(0.267857, 0.368524, 0.75);
    } else if (i == 23u) {
        // The core.
        size = vec2(aspect * 0.15 * fs, 0.15 * fs);
        colour = (min(flare_smooth(max(1.0 - edge, 0.0)), 1.0) * 0.25 + 0.75) * vec3(0.984211, 0.973395, 0.843609);
    } else {
        // The glow, under the grime.
        size = vec2(aspect * 1.5 * fs, 1.5 * fs);
        colour = vec3(0.702857, 0.785243, 0.82);
        grime = vec2(0.0, 1.0);
    }

    let rgb = colour * fi * in.tint.w;
    if (dot(rgb, vec3(0.333, 0.333, 0.334)) < 0.0038) {
        return out;
    }
    let c = (in.tex * 2.0 - 1.0) / vec2(aspect, 1.0) * size;
    let turn = angle * 0.017453;
    let sn = sin(turn);
    let cs = cos(turn);
    let q = vec2(cs * c.x + sn * c.y, -sn * c.x + cs * c.y) * vec2(aspect, 1.0) + centre;
    out.clip_pos = vec4(q.x, -q.y, 1.0, 1.0);
    out.light = vec4(grime, q * 0.5 + 0.5);
    out.tint = vec4(rgb, whole);
    out.tex = uv;
    out.primary = channel;
    return out;
}
#endif

@vertex
fn vertex_prop(in: PropVertexIn) -> PropOut {
    let model_mat = mat4x4<f32>(
        in.model_col0,
        in.model_col1,
        in.model_col2,
        in.model_col3,
    );
    var out: PropOut;
#ifdef FLARE
    return flare_vertex(in, model_mat * vec4(in.pos, 1.0));
#else
#ifdef CLOUD
    // bo2zm: Black Ops II particle clouds (`particlecloud_*` vertex shader):
    // the point of the cloud's box to the world by the cloud's placement,
    // then out to a quad facing the camera, the corner (texcoord less one
    // half) times the particle's width and height (instance primary.xy);
    // the colour is the cloud's (instance light), and nothing feathers it.
    let centre = model_mat * vec4(in.pos, 1.0);
    let corner = in.tex - vec2(0.5);
    let right = cam.world_from_view[0].xyz;
    let up = cam.world_from_view[1].xyz;
    let world = vec4(centre.xyz + right * (corner.x * in.primary.x) - up * (corner.y * in.primary.y), 1.0);
    out.clip_pos = cam.clip_from_world * world;
    out.nrm = cam.world_from_view[2].xyz;
    out.tint = in.light;
    out.tex = in.tex;
    out.light = vec4(0.0);
    out.world_position = world.xyz;
    out.primary = vec4(0.0);
#else
    var world = model_mat * vec4(in.pos, 1.0);
#ifdef EFFECT
    // bo2zm M3: BO2's eye offset (an `_eo` effect material's
    // eyeOffsetParms.x, instance slot 23): the sprite is drawn that many
    // units nearer the eye along its line of sight (at least half way),
    // where it already shows on screen, so a flame on a wall or a pole is
    // not cut into by it.
    let eo = in.primary.w;
    if (eo > 0.0) {
        let to_sprite = world.xyz - cam.world_position;
        let d = max(length(to_sprite), 0.0001);
        world = vec4(cam.world_position + to_sprite * (max(d - eo, d * 0.5) / d), 1.0);
    }
#endif
#ifdef DECAL
    // bo2zm M4 retest 5: a projected decal's box is already in the world;
    // its instance columns are the box from the world, for the pixels.
    world = vec4(in.pos, 1.0);
    out.box0 = in.model_col0;
    out.box1 = in.model_col1;
    out.box2 = in.model_col2;
    out.box3 = in.model_col3;
#endif
    out.clip_pos = cam.clip_from_world * world;
    out.nrm = (model_mat * vec4(in.nrm, 0.0)).xyz;
    out.tint = in.tint;
    out.tex = in.tex;
    out.light = in.light;
    out.world_position = world.xyz;
    out.primary = in.primary;
    out.model_light = in.sway.w;
#ifdef GLASS
    out.glass = in.sway.xyz;
#endif
#ifdef FOLIAGE
    // bo2mp: Black Ops II's tree canopy (`mc_treecanopy`, `mlv_treecanopy`
    // vertex shaders, fxc disassembly): the normal turned toward the line
    // out from the model's origin by the bend (treeCanopyLightingParms.x);
    // the sun's visibility from the light grid or, with baked vertex light,
    // that light's alpha. A canopy row (its probe slot negative) carries the
    // material's constants (see `t6_canopy_params`); any other row draws
    // with no bend, white scatter and white tint.
    var own = in.tint.a;
    var own_rgb = in.tint.rgb;
    var vis = in.primary.y;
    if (in.tint.a > 1.5) {
        // Baked: 2 + the model's own alpha byte + 256 * the visibility byte;
        // each of rgb the model's own byte + the light's byte / 256.
        let code = u32(in.tint.a - 1.5);
        own = f32(code & 255u) / 255.0;
        vis = f32(code >> 8u) / 255.0;
        own_rgb = floor(in.tint.rgb) / 255.0;
        out.tint = vec4(fract(in.tint.rgb) * (256.0 / 255.0), in.tint.a);
    }
    // bo2mp: the swaying canopy (`*_treecanopy_sm_*` vertex shaders, fxc
    // disassembly): the vertex's own red picks one of sixteen wind variants,
    // whose rustle times treeCanopySwayParms.z turns the point about the
    // model's origin (the game's own rotation, terms as it has them); the
    // turn shows by sway.x + (1 - sway.x) * (own green * sway.y), and the
    // placed point moves that far along the wind. The normal stays.
    let sway = in.sway.xyz;
    if (sway.x != 0.0 || sway.y != 0.0) {
        let variant = clamp(i32(floor(own_rgb.r * 15.0)), 0, 15);
        let turn = sway.z * t6_dyn.wind[1 + variant].yzw;
        let s = sin(turn);
        let c = cos(turn);
        let r0 = vec3(c.y * c.z, -c.y * s.z, s.y);
        let r1 = vec3(c.x * s.z - s.x * s.y * s.z, c.x * c.z - s.x * s.y * s.z, -s.x * c.y);
        let r2 = vec3(s.x * s.z - s.y * c.x * c.z, s.y * c.x * s.z + s.x * c.z, c.x * c.y);
        let p = in.pos;
        let turned = vec3(dot(p, r0), dot(p, r1), dot(p, r2));
        let blend = sway.x + (1.0 - sway.x) * (own_rgb.g * sway.y);
        var moved = model_mat * vec4(p + blend * (turned - p), 1.0);
        let along = t6_dyn.wind[0].xyz;
        if (dot(along, along) > 0.0) {
            moved = vec4(moved.xyz + normalize(along) * blend, moved.w);
        }
        out.clip_pos = cam.clip_from_world * moved;
        out.world_position = moved.xyz;
    }
    var bend = 0.0;
    var scatter = vec3(1.0);
    var tint = vec3(1.0);
    if (in.primary.w < 0.0) {
        let p = u32(-in.primary.w - 0.5);
        let t = u32(in.primary.x + 0.5);
        bend = f32(p & 63u) / 63.0;
        scatter = vec3(f32((p >> 6u) & 63u), f32((p >> 12u) & 63u), f32((p >> 18u) & 63u)) / 63.0;
        tint = vec3(f32(t & 63u), f32((t >> 6u) & 63u), f32((t >> 12u) & 63u)) / 63.0;
    }
    let out_dir = (model_mat * vec4(in.pos, 0.0)).xyz;
    let out_len = length(out_dir);
    if (out_len > 0.0001) {
        out.nrm = mix(out.nrm, out_dir / out_len, bend);
    }
    out.canopy = vec4(scatter, vis);
    out.canopy_tint = vec4(tint, own);
#endif
#endif
    return out;
#endif
}

// codmp: Black Ops II's scaled glass (`sw4_3d_glass_scaled`, a broken
// window's shards; fxc disassembly) samples each map at its texcoord scaled
// about the centre, (uv - 0.5) * scale + 0.5: the colour map by
// Color_Map_Size_Scale (`which` 0), the normal map by Normal_Map_Size_Scale
// (1), the specular-gloss map by SpecGloss_Map_Size_Scale (2). `code` is
// instance slot 26: 1 + each scale times 16, eight bits apiece; 0 = all 1.
fn glass_uv(uv: vec2<f32>, code: f32, which: u32) -> vec2<f32> {
    if (code < 0.5) {
        return uv;
    }
    let scale = f32(((u32(code + 0.5) - 1u) >> (8u * which)) & 255u) / 16.0;
    return (uv - 0.5) * scale + 0.5;
}

@fragment
fn fragment_prop(in: PropOut) -> @location(0) vec4<f32> {
#ifdef FLARE
    // bo2mp: the lens flare's pixels (see `flare_vertex`): the sprite cell,
    // whole or one channel of it, times the ghost's colour, times the grime
    // map at the quad's screen point (bias + map * scale), a fifth of it.
    let spr = textureSample(colour_map, colour_sampler, in.tex);
    let col = select(vec4(dot(in.primary, spr)), spr, in.tint.w != 0.0) * in.tint;
    let grime = textureSample(colour_map1, colour_sampler, in.light.zw) * in.light.y + in.light.x;
    // Its alpha is 0 for every one-channel ghost, so the game's blend adds
    // the colour as it is (alpha 1 under our additive blend).
    let c = saturate(col * grime) * 0.2;
    return vec4(bo2_added(c.rgb), 1.0);
#else
#ifdef GHOST
    // bo2zm: Black Ops II's ghost shader (`sw4_3d_char_ghost2`, from the
    // fxc disassembly; Mob's Afterlife ghost hands), with the material's
    // own numbers: two edge noises drifting over the surface (the second
    // turned, here the colour map's again), their product shaping a rim
    // (facing the eye) and a hollow (edge-on) term; the alpha is the
    // smaller of the two times Overall_Intensity, square-rooted. The colour
    // is (Inside_Color + Inside_Add) times the vertex colour, squared,
    // times the light, times Overall_Intensity: a bright cyan glow.
    let t = t6_dyn.count.y;
    let to_eye = normalize(cam.world_position - in.world_position);
    let ndv = saturate(dot(normalize(in.nrm), to_eye));
    let e2 = raw_texel(textureSample(colour_map, colour_sampler, in.tex * 5.0 + fract(t * vec2(-0.03, 0.01) * 5.0)).rgb).x;
    let d = in.tex - vec2(0.5);
    let turned = vec2(dot(d, vec2(0.982287, -0.187382)), dot(d, vec2(0.187382, 0.982287))) + vec2(0.5);
    let e3 = raw_texel(textureSample(colour_map, colour_sampler, turned * 0.5 + fract(t * vec2(0.05, 0.05) * 0.5)).rgb).x;
    let e = e2 * e3;
    let rim_t = saturate((ndv + 0.02) / 0.73);
    let hollow_t = saturate((ndv - 0.89) / -1.21);
    let rim = rim_t * rim_t * (3.0 - 2.0 * rim_t) * saturate(e * 0.3);
    let hollow = hollow_t * hollow_t * (3.0 - 2.0 * hollow_t) * (e * 1.09);
    let ghost_a = sqrt(max(min(rim, hollow) * 3.99, 0.0));
    let ghost_exposure = max(t6.sun_dir_exposure.w, 0.000001);
    var ghost_light = in.light.rgb / ghost_exposure;
    var ghost_tint = in.tint.rgb;
    if (in.light.w > 0.0) {
        ghost_light = in.tint.rgb * in.tint.rgb * 32.0;
        ghost_tint = vec3(1.0);
    }
    let inside = vec3(0.0, 3.75, 5.0) + vec3(0.5);
    let ghost_c = inside * ghost_tint;
    return vec4(bo2_out(ghost_c * ghost_c * ghost_light * 3.99), saturate(ghost_a));
#else
#ifdef OBJECTIVE
    // bo2zm M3: Black Ops II's objective shader (`mc_objective`, from the
    // fxc disassembly; the magic box's question marks): the colour pulses
    // between colorObjMin and colorObjMax once a second (gameTime),
    // shifted across the surface by how it faces the eye, times one half
    // (its colour map is black), added on (its blend is one, one). The
    // colour map here holds the two colours (texels 0 and 1).
    let lo = textureSampleLevel(colour_map, colour_sampler, vec2(0.25, 0.5), 0.0).rgb;
    let hi = textureSampleLevel(colour_map, colour_sampler, vec2(0.75, 0.5), 0.0).rgb;
    let to_eye = normalize(cam.world_position - in.world_position);
    let nz = -dot(normalize(in.nrm), to_eye);
    let pulse = 0.5 - 0.5 * sin((nz * -0.5 + t6_dyn.count.y) * 6.28318);
    return vec4(bo2_out(mix(lo, hi, pulse) * 0.5), 1.0);
#else
#ifdef ANIM
    let screen = anim_colour(anim_texel(in.tex), in.world_position);
    let layered = three_layer_colour(in.tex, in.world_position);
    return select(screen, layered, textureLoad(colour_map1, vec2(1, 0), 0).y > 0.5);
#else
#ifdef DECAL
    // bo2zm M4 retest 5: Black Ops II's projected decal (`mc_projecteddecal`,
    // from the fxc disassembly; the blood a hit paints on a zombie): the
    // scene behind the box's pixel, from the depth, into the box (-1..1);
    // outside it nothing is drawn. Inside, the texture projected along the
    // box's x axis (at y, z), its alpha fading to the box's x faces, its
    // colour the texture squared times the light grid's light, no fog.
    let scene_d = textureLoad(soft_depth, vec2<i32>(in.clip_pos.xy), 0);
    if (scene_d <= 0.0) {
        discard;
    }
    let suv = (in.clip_pos.xy - cam.viewport.xy) / cam.viewport.zw;
    let scene_clip = vec4(suv.x * 2.0 - 1.0, 1.0 - suv.y * 2.0, scene_d, 1.0);
    let scene_h = cam.world_from_clip * scene_clip;
    let box_from_world = mat4x4<f32>(in.box0, in.box1, in.box2, in.box3);
    let b = (box_from_world * vec4(scene_h.xyz / scene_h.w, 1.0)).xyz;
    if (any(abs(b) > vec3(1.0))) {
        discard;
    }
    let decal = textureSampleLevel(colour_map, colour_sampler, b.yz * 0.5 + 0.5, 0.0);
    let dtex = raw_texel(decal.rgb);
    let decal_exposure = max(t6.sun_dir_exposure.w, 0.000001);
    return vec4(bo2_out(dtex * dtex * in.light.rgb / decal_exposure), (1.0 - abs(b.x)) * decal.a);
#else
#ifdef SCROLL
    // bo2zm 27c: BO2's scrolling cloud sheets (`mc_sw4_3d_scrolling_zfeather`,
    // from the fxc disassembly): the texcoord times ColorScale.x (instance
    // light.w), drifting by the game time times ColorSpeed (light.yz) times
    // ColorScale.x.
    let albedo = textureSample(
        colour_map,
        colour_sampler,
        in.tex * in.light.w + fract(t6_dyn.count.y * in.light.yz * in.light.w),
    );
#else
#ifdef GLASS
    let albedo = textureSample(colour_map, colour_sampler, glass_uv(in.tex, in.glass.z, 0u));
#else
    let albedo = textureSample(colour_map, colour_sampler, in.tex);
#endif
#endif
#ifdef ALPHA_TEST
    // The game's alpha-tested shaders discard below 128/255.
    if (albedo.a < 0.501961) {
        discard;
    }
#endif
    let texel = raw_texel(albedo.rgb);
#ifdef EFFECT
    // bo2zm: Black Ops II effect shaders (`effect_*`): vertex colour times
    // texture, written as is. Their vertex shaders fade the alpha by the
    // view depth times the material's `featherParms.x` (instance slot 22),
    // so a sprite fades out as it nears the camera.
    let feather = in.primary.z;
    let near = select(
        1.0,
        saturate(distance(in.world_position, cam.world_position) * feather),
        feather > 0.0,
    );
    // bo2zm M4 retest 5: the `_ds` (dissolve) ones, from the fxc
    // disassembly: alpha = saturate(alphaDissolveParms.x * (texture alpha +
    // vertex alpha - 1)) (minus instance slot 20, which is otherwise a
    // primary light, 0 or more), so the texture's faint floor never shows
    // and the sprite dissolves as it fades.
    let dissolve = -min(in.primary.x, 0.0);
    var a = albedo.a * in.tint.a * near;
    if (dissolve > 0.0) {
        a = saturate(dissolve * (albedo.a + in.tint.a * near - 1.0));
    }
    // Nothing to blend where the sprite is clear (most of a puff's corners,
    // a near-faded sprite): stop before the depth read and the blend.
    if (a < 0.002) {
        discard;
    }
#ifdef SOFT
    // BO2's zfeather pixel shaders: the alpha fades as the sprite nears the
    // scene behind it, (scene depth - sprite depth) * featherParms.x.
    if (feather > 0.0) {
        a *= soft_fade(in.clip_pos, in.world_position, feather);
        if (a < 0.002) {
            discard;
        }
    }
#endif
    // bo2zm M3: BO2's falloff effect shaders (`falloffParms`, from the fxc
    // disassembly): the colour times f * lerp(end, begin, f), f =
    // saturate(dot(eye ray, normal)^2 * parms.z + parms.w): a sprite (a
    // flame sheet, a zombie's eye beam) fades as it turns edge-on to the eye.
    // Instance slots 16..19 (see the extraction).
    var tint = in.tint.rgb;
    if (in.light.w > 0.5) {
        let ray = normalize(in.world_position - cam.world_position);
        let d = dot(ray, normalize(in.nrm));
        let f = saturate(d * d * in.light.x + in.light.y);
        tint = tint * f * mix(vec3(in.light.z), vec3(1.0), f);
    }
#ifdef EFFECT_ADD
    // Additive ones take no fog (it would add the fog colour over the whole
    // quad); the blend scales the colour by the alpha.
    return vec4(bo2_added(texel * tint), a);
#else
#ifdef EFFECT_MULTIPLY
    return vec4(mix(vec3(1.0), texel * tint, a), a);
#else
    // Blended ones go toward the effect fog by its keep factor.
    let fx_fog = effect_fog(in.world_position);
    let fx = mix(fx_fog.rgb, texel * tint, fx_fog.w);
    return vec4(bo2_final(fx), a);
#endif
#endif
#else
#ifdef UNLIT
#ifdef MULTIPLY
    let a = albedo.a * in.tint.a;
    return vec4(mix(vec3(1.0), texel * in.tint.rgb, a), a);
#else
#ifdef SCROLL
    // A cloud sheet's colour is the texture times hdrAmount, squared
    // (instance primary.x holds hdrAmount squared), no vertex colour; the
    // vertex alpha fades it, and so does how near the scene behind it lies
    // (1 / FadeFactor, instance light.x).
    var sheet_a = albedo.a * in.tint.a;
#ifdef SOFT
    sheet_a *= soft_fade(in.clip_pos, in.world_position, in.light.x);
#endif
    if (sheet_a < 0.002) {
        discard;
    }
    return vec4(
        unlit_out(unlit_fog(texel * texel * in.primary.x, in.world_position)),
        sheet_a,
    );
#else
    // bo2zm round 1: a zombie's eyes (`unlit_cheap_zombie_eyes`): BO2's
    // shader scales the texture by the scriptVector2.y its client script set
    // (2 alive, Origins 3, 0.25 once the eyes go out) before squaring it;
    // it rides negated in instance slot 22, which no other unlit row makes
    // negative.
    var c = texel * in.tint.rgb;
    if (in.primary.z < 0.0) {
        c *= -in.primary.z;
    }
    return vec4(
        unlit_out(unlit_fog(c * c * f32(#{UNLIT_SCALE}), in.world_position)),
        albedo.a * in.tint.a,
    );
#endif
#endif
#else
    // The grid light is already times the exposure; the BO2 output adds it
    // again, so divide it out here.
    let exposure = max(t6.sun_dir_exposure.w, 0.000001);
    var light = in.light.rgb / exposure;
    // bo2mp: a placed model with its own light grid set (see
    // `t6_model_cube`), and its vertex normal.
    let model_light = u32(in.model_light + 0.5);
    let n_v_len = length(in.nrm);
    let n_v = select(vec3(0.0, 0.0, 1.0), in.nrm / n_v_len, n_v_len > 0.000001);
#ifdef FOLIAGE
    // bo2mp: Black Ops II's tree canopy (`mc_treecanopy`, `mlv_treecanopy`,
    // fxc disassembly; the game lights each vertex, here each pixel): the
    // colour map times colorTint, squared, times the light: the ambient (the
    // light grid's, at least 0.1, or the baked vertex light (c^2) * 32)
    // times the model's own vertex alpha (its rgb picks the wind variant),
    // plus the sun, never the primary light: its colour by its visibility
    // and |n.L|, times the scatter colour, going to white as the leaf turns
    // its sun side to the eye. Its lit pass writes alpha saturate(2 * texel
    // alpha): the leaves the depth pass keeps (alpha 128/255 and up) come
    // out solid.
    let leaf = texel * in.canopy_tint.rgb;
    var ambient = max(light, vec3(0.1));
    if (in.light.w > 0.0) {
        ambient = in.tint.rgb * in.tint.rgb * 32.0;
    }
    let n_len = length(in.nrm);
    let leaf_n = select(vec3(0.0, 0.0, 1.0), in.nrm / n_len, n_len > 0.000001);
    // bo2mp: with its own light grid set, the canopy's ambient is that
    // set's colour toward the leaf (`mc_treecanopy`); with baked light,
    // times the baked light over the set's harmonics (`mlv_treecanopy`).
    if (model_light > 0u) {
        var ratio = vec3(1.0);
        if (in.light.w > 0.0) {
            ratio = ambient / t6_model_sh(model_light, leaf_n);
        }
        ambient = t6_model_cube(model_light, leaf_n) * ratio;
    }
    let ndl = dot(leaf_n, t6.sun_dir_exposure.xyz);
    let to_eye = normalize(cam.world_position - in.world_position);
    let facing = saturate(sign(ndl) * dot(leaf_n, to_eye) * 0.5 + 0.5);
    let sun = t6.sun_color.rgb * (in.canopy.w * abs(ndl)) * mix(in.canopy.rgb, vec3(1.0), facing);
    let leaf_light = ambient * in.canopy_tint.w + sun + dyn_lights(in.world_position, leaf_n);
    return vec4(
        bo2_out(bo2_fog(leaf * leaf * leaf_light, in.world_position)),
        saturate(albedo.a * 2.0),
    );
    let tinted = texel;
    let out_a = saturate(albedo.a * 2.0);
#else
    let tinted = texel * in.tint.rgb;
    let out_a = albedo.a * in.tint.a;
#endif
    var base = tinted * tinted;
    if (in.light.w > 0.0) {
        light = in.tint.rgb * in.tint.rgb * 32.0;
        base = texel * texel;
    }
    // bo2mp: Black Ops II's static model shaders (fxc disassembly): a
    // model with its own light grid set takes that set's colour toward
    // the normal (`mc_lit_sm`); with baked vertex light (`mlv_lit_sm`),
    // that colour times the baked light over the set's harmonics toward
    // the vertex normal.
    var model_ratio = vec3(1.0);
    if (model_light > 0u) {
        if (in.light.w > 0.0) {
            model_ratio = light / t6_model_sh(model_light, n_v);
        }
        light = t6_model_cube(model_light, n_v) * model_ratio;
    }
#ifdef SHINE
    // bo2zm: Black Ops II's lit model shader (`mc_lit_sm_r0c0n0s0`, its
    // light-probe technique, from the fxc disassembly): as the world's, the
    // light here from the light grid. A model's vertices carry no tangent
    // frame here, so it comes from the screen-space derivatives of position
    // and texcoord (a cotangent frame).
    let vn = normalize(in.nrm);
    let dp1 = dpdx(in.world_position);
    let dp2 = dpdy(in.world_position);
    let duv1 = dpdx(in.tex);
    let duv2 = dpdy(in.tex);
    let dp2perp = cross(dp2, vn);
    let dp1perp = cross(vn, dp1);
    let tg = dp2perp * duv1.x + dp1perp * duv2.x;
    let bt = dp2perp * duv1.y + dp1perp * duv2.y;
    let frame = inverseSqrt(max(max(dot(tg, tg), dot(bt, bt)), 1e-20));
#ifdef FLOW
    // bo2mp: a lava prop (`mc_cod7emissiveflow`), as the world's.
    return vec4(bo2_out(bo2_flow(in.tex, in.tint, vn, tg * frame, bt * frame, in.world_position)), 1.0);
#endif
#ifdef TILE
    // bo2mp: Black Ops II's emissive tile (`sw4_3d_cod7_emissive_tile`,
    // Magma's lava rocks; its lit technique, fxc disassembly): the colour
    // map tiled by Micro_Scale times the vertex colour, squared, lit (here
    // the grid light, the primary light by the macro normal map times
    // Macro_Height; the micro normal map is not drawn), plus the glow
    // squared: (emissive A + emissive B) * A's alpha * hdrAmount, combined
    // by the assets lane in the specular map's place. Auxiliary texel 0:
    // Micro_Scale, Micro_Height, Macro_Height.
    let tc = textureLoad(aux_map, vec2<i32>(0, 0), 0);
    let tcol = textureSample(colour_map, colour_sampler, in.tex * tc.xy);
    let ttint = select(in.tint.rgb, vec3(1.0), in.light.w > 0.0);
    let tbase = raw_texel(tcol.rgb) * ttint;
    let tnm = (textureSample(normal_map, colour_sampler, in.tex).xy * 4.015748 - 2.015748) * tc.w;
    let tn = normalize(tnm.x * tg * frame + tnm.y * bt * frame + vn);
    let tray = primary_light_ray(u32(in.primary.x + 0.5), in.world_position);
    var tlight = light + tray.colour * in.primary.y * saturate(dot(tn, tray.dir));
    tlight += dyn_lights(in.world_position, tn);
    let tglow = textureSample(specular_map, colour_sampler, in.tex).rgb;
    return vec4(
        bo2_out(bo2_fog(tbase * tbase * tlight + tglow * tglow, in.world_position)),
        1.0,
    );
#endif
#ifdef TILE_BLEND
    // bo2mp: Black Ops II's tile blend (`sw4_3d_cod7_tile_blend`, `_edge`,
    // `_spec`; cliffs, rocks, vista mountains; lit technique, fxc
    // disassembly): Micro_1 (colour map, at uv * Micro_1_Scale) and Micro_2
    // (specular map's place, at uv * Micro_2_Scale) mixed by the macro
    // map's green squared (normal map's place), times 1 + blue^2 *
    // (EdgeHighlight - 1) (the edge variant; 1 elsewhere), times the vertex
    // colour, squared; the ambient light times saturate(red^2 +
    // AO_Diffuse_Adj); the primary light by the vertex normal (the macro
    // and micro normal maps and the spec variant's specular are not drawn).
    let bc0 = textureLoad(aux_map, vec2<i32>(0, 0), 0);
    let bc1 = textureLoad(aux_map, vec2<i32>(1, 0), 0);
    let bm = raw_texel(textureSample(normal_map, colour_sampler, in.tex).rgb);
    let b1 = raw_texel(textureSample(colour_map, colour_sampler, in.tex * bc0.xy).rgb);
    let b2 = raw_texel(textureSample(specular_map, colour_sampler, in.tex * bc0.zw).rgb);
    let bedge = 1.0 + bm.z * bm.z * (bc1.y - 1.0);
    let btint = select(in.tint.rgb, vec3(1.0), in.light.w > 0.0);
    let bcol = saturate(mix(b1, b2, bm.y * bm.y) * bedge) * btint;
    let bao = saturate(bm.x * bm.x + bc1.x);
    let bray = primary_light_ray(u32(in.primary.x + 0.5), in.world_position);
    var blight = light * bao + bray.colour * in.primary.y * saturate(dot(vn, bray.dir));
    blight += dyn_lights(in.world_position, vn);
    return vec4(bo2_out(bo2_fog(bcol * bcol * blight, in.world_position)), 1.0);
#endif
#ifdef FLAG
    // bo2mp: Black Ops II's tattered flag (`sw4_3d_phong_simple_flag_tatters`,
    // Dig's tarps; its lit technique, fxc disassembly): the frayed edge
    // texture (specular map's place, at uv * EdgeScale, auxiliary texel 0)
    // mixed into the diffuse by the vertex colour's red, squared; alpha
    // mixes the edge's alpha toward 1 by saturate(2 * red), tested at
    // 128/255. Lit by the grid and the primary light (its gloss, detail
    // normal and wind are not drawn).
    let gc = textureLoad(aux_map, vec2<i32>(0, 0), 0);
    let gedge = textureSample(specular_map, colour_sampler, in.tex * gc.xy);
    let gw = select(in.tint.r, 1.0, in.light.w > 0.0);
    let edge_a = mix(gedge.a, 1.0, saturate(2.0 * gw));
    if (edge_a < 0.501961) {
        discard;
    }
    let gcol = mix(raw_texel(gedge.rgb), texel, gw);
    let gray = primary_light_ray(u32(in.primary.x + 0.5), in.world_position);
    var glight = light + gray.colour * in.primary.y * saturate(dot(vn, gray.dir));
    glight += dyn_lights(in.world_position, vn);
    return vec4(bo2_out(bo2_fog(gcol * gcol * glight, in.world_position)), 1.0);
#endif
#ifdef GLASS
    let normal_uv = glass_uv(in.tex, in.glass.z, 1u);
    let spec_uv = glass_uv(in.tex, in.glass.z, 2u);
#else
    let normal_uv = in.tex;
    let spec_uv = in.tex;
#endif
    let nm = textureSample(normal_map, colour_sampler, normal_uv).xy * 4.015748 - 2.015748;
    let n = normalize(nm.x * tg * frame + nm.y * bt * frame + vn);
    // bo2mp: its own light grid set read toward the bumped normal; the
    // reflection scales by the set's harmonics toward the vertex normal
    // (the baked light, when baked).
    var grid_seen = light;
    if (model_light > 0u) {
        grid_seen = t6_model_sh(model_light, vn) * model_ratio;
        light = t6_model_cube(model_light, n) * model_ratio;
    }
    let spec_tex = textureSample(specular_map, colour_sampler, spec_uv);
    let spec_raw = raw_texel(spec_tex.rgb);
    let spec = spec_raw * spec_raw;
    let gloss = spec_tex.a;
    let vdir = normalize(in.world_position - cam.world_position);
    // The reflection, when the model has a probe (instance slot 23, plus
    // one): scaled by the light here over the light the probe saw.
    let ndotv = saturate(dot(n, -vdir));
    let fk = gloss * vec4(1.041667, 0.475, 0.018229, 0.25) + vec4(0.0, 0.0, -0.015625, 0.75);
    var f = min(exp2(-9.28 * ndotv), fk.y);
    f = fk.x * f + fk.z;
    let env_weight = saturate(spec * (fk.w - f) + f);
    let r = vdir - 2.0 * dot(vdir, n) * n;
    let probe_plus = u32(in.primary.w + 0.5);
    let probe = select(0u, probe_plus - 1u, probe_plus > 0u);
    let env = textureSampleLevel(probe_cubes, probe_sampler, r, i32(probe), 4.0 - 4.0 * gloss);
    let env_rgb = env.rgb / (env.a + 0.000001);
    let pi = min(probe, 31u) * 3u;
    let s0 = probes.v[pi];
    let s1 = probes.v[pi + 1u];
    let s2 = probes.v[pi + 2u];
    let sh = dot(s2, vec4(vn.z * vn.x, vn.y * vn.z, vn.x * vn.y, vn.x * vn.x - vn.y * vn.y))
        + dot(s1, vec4(vn, 1.0)) + s0.w * vn.z * vn.z;
    let probe_light = max(sh * s0.xyz, vec3(0.1));
    let reflection = select(vec3(0.0), env_weight * env_rgb * grid_seen / probe_light, probe_plus > 0u);
    // The primary light: diffuse and its highlight.
    let ray = primary_light_ray(u32(in.primary.x + 0.5), in.world_position);
    let lifted = ray.colour * in.primary.y;
    let ndotl = saturate(dot(n, ray.dir));
    light += lifted * ndotl;
    light += dyn_lights(in.world_position, n);
    let h = normalize(ray.dir - vdir);
    let ndoth = saturate(dot(n, h));
    let ldoth = saturate(dot(h, ray.dir));
    let power = exp2(gloss * 13.0);
    let lobe = exp2(log2(max(ndoth, 0.000001)) * power) * (power * 0.125 + 0.25);
    let k = saturate(gloss + 0.545);
    let vis = ndotl / (ldoth * ldoth * k - k + 1.01);
    let fres = spec + (1.0 - spec) * exp2(-10.0 * ldoth);
    let highlight = fres * lobe * vis * lifted;
#ifdef GLASS
    // bo2mp: Black Ops II's view-model glass (`sw4_3d_viewmodel_transparent`,
    // the riot shield's window; techniques 4 and 6, fxc disassembly): the
    // alpha a is the colour map's times the vertex colour's; the highlight
    // times Specular_Amount and the reflection times Reflection_Amount (both
    // 1 on the shield; a broken window's shards, `sw4_3d_glass_scaled`, have
    // 0.5 and 1, instance slots 24-25 plus one) are divided by a, so the
    // clear glass still shines once blended; the alpha written is sqrt(a).
    let amounts = select(vec2(1.0), in.glass.xy - 1.0, in.glass.xy > vec2(0.5));
    let ga = albedo.a * in.tint.a;
    let shine = reflection * amounts.y + highlight * amounts.x;
    return vec4(
        bo2_out(bo2_fog(base * light + shine / max(ga, 0.003922), in.world_position)),
        sqrt(ga),
    );
#else
    // The vertex alpha fades a blended model surface (a decal's painted
    // patches); opaque surfaces ignore the alpha.
    return vec4(
        bo2_out(bo2_fog(base * light + reflection + highlight, in.world_position)),
        out_a,
    );
#endif
#else
    let n = normalize(in.nrm);
    light += primary_light(u32(in.primary.x + 0.5), in.world_position, n) * in.primary.y;
    light += dyn_lights(in.world_position, n);
#ifdef GLASS
    // bo2mp: view-model glass with no normal or specular map: the alpha as
    // BO2 writes it, sqrt(a) (see the shine path).
    return vec4(bo2_out(bo2_fog(base * light, in.world_position)), sqrt(albedo.a * in.tint.a));
#else
    return vec4(bo2_out(bo2_fog(base * light, in.world_position)), out_a);
#endif
#endif
#endif
#endif
#endif
#endif
#endif
#endif
#endif
}

// bo2zm: the Black Ops II sky: a full-screen triangle at the far plane
// (reverse-Z: depth 0, drawn only where nothing else is), sampling the
// sky's cube map by view direction as the game's skycubemaphdr shader does
// (world x, y, z; rgb / a), times its brightness, then the BO2 output (times
// the exposure, square-rooted). bo2mp: its fog as that shader applies it:
// the pixel shader works the fog out again and multiplies it by the vertex
// shader's fog times opacity, so the sky fogs by fog^2 * opacity. Its
// skyColorMultiplier (GfxWorld::skyDynIntensity, factors 1 on both
// Nuketowns) and colour matrix (identity) change nothing.
@group(1) @binding(3) var sky_map: texture_cube<f32>;
@group(1) @binding(4) var sky_sampler: sampler;
@group(1) @binding(5) var<uniform> sky_t6: T6Lighting;
// bo2zm: the light table again, for the sky fog (slots 256, 257).
@group(1) @binding(6) var<uniform> sky_lights: T6Lights;

struct SkyOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vertex_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    let x = f32((index & 1u) * 4u) - 1.0;
    let y = f32((index >> 1u) * 4u) - 1.0;
    var out: SkyOut;
    out.clip_pos = vec4(x, y, 0.0, 1.0);
    out.ndc = vec2(x, y);
    return out;
}

@fragment
fn fragment_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let near = cam.world_from_clip * vec4(in.ndc, 1.0, 1.0);
    let dir = normalize(near.xyz / near.w - cam.world_position);
    // The game's sky vertex shader turns the lookup about z:
    // x' = x * r.y + y * r.z, y' = x * r.x + y * r.y.
    let r = sky_t6.sky;
    let z = select(dir.z, -dir.z, r.w < 0.0);
    let lookup = vec3(dir.x * r.y + dir.y * r.z, dir.x * r.x + dir.y * r.y, z);
    let texel = textureSample(sky_map, sky_sampler, lookup);
    let parms = sky_t6.sky_parms;
    let decoded = select(texel.rgb * texel.rgb, texel.rgb / (texel.a + 0.000001), parms.x > 0.0);
    let sky = decoded * abs(r.w);
    // The game puts the sky 2e7 units out and fogs it there, taking only
    // its skyColorParm w of the fog (1 on most maps, Buried 0.16).
    var fogged = sky;
    if (sky_lights.v[1028u].w >= 0.5) {
        let f = fog_at(
            cam.world_position + dir * 20000000.0,
            sky_lights.v[1028u],
            sky_lights.v[1029u],
            sky_lights.v[1030u],
            sky_lights.v[1031u],
            sky_lights.v[1024u],
        );
        fogged = mix(sky, f.tint, f.fog * f.fog * f.opacity * parms.y);
    }
    return vec4(bo2_final(sqrt(max(fogged * sky_t6.sun_dir_exposure.w, vec3(0.0)))), 1.0);
}
