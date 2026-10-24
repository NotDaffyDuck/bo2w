//! Spot shadow atlas layout and the caster bookkeeping around it.
//!
//! BO2 draws every spot shadow into one depth target, a 2 x 2 grid of square
//! tiles, one tile per shadowed light. The rest of this module decides which
//! entities and static models a light can see (a per-light visibility bit
//! per entity and per dynamic brush), and shapes the lists the shadow passes
//! draw from.

use crate::cull::{PrimaryLightCull, cull_box_from_cone, cull_point_from_cone_expanded};
use crate::scene_light::gfx_light_is_spot;
use crate::smodel_lighting::STATIC_MODEL_FLAG_NO_CAST_SHADOW;
use crate::spot_shadow_choose::{SPOT_SHADOW_SM_LIGHT_CAP, SpotShadowableLight};

/// Render target slot of the spot shadow depth atlas.
pub const GFX_SPOT_SHADOW_RT: u8 = 0x10;

/// sm_spotQuality when the dvar is not set: the smallest tile.
pub const SM_SPOT_QUALITY_DEFAULT: u32 = 0;

/// What a tile covers of the atlas along each axis; also the scale that puts
/// the light's clip space into its tile.
pub(crate) const SPOT_SHADOW_TILE_SCALE: f32 = 0.5;

/// Entity slots per view in the visibility bit table.
pub const SPOT_SHADOW_CFG_INDEX_ENTS: u32 = 0x800;

/// Entities whose last-seen origin is tracked per local client.
pub const SPOT_SHADOW_ENTITY_ORIGIN_TRACK_ENTS: u32 = 0x800;

/// An entity must move more than 16 units (squared: 256) before its light
/// links are rebuilt.
pub const SPOT_SHADOW_ENTITY_RELINK_THRESH_SQ: f32 = 256.0;

/// Entries in the per-light "this entity casts" mark row.
pub const SPOT_SHADOW_ENT_MARK_LEN: usize = 0x1000;

/// Scene model-object entries each shadow slot can hold.
pub const SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE: usize = 0x200;

/// Scene model entries each shadow slot can hold.
pub const SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE: usize = 0x400;

const ATLAS_TILES_PER_SIDE: u32 = 2;
const TILE_BASE: u32 = 512;

/// Entities are linked to a light a little early so a shadow does not pop in
/// at the edge of the light's reach.
const LINK_ENTITY_PAD: f32 = 16.0;

/// Entity info bits that keep an entity out of every shadow.
const ENT_SKIP_FLAGS: u32 = 0x0208_0000;
const ENTNUM_SHIFT: u32 = 7;
const ENTNUM_MASK: u32 = 0xfff;

/// Tile edge in pixels: each sm_spotQuality step doubles it.
pub const fn spot_shadow_tile(spot_quality: u32) -> u32 {
    TILE_BASE << spot_quality
}

/// Where one shadow's tile sits in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpotShadowSlotPlan {
    pub render_target_id: u8,
    pub viewport_x: u32,
    pub viewport_y: u32,
    pub tile: u32,
    /// Only the first tile of a frame clears the atlas.
    pub clear: bool,
}

impl SpotShadowSlotPlan {
    /// One texel in tile space, as the lookup shader wants it.
    pub fn pixel_adjust(self) -> [f32; 4] {
        let texel = 1.0 / self.tile as f32;
        [texel, texel, 0.0, 0.0]
    }

    /// The tile's corner in atlas units: 0 or 0.5 along each axis.
    pub fn tile_offset(self) -> [f32; 2] {
        let tile = self.tile as f32;
        [
            SPOT_SHADOW_TILE_SCALE * (self.viewport_x as f32 / tile),
            SPOT_SHADOW_TILE_SCALE * (self.viewport_y as f32 / tile),
        ]
    }
}

/// Tiles fill the atlas left to right, then top to bottom.
pub fn spot_shadow_slot_plan(slot_index: u32, tile: u32) -> SpotShadowSlotPlan {
    SpotShadowSlotPlan {
        render_target_id: GFX_SPOT_SHADOW_RT,
        viewport_x: (slot_index % ATLAS_TILES_PER_SIDE) * tile,
        viewport_y: (slot_index / ATLAS_TILES_PER_SIDE) * tile,
        tile,
        clear: slot_index == 0,
    }
}

/// Hands out the next free tile, or nothing once the atlas is full.
pub fn spot_shadow_take_slot(shadowable_count: &mut u32) -> Option<u32> {
    let slot = *shadowable_count;
    if slot as usize >= SPOT_SHADOW_SM_LIGHT_CAP {
        return None;
    }
    *shadowable_count = slot + 1;
    Some(slot)
}

/// Whether the packed draw lists have any work entries to draw from.
pub fn spot_shadow_packed_lists_ready(work_entry_n: u32) -> bool {
    work_entry_n != 0
}

// ---- Light visibility bits ----

/// Bit for `row` and `light_index` in a table that holds one bit per
/// non-sun primary light for every row. Arithmetic wraps; a result below zero
/// has no bit.
fn light_vis_bit(row: i32, light_index: u32, sun_primary: u32, primary_count: u32) -> Option<u32> {
    let first_lit = sun_primary.wrapping_add(1) as i32;
    let lights_per_row = (primary_count as i32).wrapping_sub(first_lit);
    let bit = row
        .wrapping_mul(lights_per_row)
        .wrapping_sub(first_lit)
        .wrapping_add(light_index as i32);
    u32::try_from(bit).ok()
}

