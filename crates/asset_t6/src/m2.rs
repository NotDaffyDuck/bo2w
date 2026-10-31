//! bo2zm M2: what a T6 zone walk keeps for guns, effects and sound.
//!
//! Weapons (`WeaponVariantDef` + its shared `WeaponDef`), effects
//! (`FxEffectDef` and its elements), tracers, the bullet impact table, the
//! footstep tables, sound banks (aliases) and the sound driver's falloff
//! curves. Everything is copied while the asset is still in place; asset
//! pointers become names (or keys into the capture's lists), so nothing
//! here points into a zone that has closed.

use fastfile_t6::layout::{
    Font_s, FootstepFXTableDef, FootstepTableDef, FxEffectDef, FxElemDef, FxElemMarkVisuals,
    FxElemVelStateSample, FxElemVisStateSample, FxHitRow, FxHitTable, FxSpotLightDef,
    FxStreakDef, Glyph, KerningPairs, SndAlias, SndAliasList, SndBank, SndCurve, SndDriverGlobals,
    SndDuck, SndDuckGroup, SndMaster, SndPan, SndRadverb, SndVolumeGroup, TracerDef,
    WeaponAttachment, WeaponAttachmentUnique, WeaponCamo, WeaponCamoMaterial, WeaponCamoMaterialSet,
    WeaponCamoSet, WeaponDef, WeaponVariantDef,
};
use fastfile_t6::{XAssetKind, RawRef, ZoneByteReader};

use crate::zone_grab::{AssetIdentity, ZoneCapture, deref, string_field};

type Result<T> = fastfile_t6::Result<T>;

/// `WeaponDef` fields that are `const char*` (sound aliases, names), by
/// offset, from the generated zone walk (`ld_WeaponDef`'s `xstring` calls).
pub const WEAPON_DEF_STRINGS: [usize; 104] = [
    0, 12, 64, 148, 152, 156, 160, 164, 168, 172, 176, 180, 184, 188, 192, 196, 200, 204, 208, 212,
    216, 220, 224, 228, 232, 236, 240, 244, 248, 252, 256, 260, 264, 268, 272, 276, 280, 284, 288,
    292, 296, 300, 304, 308, 312, 316, 320, 324, 328, 332, 336, 340, 344, 348, 352, 356, 360, 364,
    368, 372, 376, 380, 384, 388, 392, 396, 400, 404, 408, 412, 416, 420, 424, 428, 432, 440, 444,
    448, 980, 1100, 1104, 1108, 1112, 1116, 1120, 1388, 1652, 1656, 1820, 1824, 1828, 1832, 1920,
    2124, 2128, 2236, 2240, 2284, 2304, 2308, 2312, 2316, 2356, 2360,
];

/// `WeaponDef` fields that point at an `FxEffectDef`.
pub const WEAPON_DEF_FX: [usize; 20] = [
    108, 112, 116, 464, 468, 472, 476, 1776, 1784, 1792, 1800, 1808, 1816, 1888, 1916, 2372, 2376,
    2404, 2408, 2412,
];

/// `WeaponDef` fields that point at a `Material`.
pub const WEAPON_DEF_MATERIALS: [usize; 7] = [528, 532, 936, 940, 948, 956, 1632];

/// `WeaponDef` fields that point at one `XModel` (`gunXModel` and
/// `worldModel` are arrays of 16, read separately).
pub const WEAPON_DEF_XMODELS: [usize; 6] = [8, 920, 924, 928, 932, 1768];

/// `WeaponDef` fields that point at a `TracerDef`.
pub const WEAPON_DEF_TRACERS: [usize; 2] = [2320, 2324];

/// Number of `szXAnims` slots in T6 (`NUM_WEAP_ANIMS`).
pub const GUN_MOTION_TOTAL: usize = 88;

/// Number of surface types T6 keeps a `bounceSound` alias for.
pub const SURF_TYPE_COUNT: usize = 32;

/// bo2zm M3: a font (`Font_s`): its glyphs on the sheet its material draws
/// (Black Ops II's game text is `fonts/distFont`, a distance-field sheet).
#[derive(Clone, Debug, Default)]
pub struct FontRef {
    pub name: String,
    pub raster_rows: i32,
    pub material: String,
    pub glyphs: Vec<GlyphRef>,
    /// (first letter, second letter, amount).
    pub kerning: Vec<(u16, u16, i32)>,
}

/// One letter_shape: its letter, its box against the pen (x0, y0 from the pen and
/// the baseline, step_along_x the advance, pixel size) and its place on the sheet.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlyphRef {
    pub letter: u16,
    pub x0: i8,
    pub y0: i8,
    pub step_along_x: u8,
    pub texel_span: u8,
    pub raster_rows: u8,
    pub st: [f32; 4],
}

