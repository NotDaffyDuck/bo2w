//! Spot shadow atlas layout and the caster bookkeeping around it.
//!
//! BO2 draws every spot shadow into one depth target, a 2 x 2 grid of square
//! tiles, one tile per shadowed light. The rest of this module decides which
//! entities and static models a light can see (a per-light visibility bit
//! per entity and per dynamic brush), and shapes the lists the shadow passes
//! draw from.

use core::ops::Range;

use crate::light_reach::{LampReach, cone_reach_box, cone_reach_point};
use crate::stage_light::is_cone_lamp;
use crate::prop_light::PROP_FLAG_NO_CAST_SHADOW;
use crate::cone_pick::{CONE_TILE_COUNT, ConeCasterOffer};

/// Render target slot of the spot shadow depth atlas.
pub const GFX_SPOT_SHADOW_RT: u8 = 0x10;

/// sm_spotQuality when the dvar is not set: the smallest tile.
pub const SM_SPOT_QUALITY_DEFAULT: u32 = 0;

/// What a tile covers of the atlas along each axis; also the scale that puts
/// the light's clip space into its tile.
pub(crate) const SPOT_SHADOW_TILE_SCALE: f32 = 0.5;

/// Entity slots per view in the visibility bit table.
pub const SPOT_SHADOW_CONFIG_ENTITY_LIMIT: u32 = 0x800;

/// Entities whose last-seen origin is tracked per local client.
pub const SPOT_SHADOW_ORIGIN_TRACKED_LIMIT: u32 = 0x800;

/// An entity must move more than 16 units (squared: 256) before its light
/// links are rebuilt.
pub const SPOT_SHADOW_RELINK_MOVE_DIST_SQUARED: f32 = 256.0;

/// Entries in the per-light "this entity casts" mark row.
pub const SPOT_SHADOW_ENTITY_MARK_CAPACITY: usize = 0x1000;

/// Scene model-object entries each shadow slot can hold.
pub const SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN: usize = 0x200;

/// Scene model entries each shadow slot can hold.
pub const SPOT_SHADOW_STATIC_VISIBILITY_SPAN: usize = 0x400;

const ATLAS_TILES_PER_SIDE: u32 = 2;
const TILE_BASE: u32 = 512;

/// Entities are linked to a light a little early so a shadow does not pop in
/// at the edge of the light's reach.
const LINK_ENTITY_PAD: f32 = 16.0;

/// Entity info bits that keep an entity out of every shadow.
const ENT_SKIP_FLAGS: u32 = 0x0208_0000;
const ENTNUM_SHIFT: u32 = 7;
const ENTNUM_MASK: u32 = 0xfff;

/// The cull gate value of a scene model object whose pose has settled.
const POSE_SETTLED_GATE: u32 = 2;

/// Tile edge in pixels: each sm_spotQuality step doubles it.
pub const fn spot_shadow_tile(spot_quality: u32) -> u32 {
    TILE_BASE << spot_quality
}

/// Where one shadow's tile sits in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConeSlotPlan {
    pub target_surface_code: u8,
    pub tile_x: u32,
    pub tile_y: u32,
    pub tile: u32,
    /// Only the first tile of a frame clears the atlas.
    pub clears_atlas: bool,
}

impl ConeSlotPlan {
    /// One texel in tile space, as the lookup shader wants it.
    pub fn texel_offset_correction(self) -> [f32; 4] {
        let texel = 1.0 / self.tile as f32;
        [texel, texel, 0.0, 0.0]
    }

    /// The tile's corner in atlas units: 0 or 0.5 along each axis.
    pub fn tile_offset(self) -> [f32; 2] {
        let tile = self.tile as f32;
        [
            SPOT_SHADOW_TILE_SCALE * (self.tile_x as f32 / tile),
            SPOT_SHADOW_TILE_SCALE * (self.tile_y as f32 / tile),
        ]
    }
}

/// Tiles fill the atlas left to right, then top to bottom.
pub fn plan_cone_slots(slot_index: u32, tile: u32) -> ConeSlotPlan {
    let column = slot_index % ATLAS_TILES_PER_SIDE;
    let row = slot_index / ATLAS_TILES_PER_SIDE;
    ConeSlotPlan {
        target_surface_code: GFX_SPOT_SHADOW_RT,
        tile_x: column * tile,
        tile_y: row * tile,
        tile,
        clears_atlas: (row, column) == (0, 0),
    }
}

/// Hands out the next free tile, or nothing once the atlas is full.
pub fn claim_cone_slot(shadow_candidate_total: &mut u32) -> Option<u32> {
    let next = *shadow_candidate_total;
    let room = usize::try_from(next).is_ok_and(|taken| taken < CONE_TILE_COUNT);
    if room {
        *shadow_candidate_total += 1;
        Some(next)
    } else {
        None
    }
}

/// Whether the packed draw lists have any work entries to draw from.
pub fn cone_lists_ready(work_entry_tally: u32) -> bool {
    work_entry_tally > 0
}

// ---- Light visibility bits ----

/// Bit for `row` and `shadow_light_slot` in a table that holds one bit per
/// non-sun primary light for every row. The sums wrap at 32 bits and are read
/// as signed: a result below zero has no bit.
fn light_vis_bit(row: u32, shadow_light_slot: u32, sunlight_primary_flag_index: u32, main_light_total: u32) -> Option<u32> {
    let first_lit = sunlight_primary_flag_index.wrapping_add(1);
    let per_row = main_light_total.wrapping_sub(first_lit);
    let bit = row
        .wrapping_mul(per_row)
        .wrapping_add(shadow_light_slot)
        .wrapping_sub(first_lit);
    (bit as i32 >= 0).then_some(bit)
}