fn entity_vis_bit(
    view: u32,
    entnum: u32,
    light_index: u32,
    sun_primary: u32,
    primary_count: u32,
) -> Option<u32> {
    let row = (SPOT_SHADOW_CFG_INDEX_ENTS as i32)
        .wrapping_mul(view as i32)
        .wrapping_add(entnum as i32);
    light_vis_bit(row, light_index, sun_primary, primary_count)
}

/// Bit for a dynamic brush: brushes are rows of their own, indexed by id.
pub fn spot_shadow_dyn_brush_vis_bit_index(
    brush_id: u32,
    light_index: u32,
    sun_primary: u32,
    primary_count: u32,
) -> Option<u32> {
    light_vis_bit(brush_id as i32, light_index, sun_primary, primary_count)
}

/// Words the entity visibility table needs to reach the last bit of the last
/// entity of the last view; zero when any count is zero.
pub fn spot_shadow_primary_vis_word_count(
    view_count: u32,
    ent_count: u32,
    sun_primary: u32,
    primary_count: u32,
) -> usize {
    if view_count == 0 || ent_count == 0 || primary_count == 0 {
        return 0;
    }
    entity_vis_bit(
        view_count - 1,
        ent_count - 1,
        primary_count - 1,
        sun_primary,
        primary_count,
    )
    .map_or(0, |bit| bit as usize / 32 + 1)
}

/// Whether a bit is set; bits past the table read as clear.
pub fn spot_shadow_primary_vis_bit(words: &[u32], bit: u32) -> bool {
    words
        .get((bit >> 5) as usize)
        .is_some_and(|word| word & (1 << (bit & 31)) != 0)
}

fn write_vis_bit(words: &mut [u32], bit: u32, set: bool) {
    let Some(word) = words.get_mut((bit >> 5) as usize) else {
        return;
    };
    let mask = 1u32 << (bit & 31);
    if set {
        *word |= mask;
    } else {
        *word &= !mask;
    }
}

/// Whether a point, grown by `slack`, is touched by the light: inside its
/// sphere and, for a spot, inside its widened cone.
fn light_touches(light: &PrimaryLightCull, point: [f32; 3], slack: f32) -> bool {
    let dx = point[0] - light.origin[0];
    let dy = point[1] - light.origin[1];
    let dz = point[2] - light.origin[2];
    let reach = light.radius + slack;
    let inside_sphere = dx * dx + dy * dy + dz * dz < reach * reach;
    if !inside_sphere {
        return false;
    }
    let outside_cone = gfx_light_is_spot(light.light_type)
        && light.cos_half_fov_expanded > 0.0
        && cull_point_from_cone_expanded(
            light.origin,
            light.direction,
            light.cos_half_fov_expanded,
            point,
            slack,
        );
    !outside_cone
}

/// Sets or clears the bit of every non-sun primary light for one row.
fn link_row(
    words: &mut [u32],
    lights: &[PrimaryLightCull],
    sun_primary: u32,
    point: [f32; 3],
    slack: f32,
    bit_for: impl Fn(u32) -> Option<u32>,
) {
    let first = sun_primary.saturating_add(1) as usize;
    for (light_index, light) in lights.iter().enumerate().skip(first) {
        let Some(bit) = bit_for(light_index as u32) else {
            continue;
        };
        write_vis_bit(words, bit, light_touches(light, point, slack));
    }
}

/// Records which primary lights reach an entity at `origin`.
pub fn spot_shadow_link_entity(
    words: &mut [u32],
    view: u32,
    entnum: u32,
    origin: [f32; 3],
    dobj_radius: f32,
    lights: &[PrimaryLightCull],
    sun_primary: u32,
) {
    let primary_count = lights.len() as u32;
    link_row(
        words,
        lights,
        sun_primary,
        origin,
        dobj_radius + LINK_ENTITY_PAD,
        |light_index| entity_vis_bit(view, entnum, light_index, sun_primary, primary_count),
    );
}

/// Records which primary lights reach a dynamic brush at `origin`.
pub fn spot_shadow_link_dyn_brush_vis(
    words: &mut [u32],
    brush_id: u32,
    origin: [f32; 3],
    extra: f32,
    lights: &[PrimaryLightCull],
    sun_primary: u32,
) {
    let primary_count = lights.len() as u32;
    link_row(words, lights, sun_primary, origin, extra, |light_index| {
        spot_shadow_dyn_brush_vis_bit_index(brush_id, light_index, sun_primary, primary_count)
    });
}

/// A dynamic brush is drawn into a shadow when the light walks the primary
/// lists and the brush's bit for it is set.
pub fn spot_shadow_dyn_brush_tess_admits(walks_primary: bool, vis_bit: bool) -> bool {
    walks_primary && vis_bit
}

// ---- Entity origin tracking ----

