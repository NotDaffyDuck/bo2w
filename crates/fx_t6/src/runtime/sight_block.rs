//! Elements that block the view for visibility checks, and the buffer they
//! are collected into each frame.
//!
//! A blocker is a sphere with a strength. The strength comes from the
//! element's alpha, scaled down by how far the camera is from it.

use crate::runtime::vec::vec3_distance;

/// How many records a buffer holds. Record 0 is never used.
const SLOT_CAP: usize = 0x100;

/// Radius is stored in sixteenths of a unit.
const RADIUS_SCALE: f32 = 16.0;

/// Strength is stored as the share of full strength that gets through, in
/// 65536ths.
const THROUGH_SCALE: f32 = 65536.0;

/// A distance fade is a byte.
const FADE_MAX: f32 = 255.0;

/// Added before a fade is cut to a whole number, so it rounds.
const FADE_ROUND: f32 = 0.5;

/// A byte as a share of full, the constant the game multiplies by in place of
/// dividing by 255.
const BYTE_TO_UNIT: f64 = f64::from_bits(0x3f70_1010_2000_0000);

/// The element flag that makes an element a view blocker.
const ELEM_FLAG_VIS_BLOCKER: i32 = 0x1000;

/// One blocker as stored.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SightOccluderEntry {
    pub centre: [f32; 3],
    /// The radius in sixteenths of a unit.
    pub third_knob_sixteenths: i16,
    /// The share of view that gets through, in 65536ths.
    pub inverse_fourth_knob_sixteenths: i16,
}

/// The blockers collected this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SightOccluderStore {
    /// The index of the last record in use.
    pub count: u32,
    pub recs: [SightOccluderEntry; SLOT_CAP],
}

impl Default for SightOccluderStore {
    fn default() -> Self {
        Self {
            count: 0,
            recs: [SightOccluderEntry::default(); SLOT_CAP],
        }
    }
}

/// Cuts toward zero to a whole number, then keeps the low 16 bits.
fn wrap_to_i16(x: f32) -> i16 {
    (x as i32) as i16
}

/// 1 up to `base` units away, falling straight to 0 over the next `span`
/// units.
fn fade_range(dist: f32, base: f32, span: f32) -> f32 {
    let past = dist - base;
    match past {
        p if p < 0.0 => 1.0,
        p if p < span => 1.0 - p / span,
        _ => 0.0,
    }
}

/// The distance fade at `dist` as a byte, from a fade range on each side:
/// one that fades with distance and one that fades in. `None` when neither
/// has a range.
pub fn evaluate_distance_fade(
    dist: f32,
    in_base: f32,
    fade_in_span: f32,
    out_base: f32,
    fade_out_span: f32,
) -> Option<u32> {
    let ease_on = (fade_in_span != 0.0).then(|| fade_range(dist, in_base, fade_in_span));
    let ease_off = (fade_out_span != 0.0).then(|| 1.0 - fade_range(dist, out_base, fade_out_span));
    // With only one side in play the other counts as full.
    let weakest = match (ease_on, ease_off) {
        (None, None) => return None,
        (Some(a), None) | (None, Some(a)) => a.min(1.0),
        (Some(a), Some(b)) => a.min(b),
    };
    Some((weakest * FADE_MAX + FADE_ROUND) as i32 as u32)
}

/// A blocker's strength from the element's alpha and its distance fade.
fn strength(tint_opacity: u8, fade_far: u32) -> f32 {
    let product = u32::from(tint_opacity).wrapping_mul(fade_far) >> 8;
    product as f32 * BYTE_TO_UNIT as f32
}

/// Adds a blocker; false when the buffer is full.
fn add(buf: &mut SightOccluderStore, spot: [f32; 3], reach: f32, strength: f32) -> bool {
    let slot = buf.count.wrapping_add(1);
    let Some(rec) = buf.recs.get_mut(slot as usize) else {
        return false;
    };
    *rec = SightOccluderEntry {
        centre: spot,
        third_knob_sixteenths: wrap_to_i16(reach * RADIUS_SCALE),
        inverse_fourth_knob_sixteenths: wrap_to_i16((1.0 - strength) * THROUGH_SCALE),
    };
    buf.count = slot;
    true
}

/// Adds an element to `buf` as a blocker, if its flags say it is one and its
/// distance from `eye` leaves it any strength. `start_extent` is its radius.
/// True if it was added.
#[allow(clippy::too_many_arguments)]
pub fn vis_blocker_add_prepared(
    buf: &mut SightOccluderStore,
    flags: i32,
    spot: [f32; 3],
    start_extent: f32,
    tint_opacity: u8,
    ease_on: [f32; 2],
    ease_off: [f32; 2],
    eye: [f32; 3],
) -> bool {
    let [in_base, in_amp] = ease_on;
    let [out_base, out_amp] = ease_off;
    let fade = (flags & ELEM_FLAG_VIS_BLOCKER != 0)
        .then(|| vec3_distance(eye, spot))
        .and_then(|dist| evaluate_distance_fade(dist, in_base, in_amp, out_base, out_amp));
    fade.is_some_and(|fade| add(buf, spot, start_extent, strength(tint_opacity, fade)))
}