/// Bit for an entity: each view owns `SPOT_SHADOW_CONFIG_ENTITY_LIMIT` rows.
fn entity_vis_bit(
    view: u32,
    ent_no: u32,
    shadow_light_slot: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
) -> Option<u32> {
    let row = view
        .wrapping_mul(SPOT_SHADOW_CONFIG_ENTITY_LIMIT)
        .wrapping_add(ent_no);
    light_vis_bit(row, shadow_light_slot, sunlight_primary_flag_index, main_light_total)
}

/// Bit for a dynamic brush: brushes are rows of their own, indexed by id.
pub fn debris_lit_bit(
    brush: u32,
    shadow_light_slot: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
) -> Option<u32> {
    light_vis_bit(brush, shadow_light_slot, sunlight_primary_flag_index, main_light_total)
}

/// Words the entity visibility table needs to reach the last bit of the last
/// entity of the last view; zero when any count is zero.
pub fn spot_shadow_main_light_visibility_words(
    views: u32,
    ents: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
) -> usize {
    let last = (
        views.checked_sub(1),
        ents.checked_sub(1),
        main_light_total.checked_sub(1),
    );
    let (Some(last_view), Some(last_ent), Some(last_light)) = last else {
        return 0;
    };
    match entity_vis_bit(last_view, last_ent, last_light, sunlight_primary_flag_index, main_light_total) {
        Some(bit) => (bit >> 5) as usize + 1,
        None => 0,
    }
}

/// Whether a bit is set; bits past the table read as clear.
pub fn lamp_lit_bit(lit: &[u32], bit: u32) -> bool {
    lit
        .get((bit >> 5) as usize)
        .is_some_and(|word| word & (1 << (bit & 31)) != 0)
}

/// Stores `set` in a bit; a bit past the table is dropped.
fn write_vis_bit(lit: &mut [u32], bit: u32, set: bool) {
    if let Some(word) = lit.get_mut((bit >> 5) as usize) {
        let shift = bit & 31;
        *word = (*word & !(1u32 << shift)) | (u32::from(set) << shift);
    }
}

/// Squared distance between two points.
fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(&p, q)| (p - q) * (p - q))
        .fold(0.0, |total, part| total + part)
}

/// A spot light's widened cone cosine, when it has a usable cone.
fn usable_cone(light: &LampReach) -> Option<f32> {
    let cos = light.widened_half_view_cosine;
    (is_cone_lamp(light.kind) && cos > 0.0).then_some(cos)
}

/// Whether a point, grown by `slack`, is touched by the light: inside its
/// sphere and, for a spot, inside its widened cone.
fn light_touches(lamp: &LampReach, at: [f32; 3], slack: f32) -> bool {
    let reach = lamp.radius + slack;
    if distance_squared(at, lamp.origin) < reach * reach {
        usable_cone(lamp).is_none_or(|cos| {
            !cone_reach_point(lamp.origin, lamp.direction, cos, at, slack)
        })
    } else {
        false
    }
}

/// Sets or clears the bit of every non-sun primary light for one row.
/// `bit_for` gets the light's index and the number of lights.
fn link_row(
    lit: &mut [u32],
    lights: &[LampReach],
    sunlight_primary_flag_index: u32,
    at: [f32; 3],
    slack: f32,
    bit_for: impl Fn(u32, u32) -> Option<u32>,
) {
    let light_total = lights.len() as u32;
    let first = sunlight_primary_flag_index.saturating_add(1) as usize;
    let Some(non_sun) = lights.get(first..) else {
        return;
    };
    for (step, lamp) in non_sun.iter().enumerate() {
        let shadow_light_slot = (first + step) as u32;
        if let Some(bit) = bit_for(shadow_light_slot, light_total) {
            write_vis_bit(lit, bit, light_touches(lamp, at, slack));
        }
    }
}

/// Records which primary lights reach an entity at `origin`.
pub fn cone_shadow_link_entity(
    lit: &mut [u32],
    view: u32,
    ent_no: u32,
    at: [f32; 3],
    bound_radius: f32,
    lights: &[LampReach],
    sunlight_primary_flag_index: u32,
) {
    let slack = bound_radius + LINK_ENTITY_PAD;
    link_row(lit, lights, sunlight_primary_flag_index, at, slack, |light, total| {
        entity_vis_bit(view, ent_no, light, sunlight_primary_flag_index, total)
    });
}

/// Records which primary lights reach a dynamic brush at `origin`.
pub fn cone_shadow_link_debris_solid_vis(
    lit: &mut [u32],
    brush: u32,
    at: [f32; 3],
    extra: f32,
    lights: &[LampReach],
    sunlight_primary_flag_index: u32,
) {
    link_row(lit, lights, sunlight_primary_flag_index, at, extra, |light, total| {
        light_vis_bit(brush, light, sunlight_primary_flag_index, total)
    });
}

/// A dynamic brush is drawn into a shadow when the light walks the primary
/// lists and the brush's bit for it is set.
pub fn mover_brush_admitted(walks_main: bool, bit_set: bool) -> bool {
    matches!((walks_main, bit_set), (true, true))
}

// ---- Entity origin tracking ----

