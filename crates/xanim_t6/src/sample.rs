use dobj_t6::{QUAT16_SCALE, quat16};

/// A group 1 or 3 rotation (yaw only): the stored (z, w) int16 pair, x and
/// y zero. Not normalised: the blend's finish step does that.
pub fn quat2(zw: [i16; 2]) -> [f32; 4] {
    [
        0.0,
        0.0,
        f32::from(zw[0]) * QUAT16_SCALE,
        f32::from(zw[1]) * QUAT16_SCALE,
    ]
}

/// A group 2 or 4 rotation: the stored x y z w int16. Not normalised.
pub fn quat4(q: [i16; 4]) -> [f32; 4] {
    quat16(q)
}

/// A translation key (groups 5 and 6, and the delta track): `mins + step *
/// key` per axis. The facts read it as `mins + size * key / 255` (u8) or
/// `/ 65535` (u16), with the divisors inferred and the sampler itself only
/// multiplying and adding; the captured step is used as stored, which is
/// the size BO2's anims already play at (see QUESTIONS.md).
pub fn unpack_trans<Q: Into<f32> + Copy>(mins: [f32; 3], step: [f32; 3], key: [Q; 3]) -> [f32; 3] {
    core::array::from_fn(|i| mins[i] + step[i] * key[i].into())
}

/// A frame position: `numframes * t`, `t` the 0..1 anim time. `t == 1` is
/// frame `numframes`.
pub fn frame_position(t: f32, numframes: u16) -> f32 {
    f32::from(numframes) * t
}

/// `XAnim_GetTimeIndex`: the key pair frame position `f` falls in and the
/// fraction between them. `keys` are the bone's key frame numbers, rising.
/// The game finds `k` with `keys[k] <= floor(f) < keys[k + 1]` (searched
/// flat under 64 keys, through a table of 256-key blocks from 64; both give
/// the same `k`). The fraction is `(f - keys[k]) / (keys[k + 1] - keys[k])`.
/// At or past the last key (the non-loop end), before the first, or with
/// fewer than two keys: one key alone, fraction 0.
pub fn time_index(keys: &[u16], f: f32) -> (usize, usize, f32) {
    let Some(last) = keys.len().checked_sub(1) else {
        return (0, 0, 0.0);
    };
    let frame = f.floor();
    let next = keys.partition_point(|&k| f32::from(k) <= frame);
    if next == 0 {
        return (0, 0, 0.0);
    }
    if next > last {
        return (last, last, 0.0);
    }
    let k = next - 1;
    let (a, b) = (f32::from(keys[k]), f32::from(keys[next]));
    (k, next, (f - a) / (b - a))
}

/// [`time_index`] for a bone keyed on every frame (`keys[i] == i` for
/// `count` keys).
pub fn time_index_dense(count: usize, f: f32) -> (usize, usize, f32) {
    let Some(last) = count.checked_sub(1) else {
        return (0, 0, 0.0);
    };
    if f <= 0.0 {
        return (0, 0, 0.0);
    }
    let frame = f.floor();
    if frame >= last as f32 {
        return (last, last, 0.0);
    }
    let k = frame as usize;
    (k, k + 1, f - frame)
}

/// The frame a "jumpcut" notetrack at 0..1 time `note_time` cuts after:
/// `trunc(note_time * numframes - 1)`.
pub fn jumpcut_frame(note_time: f32, numframes: u16) -> i32 {
    (note_time * f32::from(numframes) - 1.0) as i32
}

/// The jump-cut rule: when a jumpcut is on frame `floor(f)`, `f` snaps to
/// that frame so the pair across the cut does not interpolate.
pub fn snap_jumpcut(f: f32, jumpcut_frames: &[i32]) -> f32 {
    let frame = f.floor();
    if jumpcut_frames.contains(&(frame as i32)) {
        frame
    } else {
        f
    }
}

/// The sample between two keys: a per-component lerp (no flip, no
/// normalise).
pub fn lerp4(q_from: [f32; 4], q_to: [f32; 4], blend_k: f32) -> [f32; 4] {
    core::array::from_fn(|i| q_from[i] + (q_to[i] - q_from[i]) * blend_k)
}

pub fn blend_triple(p0: [f32; 3], p1: [f32; 3], mix: f32) -> [f32; 3] {
    core::array::from_fn(|i| p0[i] + (p1[i] - p0[i]) * mix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_are_scaled_not_normalised() {
        assert_eq!(quat2([0, 32767]), [0.0, 0.0, 0.0, 32767.0 * QUAT16_SCALE]);
        let q = quat4([16384, 0, 0, 16384]);
        let len = (q[0] * q[0] + q[3] * q[3]).sqrt();
        assert!((len - 0.7071).abs() < 1e-3);
        assert_eq!(unpack_trans([1.0, 2.0, 3.0], [0.5; 3], [2u8, 4, 6]), [2.0, 4.0, 6.0]);
    }

    #[test]
    fn key_search_finds_the_pair_around_the_frame() {
        let keys = [0, 4, 10];
        assert_eq!(time_index(&keys, 0.0), (0, 1, 0.0));
        assert_eq!(time_index(&keys, 2.0), (0, 1, 0.5));
        // On a key: that key and the next, fraction 0.
        assert_eq!(time_index(&keys, 4.0), (1, 2, 0.0));
        assert_eq!(time_index(&keys, 7.0), (1, 2, 0.5));
        // The end and past it: the last key alone.
        assert_eq!(time_index(&keys, 10.0), (2, 2, 0.0));
        assert_eq!(time_index(&keys, 11.0), (2, 2, 0.0));
        assert_eq!(time_index(&[3], 5.0), (0, 0, 0.0));
        assert_eq!(time_index(&[], 5.0), (0, 0, 0.0));
        assert_eq!(time_index_dense(11, 2.5), (2, 3, 0.5));
        assert_eq!(time_index_dense(11, 10.0), (10, 10, 0.0));
        assert_eq!(time_index_dense(11, -1.0), (0, 0, 0.0));
    }

    #[test]
    fn large_key_lists_search_the_same() {
        let keys: Vec<u16> = (0..300).map(|i| i * 2).collect();
        assert_eq!(time_index(&keys, 513.0), (256, 257, 0.5));
        assert_eq!(time_index(&keys, frame_position(1.0, 598)), (299, 299, 0.0));
    }

    #[test]
    fn a_jumpcut_snaps_its_frame() {
        // A cut at frame 5 of 10: frame 4 does not blend into frame 5.
        let cut = jumpcut_frame(0.5, 10);
        assert_eq!(cut, 4);
        assert_eq!(snap_jumpcut(4.6, &[cut]), 4.0);
        assert_eq!(snap_jumpcut(5.6, &[cut]), 5.6);
        assert_eq!(jumpcut_frame(0.0, 10), -1);
    }
}
