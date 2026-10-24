//! BO2's laser sight (CG_Laser*, AddCore), from facts/fx-beam-laser-t6.md
//! section 4. Item numbers below are that section's.
//!
//! TODO(fact): the laser builder's argument roles are [read, inferred]; a
//! laser-sight shot pair settles them.

use crate::beam::{FxBeamKind, FxBeamTess, beam_color_key};
use crate::post_light::FxPostLight;
use crate::runtime::vec3_distance;

/// The attach point on the gun (item 9).
pub const FX_LASER_TAG: &str = "tag_laser";

/// CG_LaserAdd holds this many entities a frame (item 10); MW2 held 48.
pub const FX_LASER_ADD_CAP: usize = 96;

/// The laser trace's contents mask (item 2).
pub const FX_LASER_TRACE_CONTENTS: u32 = 0x280_f823;

/// Per-entity trace cache slots, least recently used goes first (item 2).
pub const FX_LASER_TRACE_CACHE_SLOTS: usize = 10;

/// The light runs on by `laserLightBodyTweak` past surfaces with any of these
/// flags (item 8).
pub const FX_LASER_LIGHT_BODY_SURF: u32 = 0x200_c000;

/// A light span shorter than this collapses to its midpoint +/- the pad (item 8).
pub const FX_LASER_LIGHT_MIN_SPAN: f32 = 4.0;
pub const FX_LASER_LIGHT_SPAN_PAD: f32 = 2.0;

pub const FX_LASER_MATERIAL: &str = "gfx_laser";
pub const FX_LASER_VIEWMODEL_MATERIAL: &str = "gfx_laser_viewmodel";
pub const FX_LASER_LIGHT_MATERIAL: &str = "gfx_laser_light";

/// The first-person beam's draw-surf override (item 6); the rest take 0,
/// which the beam turns into 0x18.
pub const FX_LASER_VIEWMODEL_DRAW_SURF_OVERRIDE: u8 = 0xf;

/// The laser dvars, BO2's defaults (section 4's table).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxLaserDvars {
    /// `cg_laserLight`, the post light's master gate.
    /// TODO(fact): its default is not found; on, as MW2's was.
    pub light: bool,
    pub range: f32,
    pub range_player: f32,
    pub light_range_player: f32,
    pub radius: f32,
    pub light_radius: f32,
    pub light_begin_offset: f32,
    pub light_end_offset: f32,
    pub end_offset: f32,
    pub light_body_tweak: f32,
    pub flare_pct: f32,
    pub fade_ads: bool,
}

impl Default for FxLaserDvars {
    fn default() -> Self {
        Self {
            light: true,
            range: 1500.0,
            range_player: 2000.0,
            light_range_player: 2000.0,
            radius: 0.35,
            light_radius: 0.45,
            light_begin_offset: 10.0,
            light_end_offset: -1.0,
            end_offset: -2.0,
            light_body_tweak: 15.0,
            flare_pct: 0.25,
            fade_ads: true,
        }
    }
}

impl FxLaserDvars {
    /// The local player's own laser reaches further (item 2).
    #[must_use]
    pub const fn range_for(&self, local_player: bool) -> f32 {
        if local_player {
            self.range_player
        } else {
            self.range
        }
    }

    /// The light's own, longer trace when the beam hit nothing (item 8).
    #[must_use]
    pub fn light_retrace_range(&self, local_player: bool, hit: FxLaserHit) -> Option<f32> {
        (local_player && hit.fraction >= 1.0 && self.light_range_player > self.range_player)
            .then_some(self.light_range_player)
    }
}

/// One laser trace as the laser reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxLaserHit {
    pub fraction: f32,
    pub surface_flags: u32,
    pub startsolid: bool,
}

impl FxLaserHit {
    pub const MISS: Self = Self {
        fraction: 1.0,
        surface_flags: 0,
        startsolid: false,
    };
}

/// Where one laser comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxLaserAim {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
    pub range: f32,
    /// The local player's own gun, drawn first person (item 6).
    pub viewmodel: bool,
    /// The owner's weapon position fraction, 1 at full ADS (item 5).
    pub weapon_pos_frac: f32,
}

/// The beam and light one laser draws this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxLaserSight {
    pub beam: FxBeamTess,
    pub material: &'static str,
    pub light: Option<FxPostLight>,
}

/// How much the far end widens: 1 + flare x distance x 0.01 (item 1).
#[inline]
#[must_use]
pub fn laser_flare(flare_pct: f32, view_dist: f32) -> f32 {
    1.0 + flare_pct * view_dist * 0.01
}