/// Slot of an entity in the per-client origin tracker.
pub fn spot_shadow_entity_origin_track_index(local_client: u32, entnum: u32) -> Option<usize> {
    if entnum >= SPOT_SHADOW_ENTITY_ORIGIN_TRACK_ENTS {
        return None;
    }
    local_client
        .checked_mul(SPOT_SHADOW_ENTITY_ORIGIN_TRACK_ENTS)?
        .checked_add(entnum)
        .map(|index| index as usize)
}

/// Whether an entity moved far enough since its last link to need a new one.
pub fn spot_shadow_entity_should_relink(
    last: [f32; 3],
    now: [f32; 3],
    threshold_squared: f32,
) -> bool {
    let dx = now[0] - last[0];
    let dy = now[1] - last[1];
    let dz = now[2] - last[2];
    threshold_squared < dx * dx + dy * dy + dz * dz
}

// ---- Casters ----

fn entnum_of(info: u32) -> u32 {
    (info >> ENTNUM_SHIFT) & ENTNUM_MASK
}

/// A scene entry as the occupancy fill sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowSceneSlot {
    pub info: u32,
    pub box_mid: Option<[f32; 3]>,
    pub box_half: Option<[f32; 3]>,
}

/// One candidate caster for a light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowCasterIn {
    pub info: u32,
    /// The entity's bit for this light is set.
    pub vis_lit: bool,
    pub bounds_ok: bool,
    pub box_mid: [f32; 3],
    pub box_half: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpotShadowOccupancyError {
    /// The visibility table has not been built this frame.
    VisUnread,
}

/// Turns scene entries into caster candidates for one light, reading each
/// entry's visibility bit. Returns how many entries were written to `out`.
pub fn spot_shadow_fill_occupancy(
    slots: &[SpotShadowSceneSlot],
    vis: Option<&[u32]>,
    view: u32,
    light_index: u32,
    sun_primary: u32,
    primary_count: u32,
    out: &mut [SpotShadowCasterIn],
) -> Result<usize, SpotShadowOccupancyError> {
    let words = vis.ok_or(SpotShadowOccupancyError::VisUnread)?;
    let mut written = 0;
    for (slot, caster) in slots.iter().zip(out.iter_mut()) {
        let vis_lit = entity_vis_bit(
            view,
            entnum_of(slot.info),
            light_index,
            sun_primary,
            primary_count,
        )
        .is_some_and(|bit| spot_shadow_primary_vis_bit(words, bit));
        *caster = SpotShadowCasterIn {
            info: slot.info,
            vis_lit,
            bounds_ok: slot.box_mid.is_some() && slot.box_half.is_some(),
            box_mid: slot.box_mid.unwrap_or([0.0; 3]),
            box_half: slot.box_half.unwrap_or([0.0; 3]),
        };
        written += 1;
    }
    Ok(written)
}

/// Whether a candidate is worth looking at: not flagged out, not the local
/// client's own entity, and reached by the light.
fn caster_wanted(caster: &SpotShadowCasterIn, local_client_entnum: u32) -> bool {
    caster.info & ENT_SKIP_FLAGS == 0
        && entnum_of(caster.info) != local_client_entnum
        && caster.vis_lit
}

fn mark_caster(row: &mut [u8], entnum: u32) {
    if let Some(mark) = row.get_mut(entnum as usize) {
        *mark = 1;
    }
}

/// Marks every entity that casts into `light`'s shadow. A light that does not
/// walk casters marks nothing. Scene entities also need their bounds inside
/// the light's cone; static models are marked on visibility alone.
pub fn spot_shadow_walk_casters(
    walks_casters: bool,
    light: &SpotShadowableLight,
    ents: &[SpotShadowCasterIn],
    smodels: &[SpotShadowCasterIn],
    local_client_entnum: u32,
    mark_row: &mut [u8],
) {
    if !walks_casters {
        return;
    }
    for ent in ents
        .iter()
        .filter(|ent| caster_wanted(ent, local_client_entnum))
    {
        let in_cone = ent.bounds_ok
            && !cull_box_from_cone(
                light.origin,
                light.dir,
                light.cos_half_fov_expanded,
                ent.box_mid,
                ent.box_half,
            );
        if in_cone {
            mark_caster(mark_row, entnum_of(ent.info));
        }
    }
    for model in smodels
        .iter()
        .filter(|model| caster_wanted(model, local_client_entnum))
    {
        mark_caster(mark_row, entnum_of(model.info));
    }
}

/// Whether the entity behind a scene model was marked as a caster.
pub fn spot_shadow_xmodel_is_caster(row: &[u8], scene_entnum: Option<u32>) -> bool {
    scene_entnum
        .and_then(|entnum| row.get(entnum as usize))
        .is_some_and(|&mark| mark != 0)
}

// ---- Per-slot draw lists ----

/// Copies the world surfaces a light draws, if it walks the primary lists.
pub fn spot_shadow_tess_bsp_surfs(walks_primary: bool, surfs: &[u16], out: &mut [u16]) -> usize {
    if !walks_primary {
        return 0;
    }
    let count = surfs.len().min(out.len());
    out[..count].copy_from_slice(&surfs[..count]);
    count
}

/// A static model casts unless its no-cast-shadow flag is set.
pub fn spot_shadow_smodel_casts(flags: u8) -> bool {
    flags & STATIC_MODEL_FLAG_NO_CAST_SHADOW == 0
}