/// Slot of an entity in the per-client origin tracker.
pub fn cone_origin_slot(client: u32, ent_no: u32) -> Option<usize> {
    let per_client = u64::from(SPOT_SHADOW_ORIGIN_TRACKED_LIMIT);
    let ent_no = Some(u64::from(ent_no)).filter(|&ent| ent < per_client)?;
    let index = u64::from(client) * per_client + ent_no;
    u32::try_from(index).ok().map(|index| index as usize)
}

/// Whether an entity moved far enough since its last link to need a new one.
pub fn cone_relink_due(
    last: [f32; 3],
    now: [f32; 3],
    threshold_squared: f32,
) -> bool {
    distance_squared(now, last) > threshold_squared
}

// ---- Casters ----

fn entnum_of(info: u32) -> u32 {
    (info >> ENTNUM_SHIFT) & ENTNUM_MASK
}

/// A scene entry as the occupancy fill sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeViewSlot {
    pub ent_bits: u32,
    pub mid_point: Option<[f32; 3]>,
    pub half_ext: Option<[f32; 3]>,
}

/// One candidate caster for a light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeLightOccluderFeed {
    pub ent_bits: u32,
    /// The entity's bit for this light is set.
    pub sight_lit: bool,
    pub box_valid: bool,
    pub mid_point: [f32; 3],
    pub half_ext: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConeShadowFillFault {
    /// The visibility table has not been built this frame.
    VisUnread,
}

/// A caster candidate from a scene entry. Its bounds count only when both
/// halves are known; missing halves read as zero.
fn caster_from_slot(slot: &ConeViewSlot, sight_lit: bool) -> ConeLightOccluderFeed {
    let bits = slot.ent_bits;
    let (centre, extent, box_valid) = match (slot.mid_point, slot.half_ext) {
        (Some(known_mid), Some(known_half)) => (known_mid, known_half, true),
        (got_mid, got_half) => (got_mid.unwrap_or_default(), got_half.unwrap_or_default(), false),
    };
    ConeLightOccluderFeed {
        ent_bits: bits,
        sight_lit,
        box_valid,
        mid_point: centre,
        half_ext: extent,
    }
}

/// Turns scene entries into caster candidates for one light, reading each
/// entry's visibility bit. Returns how many entries were written to `out`.
pub fn populate_spot_shadow_occupancy(
    slots: &[ConeViewSlot],
    vis_words: Option<&[u32]>,
    view: u32,
    shadow_light_slot: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
    out: &mut [ConeLightOccluderFeed],
) -> Result<usize, ConeShadowFillFault> {
    let Some(words) = vis_words else {
        return Err(ConeShadowFillFault::VisUnread);
    };
    let bit_of = |ent| entity_vis_bit(view, ent, shadow_light_slot, sunlight_primary_flag_index, main_light_total);
    let lit = |info: u32| {
        bit_of(entnum_of(info)).is_some_and(|bit| lamp_lit_bit(words, bit))
    };
    let filled = slots.len().min(out.len());
    for (caster, slot) in out[..filled].iter_mut().zip(&slots[..filled]) {
        *caster = caster_from_slot(slot, lit(slot.ent_bits));
    }
    Ok(filled)
}

/// Whether a candidate is worth looking at: not flagged out, not the local
/// client's own entity, and reached by the light.
fn caster_wanted(caster: &ConeLightOccluderFeed, viewing_player_entity: u32) -> bool {
    caster.ent_bits & ENT_SKIP_FLAGS == 0
        && entnum_of(caster.ent_bits) != viewing_player_entity
        && caster.sight_lit
}

/// Whether a scene entity's bounds are known and inside the light's cone.
fn inside_cone(lamp: &ConeCasterOffer, ent: &ConeLightOccluderFeed) -> bool {
    ent.box_valid
        && !cone_reach_box(
            lamp.origin,
            lamp.aim,
            lamp.widened_half_view_cosine,
            ent.mid_point,
            ent.half_ext,
        )
}

/// Marks every entity that casts into `light`'s shadow. A light that does not
/// walk casters marks nothing. Scene entities also need their bounds inside
/// the light's cone; static models are marked on visibility alone.
pub fn cone_shadow_walk_casters(
    scans_casters: bool,
    light: &ConeCasterOffer,
    ents: &[ConeLightOccluderFeed],
    fixed_props: &[ConeLightOccluderFeed],
    viewing_player_entity: u32,
    marks: &mut [u8],
) {
    if scans_casters {
        let wanted = |caster: &&ConeLightOccluderFeed| caster_wanted(caster, viewing_player_entity);
        let scene = ents
            .iter()
            .filter(wanted)
            .filter(|ent| inside_cone(light, ent));
        for caster in scene.chain(fixed_props.iter().filter(wanted)) {
            if let Some(mark) = marks.get_mut(entnum_of(caster.ent_bits) as usize) {
                *mark = 1;
            }
        }
    }
}

/// Whether the entity behind a scene model was marked as a caster.
pub fn model_casts_cone(marks: &[u8], actor: Option<u32>) -> bool {
    actor
        .and_then(|ent_no| marks.get(ent_no as usize))
        .is_some_and(|&mark| mark != 0)
}

// ---- Per-slot draw lists ----

/// Copies the world surfaces a light draws, if it walks the primary lists.
pub fn cone_world_facets(walks_main: bool, world_surfs: &[u16], dest: &mut [u16]) -> usize {
    let copied = if walks_main {
        world_surfs.len().min(dest.len())
    } else {
        0
    };
    dest[..copied].copy_from_slice(&world_surfs[..copied]);
    copied
}

