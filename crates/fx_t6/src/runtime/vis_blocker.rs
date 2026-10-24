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
pub struct FxVisBlockerRec {
    pub origin: [f32; 3],
    /// The radius in sixteenths of a unit.
    pub param3_x16: i16,
    /// The share of view that gets through, in 65536ths.
    pub one_minus_param4_x16: i16,
}

/// The blockers collected this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxVisBlockerBuf {
    /// The index of the last record in use.
    pub count: u32,
    pub recs: [FxVisBlockerRec; SLOT_CAP],
}

impl Default for FxVisBlockerBuf {
    fn default() -> Self {
        Self {
            count: 0,
            recs: [FxVisBlockerRec::default(); SLOT_CAP],
        }
    }
}

/// Cuts toward zero to a whole number, then keeps the low 16 bits.
fn wrap_to_i16(x: f32) -> i16 {
    (x as i32) as i16
}

/// 1 up to `base` units away, falling straight to 0 over the next `amp`
/// units.
fn fade_range(dist: f32, base: f32, amp: f32) -> f32 {
    let past = dist - base;
    if past < 0.0 {
        1.0
    } else if past < amp {
        1.0 - past / amp
    } else {
        0.0
    }
}

/// The distance fade at `dist` as a byte, from a fade range on each side:
/// one that fades with distance and one that fades in. `None` when neither
/// has a range.
pub fn evaluate_distance_fade(
    dist: f32,
    fade_in_base: f32,
    fade_in_amp: f32,
    fade_out_base: f32,
    fade_out_amp: f32,
) -> Option<u32> {
    if fade_in_amp == 0.0 && fade_out_amp == 0.0 {
        return None;
    }
    let mut fade_in = 1.0;
    let mut fade_out = 1.0;
    if fade_in_amp != 0.0 {
        fade_in = fade_range(dist, fade_in_base, fade_in_amp);
    }
    if fade_out_amp != 0.0 {
        fade_out = 1.0 - fade_range(dist, fade_out_base, fade_out_amp);
    }
    let fade = if fade_in < fade_out {
        fade_in
    } else {
        fade_out
    };
    Some((fade * FADE_MAX + FADE_ROUND) as i32 as u32)
}

/// A blocker's strength from the element's alpha and its distance fade.
fn strength(color_alpha: u8, distance_fade: u32) -> f32 {
    let product = u32::from(color_alpha).wrapping_mul(distance_fade) >> 8;
    product as f32 * BYTE_TO_UNIT as f32
}

/// Adds a blocker; false when the buffer is full.
fn add(buf: &mut FxVisBlockerBuf, origin: [f32; 3], radius: f32, strength: f32) -> bool {
    let next = buf.count.wrapping_add(1);
    if next as usize >= SLOT_CAP {
        return false;
    }
    buf.recs[next as usize] = FxVisBlockerRec {
        origin,
        param3_x16: wrap_to_i16(radius * RADIUS_SCALE),
        one_minus_param4_x16: wrap_to_i16((1.0 - strength) * THROUGH_SCALE),
    };
    buf.count = next;
    true
}

/// Adds an element to `buf` as a blocker, if its flags say it is one and its
/// distance from `camera` leaves it any strength. `size0` is its radius.
/// True if it was added.
#[allow(clippy::too_many_arguments)]
pub fn vis_blocker_add_prepared(
    buf: &mut FxVisBlockerBuf,
    flags: i32,
    origin: [f32; 3],
    size0: f32,
    color_alpha: u8,
    fade_in: [f32; 2],
    fade_out: [f32; 2],
    camera: [f32; 3],
) -> bool {
    if flags & ELEM_FLAG_VIS_BLOCKER == 0 {
        return false;
    }
    let Some(fade) = evaluate_distance_fade(
        vec3_distance(camera, origin),
        fade_in[0],
        fade_in[1],
        fade_out[0],
        fade_out[1],
    ) else {
        return false;
    };
    add(buf, origin, size0, strength(color_alpha, fade))
}

/// Swaps the two buffers and empties the second.
pub fn vis_blocker_generate_verts(write: &mut FxVisBlockerBuf, read: &mut FxVisBlockerBuf) {
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
        let mut buf = FxVisBlockerBuf::default();
        assert!(add(&mut buf, [1.0, 2.0, 3.0], 2.5, 0.75));
        assert_eq!(buf.count, 1);
        assert_eq!(
            buf.recs[1],
            FxVisBlockerRec {
                origin: [1.0, 2.0, 3.0],
                param3_x16: 40,
                one_minus_param4_x16: 16384,
            }
        );
        assert_eq!(buf.recs[0], FxVisBlockerRec::default());
    }

    #[test]
    fn stored_values_wrap_to_sixteen_bits() {
        let mut buf = FxVisBlockerBuf::default();
        // Strength 0.5 leaves 32768 of 65536, which wraps in sixteen bits.
        add(&mut buf, [0.0; 3], 0.0, 0.5);
        assert_eq!(buf.recs[1].one_minus_param4_x16, i16::MIN);
        add(&mut buf, [0.0; 3], 0.0, 0.0);
        assert_eq!(buf.recs[2].one_minus_param4_x16, 0);
    }

    #[test]
    fn a_full_buffer_takes_no_more_blockers() {
        let mut buf = FxVisBlockerBuf::default();
        let added = (0..300)
            .filter(|_| add(&mut buf, [0.0; 3], 1.0, 0.5))
            .count();
        assert_eq!(added, SLOT_CAP - 1);
        assert_eq!(buf.count as usize, SLOT_CAP - 1);
    }

    #[test]
    fn only_flagged_elements_with_a_fade_range_are_added() {
        let mut buf = FxVisBlockerBuf::default();
        let try_add = |buf: &mut FxVisBlockerBuf, flags, fade_in| {
            vis_blocker_add_prepared(
                buf,
                flags,
                [100.0, 0.0, 0.0],
                2.5,
                255,
                fade_in,
                [0.0, 0.0],
                [0.0; 3],
            )
        };
        assert!(!try_add(&mut buf, 0, [0.0, 1000.0]));
        assert!(!try_add(&mut buf, ELEM_FLAG_VIS_BLOCKER, [0.0, 0.0]));
        assert_eq!(buf.count, 0);
        assert!(try_add(&mut buf, ELEM_FLAG_VIS_BLOCKER, [0.0, 1000.0]));
        assert_eq!(buf.count, 1);
        assert_eq!(buf.recs[1].origin, [100.0, 0.0, 0.0]);
    }

    #[test]
    fn generating_verts_swaps_the_buffers_and_empties_the_second() {
        let mut a = FxVisBlockerBuf::default();
        let mut b = FxVisBlockerBuf::default();
        add(&mut a, [1.0; 3], 1.0, 0.5);
        add(&mut b, [2.0; 3], 1.0, 0.5);
        add(&mut b, [3.0; 3], 1.0, 0.5);
        vis_blocker_generate_verts(&mut a, &mut b);
        assert_eq!(a.count, 2);
        assert_eq!(a.recs[1].origin, [2.0; 3]);
        assert_eq!(b.count, 0);
        assert_eq!(b.recs[1].origin, [1.0; 3]);
    }
}
