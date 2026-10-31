//! bo2zm: how a Black Ops II material draws in the renderer's fallback pass.
//!
//! BO2 techniques are D3D11 shaders the engine cannot run, so the fallback
//! pass draws each material from two sources. The material's own draw state
//! (`GfxStateBits::loadBits` of its main technique, read from the zone) says
//! how it blends, whether it is alpha tested, which faces it culls and how
//! far it is pushed toward the camera (decals). The technique set's name
//! says what the shader does with the colour: `*_unlit*` is unlit, `*_b1*`
//! layered, `wpc_shadowcaster_*` only casts shadow, `*_skycubemaphdr_*` is
//! the sky. Without a draw state, the name alone decides, as before: the
//! base layer's token says how it covers (`r0c0..` replaces, `b0c0..`
//! blends, `t0c0..` alpha tests), `*_blend_*` / `*_add_*` / `*_multiply_*`.

/// bo2zm: the player's Black Ops II quality setting (`bo2_quality` in
/// settings.cfg). `full` (the default) draws as close to the real game as
/// the fallback pass can; `fast` trades looks for speed: half-size
/// textures, single-layer ground, props hidden sooner.
static T6_FAST: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn t6_set_fast(fast: bool) {
    T6_FAST.store(fast, core::sync::atomic::Ordering::Relaxed);
}

pub fn t6_fast() -> bool {
    T6_FAST.load(core::sync::atomic::Ordering::Relaxed)
}

/// bo2zm: Black Ops II's texture filtering row (`tex_filter` in
/// settings.cfg): the samplers' anisotropy, 1 (low), 4 or 16 (high).
static T6_ANISO: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(16);

pub fn t6_set_aniso(aniso: u16) {
    T6_ANISO.store(aniso.clamp(1, 16), core::sync::atomic::Ordering::Relaxed);
}

pub fn t6_aniso() -> u16 {
    T6_ANISO.load(core::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum T6Blend {
    Opaque,
    Blend,
    Add,
    Multiply,
    /// Never drawn in colour: casts shadow only, a screen distortion
    /// (`distortion_*`, an effect's heat haze), which refracts the frame in
    /// a pass this renderer does not have, or a light sphere
    /// (`*_sw4_3d_light_sphere_*`, a glow the shader works out from the
    /// sphere's depth; as a plain model it drew a pale hard-edged disc, Mob
    /// of the Dead's fires).
    CasterOnly,
    /// The sky (`*_skycubemaphdr_*`): drawn behind everything from its cube
    /// colour map, never as a surface.
    Sky,
}

/// Which faces a material culls (`GFXS_CULL_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum T6Cull {
    None,
    Back,
    Front,
}

/// The extra layers of a layered material (`*_b1c1n1_m2c2..`), read from
/// its technique set's name, and where each layer's uv_semantic sits in the
/// world's second vertex stream. Per layer, that stream holds a half2
/// uv_semantic, then 4 more bytes (the layer's tangent frame) when the layer
/// has a normal map and so does the base layer (measured on every layered
/// Nuketown material: `t6mat` `T6MAT_VD1`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct T6Layers {
    /// Per extra layer 1..3: 0 none, 1 blend (`bK`: mixed in by the layer
    /// colour's alpha times the vertex colour's green, blue, alpha), 2
    /// multiply (`mK`: colour times 1 + weight * (layer - 1)), 3 add (`aK`,
    /// bo2mp: colour plus layer * layer alpha * weight, before squaring;
    /// `lit_sm_*_a1c1` pixel shader, fxc disassembly), 4 test (`tK`:
    /// the layer replaces the colour where layer alpha * weight >= 0.5;
    /// `lit_sm_*_t1c1n1`).
    pub kinds: [u8; 3],
    /// Per extra layer, its uv_semantic's byte offset in a vertex record.
    pub uv_offsets: [u8; 3],
    /// Bytes per vertex in the second stream.
    pub stride: u8,
}

