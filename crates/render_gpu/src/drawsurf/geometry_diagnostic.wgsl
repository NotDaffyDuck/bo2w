#import bevy_render::view::View

@group(0) @binding(0) var<uniform> view: View;

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
    // The vision's colour grade (see bo2_final): tone band slopes and
    // offsets, the red, green and blue rows of the shadow, mid and highlight
    // matrices, then gamma, saturation (luma weights, amount) and blend.
    grade: array<vec4<f32>, 14>,
    // The world's colour tables drawn (see world_lut): the first rows of
    // tables A and B, A's weight, and 1 when the map has them.
    lut: vec4<f32>,
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
}
@group(2) @binding(4) var<uniform> t6_dyn: T6DynLights;
// The world's colour tables (`GfxWorld::lutMaterial`).
@group(2) @binding(5) var t6_lut: texture_2d<f32>;
@group(2) @binding(6) var t6_lut_sampler: sampler;

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
    let uv = (clip.xy - view.viewport.xy) / view.viewport.zw;
    let ndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_d, 1.0);
    let scene_view = view.view_from_clip * ndc;
    let scene_z = select(1e9, -scene_view.z / scene_view.w, scene_d > 0.0);
    let sprite_z = -(view.view_from_world * vec4(world, 1.0)).z;
    return saturate((scene_z - sprite_z) * inv);
}
#endif

#ifdef SHINE
// bo2zm: a shine surface's normal and specular maps, and the reflection
// probes (their cube array and, per probe, its lightingSH: three vec4).
@group(1) @binding(8) var normal_map: texture_2d<f32>;
@group(1) @binding(9) var specular_map: texture_2d<f32>;
struct T6Probes {
    v: array<vec4<f32>, 96>,
}
@group(3) @binding(0) var probe_cubes: texture_cube_array<f32>;
@group(3) @binding(1) var probe_sampler: sampler;
@group(3) @binding(2) var<uniform> probes: T6Probes;
#endif

struct VertexIn {
    @builtin(instance_index) instance: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // bo2zm: xyz the tangent, w the binormal's sign.
    @location(2) tangent: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) uv: vec2<f32>,
    @location(5) lightmap_uv: vec2<f32>,
#ifdef LAYERS
    // bo2zm: layer 1 and 2 texcoords (a second buffer).
    @location(6) layer_uv: vec4<f32>,
#endif
}