/// The sight fades out as the gun comes up to ADS (item 5).
#[inline]
#[must_use]
pub fn laser_alpha(fade_ads: bool, weapon_pos_frac: f32) -> f32 {
    if fade_ads {
        1.0 - weapon_pos_frac.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// A cached trace is redone only on every 4th frame per entity (item 2).
#[inline]
#[must_use]
pub const fn laser_trace_due(frame: u32, entity: u32) -> bool {
    frame.wrapping_add(entity) & 3 == 3
}

/// Short light spans collapse to the midpoint +/- 2 (item 8).
#[inline]
#[must_use]
pub fn laser_light_span(begin: f32, end: f32) -> (f32, f32) {
    if end - begin < FX_LASER_LIGHT_MIN_SPAN {
        let mid = (begin + end) * 0.5;
        (mid - FX_LASER_LIGHT_SPAN_PAD, mid + FX_LASER_LIGHT_SPAN_PAD)
    } else {
        (begin, end)
    }
}

#[inline]
fn point_on_ray(origin: [f32; 3], dir: [f32; 3], t: f32) -> [f32; 3] {
    core::array::from_fn(|i| origin[i] + dir[i] * t)
}

/// The beam and light for one laser (items 1, 3-8, 11). `light_hit` is the
/// light's own trace out to `FxLaserDvars::light_retrace_range`, when it ran.
/// Nothing is drawn when the trace starts in solid (item 3).
#[must_use]
pub fn laser_sight(
    dvars: &FxLaserDvars,
    aim: FxLaserAim,
    hit: FxLaserHit,
    light_hit: Option<(f32, FxLaserHit)>,
    view: [f32; 3],
) -> Option<FxLaserSight> {
    if hit.startsolid {
        return None;
    }
    let hit_dist = hit.fraction * aim.range;
    let end = point_on_ray(aim.origin, aim.dir, hit_dist - dvars.end_offset);
    let flare = laser_flare(dvars.flare_pct, vec3_distance(end, view));

    let a = laser_alpha(dvars.fade_ads, aim.weapon_pos_frac);
    let begin_color = beam_color_key([a; 4]);
    let end_color = beam_color_key([a, a, a, (1.0 - hit.fraction) * a * a]);
    let beam = FxBeamTess {
        kind: FxBeamKind::Laser {
            end_radius: dvars.radius * flare,
        },
        depth_hack: aim.viewmodel,
        draw_surf_override: if aim.viewmodel {
            FX_LASER_VIEWMODEL_DRAW_SURF_OVERRIDE
        } else {
            0
        },
        begin: aim.origin,
        end,
        colors: [begin_color, end_color, [0; 4], [0; 4], [0; 4]],
        radius: dvars.radius,
        segment_count: 1,
    };
    let material = if aim.viewmodel {
        FX_LASER_VIEWMODEL_MATERIAL
    } else {
        FX_LASER_MATERIAL
    };

    // TODO(fact): the light also needs "the light enum ON"; which state that
    // enum reads is not named, so every drawn laser counts as on.
    let light = dvars.light.then(|| {
        let (light_dist, light_flags) = match light_hit {
            Some((range, h)) => (h.fraction * range, h.surface_flags),
            None => (hit_dist, hit.surface_flags),
        };
        let mut end_t = light_dist - dvars.light_end_offset;
        if light_flags & FX_LASER_LIGHT_BODY_SURF != 0 {
            end_t += dvars.light_body_tweak;
        }
        let (begin_t, end_t) = laser_light_span(dvars.light_begin_offset, end_t);
        FxPostLight {
            begin: point_on_ray(aim.origin, aim.dir, begin_t),
            end: point_on_ray(aim.origin, aim.dir, end_t),
            radius: dvars.light_radius * flare,
            color_packed: u32::from_le_bytes(begin_color),
            material_name: FX_LASER_LIGHT_MATERIAL,
        }
    });
    Some(FxLaserSight {
        beam,
        material,
        light,
    })
}

#[derive(Clone, Copy, Debug)]
struct CachedTrace {
    entity: u32,
    hit: FxLaserHit,
    used: u32,
}

/// get_laser_cache: the last trace per entity, refreshed on the entity's
/// every 4th frame (item 2). MW2 traced every frame.
#[derive(Clone, Debug, Default)]
pub struct FxLaserTraceCache {
    slots: [Option<CachedTrace>; FX_LASER_TRACE_CACHE_SLOTS],
}

impl FxLaserTraceCache {
    /// The entity's cached trace, or a fresh one from `trace` when it is due
    /// or not cached; a new entity takes the least recently used slot.
    pub fn hit(
        &mut self,
        frame: u32,
        entity: u32,
        trace: impl FnOnce() -> FxLaserHit,
    ) -> FxLaserHit {
        if let Some(slot) = self.slots.iter_mut().flatten().find(|s| s.entity == entity) {
            if laser_trace_due(frame, entity) {
                slot.hit = trace();
            }
            slot.used = frame;
            return slot.hit;
        }
        let hit = trace();
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                (0..FX_LASER_TRACE_CACHE_SLOTS)
                    .max_by_key(|&i| self.slots[i].map_or(u32::MAX, |s| frame.wrapping_sub(s.used)))
                    .unwrap_or(0)
            });
        self.slots[slot] = Some(CachedTrace {
            entity,
            hit,
            used: frame,
        });
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aim(viewmodel: bool, weapon_pos_frac: f32) -> FxLaserAim {
        FxLaserAim {
            origin: [0.0; 3],
            dir: [1.0, 0.0, 0.0],
            range: 2000.0,
            viewmodel,
            weapon_pos_frac,
        }
    }

    fn hit(fraction: f32, surface_flags: u32) -> FxLaserHit {
        FxLaserHit {
            fraction,
            surface_flags,
            startsolid: false,
        }
    }

    #[test]
    fn the_beam_runs_two_units_past_the_hit_and_widens_with_distance() {
        let d = FxLaserDvars::default();
        let s = laser_sight(&d, aim(false, 0.0), hit(0.5, 0), None, [0.0; 3]).unwrap();
        assert_eq!(s.beam.end, [1002.0, 0.0, 0.0]);
        assert_eq!(s.beam.radius, 0.35);
        let want = 0.35 * (1.0 + 0.25 * 1002.0 * 0.01);
        assert!((s.beam.end_radius() - want).abs() < 1e-5);
        assert_eq!(s.material, FX_LASER_MATERIAL);
        assert_eq!(s.beam.draw_surf_override(), 0x18);
    }

    #[test]
    fn the_sight_fades_out_at_ads() {
        let d = FxLaserDvars::default();
        let hip = laser_sight(&d, aim(true, 0.0), hit(0.5, 0), None, [0.0; 3]).unwrap();
        assert_eq!(hip.beam.colors[0], [255; 4]);
        assert_eq!(hip.beam.colors[1], [255, 255, 255, 128]);
        assert_eq!(hip.light.unwrap().color_packed, u32::MAX);
        let ads = laser_sight(&d, aim(true, 1.0), hit(0.5, 0), None, [0.0; 3]).unwrap();
        assert_eq!(ads.beam.colors[0], [0; 4]);
        assert_eq!(ads.light.unwrap().color_packed, 0);
    }

    #[test]
    fn the_first_person_beam_takes_the_viewmodel_material_and_depth_hack() {
        let d = FxLaserDvars::default();
        let s = laser_sight(&d, aim(true, 0.0), hit(1.0, 0), None, [0.0; 3]).unwrap();
        assert!(s.beam.depth_hack);
        assert_eq!(s.beam.draw_surf_override(), 0xf);
        assert_eq!(s.material, FX_LASER_VIEWMODEL_MATERIAL);
    }

    #[test]
    fn the_light_runs_from_ten_units_to_one_past_the_hit_or_on_into_a_body() {
        let d = FxLaserDvars::default();
        let s = laser_sight(&d, aim(false, 0.0), hit(0.05, 0), None, [0.0; 3]).unwrap();
        let light = s.light.unwrap();
        assert_eq!((light.begin[0], light.end[0]), (10.0, 101.0));
        assert!((light.radius - 0.45 * s.beam.end_radius() / 0.35).abs() < 1e-5);
        let body = laser_sight(&d, aim(false, 0.0), hit(0.05, 0x8000), None, [0.0; 3]).unwrap();
        assert_eq!(body.light.unwrap().end[0], 116.0);
        let close = laser_sight(&d, aim(false, 0.0), hit(0.005, 0), None, [0.0; 3]).unwrap();
        assert_eq!(
            (close.light.unwrap().begin[0], close.light.unwrap().end[0]),
            (8.5, 12.5)
        );
    }

    #[test]
    fn nothing_draws_from_inside_solid_and_the_light_has_its_own_gate() {
        let mut d = FxLaserDvars::default();
        let solid = FxLaserHit {
            startsolid: true,
            ..FxLaserHit::MISS
        };
        assert!(laser_sight(&d, aim(false, 0.0), solid, None, [0.0; 3]).is_none());
        d.light = false;
        let s = laser_sight(&d, aim(false, 0.0), FxLaserHit::MISS, None, [0.0; 3]).unwrap();
        assert!(s.light.is_none());
    }

    #[test]
    fn only_a_longer_player_light_range_retraces() {
        let mut d = FxLaserDvars::default();
        assert_eq!(d.light_retrace_range(true, FxLaserHit::MISS), None);
        d.light_range_player = 3000.0;
        assert_eq!(d.light_retrace_range(true, FxLaserHit::MISS), Some(3000.0));
        assert_eq!(d.light_retrace_range(false, FxLaserHit::MISS), None);
        assert_eq!(d.light_retrace_range(true, hit(0.5, 0)), None);
        assert_eq!((d.range_for(true), d.range_for(false)), (2000.0, 1500.0));
    }

    #[test]
    fn the_trace_cache_refreshes_every_fourth_frame_and_drops_the_oldest() {
        let mut cache = FxLaserTraceCache::default();
        let mut traced = 0;
        for frame in 0..8 {
            cache.hit(frame, 0, || {
                traced += 1;
                FxLaserHit::MISS
            });
        }
        // The first sight, then frames 3 and 7.
        assert_eq!(traced, 3);
        for entity in 1..=10 {
            cache.hit(8 + entity, entity, || FxLaserHit::MISS);
        }
        // Entity 0 was the oldest, so it was dropped and traces again.
        let mut again = false;
        cache.hit(20, 0, || {
            again = true;
            FxLaserHit::MISS
        });
        assert!(again);
    }
}