impl T6Layers {
    pub fn from_technique_set(name: &str) -> Self {
        let tokens: Vec<&str> = name.split('_').collect();
        let base_normal = tokens.iter().any(|t| {
            let b = t.as_bytes();
            b.len() >= 2 && matches!(b[0], b'r' | b'b' | b't') && b[1] == b'0' && t.contains("n0")
        });
        let mut layers = Self::default();
        for t in tokens {
            let b = t.as_bytes();
            if b.len() < 4
                || !matches!(b[0], b'b' | b'm' | b'a' | b't')
                || !(b'1'..=b'3').contains(&b[1])
            {
                continue;
            }
            // A layer token names its colour map: `b1c1..`, `m2c2..`.
            if b[2] != b'c' {
                continue;
            }
            let k = usize::from(b[1] - b'1');
            layers.kinds[k] = match b[0] {
                b'b' => 1,
                b'm' => 2,
                b'a' => 3,
                _ => 4,
            };
            layers.uv_offsets[k] = layers.stride;
            layers.stride += 4;
            if base_normal && t.contains(&format!("n{}", b[1] as char)) {
                layers.stride += 4;
            }
        }
        layers
    }

    pub fn any(&self) -> bool {
        self.kinds.iter().any(|&k| k != 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct T6Draw {
    pub blend: T6Blend,
    pub unlit: bool,
    /// Layered (`*_b1c1n1*`): the vertex colour carries the layer blend
    /// weights (green = layer 1), not a tint.
    pub layered: bool,
    /// Alpha tested: drawn where the colour map's alpha is at least 128/255.
    pub alpha_test: bool,
    pub cull: T6Cull,
    /// `GFXS_POLYGON_OFFSET_*` 0..2: decals sit this many steps toward the
    /// camera (the shadow-map level 3 counts as none).
    pub polygon_offset: u8,
    /// The material's sort key: blended surfaces draw in this order.
    pub draw_order: u8,
    /// Unlit materials: log2 of the colour scale (`scaleRGB`), 0..7.
    pub unlit_scale_exp: u8,
    /// Extra layers 1 and 2: 0 none, 1 blend, 2 multiply (`T6Layers`).
    pub layers: [u8; 2],
    /// bo2zm M2: an `effect_*` technique: vertex colour times texture,
    /// written in display space (no exposure), fogged in display space.
    pub effect: bool,
    /// bo2zm M2: a `particlecloud_*` technique (embers, ash): an effect
    /// whose vertices are the points of a particle cloud, each drawn as a
    /// small quad facing the camera.
    pub cloud: bool,
    /// bo2zm M3: an `mc_objective` technique (the magic box's question
    /// marks): a colour pulsing between the material's colorObjMin and
    /// colorObjMax, added on.
    pub objective: bool,
    /// bo2mp: a `treecanopy` technique (ferns, leaves): its vertex colour
    /// picks the wind variant (rgb) and scales the grid light (alpha); it
    /// is no tint (`mc_treecanopy` vertex shader, fxc disassembly).
    pub foliage: bool,
    /// bo2mp: a swaying canopy (`*_treecanopy_sm_*`): the game's vertex
    /// shader turns its vertices in the wind by treeCanopySwayParms (the
    /// plain `treecanopy` sets compile that out).
    pub sway: bool,
    /// bo2mp: BO2's water (`cod7water` pools, `cod7watershore` seas and
    /// rivers): normal maps scrolled over the world, the reflection probe by
    /// fresnel, the sun's highlight; drawn blended (the game refracts the
    /// scene behind instead).
    pub water: bool,
    /// bo2mp: BO2's emissive flow (`cod7emissiveflow`: molten lava): the
    /// rock lit by the sun, a glow from its colour remap by emissive masks.
    pub flow: bool,
    /// bo2mp: BO2's emissive tile (`sw4_3d_cod7_emissive_tile`: Magma's
    /// lava rocks): a tiled colour map lit, plus two emissive maps' glow.
    /// Packed in bit 12 of a lit code (the unlit colour scale's place).
    pub tile: bool,
    /// bo2mp: BO2's tattered flag (`sw4_3d_phong_simple_flag_tatters`: Dig's
    /// tarps): its colour mixes a frayed edge texture into the diffuse by
    /// the vertex colour's red, alpha-tested on the mix. Bit 14 of a lit
    /// code.
    pub flag: bool,
    /// bo2mp: BO2's tile blend (`sw4_3d_cod7_tile_blend`, `_edge`, `_spec`:
    /// cliffs, rocks, vista mountains): two tiled colour maps mixed by a
    /// macro mask, with macro AO. Bits 12 and 14 of a lit code together.
    pub tile_blend: bool,
    /// bo2mp: BO2's view-model glass (`sw4_3d_viewmodel_transparent`: the
    /// riot shield's window): its alpha is the square root of the colour
    /// map's times the vertex colour's, and its reflection and highlight are
    /// divided by that alpha before the blend, so the clear glass still
    /// shines (fxc disassembly, techniques 4 and 6). Bit 15 of a lit code
    /// (a particle cloud's bit, which only effects use, and they are unlit).
    pub glass: bool,
    /// bo2mp: BO2's animated screen (`sw4_3d_animated_unlit`: TVs, signs,
    /// monitors): its colour map is a grid of frames played at the
    /// material's FPS (fxc disassembly, technique 4). Bit 15 of an unlit
    /// code that is no effect (a cloud's bit, which only effects use).
    pub anim: bool,
    /// bo2zm M4 retest 5: an `mc_projecteddecal_*` technique (the blood a
    /// hit paints on a zombie): its model is a box; the pixel shader (fxc
    /// disassembly) reads the scene depth and paints the texture on what lies
    /// inside the box, projected along its x axis.
    pub projected_decal: bool,
    /// bo2zm: a ghost technique (`sw4_3d_char_ghost*`, Mob's Afterlife
    /// ghost hands): a bright cyan glow, its alpha from how the surface
    /// faces the eye times two drifting edge noises.
    pub ghost: bool,
    /// bo2zm 27c: a scrolling cloud sheet (`mc_*_scrolling_zfeather`, Die
    /// Rise's cloud floor), drawn unlit: its texture drifts with the game
    /// time and it fades where the scene behind it is near.
    pub sheet: bool,
    /// bo2zm round 1: a zombie's eyes (`unlit_cheap_zombie_eyes`): the
    /// texture is scaled by the eye brightness its client script set.
    pub eyes: bool,
    /// bo2mp: a lens flare (`sw4_3d_superflare_*`, Nuketown's sun): its
    /// vertex shader lays 25 ghosts out on screen from the flare's point,
    /// each a cell of the sprite atlas, times the grime map.
    pub flare: bool,
}

/// A view-model glass technique set (`*_sw4_3d_viewmodel_transparent_*`),
/// or the broken-window shards' (`mc_sw4_3d_glass_scaled_*`): the same pixel
/// shader but for UVs scaled about 0.5 and the highlight and reflection
/// times Specular_Amount and Reflection_Amount (fxc disassembly, technique 4).
fn glass_technique(name: &str) -> bool {
    name.contains("viewmodel_transparent") || name.contains("sw4_3d_glass_scaled")
}

/// A tile blend technique set (`*_cod7_tile_blend*`).
fn tile_blend_technique(name: &str) -> bool {
    name.contains("cod7_tile_blend")
}

/// A tattered flag technique set (`*_flag_tatters_*`).
fn flag_technique(name: &str) -> bool {
    name.contains("flag_tatters")
}

/// A layer kind in a draw code's two bits: a test layer as blend (its own
/// bit says test).
fn layer_code(kind: u8) -> u32 {
    if kind == 4 { 1 } else { u32::from(kind & 3) }
}

/// An emissive tile technique set (`*_cod7_emissive_tile_*`).
fn tile_technique(name: &str) -> bool {
    name.contains("cod7_emissive_tile")
}

/// An emissive flow technique set (`*_cod7emissiveflow_*`).
fn flow_technique(name: &str) -> bool {
    name.split('_').any(|t| t == "cod7emissiveflow")
}

/// An animated screen technique set (`*_sw4_3d_animated_unlit_*`).
fn anim_technique(name: &str) -> bool {
    name.contains("animated_unlit")
}

/// A foliage technique set (`*_treecanopy_*`).
fn foliage_technique(name: &str) -> bool {
    name.split('_').any(|t| t.starts_with("treecanopy"))
}

/// A swaying canopy technique set (`treecanopy` then `sm`).
fn sway_technique(name: &str) -> bool {
    let mut tokens = name.split('_');
    while let Some(t) = tokens.next() {
        if t.starts_with("treecanopy") {
            return tokens.next() == Some("sm");
        }
    }
    false
}

/// A projected decal technique set (`mc_projecteddecal_*`).
fn projected_decal_technique(name: &str) -> bool {
    name.trim_start_matches(',')
        .split('_')
        .any(|t| t == "projecteddecal")
}

/// A water technique set this engine draws as water (`*_cod7water_*`,
/// `*_cod7watershore_*`, `*_cod7waterflowreflect_*`: Concert's pond).
pub fn t6_water_kind(name: &str) -> Option<u8> {
    name.trim_start_matches(',')
        .split('_')
        .find_map(|t| match t {
            "cod7water" => Some(0),
            "cod7watershore" => Some(1),
            "cod7waterflowreflect" => Some(2),
            _ => None,
        })
}

/// An objective technique set (`mc_objective_*`).
fn objective_technique(name: &str) -> bool {
    name.trim_start_matches(',')
        .split('_')
        .any(|t| t == "objective")
}

/// A ghost technique set (`mc_sw4_3d_char_ghost2_*`).
fn ghost_technique(name: &str) -> bool {
    name.contains("_char_ghost")
}

/// A lens flare technique set whose vertex shader is ported: Nuketown's sun
/// (`sw4_3d_superflare_mp_nuketown_sun1`). Each superflare technique bakes
/// its own ghost table, so the others stay hidden until theirs is ported.
fn flare_technique(name: &str) -> bool {
    name.contains("superflare_mp_nuketown_sun1")
}

/// A technique set that draws an effect: `effect_*`, or `particlecloud_*`
/// (an effect's particle cloud).
fn effect_technique(name: &str) -> bool {
    let name = name.trim_start_matches(',');
    name.starts_with("effect") || name.starts_with("particlecloud")
}

/// A technique set never drawn in colour (`T6Blend::ShadowOnly`): a shadow
/// caster, a distortion or a light sphere.
fn never_coloured(name: &str) -> bool {
    name.split('_').any(|t| t == "shadowcaster")
        || name.trim_start_matches(',').starts_with("distortion")
        || name.contains("sw4_3d_light_sphere")
}

/// Decoded fields of a T6 `GfxStateBits::loadBits` pair (bit positions per
/// the T6 `GfxStateBitsLoadBits` layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct T6StateBits {
    /// GFXS_BLEND_*: 1 zero, 2 one, 3 src colour, 5 src alpha, 6 stow_kind src
    /// alpha, 9 dst colour.
    pub src_blend: u8,
    pub dst_blend: u8,
    /// GFXS_BLENDOP_*: 0 = blending off.
    pub blend_op: u8,
    /// `Some(1)` = alpha test at 128/255, `Some(0)` = above 0.
    pub alpha_test: Option<u8>,
    /// GFXS_CULL_*: 1 none, 2 back, 3 front.
    pub cull_mode: u8,
    pub color_write_rgb: bool,
    pub polygon_offset: u8,
}

impl T6StateBits {
    pub fn decode(bits: [u32; 2]) -> Self {
        let [a, b] = bits;
        let f = |v: u32, at: u32, n: u32| ((v >> at) & ((1 << n) - 1)) as u8;
        Self {
            src_blend: f(a, 0, 4),
            dst_blend: f(a, 4, 4),
            blend_op: f(a, 8, 3),
            alpha_test: (f(a, 11, 1) == 0).then(|| f(a, 12, 1)),
            cull_mode: f(a, 14, 2),
            color_write_rgb: f(a, 27, 1) != 0,
            polygon_offset: f(b, 4, 2),
        }
    }
}

const GFXS_BLEND_ZERO: u8 = 1;
const GFXS_BLEND_ONE: u8 = 2;
const GFXS_BLEND_SRCCOLOR: u8 = 3;
const GFXS_BLEND_DESTCOLOR: u8 = 9;

impl T6Draw {
    /// From the technique set's name, the material's sort key and, when the
    /// material has one, its main technique's `loadBits`.
    pub fn from_technique_set(set_name: &str, order: u8, packed: Option<[u32; 2]>) -> Self {
        // `effect_*` techniques (sprites, flashes, smoke) sample no lightmap
        // or grid: the effect system bakes their lighting into the vertex
        // colour (`lightingFrac`), so they draw unlit.
        let effect = effect_technique(set_name);
        let cloud = set_name.trim_start_matches(',').starts_with("particlecloud");
        let objective = objective_technique(set_name);
        let unlit = set_name.split('_').any(|t| t.starts_with("unlit")) || effect || objective;
        // bo2zm M3 fix list 2: a multiply layer (`m1c1`) is a layer too: its
        // vertex colour is the layer weights, not a tint (the garage door's
        // blast-crater layer drew the whole door purple, 255/63/255).
        let layered = set_name.split('_').any(|t| t.starts_with("b1"))
            || T6Layers::from_technique_set(set_name).any();
        let sky = set_name.split('_').any(|t| t.starts_with("sky"));
        let occluder_exclusive = never_coloured(set_name);
        let Some(bits) = packed else {
            return Self::from_name(set_name, order);
        };
        let st = T6StateBits::decode(bits);
        let water = t6_water_kind(set_name).is_some();
        let blend = if sky {
            T6Blend::Sky
        } else if occluder_exclusive || !st.color_write_rgb {
            T6Blend::CasterOnly
        } else if water {
            T6Blend::Blend
        } else if st.blend_op == 0 {
            T6Blend::Opaque
        } else if st.dst_blend == GFXS_BLEND_ONE {
            T6Blend::Add
        } else if (st.src_blend == GFXS_BLEND_ZERO && st.dst_blend == GFXS_BLEND_SRCCOLOR)
            || (st.src_blend == GFXS_BLEND_DESTCOLOR && st.dst_blend == GFXS_BLEND_ZERO)
        {
            T6Blend::Multiply
        } else {
            T6Blend::Blend
        };
        Self {
            blend,
            unlit,
            layered,
            alpha_test: st.alpha_test == Some(1),
            // A cloud's quads always face the camera.
            cull: match st.cull_mode {
                _ if cloud => T6Cull::None,
                2 => T6Cull::Back,
                3 => T6Cull::Front,
                _ => T6Cull::None,
            },
            polygon_offset: if st.polygon_offset == 3 {
                0
            } else {
                st.polygon_offset
            },
            draw_order: order,
            unlit_scale_exp: 0,
            layers: {
                let l = T6Layers::from_technique_set(set_name);
                [l.kinds[0], l.kinds[1]]
            },
            effect,
            cloud,
            objective,
            foliage: foliage_technique(set_name),
            sway: sway_technique(set_name),
            water,
            flow: flow_technique(set_name),
            tile: tile_technique(set_name),
            flag: flag_technique(set_name),
            tile_blend: tile_blend_technique(set_name),
            glass: glass_technique(set_name),
            anim: anim_technique(set_name),
            projected_decal: projected_decal_technique(set_name),
            ghost: ghost_technique(set_name),
            sheet: false,
            eyes: false,
            flare: flare_technique(set_name),
        }
    }

    /// The name-only reading, for a material without a draw state.
    fn from_name(set_name: &str, order: u8) -> Self {
        let has = |word: &str| set_name.split('_').any(|t| t == word);
        let unlit = set_name.split('_').any(|t| t.starts_with("unlit"));
        // bo2zm M3 fix list 2: a multiply layer (`m1c1`) is a layer too: its
        // vertex colour is the layer weights, not a tint (the garage door's
        // blast-crater layer drew the whole door purple, 255/63/255).
        let layered = set_name.split('_').any(|t| t.starts_with("b1"))
            || T6Layers::from_technique_set(set_name).any();
        let base = set_name.split('_').find(|t| {
            let b = t.as_bytes();
            b.len() >= 4 && matches!(b[0], b'r' | b'b' | b't') && b[1] == b'0' && b[2] == b'c'
        });
        let base_kind = base.map(|t| t.as_bytes()[0]);
        let tested =
            base_kind == Some(b't') || set_name.split('_').any(|t| t.starts_with("treecanopy"));
        let blend = if set_name.split('_').any(|t| t.starts_with("sky")) {
            T6Blend::Sky
        } else if never_coloured(set_name) {
            T6Blend::CasterOnly
        } else if has("multiply") {
            T6Blend::Multiply
        } else if has("add") {
            T6Blend::Add
        } else if has("blend") || base_kind == Some(b'b') {
            T6Blend::Blend
        } else if has("replace") || order <= 4 {
            T6Blend::Opaque
        } else {
            T6Blend::Blend
        };
        Self {
            blend,
            unlit,
            layered,
            alpha_test: tested,
            cull: T6Cull::None,
            polygon_offset: 0,
            draw_order: order,
            unlit_scale_exp: 0,
            layers: {
                let l = T6Layers::from_technique_set(set_name);
                [l.kinds[0], l.kinds[1]]
            },
            effect: effect_technique(set_name),
            cloud: set_name.trim_start_matches(',').starts_with("particlecloud"),
            objective: objective_technique(set_name),
            foliage: foliage_technique(set_name),
            sway: sway_technique(set_name),
            water: t6_water_kind(set_name).is_some(),
            flow: flow_technique(set_name),
            tile: tile_technique(set_name),
            flag: flag_technique(set_name),
            tile_blend: tile_blend_technique(set_name),
            glass: glass_technique(set_name),
            anim: anim_technique(set_name),
            projected_decal: projected_decal_technique(set_name),
            ghost: ghost_technique(set_name),
            sheet: false,
            eyes: false,
            flare: flare_technique(set_name),
        }
    }

    /// One word for per-surface tables: blend in bits 0..3, unlit in bit 4,
    /// layered in bit 6, alpha tested in bit 7, cull in bits 8..9 (0 none,
    /// 1 back, 2 front), polygon offset in bits 10..11, the unlit colour
    /// scale's log2 in bits 12..14, a particle cloud in bit 15, the sort
    /// key in bits 16..23, extra layers 1 and 2 in bits 24..25 and 26..27, an
    /// objective in bit 28, foliage in bit 29, water in bit 30, emissive flow in bit 31;
    /// a lit emissive tile in bit 12, a lit view-model glass in bit 15, an unlit animated screen in bit 15, a lit layer 1 test layer in bit 13
    /// (a layer 2 test layer is drawn as blend), a lit projected decal as bits
    /// 12..14 together, a ghost as objective and foliage together, a scrolling
    /// cloud sheet as cloud, water and flow together. `NONE_CODE` marks a
    /// surface with no BO2 draw (no real code has a low nibble of 0xf).
    pub fn code(self) -> u32 {
        let blend: u32 = match self.blend {
            T6Blend::Opaque => 0,
            T6Blend::Blend => 1,
            T6Blend::Add => 2,
            T6Blend::Multiply => 3,
            T6Blend::CasterOnly => 4,
            T6Blend::Sky => 5,
        };
        let cull: u32 = match self.cull {
            T6Cull::None => 0,
            T6Cull::Back => 1,
            T6Cull::Front => 2,
        };
        blend
            | if self.unlit { 0x10 } else { 0 }
            | if self.layered { 0x40 } else { 0 }
            | if self.alpha_test { 0x80 } else { 0 }
            | cull << 8
            | u32::from(self.polygon_offset & 3) << 10
            | u32::from(self.unlit_scale_exp & 7) << 12
            | u32::from(self.draw_order) << 16
            | layer_code(self.layers[0]) << 24
            | layer_code(self.layers[1]) << 26
            | if self.effect { 0x20 } else { 0 }
            | if self.cloud { 0x8000 } else { 0 }
            | if self.sheet { Self::SHEET_MARK } else { 0 }
            | if self.objective { 0x1000_0000 } else { 0 }
            | if self.foliage { 0x2000_0000 } else { 0 }
            | if self.water { 0x4000_0000 } else { 0 }
            | if self.flow { 0x8000_0000 } else { 0 }
            | if self.tile && !self.unlit { 0x1000 } else { 0 }
            | if self.layers[0] == 4 && !self.unlit {
                0x2000
            } else {
                0
            }
            | if self.flag && !self.unlit { 0x4000 } else { 0 }
            | if self.tile_blend && !self.unlit {
                0x5000
            } else {
                0
            }
            | if self.glass && !self.unlit { 0x8000 } else { 0 }
            | if self.anim && self.unlit && !self.effect { 0x8000 } else { 0 }
            | if self.projected_decal && !self.unlit && !self.effect { 0x7000 } else { 0 }
            | if self.ghost { Self::GHOST_MARK } else { 0 }
            | if self.flare { Self::FLARE_MARK } else { 0 }
    }

    /// A scrolling cloud sheet: an unlit code with the cloud bit (15) and
    /// water and flow (30, 31) together, which no water or flow surface sets.
    pub const SHEET_MARK: u32 = 0xc000_8000;
    /// A ghost: objective and foliage (28, 29) together.
    pub const GHOST_MARK: u32 = 0x3000_0000;
    /// bo2mp: a lens flare: an effect with the cloud bit (15) and water (30)
    /// but not flow (31), which no cloud, sheet or water surface sets.
    pub const FLARE_MARK: u32 = 0x4000_8020;

    pub const NONE_CODE: u32 = 0xffff_ffff;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_clouds_are_effects() {
        // Additive (one, one), back-face culled: drawn as an uncull effect.
        let add = [2 | 2 << 4 | 1 << 8 | 1 << 11 | 2 << 14 | 1 << 27, 0];
        let d = T6Draw::from_technique_set("particlecloud_zqe90390", 9, Some(add));
        assert!(d.effect && d.cloud && d.unlit);
        assert_eq!(d.blend, T6Blend::Add);
        assert_eq!(d.cull, T6Cull::None);
        assert_eq!(d.code() & 0x8020, 0x8020);
        let sprite = T6Draw::from_technique_set("effect_50567j38", 9, Some(add));
        assert!(sprite.effect && !sprite.cloud);
        assert_eq!(sprite.cull, T6Cull::Back);
    }

    #[test]
    fn riot_shield_glass() {
        // mc/mtl_t6_wpn_shield_glass_sortfx as common.ff carries it: sort 56,
        // alpha blend (src alpha, 1 - src alpha), cull back.
        let st = [0x1928_8165, 0xc];
        let g = T6Draw::from_technique_set(",mc_sw4_3d_viewmodel_transparent_z9z0z75f", 56, Some(st));
        assert!(g.glass && !g.unlit);
        assert_eq!(g.blend, T6Blend::Blend);
        assert_eq!(g.code() & 0x8000, 0x8000);
        // Another blended lit model keeps the bit clear.
        let other = T6Draw::from_technique_set("mc_lit_sm_b0c0n0_x", 56, Some(st));
        assert!(!other.glass);
        assert_eq!(other.code() & 0x8000, 0);
        // mc/glass_clear_shatter_mp_model, a broken window's shards: the
        // same glass.
        let shard = T6Draw::from_technique_set("mc_sw4_3d_glass_scaled_z6e86f4e", 56, Some(st));
        assert!(shard.glass && !shard.unlit);
        assert_eq!(shard.code() & 0x8000, 0x8000);
    }

    #[test]
    fn nuketown_technique_sets() {
        let d = |n: &str, s: u8| T6Draw::from_technique_set(n, s, None);
        assert_eq!(d("wpc_lit_sm_r0c0n0x0_f488034e", 4).blend, T6Blend::Opaque);
        assert!(!d("lit_sm_r0c0n0_b1c1n1", 4).unlit);
        assert!(d("lit_sm_r0c0n0_b1c1n1", 4).layered);
        assert!(!d("wpc_lit_sm_r0c0n0x0_f488034e", 4).layered);
        assert!(d("lit_sm_r0c0n0x0_m1c1", 4).layered);
        assert!(d("mc_lit_sm_t0c0n0_9qf6e4qj", 4).alpha_test);
        assert!(d("mlv_treecanopy_sm_5q2j2275", 4).alpha_test);
        assert!(!d("wpc_lit_sm_r0c0n0x0_f488034e", 4).alpha_test);
        assert_eq!(d("mc_lit_sm_b0c0n0_x", 4).blend, T6Blend::Blend);
        let decal = d("wpc_unlitdecalblend_multiply_35079164", 12);
        assert_eq!(decal.blend, T6Blend::Multiply);
        assert!(decal.unlit);
        assert_eq!(d("wpc_unlit_blend_2840z6q0", 40).blend, T6Blend::Blend);
        assert_eq!(d("wpc_shadowcaster_wj6w5j60", 4).blend, T6Blend::CasterOnly);
        assert_eq!(d(",distortion_81587199", 48).blend, T6Blend::CasterOnly);
        assert_eq!(
            d("mc_sw4_3d_light_sphere_46j58w7u", 48).blend,
            T6Blend::CasterOnly
        );
        assert_eq!(d("mc_skycubemaphdr_1f117w7u", 4).blend, T6Blend::Sky);
    }

    /// Draw states as Nuketown's materials carry them (t6mat T6MAT_STATES).
    #[test]
    fn nuketown_draw_states() {
        // Opaque, cull back: the rocks' "tile_blend" shader is not a blend.
        let opaque_back = [(2 << 14) | (1 << 11) | (1 << 27) | (1 << 28), 1 | (3 << 2)];
        let rock =
            T6Draw::from_technique_set("mc_sw4_3d_cod7_tile_blend_q2q2zej6", 4, Some(opaque_back));
        assert_eq!(rock.blend, T6Blend::Opaque);
        assert_eq!(rock.cull, T6Cull::Back);
        assert!(!rock.alpha_test);
        // Premultiplied blend (1, 1-sa), alpha test above 0, decal offset 1.
        let blend = [
            2 | (6 << 4) | (1 << 8) | (2 << 14) | (1 << 27),
            (3 << 2) | (1 << 4),
        ];
        let b = T6Draw::from_technique_set("wpc_lit_sm_b0c0n0x0_z1wzf739", 40, Some(blend));
        assert_eq!(b.blend, T6Blend::Blend);
        assert!(!b.alpha_test);
        assert_eq!(b.polygon_offset, 1);
        assert_eq!(b.code() >> 16, 40);
        // Multiply decal: 0 * src + src colour * dst, offset 2.
        let mul = [
            1 | (3 << 4) | (1 << 8) | (1 << 11) | (2 << 14) | (1 << 27),
            (3 << 2) | (2 << 4),
        ];
        let m = T6Draw::from_technique_set("wpc_unlitdecalblend_multiply_35079164", 12, Some(mul));
        assert_eq!(m.blend, T6Blend::Multiply);
        assert!(m.unlit);
        assert_eq!(m.polygon_offset, 2);
        // Alpha tested at 128, both faces: burning planks, hedges.
        let atest = [(1 << 12) | (1 << 14) | (1 << 27), 1 | (3 << 2)];
        let a = T6Draw::from_technique_set("mlv_treecanopy_sm_5q2j2275", 4, Some(atest));
        assert!(a.alpha_test);
        assert_eq!(a.cull, T6Cull::None);
        assert_eq!(a.blend, T6Blend::Opaque);
        // The `_sm` canopy sways; a plain one is still foliage.
        assert!(a.foliage && a.sway);
        let plain = T6Draw::from_technique_set("mc_treecanopy_5q2j2275", 4, None);
        assert!(plain.foliage && !plain.sway);
    }

    /// Layer layouts as measured on Nuketown's second vertex stream.
    #[test]
    fn layer_layouts() {
        let l = |n: &str| T6Layers::from_technique_set(n);
        assert_eq!(l("lit_sm_r0c0n0_b1c1n1").stride, 8);
        assert_eq!(l("lit_sm_r0c0_b1c1n1").stride, 4);
        assert_eq!(l("lit_sm_r0c0n0x0_m1c1").stride, 4);
        let m = l("lit_sm_r0c0n0x0_b1c1n1_m2c2");
        assert_eq!(m.stride, 12);
        assert_eq!(m.kinds, [1, 2, 0]);
        assert_eq!(m.uv_offsets[1], 8);
        assert_eq!(l("lit_sm_r0c0n0s0_b1c1n1x1_m2c2_m3c3").stride, 16);
        assert_eq!(l("lit_sm_r0c0n0x0_b1c1n1_b2c2n2").stride, 16);
        assert_eq!(l("lit_sm_r0c0n0x0_b1c1v1").stride, 4);
        // Die Rise's paper decals: a tested layer, then a blend layer.
        let t = l("lit_sm_r0c0n0x0_t1c1n1_b2c2n2s2v2");
        assert_eq!(t.kinds, [4, 1, 0]);
        assert_eq!(t.stride, 16);
        // Origins: an add layer, then a multiply layer.
        let a = l("lit_sm_r0c0n0x0_a1c1_m2c2");
        assert_eq!(a.kinds, [3, 2, 0]);
        assert_eq!(a.uv_offsets[1], 4);
        assert!(!l("wpc_lit_sm_r0c0n0x0_f488034e").any());
        assert!(!l("wpc_lit_sm_b0c0n0_19f3588f").any());
    }
}
