//! Black Ops II animations entering the set. A capture holds them whole, in
//! the same part types and packing the decoder already reads.

use std::sync::Arc;

use crate::AssetCategoryScope;
use crate::anim_set::{XAnimCapture, XAnimBuild};
use crate::motion_clip::{ClipEventMark, StoredRootMotion, StoredClipParts};

impl XAnimBuild {
    /// bo2zm: a Black Ops II animation, captured whole by `asset_t6`. Its
    /// data streams use the same part types and packing as legacy and Black
    /// Ops, so they go to the decoder as they are.
    pub fn insert_t6(&mut self, anim: &asset_t6::XAnimRef) -> bool {
        let name = anim.name.trim_start_matches(',');
        if name.is_empty() || name.len() != anim.name.len() {
            // A comma-named asset is a reference to another zone's.
            return false;
        }
        if anim.names.len() != usize::from(anim.joint_total[9]) {
            self.missed_frames += 1;
            return false;
        }
        let root_motion = anim
            .delta
            .then(|| t6_root_motion(&anim.delta_trans))
            .flatten();
        let captured = XAnimCapture {
            namespace: AssetCategoryScope::T6,
            root_motion: root_motion.map(Arc::new),
            pieces: Arc::new(t6_parts(anim, name)),
        };
        self.put_into_namespace(AssetCategoryScope::T6, captured);
        true
    }
}

/// The decoder's view of a captured animation, its streams copied over.
fn t6_parts(anim: &asset_t6::XAnimRef, name: &str) -> StoredClipParts {
    let notifies = anim
        .notifies
        .iter()
        .map(|(label, time)| ClipEventMark { name: label.clone(), time: *time })
        .collect();
    let flags = u8::from(anim.looping) | (u8::from(anim.delta) << 1);
    StoredClipParts {
        // T6 names are case-insensitive: keyed lower case.
        name: name.to_ascii_lowercase(),
        payload_u8: anim.payload_u8.clone(),
        payload_i16: anim.payload_i16.clone(),
        payload_i32: anim.payload_i32.clone(),
        noise_octet: anim.noise_octet.clone(),
        noise_word16: anim.noise_word16.clone(),
        noise_word32: anim.noise_word32.clone(),
        frame_total: anim.frame_total,
        flags,
        joint_total: anim.joint_total,
        fps: anim.frame_rate_hz,
        names: anim.names.clone(),
        notifies,
        indices: anim.indices.clone(),
        ..StoredClipParts::default()
    }
}

/// Black Ops II root keys (frame, position) packed the way the clip decoder
/// reads them: full 16-bit steps over the keys' own range.
fn t6_root_motion(keys: &[(u16, [f32; 3])]) -> Option<StoredRootMotion> {
    let (&(_, first), rest) = keys.split_first()?;
    if rest.is_empty() {
        return Some(StoredRootMotion { base: first, ..StoredRootMotion::default() });
    }
    let bounds = |axis: usize, pick: fn(f32, f32) -> f32, from: f32| {
        keys.iter().map(|(_, v)| v[axis]).fold(from, pick)
    };
    let mins: [f32; 3] = core::array::from_fn(|axis| bounds(axis, f32::min, first[axis]));
    let span: [f32; 3] = core::array::from_fn(|axis| bounds(axis, f32::max, first[axis]) - mins[axis]);
    let step = span.map(|extent| extent / f32::from(u16::MAX));
    let quantize = |axis: usize, v: &[f32; 3]| match step[axis] > 0.0 {
        true => ((v[axis] - mins[axis]) / step[axis]).round() as u16,
        false => 0,
    };
    let packed = keys
        .iter()
        .flat_map(|(_, v)| (0..3).flat_map(move |axis| quantize(axis, v).to_le_bytes()))
        .collect();
    Some(StoredRootMotion {
        size: u16::try_from(keys.len() - 1).ok()?,
        base: mins,
        scale: step,
        indices: keys.iter().map(|&(frame, _)| frame).collect(),
        packed,
        ..StoredRootMotion::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bo2_root_motion_is_kept_aside_for_who_asks() {
        let keys = [(0, [0.0, 0.0, 0.0]), (13, [8.0, 0.0, 30.0]), (26, [16.0, 0.0, 57.0])];
        let parts = StoredClipParts {
            name: "mp_mantle_up_57".to_owned(),
            frame_total: 26,
            fps: 30.0,
            flags: 2,
            ..StoredClipParts::default()
        };
        let captured = XAnimCapture {
            namespace: AssetCategoryScope::T6,
            pieces: Arc::new(parts),
            root_motion: t6_root_motion(&keys).map(Arc::new),
        };
        let mut build = XAnimBuild::default();
        build.put_into_namespace(AssetCategoryScope::T6, captured);
        let catalog = build.publish();
        let plain = catalog.clip(AssetCategoryScope::T6, "mp_mantle_up_57").unwrap();
        assert!(!plain.moves_root());
        let clip = catalog
            .clip_with_root_motion(AssetCategoryScope::T6, "mp_mantle_up_57")
            .unwrap();
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 0.01);
        assert!(close(clip.absolute_root_shift(0.5), [8.0, 0.0, 30.0]));
        assert!(close(clip.absolute_root_shift(1.0), [16.0, 0.0, 57.0]));
    }

    #[test]
    #[ignore = "reads Black Ops II's common_zm.ff named by T6_COMMON_ZM"]
    fn bo2_climbs_carry_their_root_motion() {
        let path = std::env::var("T6_COMMON_ZM").expect("T6_COMMON_ZM");
        let capture = asset_t6::capture_zone(std::path::Path::new(&path)).expect("zone");
        let mut build = XAnimBuild::default();
        for anim in &capture.animation_refs {
            build.insert_t6(anim);
        }
        let catalog = build.publish();
        let climbs = [57, 51, 45, 39, 33, 27, 21].map(|height| format!("mp_mantle_up_{height}"));
        let overs = ["mp_mantle_over_high", "mp_mantle_over_mid", "player_mantle_over_low"];
        for name in climbs.iter().map(String::as_str).chain(overs) {
            let clip = catalog
                .clip_with_root_motion(AssetCategoryScope::T6, name)
                .expect(name);
            let end = clip.absolute_root_shift(1.0);
            println!("{name}: {} ms, ends {end:?}", clip.span_in_ms());
            // How far along the climb the body is at each tenth of its time.
            let curve: Vec<String> = (0..=10)
                .map(|i| {
                    let d = clip.absolute_root_shift(i as f32 / 10.0);
                    format!("{:.0}/{:.0}", d[0], d[2])
                })
                .collect();
            println!("  x/z by tenths: {}", curve.join(" "));
            assert!(clip.moves_root(), "{name} has no root motion");
            if let Some(height) = name.strip_prefix("mp_mantle_up_") {
                let height: f32 = height.parse().unwrap();
                assert!((end[2] - height).abs() < 1.0, "{name} ends {end:?}");
            } else {
                assert!(end[0] > 20.0, "{name} ends {end:?}");
            }
        }
    }
}