/// Keeps the static models that cast; `flags_at` gives a model's flags, or
/// nothing for a model that is not drawn at all.
pub fn spot_shadow_filter_smodel_ids(
    walks_primary: bool,
    ids: &[u16],
    mut flags_at: impl FnMut(u16) -> Option<u8>,
    out: &mut [u16],
) -> usize {
    if !walks_primary {
        return 0;
    }
    let mut count = 0;
    for &id in ids {
        if count == out.len() {
            break;
        }
        if flags_at(id).is_some_and(spot_shadow_smodel_casts) {
            out[count] = id;
            count += 1;
        }
    }
    count
}

/// Whether a scene model object has a pose good enough to draw: its cull
/// gate must have settled on 2.
pub fn spot_shadow_175d0_pose_ok(cull_gate: u32) -> bool {
    cull_gate == 2
}

fn vis_index(slot: u32, entry: usize, stride: usize) -> usize {
    (slot as usize).saturating_mul(stride).saturating_add(entry)
}

/// Fills one slot's visibility bytes from the camera's. An entry whose
/// `ready` flag is false loses its visible bit. Does nothing without camera
/// data; entries past the slot's stride are dropped.
fn fill_scene_vis(
    vis: &mut [u8],
    slot: u32,
    stride: usize,
    entnums: &[u32],
    camera_vis: Option<&[u8]>,
    ready: Option<&[bool]>,
) {
    let Some(camera) = camera_vis else {
        return;
    };
    for (entry, &entnum) in entnums.iter().enumerate().take(stride) {
        let mut byte = camera.get(entnum as usize).copied().unwrap_or(0);
        if byte & 1 != 0 && ready.and_then(|flags| flags.get(entry)) == Some(&false) {
            byte = 0;
        }
        if let Some(target) = vis.get_mut(vis_index(slot, entry, stride)) {
            *target = byte;
        }
    }
}

/// Visibility bytes for the scene model objects of one shadow slot.
pub fn spot_shadow_fill_scene_dobj_vis(
    vis: &mut [u8],
    slot: u32,
    occupancy_entnums: &[u32],
    camera_vis: Option<&[u8]>,
    pose_ok: Option<&[bool]>,
) {
    fill_scene_vis(
        vis,
        slot,
        SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE,
        occupancy_entnums,
        camera_vis,
        pose_ok,
    );
}

/// Visibility bytes for the scene models of one shadow slot.
pub fn spot_shadow_fill_scene_model_vis(
    vis: &mut [u8],
    slot: u32,
    occupancy_entnums: &[u32],
    camera_vis: Option<&[u8]>,
    lod_ok: Option<&[bool]>,
) {
    fill_scene_vis(
        vis,
        slot,
        SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE,
        occupancy_entnums,
        camera_vis,
        lod_ok,
    );
}

/// Indices of the entries a slot marked visible (a byte of exactly 1).
fn visible_indices(vis: &[u8], slot: u32, count: u32, stride: usize, out: &mut [u32]) -> usize {
    let mut found = 0;
    for entry in 0..(count as usize).min(stride) {
        if found == out.len() {
            break;
        }
        if vis.get(vis_index(slot, entry, stride)) == Some(&1) {
            out[found] = entry as u32;
            found += 1;
        }
    }
    found
}

/// Scene model objects visible in a shadow slot.
pub fn spot_shadow_tess_scene_dobj_indices(
    vis: &[u8],
    slot: u32,
    scene_dobj_count: u32,
    out: &mut [u32],
) -> usize {
    visible_indices(
        vis,
        slot,
        scene_dobj_count,
        SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE,
        out,
    )
}