struct SmodelVertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(6) world_from_local_0: vec4<f32>,
    @location(7) world_from_local_1: vec4<f32>,
    @location(8) world_from_local_2: vec4<f32>,
    @location(9) world_from_local_3: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
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
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = view.clip_from_world * vec4(in.position, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    out.uv = in.uv;
    out.lightmap_uv = in.lightmap_uv;
    out.world_position = in.position;
    out.tangent = in.tangent.xyz;
    // As BO2's world vertex shaders: binormal = cross(normal, tangent)
    // times the sign of the vertex's binormal sign.
    out.binormal = cross(in.normal, in.tangent.xyz) * sign(in.tangent.w);
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
fn vertex_smodel(in: SmodelVertexIn) -> VertexOut {
    let world_from_local = mat4x4<f32>(
        in.world_from_local_0,
        in.world_from_local_1,
        in.world_from_local_2,
        in.world_from_local_3,
    );
    var out: VertexOut;
    let world = world_from_local * vec4(in.position, 1.0);
    out.clip_position = view.clip_from_world * world;
    out.normal = (world_from_local * vec4(in.normal, 0.0)).xyz;
    out.color = in.color;
    out.uv = in.uv;
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
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let normal_colour = abs(normalize(in.normal)) * 0.72 + vec3(0.08, 0.02, 0.10);
    let cell = i32(floor(in.uv.x * 16.0) + floor(in.uv.y * 16.0)) & 1;
    let diagnostic_tint = select(vec3(0.72, 0.22, 0.78), vec3(0.22, 0.72, 0.78), cell == 0);
    return vec4(mix(normal_colour, diagnostic_tint, 0.28) * max(in.color.rgb, vec3(0.35)), 1.0);
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
// (1 - 2^(-5.77 * (x - 0.75)))), then its colour table (built by
// hdr_create_lut2dv from the map's vision) grades it: three tone bands by
// the display value's luma, smoothstepped and summing to 1, weight a shadow,
// a mid and a highlight matrix on the linear colour; then the vision's gamma
// (vc_FGM), its desaturation with the Rec. 709 weights (vc_FSM; Nuketown
// 0.8535, Origins 0.9082) and how much of it is kept (vc_FBM). Bloom is not
// drawn.
fn bo2_final(encoded: vec3<f32>, l: T6Lighting) -> vec3<f32> {
    return bo2_grade(world_lut(t6_lut, t6_lut_sampler, bo2_rolloff(encoded), l.lut), l);
}

// bo2zm: the highlights' roll-off above 0.75 (gamma-encoded).
fn bo2_rolloff(encoded: vec3<f32>) -> vec3<f32> {
    let rolled = 0.75 + 0.25 * (1.0 - exp2(-5.77078 * (encoded - 0.75)));
    return select(encoded, rolled, encoded > vec3(0.75));
}

// bo2zm: the world's colour table, looked up before the vision grade (the
// game bakes both into one table each frame): a strip of 32 slices of 32 x
// 32, red across a slice, green down it, blue picking the slice (blended
// with the next one), one table per 32 rows. Tables A and B blend by A's
// weight as the camera moves between colour table volumes.
fn world_lut(lut: texture_2d<f32>, s: sampler, x: vec3<f32>, pick: vec4<f32>) -> vec3<f32> {
    if pick.w < 0.5 {
        return x;
    }
    let size = vec2<f32>(textureDimensions(lut));
    let c = saturate(x) * 31.0;
    let slice = floor(c.z);
    let next = min(slice + 1.0, 31.0);
    let f = c.z - slice;
    let u0 = (slice * 32.0 + c.x + 0.5) / size.x;
    let u1 = (next * 32.0 + c.x + 0.5) / size.x;
    let va = (pick.x + c.y + 0.5) / size.y;
    let vb = (pick.y + c.y + 0.5) / size.y;
    let a = mix(
        textureSampleLevel(lut, s, vec2(u0, va), 0.0).rgb,
        textureSampleLevel(lut, s, vec2(u1, va), 0.0).rgb,
        f,
    );
    let b = mix(
        textureSampleLevel(lut, s, vec2(u0, vb), 0.0).rgb,
        textureSampleLevel(lut, s, vec2(u1, vb), 0.0).rgb,
        f,
    );
    return mix(b, a, pick.z);
}

// bo2zm: the vision's colour grade of a rolled-off, gamma-encoded colour.
fn bo2_grade(x: vec3<f32>, l: T6Lighting) -> vec3<f32> {
    let lin = x * x;
    let g = l.grade;
    let r = saturate(dot(x, vec3(0.212585, 0.715195, 0.07222)) * g[0] + g[1]);
    let s = vec3(r.x, r.z * r.w, r.y);
    let band = s * s * (3.0 - 2.0 * s);
    let w = band / max(band.x + band.y + band.z, 0.000001);
    let c = vec4(lin, 1.0);
    let red = saturate(dot(vec3(dot(g[2], c), dot(g[3], c), dot(g[4], c)), w));
    let green = saturate(dot(vec3(dot(g[5], c), dot(g[6], c), dot(g[7], c)), w));
    let blue = saturate(dot(vec3(dot(g[8], c), dot(g[9], c), dot(g[10], c)), w));
    let graded = exp2(log2(max(vec3(red, green, blue), vec3(0.0000001))) * g[11].xyz);
    let luma = dot(graded, g[12].xyz);
    let toned = vec3(luma) + g[12].w * (graded - vec3(luma));
    return sqrt(saturate(lin + g[13].xyz * (toned - lin)));
}

// bo2zm: what every lit and unlit Black Ops II shader writes: the colour
// times the exposure (`hdrControl0.x`), square-rooted; the colour target
// holds gamma-encoded values. Then the game's final image steps.
fn bo2_out(colour: vec3<f32>) -> vec3<f32> {
    return bo2_final(sqrt(max(colour * t6.sun_dir_exposure.w, vec3(0.0))), t6);
}

// bo2zm A7: what an additive draw adds: the final image of its colour less
// that of black. The game grades the frame once, after the blend; graded on
// its own, each additive quad added the grade's black (Nuketown's shadow
// matrix biases, about 3/8/16 of 255) over its whole square: pale-blue
// rectangles round the fires and the light shafts.
fn bo2_added(encoded: vec3<f32>, l: T6Lighting) -> vec3<f32> {
    return max(bo2_final(encoded, l) - bo2_final(vec3(0.0), l), vec3(0.0));
}

// bo2zm: an unlit surface's output (additive ones add no black, above).
fn unlit_out(colour: vec3<f32>) -> vec3<f32> {
#ifdef ADD
    return bo2_added(sqrt(max(colour * t6.sun_dir_exposure.w, vec3(0.0))), t6);
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
    let eye = view.world_position;
    let d = distance(p, eye);
    let mean = fog_mean(eye.z, p.z, c.w, e.w) * fog_ln2(e.w);
    let fog = 1.0 - min(exp2(-(mean * d - a.x) / max(a.y, 1.0)), 1.0);
    let t = saturate(q.y * dot(e.xyz, (p - eye) / max(d, 0.0001)) + q.x);
    let tint = mix(c.rgb, s.rgb, t);
    let opacity = mix(a.z, s.w, t);
    return mix(colour, tint, fog * opacity);
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
    let eye = view.world_position;
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
        ray.colour = t6.sun_color.rgb;
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

// bo2zm: world surfaces without a lightmap. Unlit ones as the game's unlit
// shaders draw them: (texel * vertex colour)^2 times the material's colour
// scale, then the BO2 output. A multiply decal scales what is behind by
// 1 + a * (texel * vertex colour - 1). Lit ones without a lightmap get a
// plain sun-and-sky light. Alpha feeds the blended variants.
@fragment
fn fragment_textured(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo = textureSample(colour_map, colour_sampler, in.uv);
#ifdef ALPHA_TEST
    // The game's alpha-tested shaders discard below 128/255.
    if (albedo.a < 0.501961) {
        discard;
    }
#endif
    let texel = raw_texel(albedo.rgb);
#ifdef UNLIT
#ifdef MULTIPLY
    let a = albedo.a * in.color.a;
    return vec4(mix(vec3(1.0), texel * in.color.rgb, a), a);
#else
    let c = texel * in.color.rgb;
    return vec4(
        unlit_out(unlit_fog(c * c * f32(#{UNLIT_SCALE}), in.world_position)),
        albedo.a * in.color.a,
    );
#endif
#else
    let n = normalize(in.normal);
    let sun = normalize(vec3(0.35, 0.25, 0.9));
    let light = 0.55 + 0.45 * max(dot(n, sun), 0.0);
    return vec4(texel * light, albedo.a);
#endif
}

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
// primary light in alpha; light = base + directional * N.dir + primary *
// visibility; then the BO2 output. Normal maps, specular and fog are not
// drawn.
@fragment
fn fragment_lightmapped(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo = textureSample(colour_map, colour_sampler, in.uv);
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
    c = mix(c, raw_texel(l1.rgb), l1.a * in.color.g);
#endif
#ifdef LAYER1_MULTIPLY
    let m1 = raw_texel(textureSample(colour_map1, colour_sampler, in.layer_uv.xy).rgb);
    c = c * (1.0 + in.color.g * (m1 - 1.0));
#endif
#ifdef LAYER1_TESTED
    let t1 = textureSample(colour_map1, colour_sampler, in.layer_uv.xy);
    c = select(c, raw_texel(t1.rgb), t1.a * in.color.g >= 0.5);
#endif
#ifdef LAYER1_ADD
    let a1 = textureSample(colour_map1, colour_sampler, in.layer_uv.xy);
    c = c + raw_texel(a1.rgb) * a1.a * in.color.g;
#endif
#ifdef LAYER2_BLEND
    let l2 = textureSample(colour_map2, colour_sampler, in.layer_uv.zw);
    c = mix(c, raw_texel(l2.rgb), l2.a * in.color.b);
#endif
#ifdef LAYER2_MULTIPLY
    let m2 = raw_texel(textureSample(colour_map2, colour_sampler, in.layer_uv.zw).rgb);
    c = c * (1.0 + in.color.b * (m2 - 1.0));
#endif
#ifdef LAYER2_TESTED
    let t2 = textureSample(colour_map2, colour_sampler, in.layer_uv.zw);
    c = select(c, raw_texel(t2.rgb), t2.a * in.color.b >= 0.5);
#endif
#ifdef LAYER2_ADD
    let a2 = textureSample(colour_map2, colour_sampler, in.layer_uv.zw);
    c = c + raw_texel(a2.rgb) * a2.a * in.color.b;
#endif
#endif
    let base = c * c;
#else
    let tinted = texel * in.color.rgb;
    let base = tinted * tinted;
#endif
    let v = in.lightmap_uv.y / 3.0;
    let page0 = textureSample(lightmap, lightmap_sampler, vec2(in.lightmap_uv.x, v));
    let page1 = textureSample(lightmap, lightmap_sampler, vec2(in.lightmap_uv.x, v + 1.0 / 3.0));
    let page2 = textureSample(lightmap, lightmap_sampler, vec2(in.lightmap_uv.x, v + 2.0 / 3.0));
    let ambient = page0.rgb / (page0.a + 0.000001);
    let directional = page1.rgb / (page1.a + 0.000001);
    let dir = page2.rgb * 2.0 - 1.0;
#ifdef SHINE
    // bo2zm: Black Ops II's lit world shader (`wpc_lit_sm_r0c0n0s0`, its
    // sun-shadow technique, from the fxc disassembly), with the lightmap's
    // baked sun visibility (page 2 alpha) for its shadow map.
    let vn = normalize(in.normal);
    let nm = textureSample(normal_map, colour_sampler, in.uv).xy * 4.015748 - 2.015748;
    let n = normalize(nm.x * in.tangent + nm.y * in.binormal + vn);
    let spec_tex = textureSample(specular_map, colour_sampler, in.uv);
    let spec_raw = raw_texel(spec_tex.rgb);
    let spec = spec_raw * spec_raw;
    let gloss = spec_tex.a;
    let vdir = normalize(in.world_position - view.world_position);
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
    let lit = ray.colour * page2.a;
    let ndotl = saturate(dot(n, ray.dir));
    light += lit * ndotl;
    light += dyn_lights(in.world_position, n);
    let h = normalize(ray.dir - vdir);
    let ndoth = saturate(dot(n, h));
    let ldoth = saturate(dot(h, ray.dir));
    let power = exp2(gloss * 13.0);
    let lobe = exp2(log2(max(ndoth, 0.000001)) * power) * (power * 0.125 + 0.25);
    let k = saturate(gloss + 0.545);
    let vis = ndotl / (ldoth * ldoth * k - k + 1.01);
    let fres = spec + (1.0 - spec) * exp2(-10.0 * ldoth);
    let highlight = fres * lobe * vis * lit;
    let colour = base * light + reflection + highlight;
    return vec4(bo2_out(bo2_fog(colour, in.world_position)), albedo.a);
#else
    let n = normalize(in.normal);
    var light = ambient + directional * saturate(dot(dir, n));
    light += primary_light(in.light_index, in.world_position, n) * page2.a;
    light += dyn_lights(in.world_position, n);
    return vec4(bo2_out(bo2_fog(base * light, in.world_position)), albedo.a);
#endif
}

// bo2zm: Black Ops II props. The instance carries its world-from-local
// columns, its light: rgb from the light grid (linear, already times the
// exposure, w = 0), or for an instance with baked vertex light w = the
// exposure, the vertex colour then holding that light ((c^2) * 32, as the
// game's mlv_* shaders read it); then its primary light and that light's
// visibility from the light grid.
struct PropVertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(6) world_from_local_0: vec4<f32>,
    @location(7) world_from_local_1: vec4<f32>,
    @location(8) world_from_local_2: vec4<f32>,
    @location(9) world_from_local_3: vec4<f32>,
    @location(10) light: vec4<f32>,
    @location(11) primary: vec4<f32>,
}

struct PropOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) light: vec4<f32>,
    @location(4) world_position: vec3<f32>,
    @location(5) primary: vec4<f32>,
#ifdef DECAL
    // A projected decal's box from the world (columns).
    @location(6) @interpolate(flat) box0: vec4<f32>,
    @location(7) @interpolate(flat) box1: vec4<f32>,
    @location(8) @interpolate(flat) box2: vec4<f32>,
    @location(9) @interpolate(flat) box3: vec4<f32>,
#endif
}

@vertex
fn vertex_prop(in: PropVertexIn) -> PropOut {
    let world_from_local = mat4x4<f32>(
        in.world_from_local_0,
        in.world_from_local_1,
        in.world_from_local_2,
        in.world_from_local_3,
    );
    var out: PropOut;
#ifdef CLOUD
    // bo2zm: Black Ops II particle clouds (`particlecloud_*` vertex shader):
    // the point of the cloud's box to the world by the cloud's placement,
    // then out to a quad facing the camera, the corner (texcoord less one
    // half) times the particle's width and height (instance primary.xy);
    // the colour is the cloud's (instance light), and nothing feathers it.
    let centre = world_from_local * vec4(in.position, 1.0);
    let corner = in.uv - vec2(0.5);
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let world = vec4(centre.xyz + right * (corner.x * in.primary.x) - up * (corner.y * in.primary.y), 1.0);
    out.clip_position = view.clip_from_world * world;
    out.normal = view.world_from_view[2].xyz;
    out.color = in.light;
    out.uv = in.uv;
    out.light = vec4(0.0);
    out.world_position = world.xyz;
    out.primary = vec4(0.0);
#else
    var world = world_from_local * vec4(in.position, 1.0);
#ifdef EFFECT
    // bo2zm M3: BO2's eye offset (an `_eo` effect material's
    // eyeOffsetParms.x, instance slot 23): the sprite is drawn that many
    // units nearer the eye along its line of sight (at least half way),
    // where it already shows on screen, so a flame on a wall or a pole is
    // not cut into by it.
    let eo = in.primary.w;
    if (eo > 0.0) {
        let to_sprite = world.xyz - view.world_position;
        let d = max(length(to_sprite), 0.0001);
        world = vec4(view.world_position + to_sprite * (max(d - eo, d * 0.5) / d), 1.0);
    }
#endif
#ifdef DECAL
    // bo2zm M4 retest 5: a projected decal's box is already in the world;
    // its instance columns are the box from the world, for the pixels.
    world = vec4(in.position, 1.0);
    out.box0 = in.world_from_local_0;
    out.box1 = in.world_from_local_1;
    out.box2 = in.world_from_local_2;
    out.box3 = in.world_from_local_3;
#endif
    out.clip_position = view.clip_from_world * world;
    out.normal = (world_from_local * vec4(in.normal, 0.0)).xyz;
    out.color = in.color;
    out.uv = in.uv;
    out.light = in.light;
    out.world_position = world.xyz;
    out.primary = in.primary;
#endif
    return out;
}

@fragment
fn fragment_prop(in: PropOut) -> @location(0) vec4<f32> {
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
    let to_eye = normalize(view.world_position - in.world_position);
    let ndv = saturate(dot(normalize(in.normal), to_eye));
    let e2 = raw_texel(textureSample(colour_map, colour_sampler, in.uv * 5.0 + fract(t * vec2(-0.03, 0.01) * 5.0)).rgb).x;
    let d = in.uv - vec2(0.5);
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
    var ghost_tint = in.color.rgb;
    if (in.light.w > 0.0) {
        ghost_light = in.color.rgb * in.color.rgb * 32.0;
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
    let to_eye = normalize(view.world_position - in.world_position);
    let nz = -dot(normalize(in.normal), to_eye);
    let pulse = 0.5 - 0.5 * sin((nz * -0.5 + t6_dyn.count.y) * 6.28318);
    return vec4(bo2_out(mix(lo, hi, pulse) * 0.5), 1.0);
#else
#ifdef DECAL
    // bo2zm M4 retest 5: Black Ops II's projected decal (`mc_projecteddecal`,
    // from the fxc disassembly; the blood a hit paints on a zombie): the
    // scene behind the box's pixel, from the depth, into the box (-1..1);
    // outside it nothing is drawn. Inside, the texture projected along the
    // box's x axis (at y, z), its alpha fading to the box's x faces, its
    // colour the texture squared times the light grid's light, no fog.
    let scene_d = textureLoad(soft_depth, vec2<i32>(in.clip_position.xy), 0);
    if (scene_d <= 0.0) {
        discard;
    }
    let suv = (in.clip_position.xy - view.viewport.xy) / view.viewport.zw;
    let scene_clip = vec4(suv.x * 2.0 - 1.0, 1.0 - suv.y * 2.0, scene_d, 1.0);
    let scene_h = view.world_from_clip * scene_clip;
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
        in.uv * in.light.w + fract(t6_dyn.count.y * in.light.yz * in.light.w),
    );
#else
    let albedo = textureSample(colour_map, colour_sampler, in.uv);
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
        saturate(distance(in.world_position, view.world_position) * feather),
        feather > 0.0,
    );
    // bo2zm M4 retest 5: the `_ds` (dissolve) ones, from the fxc
    // disassembly: alpha = saturate(alphaDissolveParms.x * (texture alpha +
    // vertex alpha - 1)) (minus instance slot 20, which is otherwise a
    // primary light, 0 or more), so the texture's faint floor never shows
    // and the sprite dissolves as it fades.
    let dissolve = -min(in.primary.x, 0.0);
    var a = albedo.a * in.color.a * near;
    if (dissolve > 0.0) {
        a = saturate(dissolve * (albedo.a + in.color.a * near - 1.0));
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
        a *= soft_fade(in.clip_position, in.world_position, feather);
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
    var tint = in.color.rgb;
    if (in.light.w > 0.5) {
        let ray = normalize(in.world_position - view.world_position);
        let d = dot(ray, normalize(in.normal));
        let f = saturate(d * d * in.light.x + in.light.y);
        tint = tint * f * mix(vec3(in.light.z), vec3(1.0), f);
    }
#ifdef EFFECT_ADD
    // Additive ones take no fog (it would add the fog colour over the whole
    // quad); the blend scales the colour by the alpha.
    return vec4(bo2_added(texel * tint, t6), a);
#else
#ifdef EFFECT_MULTIPLY
    return vec4(mix(vec3(1.0), texel * tint, a), a);
#else
    // Blended ones go toward the effect fog by its keep factor.
    let fx_fog = effect_fog(in.world_position);
    let fx = mix(fx_fog.rgb, texel * tint, fx_fog.w);
    return vec4(bo2_final(fx, t6), a);
#endif
#endif
#else
#ifdef UNLIT
#ifdef MULTIPLY
    let a = albedo.a * in.color.a;
    return vec4(mix(vec3(1.0), texel * in.color.rgb, a), a);
#else
#ifdef SCROLL
    // A cloud sheet's colour is the texture times hdrAmount, squared
    // (instance primary.x holds hdrAmount squared), no vertex colour; the
    // vertex alpha fades it, and so does how near the scene behind it lies
    // (1 / FadeFactor, instance light.x).
    var sheet_a = albedo.a * in.color.a;
#ifdef SOFT
    sheet_a *= soft_fade(in.clip_position, in.world_position, in.light.x);
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
    var c = texel * in.color.rgb;
    if (in.primary.z < 0.0) {
        c *= -in.primary.z;
    }
    return vec4(
        unlit_out(unlit_fog(c * c * f32(#{UNLIT_SCALE}), in.world_position)),
        albedo.a * in.color.a,
    );
#endif
#endif
#else
    // The grid light is already times the exposure; the BO2 output adds it
    // again, so divide it out here.
    let exposure = max(t6.sun_dir_exposure.w, 0.000001);
    var light = in.light.rgb / exposure;
    let tinted = texel * in.color.rgb;
    var base = tinted * tinted;
    if (in.light.w > 0.0) {
        light = in.color.rgb * in.color.rgb * 32.0;
        base = texel * texel;
    }
#ifdef SHINE
    // bo2zm: Black Ops II's lit model shader (`mc_lit_sm_r0c0n0s0`, its
    // light-probe technique, from the fxc disassembly): as the world's, the
    // light here from the light grid. A model's vertices carry no tangent
    // frame here, so it comes from the screen-space derivatives of position
    // and texcoord (a cotangent frame).
    let vn = normalize(in.normal);
    let dp1 = dpdx(in.world_position);
    let dp2 = dpdy(in.world_position);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let dp2perp = cross(dp2, vn);
    let dp1perp = cross(vn, dp1);
    let tg = dp2perp * duv1.x + dp1perp * duv2.x;
    let bt = dp2perp * duv1.y + dp1perp * duv2.y;
    let frame = inverseSqrt(max(max(dot(tg, tg), dot(bt, bt)), 1e-20));
    let nm = textureSample(normal_map, colour_sampler, in.uv).xy * 4.015748 - 2.015748;
    let n = normalize(nm.x * tg * frame + nm.y * bt * frame + vn);
    let spec_tex = textureSample(specular_map, colour_sampler, in.uv);
    let spec_raw = raw_texel(spec_tex.rgb);
    let spec = spec_raw * spec_raw;
    let gloss = spec_tex.a;
    let vdir = normalize(in.world_position - view.world_position);
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
    let reflection = select(vec3(0.0), env_weight * env_rgb * light / probe_light, probe_plus > 0u);
    // The primary light: diffuse and its highlight.
    let ray = primary_light_ray(u32(in.primary.x + 0.5), in.world_position);
    let lit = ray.colour * in.primary.y;
    let ndotl = saturate(dot(n, ray.dir));
    light += lit * ndotl;
    light += dyn_lights(in.world_position, n);
    let h = normalize(ray.dir - vdir);
    let ndoth = saturate(dot(n, h));
    let ldoth = saturate(dot(h, ray.dir));
    let power = exp2(gloss * 13.0);
    let lobe = exp2(log2(max(ndoth, 0.000001)) * power) * (power * 0.125 + 0.25);
    let k = saturate(gloss + 0.545);
    let vis = ndotl / (ldoth * ldoth * k - k + 1.01);
    let fres = spec + (1.0 - spec) * exp2(-10.0 * ldoth);
    let highlight = fres * lobe * vis * lit;
    // The vertex alpha fades a blended model surface (a decal's painted
    // patches); opaque surfaces ignore the alpha.
    return vec4(
        bo2_out(bo2_fog(base * light + reflection + highlight, in.world_position)),
        albedo.a * in.color.a,
    );
#else
    let n = normalize(in.normal);
    light += primary_light(u32(in.primary.x + 0.5), in.world_position, n) * in.primary.y;
    light += dyn_lights(in.world_position, n);
    return vec4(bo2_out(bo2_fog(base * light, in.world_position)), albedo.a * in.color.a);
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
// the exposure, square-rooted). Its fog is not applied.
@group(1) @binding(3) var sky_map: texture_cube<f32>;
@group(1) @binding(4) var sky_sampler: sampler;
@group(1) @binding(5) var<uniform> sky_t6: T6Lighting;
// bo2zm: the light table again, for the sky fog (slots 256, 257).
@group(1) @binding(6) var<uniform> sky_lights: T6Lights;
@group(1) @binding(10) var sky_lut: texture_2d<f32>;
@group(1) @binding(11) var sky_lut_sampler: sampler;

struct SkyOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vertex_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    let x = f32((index & 1u) * 4u) - 1.0;
    let y = f32((index >> 1u) * 4u) - 1.0;
    var out: SkyOut;
    out.clip_position = vec4(x, y, 0.0, 1.0);
    out.ndc = vec2(x, y);
    return out;
}

@fragment
fn fragment_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let near = view.world_from_clip * vec4(in.ndc, 1.0, 1.0);
    let dir = normalize(near.xyz / near.w - view.world_position);
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
    let full = fog_apply(
        sky,
        view.world_position + dir * 20000000.0,
        sky_lights.v[1028u],
        sky_lights.v[1029u],
        sky_lights.v[1030u],
        sky_lights.v[1031u],
        sky_lights.v[1024u],
    );
    let fogged = mix(sky, full, parms.y);
    let sky_x = bo2_rolloff(sqrt(max(fogged * sky_t6.sun_dir_exposure.w, vec3(0.0))));
    return vec4(bo2_grade(world_lut(sky_lut, sky_lut_sampler, sky_x, sky_t6.lut), sky_t6), 1.0);
}