/// A static model casts unless its no-cast-shadow flag is set.
pub fn spot_shadow_static_model_casts(flags: u8) -> bool {
    let no_cast = flags & PROP_FLAG_NO_CAST_SHADOW;
    no_cast == 0
}

/// Keeps the static models that cast; `flags_of` gives a model's flags, or
/// nothing for a model that is not drawn at all.
pub fn cone_shadow_prune_static_ids(
    traverses_main_set: bool,
    model_ids: &[u16],
    mut flags_of: impl FnMut(u16) -> Option<u8>,
    out: &mut [u16],
) -> usize {
    let mut kept = 0;
    if traverses_main_set {
        let casting = model_ids
            .iter()
            .copied()
            .filter(|&id| flags_of(id).is_some_and(spot_shadow_static_model_casts));
        // The output is pulled first, so no more flags are read once it is full.
        for (dst, id) in out.iter_mut().zip(casting) {
            *dst = id;
            kept += 1;
        }
    }
    kept
}

/// Whether a scene model object has a pose good enough to draw: its cull
/// gate must have settled on 2.
pub fn cone_pose_ready(visibility_gate_bits: u32) -> bool {
    visibility_gate_bits == POSE_SETTLED_GATE
}

/// The part of a visibility table that belongs to one slot, cut short where
/// the table ends.
fn slot_span(table_len: usize, slot: u32, stride: usize) -> Range<usize> {
    let start = (slot as usize).saturating_mul(stride).min(table_len);
    start..start.saturating_add(stride).min(table_len)
}

/// Copies the camera's visibility byte of each entry into a slot's row. An
/// entry whose `ready` flag is false loses its visible bit; entries past the
/// row are dropped.
fn copy_camera_vis(row: &mut [u8], entnums: &[u32], camera: &[u8], ready: Option<&[bool]>) {
    for (entry, (dst, &entnum)) in row.iter_mut().zip(entnums).enumerate() {
        let seen = camera.get(entnum as usize).copied().unwrap_or(0);
        let unready = ready.is_some_and(|flags| flags.get(entry) == Some(&false));
        *dst = if unready && seen & 1 == 1 { 0 } else { seen };
    }
}

/// Visibility bytes for the scene model objects of one shadow slot.
pub fn populate_spot_animated_visibility(
    bytes: &mut [u8],
    slot: u32,
    entnums: &[u32],
    camera_bytes: Option<&[u8]>,
    pose_accepted_cases: Option<&[bool]>,
) {
    if let Some(camera) = camera_bytes {
        let span = slot_span(bytes.len(), slot, SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN);
        copy_camera_vis(&mut bytes[span], entnums, camera, pose_accepted_cases);
    }
}

/// Visibility bytes for the scene models of one shadow slot.
pub fn populate_spot_static_visibility(
    bytes: &mut [u8],
    slot: u32,
    entnums: &[u32],
    camera_bytes: Option<&[u8]>,
    lod_passed: Option<&[bool]>,
) {
    if let Some(camera) = camera_bytes {
        let span = slot_span(bytes.len(), slot, SPOT_SHADOW_STATIC_VISIBILITY_SPAN);
        copy_camera_vis(&mut bytes[span], entnums, camera, lod_passed);
    }
}

/// Positions in a slot's row of the first `count` entries marked visible (a
/// byte of exactly 1), as many as `out` holds.
fn list_visible(row: &[u8], count: u32, out: &mut [u32]) -> usize {
    let scanned = row.len().min(count as usize);
    let visible = (0u32..)
        .zip(&row[..scanned])
        .filter_map(|(entry, &byte)| (byte == 1).then_some(entry));
    let mut listed = 0;
    for (dst, entry) in out.iter_mut().zip(visible) {
        *dst = entry;
        listed += 1;
    }
    listed
}

/// Scene model objects visible in a shadow slot.
pub fn split_moving_ids(
    vis: &[u8],
    slot: u32,
    dobj_total: u32,
    found: &mut [u32],
) -> usize {
    let span = slot_span(vis.len(), slot, SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN);
    list_visible(&vis[span], dobj_total, found)
}