/// Swaps the two buffers and empties the second.
pub fn vis_blocker_generate_verts(write: &mut SightOccluderStore, read: &mut SightOccluderStore) {
    core::mem::swap(write, read);
    read.count = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_is_full_inside_its_base_and_gone_past_its_range() {
        assert_eq!(fade_range(10.0, 50.0, 100.0), 1.0);
        assert_eq!(fade_range(100.0, 50.0, 100.0), 0.5);
        assert_eq!(fade_range(150.0, 50.0, 100.0), 0.0);
        assert_eq!(fade_range(500.0, 50.0, 100.0), 0.0);
    }

    #[test]
    fn with_no_fade_ranges_there_is_no_distance_fade() {
        assert_eq!(evaluate_distance_fade(50.0, 0.0, 0.0, 0.0, 0.0), None);
    }

    #[test]
    fn the_first_range_fades_the_element_with_distance() {
        assert_eq!(
            evaluate_distance_fade(50.0, 0.0, 100.0, 0.0, 0.0),
            Some(128)
        );
        assert_eq!(evaluate_distance_fade(0.0, 0.0, 100.0, 0.0, 0.0), Some(255));
        assert_eq!(evaluate_distance_fade(500.0, 0.0, 100.0, 0.0, 0.0), Some(0));
    }

    #[test]
    fn the_second_range_fades_the_element_in_with_distance() {
        assert_eq!(evaluate_distance_fade(50.0, 0.0, 0.0, 100.0, 50.0), Some(0));
        assert_eq!(
            evaluate_distance_fade(125.0, 0.0, 0.0, 100.0, 50.0),
            Some(128)
        );
        assert_eq!(
            evaluate_distance_fade(200.0, 0.0, 0.0, 100.0, 50.0),
            Some(255)
        );
    }

    #[test]
    fn the_weaker_of_the_two_fades_wins() {
        // Distance 125: the first range gives 1.0, the second 0.5.
        assert_eq!(
            evaluate_distance_fade(125.0, 500.0, 100.0, 100.0, 50.0),
            Some(128)
        );
    }

    #[test]
    fn a_fully_opaque_element_at_full_fade_blocks_almost_everything() {
        let s = strength(255, 255);
        assert!((s - 254.0 / 255.0).abs() < 1e-4, "{s}");
        assert_eq!(strength(0, 255), 0.0);
        assert_eq!(strength(255, 0), 0.0);
    }

    #[test]
    fn a_blocker_is_stored_in_the_next_record() {
        let mut buf = SightOccluderStore::default();
        assert!(add(&mut buf, [1.0, 2.0, 3.0], 2.5, 0.75));
        assert_eq!(buf.count, 1);
        assert_eq!(
            buf.recs[1],
            SightOccluderEntry {
                centre: [1.0, 2.0, 3.0],
                third_knob_sixteenths: 40,
                inverse_fourth_knob_sixteenths: 16384,
            }
        );
        assert_eq!(buf.recs[0], SightOccluderEntry::default());
    }

    #[test]
    fn stored_values_wrap_to_sixteen_bits() {
        let mut buf = SightOccluderStore::default();
        // Strength 0.5 leaves 32768 of 65536, which wraps in sixteen bits.
        add(&mut buf, [0.0; 3], 0.0, 0.5);
        assert_eq!(buf.recs[1].inverse_fourth_knob_sixteenths, i16::MIN);
        add(&mut buf, [0.0; 3], 0.0, 0.0);
        assert_eq!(buf.recs[2].inverse_fourth_knob_sixteenths, 0);
    }

    #[test]
    fn a_full_buffer_takes_no_more_blockers() {
        let mut buf = SightOccluderStore::default();
        let added = (0..300)
            .filter(|_| add(&mut buf, [0.0; 3], 1.0, 0.5))
            .count();
        assert_eq!(added, SLOT_CAP - 1);
        assert_eq!(buf.count as usize, SLOT_CAP - 1);
    }

    #[test]
    fn only_flagged_elements_with_a_fade_range_are_added() {
        let mut buf = SightOccluderStore::default();
        let try_add = |buf: &mut SightOccluderStore, flags, ease_on| {
            vis_blocker_add_prepared(
                buf,
                flags,
                [100.0, 0.0, 0.0],
                2.5,
                255,
                ease_on,
                [0.0, 0.0],
                [0.0; 3],
            )
        };
        assert!(!try_add(&mut buf, 0, [0.0, 1000.0]));
        assert!(!try_add(&mut buf, ELEM_FLAG_VIS_BLOCKER, [0.0, 0.0]));
        assert_eq!(buf.count, 0);
        assert!(try_add(&mut buf, ELEM_FLAG_VIS_BLOCKER, [0.0, 1000.0]));
        assert_eq!(buf.count, 1);
        assert_eq!(buf.recs[1].centre, [100.0, 0.0, 0.0]);
    }

    #[test]
    fn generating_verts_swaps_the_buffers_and_empties_the_second() {
        let mut a = SightOccluderStore::default();
        let mut b = SightOccluderStore::default();
        add(&mut a, [1.0; 3], 1.0, 0.5);
        add(&mut b, [2.0; 3], 1.0, 0.5);
        add(&mut b, [3.0; 3], 1.0, 0.5);
        vis_blocker_generate_verts(&mut a, &mut b);
        assert_eq!(a.count, 2);
        assert_eq!(a.recs[1].centre, [2.0; 3]);
        assert_eq!(b.count, 0);
        assert_eq!(b.recs[1].centre, [1.0; 3]);
    }
}