/// Scene models visible in a shadow slot.
pub fn spot_shadow_tess_scene_model_indices(
    vis: &[u8],
    slot: u32,
    scene_model_count: u32,
    out: &mut [u32],
) -> usize {
    visible_indices(
        vis,
        slot,
        scene_model_count,
        SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE,
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_light::{GFX_LIGHT_TYPE_OMNI, GFX_LIGHT_TYPE_SPOT};

    fn omni(origin: [f32; 3], radius: f32) -> PrimaryLightCull {
        PrimaryLightCull {
            light_type: GFX_LIGHT_TYPE_OMNI,
            origin,
            direction: [0.0, 0.0, 1.0],
            radius,
            cos_half_fov_expanded: 0.0,
        }
    }

    fn spot(origin: [f32; 3], radius: f32) -> PrimaryLightCull {
        PrimaryLightCull {
            light_type: GFX_LIGHT_TYPE_SPOT,
            origin,
            direction: [0.0, 0.0, 1.0],
            radius,
            cos_half_fov_expanded: 0.7071,
        }
    }

    fn shadowable() -> SpotShadowableLight {
        SpotShadowableLight {
            can_use_shadow_map: true,
            color: [1.0; 3],
            dir: [0.0, 0.0, 1.0],
            origin: [0.0; 3],
            radius: 100.0,
            light_type: GFX_LIGHT_TYPE_SPOT,
            cos_half_fov_expanded: 0.7071,
            cull_dist: i16::MAX,
            view_outside_shadowmap_volume: false,
        }
    }

    fn caster(entnum: u32, vis_lit: bool, mid: [f32; 3]) -> SpotShadowCasterIn {
        SpotShadowCasterIn {
            info: entnum << 7,
            vis_lit,
            bounds_ok: true,
            box_mid: mid,
            box_half: [1.0; 3],
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
            let plan = spot_shadow_slot_plan(slot, 512);
            assert_eq!(plan.render_target_id, GFX_SPOT_SHADOW_RT);
            assert_eq!(plan.viewport_x, (slot % 2) * 512);
            assert_eq!(plan.viewport_y, (slot / 2) * 512);
            assert_eq!(plan.tile, 512);
        }
    }

    #[test]
    fn only_the_first_slot_clears_the_atlas() {
        assert!(spot_shadow_slot_plan(0, 512).clear);
        assert!(!spot_shadow_slot_plan(1, 512).clear);
        assert!(!spot_shadow_slot_plan(3, 512).clear);
    }

    #[test]
    fn the_pixel_adjust_is_one_over_the_tile() {
        let plan = spot_shadow_slot_plan(2, 1024);
        assert_eq!(plan.pixel_adjust(), [1.0 / 1024.0, 1.0 / 1024.0, 0.0, 0.0]);
    }

    #[test]
    fn the_tile_offset_is_zero_or_half_per_axis() {
        assert_eq!(spot_shadow_slot_plan(0, 512).tile_offset(), [0.0, 0.0]);
        assert_eq!(spot_shadow_slot_plan(1, 512).tile_offset(), [0.5, 0.0]);
        assert_eq!(spot_shadow_slot_plan(2, 512).tile_offset(), [0.0, 0.5]);
        assert_eq!(spot_shadow_slot_plan(3, 512).tile_offset(), [0.5, 0.5]);
    }

    #[test]
    fn the_atlas_holds_four_slots() {
        let mut taken = 0;
        let got: Vec<_> = (0..6).map(|_| spot_shadow_take_slot(&mut taken)).collect();
        assert_eq!(got, [Some(0), Some(1), Some(2), Some(3), None, None]);
        assert_eq!(taken, 4);
    }

    #[test]
    fn packed_lists_need_at_least_one_work_entry() {
        assert!(!spot_shadow_packed_lists_ready(0));
        assert!(spot_shadow_packed_lists_ready(1));
    }

    #[test]
    fn dynamic_brush_bits_skip_the_sun_and_pack_per_brush() {
        // Sun is light 0, five lights in all: four bits per brush.
        assert_eq!(spot_shadow_dyn_brush_vis_bit_index(0, 1, 0, 5), Some(0));
        assert_eq!(spot_shadow_dyn_brush_vis_bit_index(0, 4, 0, 5), Some(3));
        assert_eq!(spot_shadow_dyn_brush_vis_bit_index(1, 1, 0, 5), Some(4));
        assert_eq!(spot_shadow_dyn_brush_vis_bit_index(2, 3, 0, 5), Some(10));
    }

    #[test]
    fn a_light_before_the_first_row_has_no_bit() {
        // Light 1 on brush 0 with the sun at light 1: 0*span - 2 + 1 < 0.
        assert_eq!(spot_shadow_dyn_brush_vis_bit_index(0, 1, 1, 5), None);
        assert_eq!(spot_shadow_dyn_brush_vis_bit_index(0, 2, 1, 5), Some(0));
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
        assert_eq!(spot_shadow_primary_vis_word_count(1, 0x800, 0, 3), 128);
    }

    #[test]
    fn the_word_count_is_zero_when_a_count_is_zero() {
        assert_eq!(spot_shadow_primary_vis_word_count(0, 0x800, 0, 3), 0);
        assert_eq!(spot_shadow_primary_vis_word_count(1, 0, 0, 3), 0);
        assert_eq!(spot_shadow_primary_vis_word_count(1, 0x800, 0, 0), 0);
    }

    #[test]
    fn vis_bits_read_clear_past_the_table() {
        let words = [0b101u32, 1 << 31];
        assert!(spot_shadow_primary_vis_bit(&words, 0));
        assert!(!spot_shadow_primary_vis_bit(&words, 1));
        assert!(spot_shadow_primary_vis_bit(&words, 2));
        assert!(spot_shadow_primary_vis_bit(&words, 63));
        assert!(!spot_shadow_primary_vis_bit(&words, 64));
    }

    #[test]
    fn linking_an_entity_sets_the_bits_of_the_lights_that_reach_it() {
        // Sun at 0, then two omnis: one covers the origin, one is far away.
        let lights = [
            omni([0.0; 3], 1.0),
            omni([10.0, 0.0, 0.0], 50.0),
            omni([1000.0, 0.0, 0.0], 50.0),
        ];
        let mut words = vec![0u32; 4];
        spot_shadow_link_entity(&mut words, 0, 3, [0.0; 3], 0.0, &lights, 0);
        let near = entity_vis_bit(0, 3, 1, 0, 3).unwrap();
        let far = entity_vis_bit(0, 3, 2, 0, 3).unwrap();
        assert!(spot_shadow_primary_vis_bit(&words, near));
        assert!(!spot_shadow_primary_vis_bit(&words, far));
    }

    #[test]
    fn linking_clears_a_bit_the_light_no_longer_reaches() {
        let lights = [omni([0.0; 3], 1.0), omni([10.0, 0.0, 0.0], 50.0)];
        let mut words = vec![0u32; 2];
        spot_shadow_link_entity(&mut words, 0, 0, [0.0; 3], 0.0, &lights, 0);
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        assert!(spot_shadow_primary_vis_bit(&words, bit));
        spot_shadow_link_entity(&mut words, 0, 0, [500.0, 0.0, 0.0], 0.0, &lights, 0);
        assert!(!spot_shadow_primary_vis_bit(&words, bit));
    }

    #[test]
    fn an_entity_is_linked_a_little_beyond_the_light_radius() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        let mut words = vec![0u32; 2];
        // 110 away: outside radius 100, inside radius + pad (116).
        spot_shadow_link_entity(&mut words, 0, 0, [110.0, 0.0, 0.0], 0.0, &lights, 0);
        assert!(spot_shadow_primary_vis_bit(&words, bit));
        spot_shadow_link_entity(&mut words, 0, 0, [120.0, 0.0, 0.0], 0.0, &lights, 0);
        assert!(!spot_shadow_primary_vis_bit(&words, bit));
    }

    #[test]
    fn a_larger_entity_radius_extends_the_link() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        let mut words = vec![0u32; 2];
        spot_shadow_link_entity(&mut words, 0, 0, [130.0, 0.0, 0.0], 20.0, &lights, 0);
        assert!(spot_shadow_primary_vis_bit(&words, bit));
    }

    #[test]
    fn a_spot_does_not_link_what_is_behind_it() {
        let lights = [omni([0.0; 3], 1.0), spot([0.0; 3], 100.0)];
        let bit = entity_vis_bit(0, 0, 1, 0, 2).unwrap();
        let mut words = vec![0u32; 2];
        // The cone opens against its direction: z < 0 is in front of it.
        spot_shadow_link_entity(&mut words, 0, 0, [0.0, 0.0, -50.0], 0.0, &lights, 0);
        assert!(spot_shadow_primary_vis_bit(&words, bit));
        spot_shadow_link_entity(&mut words, 0, 0, [0.0, 0.0, 50.0], 0.0, &lights, 0);
        assert!(!spot_shadow_primary_vis_bit(&words, bit));
    }

    #[test]
    fn linking_a_dynamic_brush_uses_its_own_row() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let mut words = vec![0u32; 2];
        spot_shadow_link_dyn_brush_vis(&mut words, 2, [10.0, 0.0, 0.0], 0.0, &lights, 0);
        let bit = spot_shadow_dyn_brush_vis_bit_index(2, 1, 0, 2).unwrap();
        assert!(spot_shadow_primary_vis_bit(&words, bit));
        assert_eq!(words[0].count_ones() + words[1].count_ones(), 1);
    }

    #[test]
    fn linking_with_a_short_table_drops_the_bits_that_do_not_fit() {
        let lights = [omni([0.0; 3], 1.0), omni([0.0; 3], 100.0)];
        let mut words: Vec<u32> = Vec::new();
        spot_shadow_link_entity(&mut words, 0, 0, [0.0; 3], 0.0, &lights, 0);
        assert!(words.is_empty());
    }

    #[test]
    fn a_dynamic_brush_is_drawn_only_when_both_conditions_hold() {
        assert!(spot_shadow_dyn_brush_tess_admits(true, true));
        assert!(!spot_shadow_dyn_brush_tess_admits(true, false));
        assert!(!spot_shadow_dyn_brush_tess_admits(false, true));
    }

    #[test]
    fn origin_tracking_has_a_row_of_0x800_per_client() {
        assert_eq!(spot_shadow_entity_origin_track_index(0, 5), Some(5));
        assert_eq!(spot_shadow_entity_origin_track_index(2, 5), Some(0x1005));
        assert_eq!(spot_shadow_entity_origin_track_index(0, 0x800), None);
        assert_eq!(spot_shadow_entity_origin_track_index(u32::MAX, 1), None);
    }

    #[test]
    fn an_entity_relinks_only_past_the_threshold() {
        let last = [0.0; 3];
        assert!(!spot_shadow_entity_should_relink(
            last,
            [16.0, 0.0, 0.0],
            SPOT_SHADOW_ENTITY_RELINK_THRESH_SQ
        ));
        assert!(spot_shadow_entity_should_relink(
            last,
            [10.0, 10.0, 10.0],
            SPOT_SHADOW_ENTITY_RELINK_THRESH_SQ
        ));
    }

    #[test]
    fn occupancy_needs_the_visibility_table() {
        let mut out = [caster(0, false, [0.0; 3])];
        let slots = [SpotShadowSceneSlot {
            info: 0,
            box_mid: None,
            box_half: None,
        }];
        assert_eq!(
            spot_shadow_fill_occupancy(&slots, None, 0, 1, 0, 2, &mut out),
            Err(SpotShadowOccupancyError::VisUnread)
        );
    }

    #[test]
    fn occupancy_reads_each_entitys_bit_for_the_light() {
        let mut words = vec![0u32; 4];
        write_vis_bit(&mut words, entity_vis_bit(0, 7, 1, 0, 2).unwrap(), true);
        let slots = [
            SpotShadowSceneSlot {
                info: 7 << 7,
                box_mid: Some([1.0, 2.0, 3.0]),
                box_half: Some([4.0, 5.0, 6.0]),
            },
            SpotShadowSceneSlot {
                info: 8 << 7,
                box_mid: None,
                box_half: Some([1.0; 3]),
            },
        ];
        let mut out = [caster(0, false, [0.0; 3]); 2];
        let written =
            spot_shadow_fill_occupancy(&slots, Some(&words), 0, 1, 0, 2, &mut out).unwrap();
        assert_eq!(written, 2);
        assert!(out[0].vis_lit && out[0].bounds_ok);
        assert_eq!(out[0].box_mid, [1.0, 2.0, 3.0]);
        assert_eq!(out[0].box_half, [4.0, 5.0, 6.0]);
        assert!(!out[1].vis_lit);
        assert!(!out[1].bounds_ok);
        assert_eq!(out[1].box_mid, [0.0; 3]);
    }

    #[test]
    fn occupancy_stops_at_the_shorter_of_input_and_output() {
        let words = vec![0u32; 4];
        let slot = SpotShadowSceneSlot {
            info: 0,
            box_mid: None,
            box_half: None,
        };
        let mut out = [caster(0, false, [0.0; 3]); 2];
        let written =
            spot_shadow_fill_occupancy(&[slot; 5], Some(&words), 0, 1, 0, 2, &mut out).unwrap();
        assert_eq!(written, 2);
    }

    #[test]
    fn a_light_that_does_not_walk_casters_marks_nothing() {
        let mut row = [0u8; 16];
        let ents = [caster(3, true, [0.0, 0.0, -10.0])];
        spot_shadow_walk_casters(false, &shadowable(), &ents, &ents, u32::MAX, &mut row);
        assert_eq!(row, [0; 16]);
    }

    #[test]
    fn a_lit_entity_in_the_cone_is_marked() {
        let mut row = [0u8; 16];
        let ents = [caster(3, true, [0.0, 0.0, -10.0])];
        spot_shadow_walk_casters(true, &shadowable(), &ents, &[], u32::MAX, &mut row);
        assert_eq!(row[3], 1);
        assert_eq!(row.iter().filter(|&&mark| mark != 0).count(), 1);
    }

    #[test]
    fn an_entity_outside_the_cone_is_not_marked() {
        let mut row = [0u8; 16];
        let ents = [caster(3, true, [500.0, 0.0, -10.0])];
        spot_shadow_walk_casters(true, &shadowable(), &ents, &[], u32::MAX, &mut row);
        assert_eq!(row[3], 0);
    }

    #[test]
    fn an_entity_with_no_bounds_is_not_marked() {
        let mut row = [0u8; 16];
        let mut ent = caster(3, true, [0.0, 0.0, -10.0]);
        ent.bounds_ok = false;
        spot_shadow_walk_casters(true, &shadowable(), &[ent], &[], u32::MAX, &mut row);
        assert_eq!(row[3], 0);
    }

    #[test]
    fn an_unlit_entity_is_not_marked() {
        let mut row = [0u8; 16];
        let ents = [caster(3, false, [0.0, 0.0, -10.0])];
        spot_shadow_walk_casters(true, &shadowable(), &ents, &ents, u32::MAX, &mut row);
        assert_eq!(row[3], 0);
    }

    #[test]
    fn the_local_players_own_entity_never_casts() {
        let mut row = [0u8; 16];
        let ents = [caster(3, true, [0.0, 0.0, -10.0])];
        spot_shadow_walk_casters(true, &shadowable(), &ents, &ents, 3, &mut row);
        assert_eq!(row[3], 0);
    }

    #[test]
    fn flagged_entities_never_cast() {
        let mut row = [0u8; 16];
        for flag in [0x0008_0000u32, 0x0200_0000] {
            let mut ent = caster(3, true, [0.0, 0.0, -10.0]);
            ent.info |= flag;
            spot_shadow_walk_casters(true, &shadowable(), &[ent], &[ent], u32::MAX, &mut row);
        }
        assert_eq!(row, [0; 16]);
    }

    #[test]
    fn a_lit_static_model_is_marked_without_a_cone_test() {
        let mut row = [0u8; 16];
        // Far outside the cone, bounds missing: marked on visibility alone.
        let mut model = caster(5, true, [900.0, 0.0, 900.0]);
        model.bounds_ok = false;
        spot_shadow_walk_casters(true, &shadowable(), &[], &[model], u32::MAX, &mut row);
        assert_eq!(row[5], 1);
    }

    #[test]
    fn marks_past_the_row_are_dropped() {
        let mut row = [0u8; 4];
        let ents = [caster(9, true, [0.0, 0.0, -10.0])];
        spot_shadow_walk_casters(true, &shadowable(), &[], &ents, u32::MAX, &mut row);
        assert_eq!(row, [0; 4]);
    }

    #[test]
    fn a_scene_model_is_a_caster_when_its_entity_was_marked() {
        let row = [0, 1, 0, 0];
        assert!(spot_shadow_xmodel_is_caster(&row, Some(1)));
        assert!(!spot_shadow_xmodel_is_caster(&row, Some(2)));
        assert!(!spot_shadow_xmodel_is_caster(&row, Some(40)));
        assert!(!spot_shadow_xmodel_is_caster(&row, None));
    }

    #[test]
    fn world_surfaces_are_copied_only_when_the_light_walks_them() {
        let surfs = [4u16, 5, 6];
        let mut out = [0u16; 8];
        assert_eq!(spot_shadow_tess_bsp_surfs(false, &surfs, &mut out), 0);
        assert_eq!(spot_shadow_tess_bsp_surfs(true, &surfs, &mut out), 3);
        assert_eq!(out[..3], surfs);
    }

    #[test]
    fn world_surfaces_are_cut_to_the_output_size() {
        let surfs = [4u16, 5, 6];
        let mut out = [0u16; 2];
        assert_eq!(spot_shadow_tess_bsp_surfs(true, &surfs, &mut out), 2);
        assert_eq!(out, [4, 5]);
    }

    #[test]
    fn only_bit_zero_stops_a_static_model_casting() {
        assert!(!spot_shadow_smodel_casts(0x1));
        assert!(!spot_shadow_smodel_casts(0x3));
        assert!(spot_shadow_smodel_casts(0x0));
        assert!(spot_shadow_smodel_casts(0x2));
        assert!(spot_shadow_smodel_casts(0x10));
    }

    #[test]
    fn the_static_model_filter_keeps_casters_in_order() {
        let ids = [1u16, 2, 3, 4];
        let mut out = [0u16; 8];
        let kept = spot_shadow_filter_smodel_ids(
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
            spot_shadow_filter_smodel_ids(false, &ids, |_| Some(0), &mut out),
            0
        );
        assert_eq!(
            spot_shadow_filter_smodel_ids(true, &ids, |_| Some(0), &mut out),
            2
        );
        assert_eq!(out, [1, 2]);
    }

    #[test]
    fn a_pose_is_ready_only_at_cull_gate_two() {
        assert!(spot_shadow_175d0_pose_ok(2));
        assert!(!spot_shadow_175d0_pose_ok(0));
        assert!(!spot_shadow_175d0_pose_ok(1));
        assert!(!spot_shadow_175d0_pose_ok(3));
    }

    #[test]
    fn scene_visibility_copies_the_camera_bytes() {
        let mut vis = vec![0u8; SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE * 2];
        let camera = [0u8, 1, 3, 0];
        spot_shadow_fill_scene_dobj_vis(&mut vis, 1, &[1, 2, 3, 9], Some(&camera), None);
        let base = SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE;
        assert_eq!(vis[base..base + 4], [1, 3, 0, 0]);
        assert!(vis[..base].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn scene_visibility_without_camera_data_changes_nothing() {
        let mut vis = vec![7u8; SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE];
        spot_shadow_fill_scene_dobj_vis(&mut vis, 0, &[1, 2], None, None);
        assert!(vis.iter().all(|&byte| byte == 7));
    }

    #[test]
    fn an_unready_pose_loses_its_visible_bit() {
        let mut vis = vec![0u8; SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE];
        let camera = [1u8, 1, 2];
        let pose_ok = [false, true, false];
        spot_shadow_fill_scene_dobj_vis(&mut vis, 0, &[0, 1, 2], Some(&camera), Some(&pose_ok));
        // Entry 2 has no visible bit (2), so being unready changes nothing.
        assert_eq!(vis[..3], [0, 1, 2]);
    }

    #[test]
    fn model_visibility_uses_its_own_stride() {
        let mut vis = vec![0u8; SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE * 2];
        let camera = [1u8];
        spot_shadow_fill_scene_model_vis(&mut vis, 1, &[0], Some(&camera), None);
        assert_eq!(vis[SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE], 1);
    }

    #[test]
    fn entries_past_a_slots_stride_do_not_spill_into_the_next_slot() {
        let stride = SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE;
        let mut vis = vec![0u8; stride * 2];
        let entnums = vec![0u32; stride + 5];
        spot_shadow_fill_scene_dobj_vis(&mut vis, 0, &entnums, Some(&[1u8]), None);
        assert!(vis[..stride].iter().all(|&byte| byte == 1));
        assert!(vis[stride..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn tessellation_lists_the_visible_entries_of_a_slot() {
        let stride = SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE;
        let mut vis = vec![0u8; stride * 2];
        vis[stride + 1] = 1;
        vis[stride + 4] = 1;
        vis[stride + 5] = 3;
        vis[2] = 1;
        let mut out = [0u32; 8];
        let found = spot_shadow_tess_scene_dobj_indices(&vis, 1, 6, &mut out);
        assert_eq!(found, 2);
        assert_eq!(out[..2], [1, 4]);
    }

    #[test]
    fn tessellation_only_looks_at_the_scene_count() {
        let mut vis = vec![0u8; SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE];
        vis[2] = 1;
        vis[9] = 1;
        let mut out = [0u32; 8];
        assert_eq!(
            spot_shadow_tess_scene_model_indices(&vis, 0, 5, &mut out),
            1
        );
        assert_eq!(out[0], 2);
    }

    #[test]
    fn tessellation_stops_when_the_output_is_full() {
        let mut vis = vec![0u8; SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE];
        vis[..10].fill(1);
        let mut out = [0u32; 3];
        assert_eq!(
            spot_shadow_tess_scene_model_indices(&vis, 0, 10, &mut out),
            3
        );
        assert_eq!(out, [0, 1, 2]);
    }
}