/// Scene models visible in a shadow slot.
pub fn cone_shadow_stitch_stage_model_indices(
    vis: &[u8],
    slot: u32,
    model_total: u32,
    found: &mut [u32],
) -> usize {
    let span = slot_span(vis.len(), slot, SPOT_SHADOW_STATIC_VISIBILITY_SPAN);
    list_visible(&vis[span], model_total, found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_light::{POINT_LAMP, CONE_LAMP};

    const UP: [f32; 3] = [0.0, 0.0, 1.0];

    fn cull_light(kind: u8, at: [f32; 3], reach: f32, cone: f32) -> LampReach {
        LampReach {
            kind,
            origin: at,
            direction: UP,
            radius: reach,
            widened_half_view_cosine: cone,
        }
    }

    fn omni(origin: [f32; 3], radius: f32) -> LampReach {
        cull_light(POINT_LAMP, origin, radius, 0.0)
    }

    fn spot(origin: [f32; 3], radius: f32) -> LampReach {
        cull_light(CONE_LAMP, origin, radius, 0.7071)
    }

    /// The spot above, as the shadow chooser hands it over.
    fn shadowable() -> ConeCasterOffer {
        let cone = spot([0.0; 3], 100.0);
        ConeCasterOffer {
            supports_depth_shade: true,
            color: [1.0; 3],
            aim: cone.direction,
            origin: cone.origin,
            radius: cone.radius,
            kind: cone.kind,
            widened_half_view_cosine: cone.widened_half_view_cosine,
            hide_dist: i16::MAX,
            view_outside_shadowmap_volume: false,
        }
    }

    fn scene_slot(info: u32, mid: Option<[f32; 3]>, half: Option<[f32; 3]>) -> ConeViewSlot {
        ConeViewSlot {
            ent_bits: info,
            mid_point: mid,
            half_ext: half,
        }
    }

    fn caster(entnum: u32, sight_lit: bool, mid: [f32; 3]) -> ConeLightOccluderFeed {
        ConeLightOccluderFeed {
            ent_bits: entnum << 7,
            sight_lit,
            box_valid: true,
            mid_point: mid,
            half_ext: [1.0; 3],
        }
    }

    #[test]
    fn a_tile_is_512_pixels_and_doubles_with_quality() {
        assert_eq!(spot_shadow_tile(0), 512);
        assert_eq!(spot_shadow_tile(1), 1024);
        assert_eq!(spot_shadow_tile(2), 2048);
    }

    #[test]
    fn slots_fill_the_atlas_row_by_row() {
        for slot in 0..4 {
            let plan = plan_cone_slots(slot, 512);
            assert_eq!(plan.target_surface_code, GFX_SPOT_SHADOW_RT);
            assert_eq!(plan.tile_x, (slot % 2) * 512);
            assert_eq!(plan.tile_y, (slot / 2) * 512);
            assert_eq!(plan.tile, 512);
        }
    }

    #[test]
    fn only_the_first_slot_clears_the_atlas() {
        assert!(plan_cone_slots(0, 512).clears_atlas);
        assert!(!plan_cone_slots(1, 512).clears_atlas);
        assert!(!plan_cone_slots(3, 512).clears_atlas);
    }

    #[test]
    fn the_pixel_adjust_is_one_over_the_tile() {
        let plan = plan_cone_slots(2, 1024);
        assert_eq!(plan.texel_offset_correction(), [1.0 / 1024.0, 1.0 / 1024.0, 0.0, 0.0]);
    }

    #[test]
    fn the_tile_offset_is_zero_or_half_per_axis() {
        assert_eq!(plan_cone_slots(0, 512).tile_offset(), [0.0, 0.0]);
        assert_eq!(plan_cone_slots(1, 512).tile_offset(), [0.5, 0.0]);
        assert_eq!(plan_cone_slots(2, 512).tile_offset(), [0.0, 0.5]);
        assert_eq!(plan_cone_slots(3, 512).tile_offset(), [0.5, 0.5]);
    }

    #[test]
    fn the_atlas_holds_four_slots() {
        let mut taken = 0;
        let got: Vec<_> = (0..6).map(|_| claim_cone_slot(&mut taken)).collect();
        assert_eq!(got, [Some(0), Some(1), Some(2), Some(3), None, None]);
        assert_eq!(taken, 4);
    }

    #[test]
    fn packed_lists_need_at_least_one_work_entry() {
        assert!(!cone_lists_ready(0));
        assert!(cone_lists_ready(1));
    }

    #[test]
    fn dynamic_brush_bits_skip_the_sun_and_pack_per_brush() {
        // Sun is light 0, five lights in all: four bits per brush.
        assert_eq!(debris_lit_bit(0, 1, 0, 5), Some(0));
        assert_eq!(debris_lit_bit(0, 4, 0, 5), Some(3));
        assert_eq!(debris_lit_bit(1, 1, 0, 5), Some(4));
        assert_eq!(debris_lit_bit(2, 3, 0, 5), Some(10));
    }

    #[test]
    fn a_light_before_the_first_row_has_no_bit() {
        // Light 1 on brush 0 with the sun at light 1: 0*span - 2 + 1 < 0.
        assert_eq!(debris_lit_bit(0, 1, 1, 5), None);
        assert_eq!(debris_lit_bit(0, 2, 1, 5), Some(0));
    }

    #[test]
    fn entity_rows_are_0x800_apart_per_view() {
        // No sun, three lights: lights_per_row is 2.
        let in_view0 = entity_vis_bit(0, 5, 1, 0, 3).unwrap();
        let in_view1 = entity_vis_bit(1, 5, 1, 0, 3).unwrap();
        assert_eq!(in_view1 - in_view0, 0x800 * 2);
    }

    #[test]
    fn the_word_count_reaches_the_last_bit() {
        // One view, 0x800 entities, no sun, three lights: the last bit is
        // 0x7ff * 2 - 1 + 2 = 0xfff, which is in word 127.
        assert_eq!(spot_shadow_main_light_visibility_words(1, 0x800, 0, 3), 128);
    }

    #[test]
    fn the_word_count_is_zero_when_a_count_is_zero() {
        assert_eq!(spot_shadow_main_light_visibility_words(0, 0x800, 0, 3), 0);
        assert_eq!(spot_shadow_main_light_visibility_words(1, 0, 0, 3), 0);
        assert_eq!(spot_shadow_main_light_visibility_words(1, 0x800, 0, 0), 0);
    }

    #[test]
    fn vis_bits_read_clear_past_the_table() {
        let lit = [0b101u32, 1 << 31];
        assert!(lamp_lit_bit(&lit, 0));
        assert!(!lamp_lit_bit(&lit, 1));
        assert!(lamp_lit_bit(&lit, 2));
        assert!(lamp_lit_bit(&lit, 63));
        assert!(!lamp_lit_bit(&lit, 64));
    }

    #[test]
    fn linking_an_entity_sets_the_bits_of_the_lights_that_reach_it() {
        // Sun at 0, then two omnis: one covers the origin, one is far away.
        let lights = [
            omni([0.0; 3], 1.0),
            omni([10.0, 0.0, 0.0], 50.0),
            omni([1000.0, 0.0, 0.0], 50.0),
        ];
        let mut lit = vec![0u32; 4];
        cone_shadow_link_entity(&mut lit, 0, 3, [0.0; 3], 0.0, &lights, 0);
        let near = entity_vis_bit(0, 3, 1, 0, 3).unwrap();
        let far = entity_vis_bit(0, 3, 2, 0, 3).unwrap();
        assert!(lamp_lit_bit(&lit, near));
        assert!(!lamp_lit_bit(&lit, far));
    }

    #[test]
    fn linking_clears_a_bit_the_light_no_longer_reaches() {
        let lights = [omni([0.0; 3], 1.0), omni([10.0, 0.0, 0.0], 50.0)];
        let mut lit = vec![0u32; 2];
        cone_shadow_link_entity(&mut lit, 0, 0, [0.0; 3], 0.0, &lights, 0);
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        assert!(lamp_lit_bit(&lit, bit));
        cone_shadow_link_entity(&mut lit, 0, 0, [500.0, 0.0, 0.0], 0.0, &lights, 0);
        assert!(!lamp_lit_bit(&lit, bit));
    }

    #[test]
    fn an_entity_is_linked_a_little_beyond_the_light_radius() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        let mut lit = vec![0u32; 2];
        // 110 away: outside radius 100, inside radius + pad (116).
        cone_shadow_link_entity(&mut lit, 0, 0, [110.0, 0.0, 0.0], 0.0, &lights, 0);
        assert!(lamp_lit_bit(&lit, bit));
        cone_shadow_link_entity(&mut lit, 0, 0, [120.0, 0.0, 0.0], 0.0, &lights, 0);
        assert!(!lamp_lit_bit(&lit, bit));
    }

    #[test]
    fn a_larger_entity_radius_extends_the_link() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        let mut lit = vec![0u32; 2];
        cone_shadow_link_entity(&mut lit, 0, 0, [130.0, 0.0, 0.0], 20.0, &lights, 0);
        assert!(lamp_lit_bit(&lit, bit));
    }

    #[test]
    fn a_spot_does_not_link_what_is_behind_it() {
        let lights = [omni([0.0; 3], 1.0), spot([0.0; 3], 100.0)];
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        let mut lit = vec![0u32; 2];
        // The cone opens against its direction: z < 0 is in front of it.
        cone_shadow_link_entity(&mut lit, 0, 0, [0.0, 0.0, -50.0], 0.0, &lights, 0);
        assert!(lamp_lit_bit(&lit, bit));
        cone_shadow_link_entity(&mut lit, 0, 0, [0.0, 0.0, 50.0], 0.0, &lights, 0);
        assert!(!lamp_lit_bit(&lit, bit));
    }

    #[test]
    fn linking_a_dynamic_brush_uses_its_own_row() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let mut lit = vec![0u32; 2];
        cone_shadow_link_debris_solid_vis(&mut lit, 2, [10.0, 0.0, 0.0], 0.0, &lights, 0);
        let bit = debris_lit_bit(2, 1, 0, 2).unwrap();
        assert!(lamp_lit_bit(&lit, bit));
        assert_eq!(lit[0].count_ones() + lit[1].count_ones(), 1);
    }

    #[test]
    fn linking_with_a_short_table_drops_the_bits_that_do_not_fit() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let mut lit: Vec<u32> = Vec::new();
        cone_shadow_link_entity(&mut lit, 0, 0, [0.0; 3], 0.0, &lights, 0);
        assert!(lit.is_empty());
    }

    #[test]
    fn a_dynamic_brush_is_drawn_only_when_both_conditions_hold() {
        assert!(mover_brush_admitted(true, true));
        assert!(!mover_brush_admitted(true, false));
        assert!(!mover_brush_admitted(false, true));
    }

    #[test]
    fn origin_tracking_has_a_row_of_0x800_per_client() {
        assert_eq!(cone_origin_slot(0, 5), Some(5));
        assert_eq!(cone_origin_slot(2, 5), Some(0x1005));
        assert_eq!(cone_origin_slot(0, 0x800), None);
        assert_eq!(cone_origin_slot(u32::MAX, 1), None);
    }

    #[test]
    fn an_entity_relinks_only_past_the_threshold() {
        let last = [0.0; 3];
        assert!(!cone_relink_due(
            last,
            [16.0, 0.0, 0.0],
            SPOT_SHADOW_RELINK_MOVE_DIST_SQUARED
        ));
        assert!(cone_relink_due(
            last,
            [10.0, 10.0, 10.0],
            SPOT_SHADOW_RELINK_MOVE_DIST_SQUARED
        ));
    }

    #[test]
    fn occupancy_needs_the_visibility_table() {
        let mut out = [caster(0, false, [0.0; 3])];
        let slots = [scene_slot(0, None, None)];
        assert_eq!(
            populate_spot_shadow_occupancy(&slots, None, 0, 1, 0, 2, &mut out),
            Err(ConeShadowFillFault::VisUnread)
        );
    }

    #[test]
    fn occupancy_reads_each_entitys_bit_for_the_light() {
        let mut lit = vec![0u32; 4];
        write_vis_bit(&mut lit, entity_vis_bit(0, 7, 1, 0, 2).unwrap(), true);
        let shown = scene_slot(7 << 7, Some([1.0, 2.0, 3.0]), Some([4.0, 5.0, 6.0]));
        let half_bounds = scene_slot(8 << 7, None, Some([1.0; 3]));
        let mut out = [caster(0, false, [0.0; 3]); 2];
        let slots = [shown, half_bounds];
        let written =
            populate_spot_shadow_occupancy(&slots, Some(&lit), 0, 1, 0, 2, &mut out).unwrap();
        assert_eq!(written, 2);
        assert!(out[0].sight_lit && out[0].box_valid);
        assert_eq!(out[0].mid_point, [1.0, 2.0, 3.0]);
        assert_eq!(out[0].half_ext, [4.0, 5.0, 6.0]);
        assert!(!out[1].sight_lit);
        assert!(!out[1].box_valid);
        assert_eq!(out[1].mid_point, [0.0; 3]);
    }

    #[test]
    fn occupancy_stops_at_the_shorter_of_input_and_output() {
        let lit = vec![0u32; 4];
        let slot = scene_slot(0, None, None);
        let mut out = [caster(0, false, [0.0; 3]); 2];
        let written =
            populate_spot_shadow_occupancy(&[slot; 5], Some(&lit), 0, 1, 0, 2, &mut out).unwrap();
        assert_eq!(written, 2);
    }

    #[test]
    fn a_light_that_does_not_walk_casters_marks_nothing() {
        let mut marks = [0u8; 16];
        let ents = [caster(3, true, [0.0, 0.0, -10.0])];
        cone_shadow_walk_casters(false, &shadowable(), &ents, &ents, u32::MAX, &mut marks);
        assert_eq!(marks, [0; 16]);
    }

    #[test]
    fn a_lit_entity_in_the_cone_is_marked() {
        let mut marks = [0u8; 16];
        let ents = [caster(3, true, [0.0, 0.0, -10.0])];
        cone_shadow_walk_casters(true, &shadowable(), &ents, &[], u32::MAX, &mut marks);
        assert_eq!(marks[3], 1);
        assert_eq!(marks.iter().filter(|&&mark| mark != 0).count(), 1);
    }

    #[test]
    fn an_entity_outside_the_cone_is_not_marked() {
        let mut marks = [0u8; 16];
        let ents = [caster(3, true, [500.0, 0.0, -10.0])];
        cone_shadow_walk_casters(true, &shadowable(), &ents, &[], u32::MAX, &mut marks);
        assert_eq!(marks[3], 0);
    }

    #[test]
    fn an_entity_with_no_bounds_is_not_marked() {
        let mut marks = [0u8; 16];
        let mut ent = caster(3, true, [0.0, 0.0, -10.0]);
        ent.box_valid = false;
        cone_shadow_walk_casters(true, &shadowable(), &[ent], &[], u32::MAX, &mut marks);
        assert_eq!(marks[3], 0);
    }

    #[test]
    fn an_unlit_entity_is_not_marked() {
        let mut marks = [0u8; 16];
        let ents = [caster(3, false, [0.0, 0.0, -10.0])];
        cone_shadow_walk_casters(true, &shadowable(), &ents, &ents, u32::MAX, &mut marks);
        assert_eq!(marks[3], 0);
    }

    #[test]
    fn the_local_players_own_entity_never_casts() {
        let mut marks = [0u8; 16];
        let ents = [caster(3, true, [0.0, 0.0, -10.0])];
        cone_shadow_walk_casters(true, &shadowable(), &ents, &ents, 3, &mut marks);
        assert_eq!(marks[3], 0);
    }

    #[test]
    fn flagged_entities_never_cast() {
        let mut marks = [0u8; 16];
        for flag in [0x0008_0000u32, 0x0200_0000] {
            let mut ent = caster(3, true, [0.0, 0.0, -10.0]);
            ent.ent_bits |= flag;
            cone_shadow_walk_casters(true, &shadowable(), &[ent], &[ent], u32::MAX, &mut marks);
        }
        assert_eq!(marks, [0; 16]);
    }

    #[test]
    fn a_lit_static_model_is_marked_without_a_cone_test() {
        let mut marks = [0u8; 16];
        // Far outside the cone, bounds missing: marked on visibility alone.
        let mut model = caster(5, true, [900.0, 0.0, 900.0]);
        model.box_valid = false;
        cone_shadow_walk_casters(true, &shadowable(), &[], &[model], u32::MAX, &mut marks);
        assert_eq!(marks[5], 1);
    }

    #[test]
    fn marks_past_the_row_are_dropped() {
        let mut marks = [0u8; 4];
        let ents = [caster(9, true, [0.0, 0.0, -10.0])];
        cone_shadow_walk_casters(true, &shadowable(), &[], &ents, u32::MAX, &mut marks);
        assert_eq!(marks, [0; 4]);
    }

    #[test]
    fn a_scene_model_is_a_caster_when_its_entity_was_marked() {
        let marks = [0, 1, 0, 0];
        assert!(model_casts_cone(&marks, Some(1)));
        assert!(!model_casts_cone(&marks, Some(2)));
        assert!(!model_casts_cone(&marks, Some(40)));
        assert!(!model_casts_cone(&marks, None));
    }

    #[test]
    fn world_surfaces_are_copied_only_when_the_light_walks_them() {
        let surfs = [4u16, 5, 6];
        let mut out = [0u16; 8];
        assert_eq!(cone_world_facets(false, &surfs, &mut out), 0);
        assert_eq!(cone_world_facets(true, &surfs, &mut out), 3);
        assert_eq!(out[..3], surfs);
    }

    #[test]
    fn world_surfaces_are_cut_to_the_output_size() {
        let surfs = [4u16, 5, 6];
        let mut out = [0u16; 2];
        assert_eq!(cone_world_facets(true, &surfs, &mut out), 2);
        assert_eq!(out, [4, 5]);
    }

    #[test]
    fn only_bit_zero_stops_a_static_model_casting() {
        assert!(!spot_shadow_static_model_casts(0x1));
        assert!(!spot_shadow_static_model_casts(0x3));
        assert!(spot_shadow_static_model_casts(0x0));
        assert!(spot_shadow_static_model_casts(0x2));
        assert!(spot_shadow_static_model_casts(0x10));
    }

    #[test]
    fn the_static_model_filter_keeps_casters_in_order() {
        let ids = [1u16, 2, 3, 4];
        let mut out = [0u16; 8];
        let kept = cone_shadow_prune_static_ids(
            true,
            &ids,
            |id| match id {
                1 => Some(0),
                2 => Some(1),
                3 => None,
                _ => Some(2),
            },
            &mut out,
        );
        assert_eq!(kept, 2);
        assert_eq!(out[..2], [1, 4]);
    }

    #[test]
    fn the_static_model_filter_respects_the_walk_and_the_output_size() {
        let ids = [1u16, 2, 3];
        let mut out = [0u16; 2];
        assert_eq!(
            cone_shadow_prune_static_ids(false, &ids, |_| Some(0), &mut out),
            0
        );
        assert_eq!(
            cone_shadow_prune_static_ids(true, &ids, |_| Some(0), &mut out),
            2
        );
        assert_eq!(out, [1, 2]);
    }

    #[test]
    fn a_pose_is_ready_only_at_cull_gate_two() {
        assert!(cone_pose_ready(2));
        assert!(!cone_pose_ready(0));
        assert!(!cone_pose_ready(1));
        assert!(!cone_pose_ready(3));
    }

    #[test]
    fn scene_visibility_copies_the_camera_bytes() {
        let mut seen = vec![0u8; SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN * 2];
        let camera = [0u8, 1, 3, 0];
        populate_spot_animated_visibility(&mut seen, 1, &[1, 2, 3, 9], Some(&camera), None);
        let base = SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN;
        assert_eq!(seen[base..base + 4], [1, 3, 0, 0]);
        assert!(seen[..base].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn scene_visibility_without_camera_data_changes_nothing() {
        let mut seen = vec![7u8; SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN];
        populate_spot_animated_visibility(&mut seen, 0, &[1, 2], None, None);
        assert!(seen.iter().all(|&byte| byte == 7));
    }

    #[test]
    fn an_unready_pose_loses_its_visible_bit() {
        let mut seen = vec![0u8; SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN];
        let camera = [1u8, 1, 2];
        let pose_accepted_cases = [false, true, false];
        populate_spot_animated_visibility(&mut seen, 0, &[0, 1, 2], Some(&camera), Some(&pose_accepted_cases));
        // Entry 2 has no visible bit (2), so being unready changes nothing.
        assert_eq!(seen[..3], [0, 1, 2]);
    }

    #[test]
    fn model_visibility_uses_its_own_stride() {
        let mut seen = vec![0u8; SPOT_SHADOW_STATIC_VISIBILITY_SPAN * 2];
        let camera = [1u8];
        populate_spot_static_visibility(&mut seen, 1, &[0], Some(&camera), None);
        assert_eq!(seen[SPOT_SHADOW_STATIC_VISIBILITY_SPAN], 1);
    }

    #[test]
    fn entries_past_a_slots_stride_do_not_spill_into_the_next_slot() {
        let stride = SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN;
        let mut seen = vec![0u8; stride * 2];
        let entnums = vec![0u32; stride + 5];
        populate_spot_animated_visibility(&mut seen, 0, &entnums, Some(&[1u8]), None);
        assert!(seen[..stride].iter().all(|&byte| byte == 1));
        assert!(seen[stride..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn tessellation_lists_the_visible_entries_of_a_slot() {
        let stride = SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN;
        let mut seen = vec![0u8; stride * 2];
        seen[stride + 1] = 1;
        seen[stride + 4] = 1;
        seen[stride + 5] = 3;
        seen[2] = 1;
        let mut out = [0u32; 8];
        let found = split_moving_ids(&seen, 1, 6, &mut out);
        assert_eq!(found, 2);
        assert_eq!(out[..2], [1, 4]);
    }

    #[test]
    fn tessellation_only_looks_at_the_scene_count() {
        let mut seen = vec![0u8; SPOT_SHADOW_STATIC_VISIBILITY_SPAN];
        seen[2] = 1;
        seen[9] = 1;
        let mut out = [0u32; 8];
        assert_eq!(
            cone_shadow_stitch_stage_model_indices(&seen, 0, 5, &mut out),
            1
        );
        assert_eq!(out[0], 2);
    }

    #[test]
    fn tessellation_stops_when_the_output_is_full() {
        let mut seen = vec![0u8; SPOT_SHADOW_STATIC_VISIBILITY_SPAN];
        seen[..10].fill(1);
        let mut out = [0u32; 3];
        assert_eq!(
            cone_shadow_stitch_stage_model_indices(&seen, 0, 10, &mut out),
            3
        );
        assert_eq!(out, [0, 1, 2]);
    }
}