/// bo2mp: a weapon attachment (`WeaponAttachment`, 284 bytes): what it
/// changes on any gun (zoom, ADS time scales, sway).
#[derive(Clone, Debug, Default)]
pub struct AttachmentRef {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl AttachmentRef {
    pub fn int_from_bytes(&self, pos: usize) -> i32 {
        self.bytes.get(pos..pos + 4).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn float_from_bytes(&self, pos: usize) -> f32 {
        self.bytes.get(pos..pos + 4).map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn byte_field(&self, pos: usize) -> u8 {
        self.bytes.get(pos).copied().unwrap_or(0)
    }
}

/// bo2mp: one gun's take on an attachment (`WeaponAttachmentUnique`, 424
/// bytes, `au_<gun>_<attachment>`): its models, sight, overlay, anims.
#[derive(Clone, Debug, Default)]
pub struct AttachmentUniqueRef {
    pub name: String,
    pub bytes: Vec<u8>,
    pub view_model: String,
    pub view_model_additional: String,
    pub view_model_ads: String,
    /// `szAltWeaponName`: the weapon this attachment switches to (Hybrid, Dual Band).
    pub alt_weapon_name: String,
    pub third_person_mesh: String,
    pub view_model_tag: String,
    pub world_model_tag: String,
    pub hud_layer_surface: String,
    pub overlay_material_low: String,
    /// `szXAnims` (88 slots, empty = the gun's own).
    pub animation_refs: Vec<String>,
    pub concealed_bones: Vec<String>,
    /// bo2mp: its fire sounds (a Suppressor's), empty = the gun's own:
    /// `fireSound`, `fireSoundPlayer`, `fireLoopSound`,
    /// `fireLoopSoundPlayer`, `fireStopSound`, `fireStopSoundPlayer`,
    /// `fireLastSound`, `fireLastSoundPlayer`.
    pub fire_sounds: [String; 8],
    /// bo2mp: `szDualWieldWeaponName`: the left-hand gun a Dual Wield
    /// attachment (`dw`) puts in the other hand.
    pub dual_wield_weapon_name: String,
    /// bo2mp: its muzzle flashes (`viewFlashEffect`, `worldFlashEffect`: a
    /// Suppressor's small ones), empty = the gun's own.
    pub view_flash: String,
    pub world_flash: String,
    /// bo2mp: `locationDamageMultipliers` (21 hit locations), None = no table.
    pub location_damage: Option<Vec<f32>>,
}

impl AttachmentUniqueRef {
    pub fn int_from_bytes(&self, pos: usize) -> i32 {
        self.bytes.get(pos..pos + 4).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn float_from_bytes(&self, pos: usize) -> f32 {
        self.bytes.get(pos..pos + 4).map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn vec3_at(&self, pos: usize) -> [f32; 3] {
        [self.float_from_bytes(pos), self.float_from_bytes(pos + 4), self.float_from_bytes(pos + 8)]
    }
}

/// bo2zm M3: a weapon camo (`WeaponCamo`): per camo index its images
/// (`WeaponCamoSet`: solid, pattern, the pattern's offset and scale) and
/// its material swaps (`WeaponCamoMaterialSet`: each base material, the
/// camo material drawn instead, the shader constants).
#[derive(Clone, Debug, Default)]
pub struct WeaponCamoRef {
    pub name: String,
    pub solid_base_image: String,
    pub pattern_base_image: String,
    pub sets: Vec<WeaponCamoSetRef>,
    pub material_sets: Vec<Vec<WeaponCamoMaterialRef>>,
}

#[derive(Clone, Debug, Default)]
pub struct WeaponCamoSetRef {
    pub solid: String,
    pub pattern: String,
    pub offset: [f32; 2],
    pub scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct WeaponCamoMaterialRef {
    pub replace_flags: u16,
    pub base: Vec<String>,
    pub camo: Vec<String>,
    pub consts: [f32; 8],
}

/// One weapon: its variant and its shared definition, as bytes plus every
/// name they point at.
#[derive(Clone, Debug, Default)]
pub struct WeaponRef {
    pub name: String,
    /// `WeaponVariantDef`, 716 bytes.
    pub variant: Vec<u8>,
    /// `WeaponDef`, 2448 bytes (empty when the variant has none).
    pub def: Vec<u8>,
    pub shown_label: String,
    pub alt_weapon_name: String,
    pub ammo_name: String,
    pub collision_label: String,
    /// The variant's `szXAnims` (88 slots).
    pub animation_refs: Vec<String>,
    /// The variant's `hideTags` (bone names).
    pub concealed_bones: Vec<String>,
    /// The variant's `attachViewModel` slots that hold a model: (slot,
    /// model, `attachViewModelTag`, offset, rotation).
    pub attach_view_models: Vec<(usize, String, String, [f32; 3], [f32; 3])>,
    /// The same for `attachWorldModel`: the magazine, scope or rocket Black
    /// Ops II hangs on the gun everyone else sees (on a wall, in the box).
    pub attach_world_models: Vec<(usize, String, String, [f32; 3], [f32; 3])>,
    /// `gunXModel[16]` and `worldModel[16]`.
    pub gun_models: Vec<String>,
    pub world_models: Vec<String>,
    /// `WeaponDef` string fields by offset (`WEAPON_DEF_STRINGS`).
    pub strings: Vec<(usize, String)>,
    /// `WeaponDef` asset pointers by offset: effects, materials, models,
    /// tracers (`WEAPON_DEF_FX` etc.).
    pub fx: Vec<(usize, String)>,
    pub materials: Vec<(usize, String)>,
    pub models: Vec<(usize, String)>,
    pub tracers: Vec<(usize, String)>,
    /// The variant's materials: overlay (scope), low-res overlay, d-pad icon.
    pub hud_layer_surface: String,
    pub overlay_material_low: String,
    pub dpad_icon: String,
    /// `notetrackSoundMapKeys` / `Values` (20 pairs, script strings).
    pub notetrack_sounds: Vec<(String, String)>,
    /// `bounceSound`: a projectile's bounce alias per surface type (32).
    pub bounce_sounds: Vec<String>,
    pub along_rebound: Option<Vec<f32>>,
    pub across_rebound: Option<Vec<f32>>,
    pub location_damage: Option<Vec<f32>>,
    /// bo2zm M3: its camo (`weaponCamo`), by name.
    pub camo: String,
}

impl WeaponRef {
    fn at<const N: usize>(bytes: &[u8], pos: usize) -> Option<[u8; N]> {
        bytes.get(pos..pos + N)?.try_into().ok()
    }

    pub fn def_i32(&self, pos: usize) -> i32 {
        Self::at::<4>(&self.def, pos).map_or(0, i32::from_le_bytes)
    }

    pub fn def_f32(&self, pos: usize) -> f32 {
        Self::at::<4>(&self.def, pos).map_or(0.0, f32::from_le_bytes)
    }

    pub fn def_u8(&self, pos: usize) -> u8 {
        self.def.get(pos).copied().unwrap_or(0)
    }

    pub fn def_vec3(&self, pos: usize) -> [f32; 3] {
        [
            self.def_f32(pos),
            self.def_f32(pos + 4),
            self.def_f32(pos + 8),
        ]
    }

    pub fn var_i32(&self, pos: usize) -> i32 {
        Self::at::<4>(&self.variant, pos).map_or(0, i32::from_le_bytes)
    }

    pub fn var_f32(&self, pos: usize) -> f32 {
        Self::at::<4>(&self.variant, pos).map_or(0.0, f32::from_le_bytes)
    }

    pub fn var_u8(&self, pos: usize) -> u8 {
        self.variant.get(pos).copied().unwrap_or(0)
    }

    /// A `WeaponDef` string field by offset ("" when unset).
    pub fn def_string(&self, pos: usize) -> &str {
        named_at(&self.strings, pos)
    }

    pub fn def_fx(&self, pos: usize) -> &str {
        named_at(&self.fx, pos)
    }

    pub fn def_material(&self, pos: usize) -> &str {
        named_at(&self.materials, pos)
    }

    pub fn def_model(&self, pos: usize) -> &str {
        named_at(&self.models, pos)
    }

    pub fn def_tracer(&self, pos: usize) -> &str {
        named_at(&self.tracers, pos)
    }
}

/// The name kept for a `WeaponDef` field offset ("" when unset).
fn named_at(list: &[(usize, String)], pos: usize) -> &str {
    list.iter()
        .find(|(o, _)| *o == pos)
        .map_or("", |(_, s)| s.as_str())
}

/// One effect visual.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FxVisualRef {
    Material(String),
    /// A decal's pair (world, model).
    Mark([String; 2]),
    Model(String),
    /// A runner's effect, by name.
    Effect(String),
    Sound(String),
    Light(String),
    None,
}

/// One `FxElemDef`: its 292 bytes, samples and what it points at.
#[derive(Clone, Debug, Default)]
pub struct FxElemRef {
    pub raw: Vec<u8>,
    /// `velSamples`: (velIntervalCount + 1) x 96 bytes.
    pub speed_readings: Vec<u8>,
    /// `visSamples`: (visStateIntervalCount + 1) x 48 bytes.
    pub visibility_readings: Vec<u8>,
    pub visuals: Vec<FxVisualRef>,
    pub effect_upon_strike: String,
    pub effect_upon_demise: String,
    pub effect_spawned: String,
    pub effect_attached: String,
    /// A trail's definition: (scroll time, repeat distance, split
    /// distance, vertices as (pos xy, tex coord), indices).
    pub trail: Option<FxTrailRef>,
    /// A spot light's (fov inner fraction, start radius, end radius).
    pub spot: Option<[f32; 3]>,
    pub spawn_sound: String,
}

#[derive(Clone, Debug, Default)]
pub struct FxTrailRef {
    pub scroll_period_millis: i32,
    pub recurrence_distance: i32,
    pub split_dist: f32,
    /// (pos x, pos y, normal x, normal y, tex coord) per vertex.
    pub vertex_points: Vec<[f32; 5]>,
    pub index_run_values: Vec<u16>,
}

impl FxElemRef {
    pub fn elem_type(&self) -> u8 {
        self.raw.get(FxElemDef::elemType).copied().unwrap_or(0)
    }

    pub fn int_from_bytes(&self, pos: usize) -> i32 {
        self.raw
            .get(pos..pos + 4)
            .map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn float_from_bytes(&self, pos: usize) -> f32 {
        self.raw
            .get(pos..pos + 4)
            .map_or(0.0_f32, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// One `FxEffectDef`.
#[derive(Clone, Debug, Default)]
pub struct FxEffectRef {
    pub name: String,
    pub flags: u16,
    pub priority: u8,
    pub repeat_total: i16,
    pub single_play_total: i16,
    pub spawn_total: i16,
    pub total_size: i32,
    pub loop_lifetime_millis: i32,
    pub msec_non_looping_life: i32,
    /// Looping elements, then one-shot, then emitted.
    pub elems: Vec<FxElemRef>,
    pub bounds_dim: [f32; 3],
    pub bounds_centre: [f32; 3],
}

/// One `TracerDef`.
#[derive(Clone, Debug, Default)]
pub struct TracerRef {
    pub name: String,
    pub ty: i32,
    pub material: String,
    pub render_spacing: i32,
    pub speed: f32,
    pub ray_extent: f32,
    pub ray_thickness: f32,
    pub spiral_radius: f32,
    pub spiral_pitch_distance: f32,
    pub fade_secs: f32,
    pub fade_scale: f32,
    pub tex_repeat_rate: f32,
    pub colors: [[f32; 4]; 5],
}

/// `FxImpactTable`: 21 impact types x (32 non-flesh surfaces + 4 flesh).
#[derive(Clone, Debug, Default)]
pub struct ImpactTableRef {
    pub name: String,
    pub rows: Vec<([String; 32], [String; 4])>,
}

/// `FootstepTableDef`: per surface (32) seven alias ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FootstepTableRef {
    pub name: String,
    pub aliases: Vec<[u32; 7]>,
}

/// `FootstepFXTableDef`: per surface the effect a step kicks up.
#[derive(Clone, Debug, Default)]
pub struct FootstepFxTableRef {
    pub name: String,
    pub fx: Vec<String>,
}

/// One `SndAlias` (a variant of an alias name).
#[derive(Clone, Debug, Default)]
pub struct SndAliasRef {
    pub name: String,
    pub id: u32,
    pub subtitle: String,
    pub secondary: String,
    /// `SND_HashName` of the file name: the bank entry's id.
    pub asset_id: u32,
    pub asset_file: String,
    /// `SndAliasFlags`: flags0, flags1.
    pub flags: [u32; 2],
    pub duck: u32,
    pub context_type: u32,
    pub context_value: u32,
    pub stop_on_play: u32,
    pub futz_patch: u32,
    pub flux_time: u16,
    pub start_delay: u16,
    pub reverb_send: u16,
    pub center_send: u16,
    pub vol_min: u16,
    pub vol_max: u16,
    pub pitch_lower_bound: u16,
    pub pitch_upper_bound: u16,
    pub reach_lower: u16,
    pub reach_upper: u16,
    pub dist_reverb_max: u16,
    pub envelop_min: u16,
    pub envelop_max: u16,
    pub envelop_percentage: u16,
    pub ease_on: u16,
    pub ease_off: u16,
    pub doppler_scale: i16,
    pub min_priority_threshold: u8,
    pub max_priority_threshold: u8,
    pub chance_weight: u8,
    pub occlusion_level: u8,
    pub min_priority: u8,
    pub max_priority: u8,
    pub pan: u8,
    pub cap_total: u8,
    pub actor_cap_total: u8,
    pub duck_group: u8,
}

impl SndAliasRef {
    pub fn looping(&self) -> bool {
        self.flags[0] & 1 != 0
    }

    /// `panType`: 0 = 2D, 1 = 3D.
    pub fn has_depth_axis(&self) -> bool {
        self.flags[0] & 2 != 0
    }

    /// `loadType`: 1 loaded, 2 streamed, 3 primed.
    pub fn load_type(&self) -> u32 {
        (self.flags[0] >> 15) & 3
    }

    pub fn bus(&self) -> u32 {
        (self.flags[0] >> 11) & 0xf
    }

    pub fn volume_group(&self) -> u32 {
        (self.flags[0] >> 17) & 0x1f
    }

    /// `randomizeType` bits: 1 volume, 2 pitch, 4 variant.
    pub fn randomize(&self) -> u32 {
        (self.flags[0] >> 29) & 7
    }

    pub fn is_music(&self) -> bool {
        self.flags[0] & (1 << 6) != 0
    }

    /// bo2mp: the menu volume this variant answers to (`SndMenuCategory`: 0 effects, 1 music,
    /// 2 voice, 3 interface, 4 cinematic), as Black Ops II starts a voice: its volume group's
    /// category, except that a music alias is music and an alias with flags1 bit 27 set is
    /// cinematic.
    pub fn menu_category(&self, globals: Option<&SndDriverGlobalsRef>) -> u32 {
        if self.is_music() {
            return 1;
        }
        if self.flags[1] & (1 << 27) != 0 {
            return 4;
        }
        globals
            .and_then(|g| g.group_mix.get(self.volume_group() as usize))
            .map_or(0, |g| g.category)
    }

    /// bo2mp: Black Ops II's group scalar: the MP attenuation of this variant's volume group
    /// times that of each parent up the chain (at most 100 groups), each out of 65535.
    pub fn group_scalar(&self, globals: Option<&SndDriverGlobalsRef>) -> f32 {
        let Some(g) = globals else {
            return 1.0;
        };
        let mut scalar = 1.0;
        let mut index = self.volume_group();
        for _ in 0..100 {
            let Some(group) = g.group_mix.get(index as usize) else {
                break;
            };
            scalar *= f32::from(group.attenuation_mp) / 65535.0;
            if group.parent == u32::MAX {
                break;
            }
            index = group.parent;
        }
        scalar
    }

    pub fn volume_falloff_curve(&self) -> u32 {
        (self.flags[1] >> 2) & 0x3f
    }

    pub fn volume_min_falloff_curve(&self) -> u32 {
        (self.flags[1] >> 14) & 0x3f
    }

    pub fn volume(&self) -> (f32, f32) {
        (
            f32::from(self.vol_min) / 65535.0,
            f32::from(self.vol_max) / 65535.0,
        )
    }

    pub fn pitch(&self) -> (f32, f32) {
        (
            f32::from(self.pitch_lower_bound) / 32767.0,
            f32::from(self.pitch_upper_bound) / 32767.0,
        )
    }
}

/// One `SndAliasList`: an alias name and its variants.
#[derive(Clone, Debug, Default)]
pub struct SndAliasListRef {
    pub name: String,
    pub id: u32,
    pub seq_no: i32,
    pub entries: Vec<SndAliasRef>,
}

/// One `SndBank`.
#[derive(Clone, Debug, Default)]
pub struct SndBankRef {
    pub name: String,
    pub aliases: Vec<SndAliasListRef>,
    /// bo2mp: the ducks this bank's aliases name by id (`SndAlias::duck`).
    pub ducks: Vec<SndDuckRef>,
    /// The bank's room echoes (`SndRadverb`), by name.
    pub radverbs: Vec<SndRadverbRef>,
}

/// bo2mp: one `SndDuck`: while a sound that names it plays, every duck group is turned
/// down to its `attenuation` (and low-passed to its `filter`), faded in and out.
#[derive(Clone, Debug, Default)]
pub struct SndDuckRef {
    pub name: String,
    pub id: u32,
    /// Seconds.
    pub ease_on: f32,
    pub ease_off: f32,
    pub start_delay: f32,
    /// Inches; above zero the duck weakens with the listener's distance from the sound.
    pub distance: f32,
    /// Seconds; zero = as long as the sound plays.
    pub length: f32,
    pub update_while_paused: bool,
    /// Per duck group (`SndAlias::duckGroup`): the volume left at full duck (1 = untouched).
    pub attenuation: Vec<f32>,
    /// Per duck group: the low-pass left at full duck (1 = untouched).
    pub filter: Vec<f32>,
}

/// `SndRadverb`: one room echo preset, as the bank stores it.
#[derive(Clone, Debug, Default)]
pub struct SndRadverbRef {
    pub name: String,
    /// smoothing, earlyTime, lateTime, earlyGain, lateGain, returnGain,
    /// earlyLpf, lateLpf, inputLpf, dampLpf, wallReflect, dryGain,
    /// earlySize, lateSize, diffusion, returnHighpass (the struct's order).
    pub values: [f32; 16],
}

/// `SndCurve`: a falloff curve, 8 points.
#[derive(Clone, Debug, Default)]
pub struct SndCurveRef {
    pub name: String,
    pub id: u32,
    pub points: [[f32; 2]; 8],
}

/// `SndDriverGlobals` (the parts the engine uses: curves, volume groups).
#[derive(Clone, Debug, Default)]
pub struct SndDriverGlobalsRef {
    pub name: String,
    pub curves: Vec<SndCurveRef>,
    /// (name, parent name, attenuation sp, attenuation mp).
    pub groups: Vec<(String, String, u16, u16)>,
    /// bo2mp: each volume group as Black Ops II mixes it, in the same order as `groups`.
    pub group_mix: Vec<SndGroupMixRef>,
    /// bo2mp: the speaker pans an alias names by index (`SndAlias::pan`).
    pub pans: Vec<SndPanRef>,
    /// bo2mp: the duck groups (`SndDuckGroup` name, id); an alias's `duck_group` indexes them.
    pub duck_groups: Vec<(String, u32)>,
    /// bo2mp: the masters (`SndMaster`); the menu's audio preset picks one by name.
    pub masters: Vec<SndMasterRef>,
}

/// bo2mp: one EQ band of a `SndMaster` (`lowE`/`lowG`/`lowF`/`lowQ`, ...).
#[derive(Clone, Copy, Debug, Default)]
pub struct SndEqBandRef {
    pub enabled: bool,
    pub gain: f32,
    pub freq: f32,
    pub q: f32,
}

/// bo2mp: a `SndMaster` compressor or limiter (`compE`/`compPG`/`compMG`/`compT`/`compR`/`compTA`/`compTR`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SndDynamicsRef {
    pub turned_on: bool,
    pub pre_gain: f32,
    pub makeup_gain: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack: f32,
    pub release: f32,
}

/// bo2mp: one `SndMaster`: the master EQ, compressor, limiter and bus gains. Black Ops II's
/// audio preset (`snd_menu_presets`) picks the master named `treyarch_mix`, `bass_boost`,
/// `high_boost`, `super_crunch` or `headphones` (`SND_GetMasterByConfig`).
#[derive(Clone, Debug, Default)]
pub struct SndMasterRef {
    pub name: String,
    pub id: u32,
    pub low: SndEqBandRef,
    pub peak1: SndEqBandRef,
    pub peak2: SndEqBandRef,
    pub hi: SndEqBandRef,
    pub eq_gain: f32,
    pub comp: SndDynamicsRef,
    pub limit: SndDynamicsRef,
    /// Bus gains: reverb, fx, voice, pfutz, hdrfx, ui, music, movie, vcs.
    pub bus_gain: [f32; 9],
    /// Bus enables: reverb, fx, voice, pfutz, hdrfx, ui, music, movie.
    pub bus_enabled: [bool; 8],
    pub hdrfx_comp: bool,
    pub voice_eq: bool,
    pub voice_comp: bool,
}

/// bo2mp: one `SndPan`: how loud a sound that is not placed in the world is in each speaker.
#[derive(Clone, Debug, Default)]
pub struct SndPanRef {
    pub name: String,
    pub front: f32,
    pub back: f32,
    pub center: f32,
    pub lfe: f32,
    pub left: f32,
    pub right: f32,
}

/// bo2mp: one `SndVolumeGroup` as Black Ops II mixes a sound with it.
#[derive(Clone, Copy, Debug, Default)]
pub struct SndGroupMixRef {
    /// The parent group's index; `u32::MAX` at the top of the chain.
    pub parent: u32,
    /// `SndMenuCategory`: 0 effects, 1 music, 2 voice, 3 interface, 4 cinematic.
    pub category: u32,
    pub attenuation_mp: u16,
}

fn fixed_string(s: &ZoneByteReader<'_>, p: RawRef, off: usize, len: usize) -> Result<String> {
    let raw = s.byte_window_from(p, off, len)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
}

/// Reads `count` records of `stride` bytes laid end to end from `first`.
fn array_of<T>(
    first: RawRef,
    count: usize,
    stride: usize,
    mut read: impl FnMut(RawRef) -> Result<T>,
) -> Result<Vec<T>> {
    (0..count).map(|n| read(first.shifted_by(n * stride))).collect()
}

impl ZoneCapture {
    /// The script strings a 32-slot tag-id table names, skipping empty slots.
    fn concealed_markers(&self, s: &ZoneByteReader<'_>, table: RawRef) -> Result<Vec<String>> {
        let mut names = Vec::<String>::new();
        for id in array_of(table, 32, 2, |slot| s.ushort_from_bytes(slot, 0))? {
            if id > 0 {
                names.push(String::from(self.script_string(id)));
            }
        }
        Ok(names)
    }

    /// A script string by its zone index.
    pub fn script_string(&self, id: u16) -> &str {
        match self.strings.get(usize::from(id)) {
            Some(text) => text.as_str(),
            None => "",
        }
    }

    /// The name of the asset an asset pointer slot names: a kept asset's
    /// name, else the name at the body's start (assets whose first member
    /// is their name).
    pub(crate) fn asset_name_at(
        &mut self,
        s: &ZoneByteReader<'_>,
        slot: RawRef,
        asset_kind: XAssetKind,
    ) -> Result<String> {
        let _ = asset_kind;
        // A name with a leading comma is a reference to an asset another
        // zone defines; the name after it is the asset's own.
        let raw = if s.uint_from_bytes(slot, 0)? == 0 {
            String::default()
        } else if let Some(key) = self.asset_at(s, slot)? {
            self.kept_name(key)
        } else {
            // Not kept: read the name the body starts with, if still there.
            match deref(s, slot, 0)? {
                Some(body) => string_field(s, body, 0_usize).unwrap_or_default(),
                None => String::default(),
            }
        };
        Ok(raw.trim_start_matches(',').to_owned())
    }

    pub(crate) fn kept_name(&self, key: AssetIdentity) -> String {
        match key.ty {
            XAssetKind::Material => self.materials.get(key.index).map(|m| m.name.clone()),
            XAssetKind::XModel => self.xmodels.get(key.index).map(|m| m.name.clone()),
            XAssetKind::Image => self.images.get(key.index).map(|m| m.name.clone()),
            XAssetKind::TechniqueSet => self.technique_sets.get(key.index).map(|m| m.name.clone()),
            XAssetKind::Fx => self.fx.get(key.index).map(|m| m.name.clone()),
            XAssetKind::Tracer => self.tracers.get(key.index).map(|m| m.name.clone()),
            XAssetKind::WeaponCamo => self.weapon_camos.get(key.index).map(|m| m.name.clone()),
            _ => None,
        }
        .unwrap_or_default()
    }

    pub(crate) fn read_font(&mut self, s: &ZoneByteReader<'_>, f: RawRef) -> Result<FontRef> {
        let mut out = FontRef {
            name: string_field(s, f, Font_s::fontName)?,
            raster_rows: s.uint_from_bytes(f, Font_s::pixelHeight)? as i32,
            material: self.asset_name_at(s, f.shifted_by(Font_s::material), XAssetKind::Material)?,
            ..Default::default()
        };
        let glyph_n = s.uint_from_bytes(f, Font_s::glyphCount)? as usize;
        if let Some(glyphs) = deref(s, f, Font_s::glyphs)? {
            for i in 0..glyph_n.min(4096) {
                let g = glyphs.shifted_by(i * Glyph::SIZE);
                out.glyphs.push(GlyphRef {
                    letter: s.ushort_from_bytes(g, Glyph::letter)?,
                    x0: s.u8_at(g, Glyph::x0)? as i8,
                    y0: s.u8_at(g, Glyph::y0)? as i8,
                    step_along_x: s.u8_at(g, Glyph::step_along_x)?,
                    texel_span: s.u8_at(g, Glyph::pixelWidth)?,
                    raster_rows: s.u8_at(g, Glyph::pixelHeight)?,
                    st: [
                        s.float_from_bytes(g, Glyph::s0)?,
                        s.float_from_bytes(g, Glyph::t0)?,
                        s.float_from_bytes(g, Glyph::s1)?,
                        s.float_from_bytes(g, Glyph::t1)?,
                    ],
                });
            }
        }
        let kern_n = s.uint_from_bytes(f, Font_s::kerningPairsCount)? as usize;
        if let Some(pairs) = deref(s, f, Font_s::kerningPairs)? {
            for i in 0..kern_n.min(8192) {
                let k = pairs.shifted_by(i * KerningPairs::SIZE);
                out.kerning.push((
                    s.ushort_from_bytes(k, KerningPairs::wFirst)?,
                    s.ushort_from_bytes(k, KerningPairs::wSecond)?,
                    s.uint_from_bytes(k, KerningPairs::iKernAmount)? as i32,
                ));
            }
        }
        Ok(out)
    }

    pub(crate) fn read_weapon_camo(&mut self, s: &ZoneByteReader<'_>, c: RawRef) -> Result<WeaponCamoRef> {
        let mut out = WeaponCamoRef {
            name: string_field(s, c, WeaponCamo::name)?,
            solid_base_image: self.asset_name_at(s, c.shifted_by(WeaponCamo::solidBaseImage), XAssetKind::Image)?,
            pattern_base_image: self.asset_name_at(s, c.shifted_by(WeaponCamo::patternBaseImage), XAssetKind::Image)?,
            ..Default::default()
        };
        let set_n = s.uint_from_bytes(c, WeaponCamo::numCamoSets)? as usize;
        if let Some(sets) = deref(s, c, WeaponCamo::camoSets)? {
            for i in 0..set_n.min(256) {
                let e = sets.shifted_by(i * WeaponCamoSet::SIZE);
                out.sets.push(WeaponCamoSetRef {
                    solid: self.asset_name_at(s, e.shifted_by(WeaponCamoSet::solidCamoImage), XAssetKind::Image)?,
                    pattern: self.asset_name_at(s, e.shifted_by(WeaponCamoSet::patternCamoImage), XAssetKind::Image)?,
                    offset: [
                        s.float_from_bytes(e, WeaponCamoSet::patternOffset)?,
                        s.float_from_bytes(e, WeaponCamoSet::patternOffset + 4)?,
                    ],
                    scale: s.float_from_bytes(e, WeaponCamoSet::patternScale)?,
                });
            }
        }
        let mat_n = s.uint_from_bytes(c, WeaponCamo::numCamoMaterials)? as usize;
        if let Some(sets) = deref(s, c, WeaponCamo::camoMaterials)? {
            for i in 0..mat_n.min(64) {
                let set = sets.shifted_by(i * WeaponCamoMaterialSet::SIZE);
                let n = s.uint_from_bytes(set, WeaponCamoMaterialSet::numMaterials)? as usize;
                let mut list = Vec::new();
                if let Some(mats) = deref(s, set, WeaponCamoMaterialSet::materials)? {
                    for j in 0..n.min(256) {
                        let m = mats.shifted_by(j * WeaponCamoMaterial::SIZE);
                        let base_n = usize::from(s.ushort_from_bytes(m, WeaponCamoMaterial::numBaseMaterials)?);
                        let mut base = Vec::new();
                        let mut camo = Vec::new();
                        if let Some(b) = deref(s, m, WeaponCamoMaterial::baseMaterials)? {
                            for k in 0..base_n.min(64) {
                                base.push(self.asset_name_at(s, b.shifted_by(k * 4), XAssetKind::Material)?);
                            }
                        }
                        if let Some(b) = deref(s, m, WeaponCamoMaterial::camoMaterials)? {
                            for k in 0..base_n.min(64) {
                                camo.push(self.asset_name_at(s, b.shifted_by(k * 4), XAssetKind::Material)?);
                            }
                        }
                        let mut consts = [0.0f32; 8];
                        for (k, c) in consts.iter_mut().enumerate() {
                            *c = s.float_from_bytes(m, WeaponCamoMaterial::shaderConsts + k * 4)?;
                        }
                        list.push(WeaponCamoMaterialRef {
                            replace_flags: s.ushort_from_bytes(m, WeaponCamoMaterial::replaceFlags)?,
                            base,
                            camo,
                            consts,
                        });
                    }
                }
                out.material_sets.push(list);
            }
        }
        Ok(out)
    }

    pub(crate) fn read_attachment(&mut self, s: &ZoneByteReader<'_>, a: RawRef) -> Result<AttachmentRef> {
        Ok(AttachmentRef {
            name: string_field(s, a, WeaponAttachment::szInternalName)?,
            bytes: s.byte_window_from(a, 0, WeaponAttachment::SIZE)?.to_vec(),
        })
    }

    pub(crate) fn read_attachment_unique(
        &mut self,
        s: &ZoneByteReader<'_>,
        a: RawRef,
    ) -> Result<AttachmentUniqueRef> {
        let model = |this: &mut Self, off: usize| this.asset_name_at(s, a.shifted_by(off), XAssetKind::XModel);
        let mut out = AttachmentUniqueRef {
            name: string_field(s, a, WeaponAttachmentUnique::szInternalName)?,
            bytes: s.byte_window_from(a, 0, WeaponAttachmentUnique::SIZE)?.to_vec(),
            view_model: model(self, WeaponAttachmentUnique::viewModel)?,
            view_model_additional: model(self, WeaponAttachmentUnique::viewModelAdditional)?,
            view_model_ads: model(self, WeaponAttachmentUnique::viewModelADS)?,
            alt_weapon_name: string_field(s, a, WeaponAttachmentUnique::szAltWeaponName)?,
            third_person_mesh: model(self, WeaponAttachmentUnique::worldModel)?,
            view_model_tag: string_field(s, a, WeaponAttachmentUnique::viewModelTag)?,
            world_model_tag: string_field(s, a, WeaponAttachmentUnique::worldModelTag)?,
            hud_layer_surface: self.asset_name_at(s, a.shifted_by(WeaponAttachmentUnique::overlayMaterial), XAssetKind::Material)?,
            overlay_material_low: self.asset_name_at(s, a.shifted_by(WeaponAttachmentUnique::overlayMaterialLowRes), XAssetKind::Material)?,
            ..Default::default()
        };
        for (slot, off) in [
            WeaponAttachmentUnique::fireSound,
            WeaponAttachmentUnique::fireSoundPlayer,
            WeaponAttachmentUnique::fireLoopSound,
            WeaponAttachmentUnique::fireLoopSoundPlayer,
            WeaponAttachmentUnique::fireStopSound,
            WeaponAttachmentUnique::fireStopSoundPlayer,
            WeaponAttachmentUnique::fireLastSound,
            WeaponAttachmentUnique::fireLastSoundPlayer,
        ]
        .into_iter()
        .enumerate()
        {
            // (A bad read leaves the gun's own sound, never the unique unread.)
            out.fire_sounds[slot] = string_field(s, a, off).unwrap_or_default();
        }
        out.dual_wield_weapon_name = string_field(s, a, WeaponAttachmentUnique::szDualWieldWeaponName).unwrap_or_default();
        out.view_flash = self.asset_name_at(s, a.shifted_by(WeaponAttachmentUnique::viewFlashEffect), XAssetKind::Fx).unwrap_or_default();
        out.world_flash = self.asset_name_at(s, a.shifted_by(WeaponAttachmentUnique::worldFlashEffect), XAssetKind::Fx).unwrap_or_default();
        if let Some(arr) = deref(s, a, WeaponAttachmentUnique::szXAnims)? {
            for i in 0..GUN_MOTION_TOTAL {
                out.animation_refs.push(string_field(s, arr, i * 4)?);
            }
        }
        if let Some(arr) = deref(s, a, WeaponAttachmentUnique::hideTags)? {
            out.concealed_bones = self.concealed_markers(s, arr)?;
        }
        if let Some(arr) = deref(s, a, WeaponAttachmentUnique::locationDamageMultipliers)? {
            out.location_damage = Some(
                (0..21)
                    .map(|i| s.float_from_bytes(arr, i * 4))
                    .collect::<Result<_>>()?,
            );
        }
        Ok(out)
    }

    /// A variant's 8 attach slots that hold a model: (slot, model, tag,
    /// offset, rotation), from its model, tag, offset and rotation arrays.
    fn read_weapon_attach(
        &mut self,
        s: &ZoneByteReader<'_>,
        v: RawRef,
        [models_at, tags_at, offsets_at, rotations_at]: [usize; 4],
    ) -> Result<Vec<(usize, String, String, [f32; 3], [f32; 3])>> {
        let mut out = Vec::new();
        let Some(models) = deref(s, v, models_at)? else {
            return Ok(Vec::new());
        };
        let tags = deref(s, v, tags_at)?;
        for slot in 0_usize..8 {
            let model = self.asset_name_at(s, models.shifted_by(slot * 4), XAssetKind::XModel)?;
            if model.is_empty() {
                continue;
            }
            let tag = match tags {
                Some(names) => string_field(s, names, slot * 4)?,
                None => String::new(),
            };
            let f = |base: usize| -> Result<[f32; 3]> {
                Ok([
                    s.float_from_bytes(v, base + slot * 12)?,
                    s.float_from_bytes(v, base + slot * 12 + 4)?,
                    s.float_from_bytes(v, base + slot * 12 + 8)?,
                ])
            };
            out.push((slot, model, tag, f(offsets_at)?, f(rotations_at)?));
        }
        Ok(out)
    }

    pub(crate) fn read_weapon(&mut self, s: &ZoneByteReader<'_>, v: RawRef) -> Result<WeaponRef> {
        let variant = s.byte_window_from(v, 0, WeaponVariantDef::SIZE)?.to_vec();
        let mut w = WeaponRef {
            name: string_field(s, v, WeaponVariantDef::szInternalName)?,
            variant,
            shown_label: string_field(s, v, WeaponVariantDef::szDisplayName)?,
            alt_weapon_name: string_field(s, v, WeaponVariantDef::szAltWeaponName)?,
            ammo_name: string_field(s, v, WeaponVariantDef::szAmmoName)?,
            collision_label: string_field(s, v, WeaponVariantDef::szClipName)?,
            ..Default::default()
        };
        if let Some(arr) = deref(s, v, WeaponVariantDef::szXAnims)? {
            for i in 0..GUN_MOTION_TOTAL {
                w.animation_refs.push(string_field(s, arr, i * 4)?);
            }
        }
        w.attach_view_models = self.read_weapon_attach(
            s,
            v,
            [
                WeaponVariantDef::attachViewModel,
                WeaponVariantDef::attachViewModelTag,
                WeaponVariantDef::attachViewModelOffsets,
                WeaponVariantDef::attachViewModelRotations,
            ],
        )?;
        w.attach_world_models = self.read_weapon_attach(
            s,
            v,
            [
                WeaponVariantDef::attachWorldModel,
                WeaponVariantDef::attachWorldModelTag,
                WeaponVariantDef::attachWorldModelOffsets,
                WeaponVariantDef::attachWorldModelRotations,
            ],
        )?;
        if let Some(arr) = deref(s, v, WeaponVariantDef::hideTags)? {
            w.concealed_bones = self.concealed_markers(s, arr)?;
        }
        let variant_material = |this: &mut Self, field: usize| this.asset_name_at(s, v.shifted_by(field), XAssetKind::Material);
        w.hud_layer_surface = variant_material(self, WeaponVariantDef::overlayMaterial)?;
        w.overlay_material_low = variant_material(self, WeaponVariantDef::overlayMaterialLowRes)?;
        w.dpad_icon = variant_material(self, WeaponVariantDef::dpadIcon)?;
        let Some(d) = deref(s, v, WeaponVariantDef::weapDef)? else {
            return Ok(w);
        };
        w.def = s.byte_window_from(d, 0, WeaponDef::SIZE)?.to_vec();
        w.camo = self.asset_name_at(s, d.shifted_by(WeaponDef::weaponCamo), XAssetKind::WeaponCamo)?;
        for off in WEAPON_DEF_STRINGS {
            match string_field(s, d, off)? {
                value if value.is_empty() => {}
                value => w.strings.push((off, value)),
            }
        }
        for off in WEAPON_DEF_FX {
            let name = self.asset_name_at(s, d.shifted_by(off), XAssetKind::Fx)?;
            if !name.is_empty() {
                w.fx.push((off, name));
            }
        }
        for off in WEAPON_DEF_MATERIALS {
            let name = self.asset_name_at(s, d.shifted_by(off), XAssetKind::Material)?;
            if !name.is_empty() {
                w.materials.push((off, name));
            }
        }
        for off in WEAPON_DEF_XMODELS {
            let name = self.asset_name_at(s, d.shifted_by(off), XAssetKind::XModel)?;
            if !name.is_empty() {
                w.models.push((off, name));
            }
        }
        for off in WEAPON_DEF_TRACERS {
            let name = self.asset_name_at(s, d.shifted_by(off), XAssetKind::Tracer)?;
            if !name.is_empty() {
                w.tracers.push((off, name));
            }
        }
        for (field, out) in [
            (WeaponDef::gunXModel, &mut w.gun_models),
            (WeaponDef::worldModel, &mut w.world_models),
        ] {
            if let Some(arr) = deref(s, d, field)? {
                *out = array_of(arr, 16, 4, |slot| self.asset_name_at(s, slot, XAssetKind::XModel))?;
            }
        }
        if let (Some(keys), Some(values)) = (
            deref(s, d, WeaponDef::notetrackSoundMapKeys)?,
            deref(s, d, WeaponDef::notetrackSoundMapValues)?,
        ) {
            for i in 0..20 {
                let k = s.ushort_from_bytes(keys, i * 2)?;
                let val = s.ushort_from_bytes(values, i * 2)?;
                if k != 0 {
                    w.notetrack_sounds.push((
                        self.script_string(k).to_owned(),
                        self.script_string(val).to_owned(),
                    ));
                }
            }
        }
        if let Some(arr) = deref(s, d, WeaponDef::bounceSound)? {
            for i in 0..SURF_TYPE_COUNT {
                w.bounce_sounds.push(string_field(s, arr, i * 4)?);
            }
        }
        let floats = |base: Option<RawRef>, len: usize| -> Result<Option<Vec<f32>>> {
            base.map(|at| array_of(at, len, 4, |lane| s.float_from_bytes(lane, 0))).transpose()
        };
        w.along_rebound = floats(deref(s, d, WeaponDef::parallelBounce)?, 32)?;
        w.across_rebound = floats(deref(s, d, WeaponDef::perpendicularBounce)?, 32)?;
        w.location_damage = floats(deref(s, d, WeaponDef::locationDamageMultipliers)?, 21)?;
        Ok(w)
    }

    pub(crate) fn read_fx(&mut self, s: &ZoneByteReader<'_>, e: RawRef) -> Result<FxEffectRef> {
        let looping = s.short_from_bytes(e, FxEffectDef::elemDefCountLooping)?;
        let one_shot = s.short_from_bytes(e, FxEffectDef::elemDefCountOneShot)?;
        let emission = s.short_from_bytes(e, FxEffectDef::elemDefCountEmission)?;
        let count = (i32::from(looping) + i32::from(one_shot) + i32::from(emission)).max(0) as usize;
        let mut elems = Vec::with_capacity(count);
        if let Some(arr) = deref(s, e, FxEffectDef::elemDefs)? {
            for i in 0..count {
                elems.push(self.read_fx_elem(s, arr.shifted_by(i * FxElemDef::SIZE))?);
            }
        }
        let v3 = |off: usize| -> Result<[f32; 3]> {
            Ok([s.float_from_bytes(e, off)?, s.float_from_bytes(e, off + 4)?, s.float_from_bytes(e, off + 8)?])
        };
        Ok(FxEffectRef {
            name: string_field(s, e, FxEffectDef::name)?,
            flags: s.ushort_from_bytes(e, FxEffectDef::flags)?,
            priority: s.u8_at(e, FxEffectDef::efPriority)?,
            repeat_total: looping,
            single_play_total: one_shot,
            spawn_total: emission,
            total_size: s.int_from_bytes(e, FxEffectDef::totalSize)?,
            loop_lifetime_millis: s.int_from_bytes(e, FxEffectDef::msecLoopingLife)?,
            msec_non_looping_life: s.int_from_bytes(e, FxEffectDef::msecNonLoopingLife)?,
            elems,
            bounds_dim: v3(FxEffectDef::boundingBoxDim)?,
            bounds_centre: v3(FxEffectDef::boundingBoxCentre)?,
        })
    }

    fn read_fx_elem(&mut self, s: &ZoneByteReader<'_>, p: RawRef) -> Result<FxElemRef> {
        let raw = s.byte_window_from(p, 0, FxElemDef::SIZE)?.to_vec();
        let elem_type = raw[FxElemDef::elemType];
        let look_variant_total = raw[FxElemDef::visualCount] as i8;
        let vel_count = (raw[FxElemDef::velIntervalCount] as i8 as i32 + 1).max(0) as usize;
        let vis_count = (raw[FxElemDef::visStateIntervalCount] as i8 as i32 + 1).max(0) as usize;
        let speed_readings = match deref(s, p, FxElemDef::velSamples)? {
            None => Vec::default(),
            Some(arr) => s
                .byte_window_from(arr, 0, vel_count * FxElemVelStateSample::SIZE)?
                .to_vec(),
        };
        let visibility_readings = match deref(s, p, FxElemDef::visSamples)? {
            None => Vec::default(),
            Some(arr) => s
                .byte_window_from(arr, 0, vis_count * FxElemVisStateSample::SIZE)?
                .to_vec(),
        };
        // Visuals: a decal's mark pairs, an array when more than one, else
        // the single visual stored in place.
        let mut visuals = Vec::new();
        let vslot = p.shifted_by(FxElemDef::visuals);
        if elem_type == 11 {
            if let Some(arr) = deref(s, vslot, 0)? {
                for i in 0..look_variant_total.max(0) as usize {
                    let pair = arr.shifted_by(i * FxElemMarkVisuals::SIZE);
                    visuals.push(FxVisualRef::Mark([
                        self.asset_name_at(s, pair, XAssetKind::Material)?,
                        self.asset_name_at(s, pair.shifted_by(4), XAssetKind::Material)?,
                    ]));
                }
            }
        } else if look_variant_total >= 2 {
            if let Some(arr) = deref(s, vslot, 0)? {
                visuals = array_of(arr, look_variant_total as usize, 4, |slot| {
                    self.read_fx_visual(s, slot, elem_type)
                })?;
            }
        } else if look_variant_total == 1 {
            visuals.push(self.read_fx_visual(s, vslot, elem_type)?);
        }
        let mut trail = None;
        let mut spot = None;
        if let Some(ext) = deref(s, p, FxElemDef::extended)? {
            if elem_type == 5 {
                let vertex_tally = s.int_from_bytes(ext, FxStreakDef::vertCount)?.max(0) as usize;
                let triangle_slot_total = s.int_from_bytes(ext, FxStreakDef::indCount)?.max(0) as usize;
                let vertex_points = deref(s, ext, FxStreakDef::vertex_points)?
                    .map_or(Ok(vec![]), |vp| {
                        array_of(vp, vertex_tally, 20, |q| {
                            let mut v = [0.0f32; 5];
                            for (lane, slot) in v.iter_mut().enumerate() {
                                *slot = s.float_from_bytes(q, lane * 4)?;
                            }
                            Ok(v)
                        })
                    })?;
                let index_run_values = deref(s, ext, FxStreakDef::index_run_values)?
                    .map_or(Ok(vec![]), |ip| array_of(ip, triangle_slot_total, 2, |at| s.ushort_from_bytes(at, 0)))?;
                trail = Some(FxTrailRef {
                    scroll_period_millis: s.int_from_bytes(ext, FxStreakDef::scrollTimeMsec)?,
                    recurrence_distance: s.int_from_bytes(ext, FxStreakDef::repeatDist)?,
                    split_dist: s.float_from_bytes(ext, FxStreakDef::splitDist)?,
                    vertex_points,
                    index_run_values,
                });
            } else if elem_type == 9 {
                spot = Some([
                    s.float_from_bytes(ext, FxSpotLightDef::fovInnerFraction)?,
                    s.float_from_bytes(ext, FxSpotLightDef::startRadius)?,
                    s.float_from_bytes(ext, FxSpotLightDef::endRadius)?,
                ]);
            }
        }
        Ok(FxElemRef {
            speed_readings: speed_readings,
            visibility_readings: visibility_readings,
            visuals,
            effect_upon_strike: string_field(s, p, FxElemDef::effectOnImpact)?,
            effect_upon_demise: string_field(s, p, FxElemDef::effectOnDeath)?,
            effect_spawned: string_field(s, p, FxElemDef::effectEmitted)?,
            effect_attached: string_field(s, p, FxElemDef::effectAttached)?,
            trail,
            spot,
            spawn_sound: string_field(s, p, FxElemDef::spawnSound)?,
            raw,
        })
    }

    fn read_fx_visual(&mut self, s: &ZoneByteReader<'_>, slot: RawRef, elem_type: u8) -> Result<FxVisualRef> {
        Ok(match elem_type {
            0..=6 => {
                let name = self.asset_name_at(s, slot, XAssetKind::Material)?;
                if name.is_empty() {
                    FxVisualRef::None
                } else {
                    FxVisualRef::Material(name)
                }
            }
            7 => {
                let name = self.asset_name_at(s, slot, XAssetKind::XModel)?;
                if name.is_empty() {
                    FxVisualRef::None
                } else {
                    FxVisualRef::Model(name)
                }
            }
            12 => {
                let name = string_field(s, slot, 0)?;
                if name.is_empty() {
                    FxVisualRef::None
                } else {
                    FxVisualRef::Effect(name)
                }
            }
            10 => {
                let name = string_field(s, slot, 0)?;
                if name.is_empty() {
                    FxVisualRef::None
                } else {
                    FxVisualRef::Sound(name)
                }
            }
            9 => FxVisualRef::Light(self.asset_name_at(s, slot, XAssetKind::LightDef)?),
            _ => FxVisualRef::None,
        })
    }

    pub(crate) fn read_tracer(&mut self, s: &ZoneByteReader<'_>, t: RawRef) -> Result<TracerRef> {
        let mut colors = [[0.0f32; 4]; 5];
        for (i, c) in colors.iter_mut().enumerate() {
            for (j, v) in c.iter_mut().enumerate() {
                *v = s.float_from_bytes(t, TracerDef::colors + (i * 4 + j) * 4)?;
            }
        }
        Ok(TracerRef {
            name: string_field(s, t, TracerDef::name)?,
            ty: s.int_from_bytes(t, TracerDef::r#type)?,
            material: self.asset_name_at(s, t.shifted_by(TracerDef::material), XAssetKind::Material)?,
            render_spacing: s.int_from_bytes(t, TracerDef::drawInterval)?,
            speed: s.float_from_bytes(t, TracerDef::speed)?,
            ray_extent: s.float_from_bytes(t, TracerDef::beamLength)?,
            ray_thickness: s.float_from_bytes(t, TracerDef::beamWidth)?,
            spiral_radius: s.float_from_bytes(t, TracerDef::screwRadius)?,
            spiral_pitch_distance: s.float_from_bytes(t, TracerDef::screwDist)?,
            fade_secs: s.float_from_bytes(t, TracerDef::fadeTime)?,
            fade_scale: s.float_from_bytes(t, TracerDef::fadeScale)?,
            tex_repeat_rate: s.float_from_bytes(t, TracerDef::texRepeatRate)?,
            colors,
        })
    }

    pub(crate) fn read_impact_table(&mut self, s: &ZoneByteReader<'_>, t: RawRef) -> Result<ImpactTableRef> {
        let mut impact_rows = Vec::new();
        if let Some(arr) = deref(s, t, FxHitTable::table)? {
            for i in 0..21 {
                let e = arr.shifted_by(i * FxHitRow::SIZE);
                let mut nonflesh: [String; 32] = Default::default();
                for (j, n) in nonflesh.iter_mut().enumerate() {
                    *n = self.asset_name_at(s, e.shifted_by(FxHitRow::nonflesh + j * 4), XAssetKind::Fx)?;
                }
                let mut flesh = <[String; 4]>::default();
                for (j, n) in flesh.iter_mut().enumerate() {
                    *n = self.asset_name_at(s, e.shifted_by(FxHitRow::flesh + j * 4), XAssetKind::Fx)?;
                }
                impact_rows.push((nonflesh, flesh));
            }
        }
        Ok(ImpactTableRef {
            name: string_field(s, t, FxHitTable::name)?,
            rows: impact_rows,
        })
    }

    pub(crate) fn read_footstep_table(&mut self, s: &ZoneByteReader<'_>, t: RawRef) -> Result<FootstepTableRef> {
        let mut aliases = Vec::with_capacity(32);
        for surf in 0..32 {
            let mut row = [0u32; 7];
            for (k, a) in row.iter_mut().enumerate() {
                *a = s.uint_from_bytes(t, FootstepTableDef::sndAliasTable + (surf * 7 + k) * 4)?;
            }
            aliases.push(row);
        }
        Ok(FootstepTableRef {
            name: string_field(s, t, FootstepTableDef::name)?,
            aliases,
        })
    }

    pub(crate) fn read_footstep_fx_table(&mut self, s: &ZoneByteReader<'_>, t: RawRef) -> Result<FootstepFxTableRef> {
        let mut fx = Vec::with_capacity(32);
        for surf in 0..32 {
            fx.push(self.asset_name_at(s, t.shifted_by(FootstepFXTableDef::footstepFX + surf * 4), XAssetKind::Fx)?);
        }
        Ok(FootstepFxTableRef {
            name: string_field(s, t, FootstepFXTableDef::name)?,
            fx,
        })
    }

    pub(crate) fn read_sound_bank(&mut self, s: &ZoneByteReader<'_>, b: RawRef) -> Result<SndBankRef> {
        let count = s.uint_from_bytes(b, SndBank::aliasCount)? as usize;
        let mut aliases = Vec::new();
        if let Some(arr) = deref(s, b, SndBank::alias)? {
            aliases = array_of(arr, count, SndAliasList::SIZE, |list| {
                let n = s.int_from_bytes(list, SndAliasList::count)?.max(0) as usize;
                let head = deref(s, list, SndAliasList::head)?;
                let entries = match head {
                    None => vec![],
                    Some(head) => array_of(head, n, SndAlias::SIZE, |a| {
                        let u16a = |off: usize| s.ushort_from_bytes(a, off);
                        let u8a = |off: usize| s.u8_at(a, off);
                        Ok(SndAliasRef {
                            name: string_field(s, a, SndAlias::name)?,
                            id: s.uint_from_bytes(a, SndAlias::id)?,
                            subtitle: string_field(s, a, SndAlias::subtitle)?,
                            secondary: string_field(s, a, SndAlias::secondaryName)?,
                            asset_id: s.uint_from_bytes(a, SndAlias::assetId)?,
                            asset_file: string_field(s, a, SndAlias::assetFileName)?,
                            flags: [
                                s.uint_from_bytes(a, SndAlias::flags)?,
                                s.uint_from_bytes(a, SndAlias::flags + 4)?,
                            ],
                            duck: s.uint_from_bytes(a, SndAlias::duck)?,
                            context_type: s.uint_from_bytes(a, SndAlias::contextType)?,
                            context_value: s.uint_from_bytes(a, SndAlias::contextValue)?,
                            stop_on_play: s.uint_from_bytes(a, SndAlias::stopOnPlay)?,
                            futz_patch: s.uint_from_bytes(a, SndAlias::futzPatch)?,
                            flux_time: u16a(SndAlias::fluxTime)?,
                            start_delay: u16a(SndAlias::startDelay)?,
                            reverb_send: u16a(SndAlias::reverbSend)?,
                            center_send: u16a(SndAlias::centerSend)?,
                            vol_min: u16a(SndAlias::volMin)?,
                            vol_max: u16a(SndAlias::volMax)?,
                            pitch_lower_bound: u16a(SndAlias::pitchMin)?,
                            pitch_upper_bound: u16a(SndAlias::pitchMax)?,
                            reach_lower: u16a(SndAlias::distMin)?,
                            reach_upper: u16a(SndAlias::distMax)?,
                            dist_reverb_max: u16a(SndAlias::distReverbMax)?,
                            envelop_min: u16a(SndAlias::envelopMin)?,
                            envelop_max: u16a(SndAlias::envelopMax)?,
                            envelop_percentage: u16a(SndAlias::envelopPercentage)?,
                            ease_on: u16a(SndAlias::fadeIn)?,
                            ease_off: u16a(SndAlias::fadeOut)?,
                            doppler_scale: s.short_from_bytes(a, SndAlias::dopplerScale)?,
                            min_priority_threshold: u8a(SndAlias::minPriorityThreshold)?,
                            max_priority_threshold: u8a(SndAlias::maxPriorityThreshold)?,
                            chance_weight: u8a(SndAlias::chance_weight)?,
                            occlusion_level: u8a(SndAlias::occlusionLevel)?,
                            min_priority: u8a(SndAlias::minPriority)?,
                            max_priority: u8a(SndAlias::maxPriority)?,
                            pan: u8a(SndAlias::pan)?,
                            cap_total: u8a(SndAlias::limitCount)?,
                            actor_cap_total: u8a(SndAlias::entityLimitCount)?,
                            duck_group: u8a(SndAlias::duckGroup)?,
                        })
                    })?,
                };
                Ok(SndAliasListRef {
                    name: string_field(s, list, SndAliasList::name)?,
                    id: s.uint_from_bytes(list, SndAliasList::id)?,
                    seq_no: s.int_from_bytes(list, SndAliasList::sequence)?,
                    entries,
                })
            })?;
        }
        let duck_count = s.uint_from_bytes(b, SndBank::duckCount)? as usize;
        let mut ducks = Vec::with_capacity(duck_count);
        if let Some(arr) = deref(s, b, SndBank::ducks)? {
            for i in 0..duck_count {
                let d = arr.shifted_by(i * SndDuck::SIZE);
                let per_group = |off: usize| -> Result<Vec<f32>> {
                    let Some(p) = deref(s, d, off)? else {
                        return Ok(Vec::new());
                    };
                    (0..32).map(|k| s.float_from_bytes(p, k * 4)).collect()
                };
                ducks.push(SndDuckRef {
                    name: fixed_string(s, d, SndDuck::name, 32)?,
                    id: s.uint_from_bytes(d, SndDuck::id)?,
                    ease_on: s.float_from_bytes(d, SndDuck::fadeIn)?,
                    ease_off: s.float_from_bytes(d, SndDuck::fadeOut)?,
                    start_delay: s.float_from_bytes(d, SndDuck::startDelay)?,
                    distance: s.float_from_bytes(d, SndDuck::distance)?,
                    length: s.float_from_bytes(d, SndDuck::length)?,
                    update_while_paused: s.u8_at(d, SndDuck::updateWhilePaused)? != 0,
                    attenuation: per_group(SndDuck::attenuation)?,
                    filter: per_group(SndDuck::filter)?,
                });
            }
        }
        let mut radverbs = Vec::new();
        let rcount = s.uint_from_bytes(b, SndBank::radverbCount)? as usize;
        if let Some(arr) = deref(s, b, SndBank::radverbs)? {
            for i in 0..rcount {
                let r = arr.shifted_by(i * SndRadverb::SIZE);
                let mut values = [0.0f32; 16];
                for (k, v) in values.iter_mut().enumerate() {
                    *v = s.float_from_bytes(r, SndRadverb::smoothing + k * 4)?;
                }
                radverbs.push(SndRadverbRef {
                    name: fixed_string(s, r, SndRadverb::name, 32)?,
                    values,
                });
            }
        }
        Ok(SndBankRef {
            name: string_field(s, b, SndBank::name)?,
            aliases,
            ducks,
            radverbs,
        })
    }

    pub(crate) fn read_snd_driver_globals(&mut self, s: &ZoneByteReader<'_>, g: RawRef) -> Result<SndDriverGlobalsRef> {
        let mut out = SndDriverGlobalsRef {
            name: string_field(s, g, SndDriverGlobals::name)?,
            ..Default::default()
        };
        let curve_count = s.uint_from_bytes(g, SndDriverGlobals::curveCount)? as usize;
        if let Some(arr) = deref(s, g, SndDriverGlobals::curves)? {
            for i in 0..curve_count {
                let c = arr.shifted_by(i * SndCurve::SIZE);
                let mut points = [[0.0f32; 2]; 8];
                for (k, pt) in points.iter_mut().enumerate() {
                    pt[0] = s.float_from_bytes(c, SndCurve::points + k * 8)?;
                    pt[1] = s.float_from_bytes(c, SndCurve::points + k * 8 + 4)?;
                }
                out.curves.push(SndCurveRef {
                    name: fixed_string(s, c, SndCurve::name, 32)?,
                    id: s.uint_from_bytes(c, SndCurve::id)?,
                    points,
                });
            }
        }
        let group_count = s.uint_from_bytes(g, SndDriverGlobals::groupCount)? as usize;
        if let Some(arr) = deref(s, g, SndDriverGlobals::groups)? {
            for i in 0..group_count {
                let c = arr.shifted_by(i * SndVolumeGroup::SIZE);
                out.groups.push((
                    fixed_string(s, c, SndVolumeGroup::name, 32)?,
                    fixed_string(s, c, SndVolumeGroup::parentName, 32)?,
                    s.ushort_from_bytes(c, SndVolumeGroup::attenuationSp)?,
                    s.ushort_from_bytes(c, SndVolumeGroup::attenuationMp)?,
                ));
                out.group_mix.push(SndGroupMixRef {
                    parent: s.uint_from_bytes(c, SndVolumeGroup::parentIndex)?,
                    category: s.uint_from_bytes(c, SndVolumeGroup::category)?,
                    attenuation_mp: s.ushort_from_bytes(c, SndVolumeGroup::attenuationMp)?,
                });
            }
        }
        let pan_count = s.uint_from_bytes(g, SndDriverGlobals::panCount)? as usize;
        if let Some(arr) = deref(s, g, SndDriverGlobals::pans)? {
            for i in 0..pan_count {
                let c = arr.shifted_by(i * SndPan::SIZE);
                out.pans.push(SndPanRef {
                    name: fixed_string(s, c, SndPan::name, 32)?,
                    front: s.float_from_bytes(c, SndPan::front)?,
                    back: s.float_from_bytes(c, SndPan::back)?,
                    center: s.float_from_bytes(c, SndPan::center)?,
                    lfe: s.float_from_bytes(c, SndPan::lfe)?,
                    left: s.float_from_bytes(c, SndPan::left)?,
                    right: s.float_from_bytes(c, SndPan::right)?,
                });
            }
        }
        let duck_group_count = s.uint_from_bytes(g, SndDriverGlobals::duckGroupCount)? as usize;
        if let Some(arr) = deref(s, g, SndDriverGlobals::duckGroups)? {
            for i in 0..duck_group_count {
                let c = arr.shifted_by(i * SndDuckGroup::SIZE);
                out.duck_groups.push((
                    fixed_string(s, c, SndDuckGroup::name, 32)?,
                    s.uint_from_bytes(c, SndDuckGroup::id)?,
                ));
            }
        }
        let master_count = s.uint_from_bytes(g, SndDriverGlobals::masterCount)? as usize;
        if let Some(arr) = deref(s, g, SndDriverGlobals::masters)? {
            for i in 0..master_count {
                let c = arr.shifted_by(i * SndMaster::SIZE);
                let band = |e: usize, g: usize, f: usize, q: usize| -> Result<SndEqBandRef> {
                    Ok(SndEqBandRef { enabled: s.int_from_bytes(c, e)? != 0, gain: s.float_from_bytes(c, g)?, freq: s.float_from_bytes(c, f)?, q: s.float_from_bytes(c, q)? })
                };
                let dynamics = |o: [usize; 7]| -> Result<SndDynamicsRef> {
                    Ok(SndDynamicsRef {
                        turned_on: s.int_from_bytes(c, o[0])? != 0,
                        pre_gain: s.float_from_bytes(c, o[1])?,
                        makeup_gain: s.float_from_bytes(c, o[2])?,
                        threshold_db: s.float_from_bytes(c, o[3])?,
                        ratio: s.float_from_bytes(c, o[4])?,
                        attack: s.float_from_bytes(c, o[5])?,
                        release: s.float_from_bytes(c, o[6])?,
                    })
                };
                let mut bus_gain = [0.0f32; 9];
                for (k, o) in [SndMaster::busReverbG, SndMaster::busFxG, SndMaster::busVoiceG, SndMaster::busPfutzG, SndMaster::busHdrfxG, SndMaster::busUiG, SndMaster::busMusicG, SndMaster::busMovieG, SndMaster::busVcsG]
                    .into_iter()
                    .enumerate()
                {
                    bus_gain[k] = s.float_from_bytes(c, o)?;
                }
                let mut bus_enabled = [false; 8];
                for (k, o) in [SndMaster::busReverbE, SndMaster::busFxE, SndMaster::busVoiceE, SndMaster::busPfutzE, SndMaster::busHdrfxE, SndMaster::busUiE, SndMaster::busMusicE, SndMaster::busMovieE]
                    .into_iter()
                    .enumerate()
                {
                    bus_enabled[k] = s.int_from_bytes(c, o)? != 0;
                }
                out.masters.push(SndMasterRef {
                    name: fixed_string(s, c, SndMaster::name, 32)?,
                    id: s.uint_from_bytes(c, SndMaster::id)?,
                    low: band(SndMaster::lowE, SndMaster::lowG, SndMaster::lowF, SndMaster::lowQ)?,
                    peak1: band(SndMaster::peak1E, SndMaster::peak1G, SndMaster::peak1F, SndMaster::peak1Q)?,
                    peak2: band(SndMaster::peak2E, SndMaster::peak2G, SndMaster::peak2F, SndMaster::peak2Q)?,
                    hi: band(SndMaster::hiE, SndMaster::hiG, SndMaster::hiF, SndMaster::hiQ)?,
                    eq_gain: s.float_from_bytes(c, SndMaster::eqG)?,
                    comp: dynamics([SndMaster::compE, SndMaster::compPG, SndMaster::compMG, SndMaster::compT, SndMaster::compR, SndMaster::compTA, SndMaster::compTR])?,
                    limit: dynamics([SndMaster::limitE, SndMaster::limitPG, SndMaster::limitMG, SndMaster::limitT, SndMaster::limitR, SndMaster::limitTA, SndMaster::limitTR])?,
                    bus_gain,
                    bus_enabled,
                    hdrfx_comp: s.int_from_bytes(c, SndMaster::hdrfxCompE)? != 0,
                    voice_eq: s.int_from_bytes(c, SndMaster::voiceEqE)? != 0,
                    voice_comp: s.int_from_bytes(c, SndMaster::voiceCompE)? != 0,
                });
            }
        }
        Ok(out)
    }
}
