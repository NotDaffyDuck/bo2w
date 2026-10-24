//! Black Ops II animations entering the set. A capture holds them whole, in
//! the same part types and packing the decoder already reads.

use std::sync::Arc;

use crate::AssetNamespace;
use crate::anim_set::{CapturedXAnim, XAnimBuild};
use crate::xanim_clip::{ClipNotify, RawDeltaTrans, RawXAnimParts};

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
        if anim.names.len() != usize::from(anim.bone_count[9]) {
            self.capture_gaps += 1;
            return false;
        }
        self.insert_in(
            AssetNamespace::T6,
            CapturedXAnim {
                namespace: AssetNamespace::T6,
                root_motion: if anim.delta {
                    t6_root_motion(&anim.delta_trans).map(Arc::new)
                } else {
                    None
                },
                parts: Arc::new(RawXAnimParts {
                    // T6 names are case-insensitive: keyed lower case.
                    name: name.to_ascii_lowercase(),
                    data_byte: anim.data_byte.clone(),
                    data_short: anim.data_short.clone(),
                    data_int: anim.data_int.clone(),
                    random_data_byte: anim.random_data_byte.clone(),
                    random_data_short: anim.random_data_short.clone(),
                    random_data_int: anim.random_data_int.clone(),
                    numframes: anim.numframes,
                    flags: u8::from(anim.looping) | (u8::from(anim.delta) << 1),
                    bone_count: anim.bone_count,
                    framerate: anim.framerate,
                    names: anim.names.clone(),
                    notifies: anim
                        .notifies
                        .iter()
                        .map(|(name, time)| ClipNotify {
                            name: name.clone(),
                            time: *time,
                        })
                        .collect(),
                    indices: anim.indices.clone(),
                    delta_trans: None,
                }),
            },
        );
        true
    }
}

/// Black Ops II root keys (frame, position) packed the way the clip decoder
/// reads them: full 16-bit steps over the keys' own range.
fn t6_root_motion(keys: &[(u16, [f32; 3])]) -> Option<RawDeltaTrans> {
    let (&(_, first), rest) = keys.split_first()?;
    if rest.is_empty() {
        return Some(RawDeltaTrans {
            size: 0,
            small: false,
            mins: first,
            step: [0.0; 3],
            indices: Vec::new(),
            packed: Vec::new(),
        });
    }
    let mut mins = first;
    let mut maxs = first;
    for (_, v) in rest {
        for a in 0..3 {
            mins[a] = mins[a].min(v[a]);
            maxs[a] = maxs[a].max(v[a]);
        }
    }
    let step: [f32; 3] = core::array::from_fn(|a| (maxs[a] - mins[a]) / f32::from(u16::MAX));
    let mut packed = Vec::with_capacity(keys.len() * 6);
    for (_, v) in keys {
        for a in 0..3 {
            let q = if step[a] > 0.0 {
                ((v[a] - mins[a]) / step[a]).round() as u16
            } else {
                0
            };
            packed.extend_from_slice(&q.to_le_bytes());
        }
    }
    Some(RawDeltaTrans {
        size: u16::try_from(keys.len() - 1).ok()?,
        small: false,
        mins,
        step,
        indices: keys.iter().map(|&(frame, _)| frame).collect(),
        packed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bo2_root_motion_is_kept_aside_for_who_asks() {
        let keys = [(0, [0.0, 0.0, 0.0]), (13, [8.0, 0.0, 30.0]), (26, [16.0, 0.0, 57.0])];
        let mut build = XAnimBuild::default();
        build.insert_in(
            AssetNamespace::T6,
            CapturedXAnim {
                namespace: AssetNamespace::T6,
                parts: Arc::new(RawXAnimParts {
                    name: "mp_mantle_up_57".to_owned(),
                    numframes: 26,
                    framerate: 30.0,
                    flags: 2,
                    ..RawXAnimParts::default()
                }),
                root_motion: t6_root_motion(&keys).map(Arc::new),
            },
        );
        let catalog = build.publish();
        let plain = catalog.clip(AssetNamespace::T6, "mp_mantle_up_57").unwrap();
        assert!(!plain.has_delta());
        let clip = catalog
            .clip_with_root_motion(AssetNamespace::T6, "mp_mantle_up_57")
            .unwrap();
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 0.01);
        assert!(close(clip.abs_delta_trans(0.5), [8.0, 0.0, 30.0]));
        assert!(close(clip.abs_delta_trans(1.0), [16.0, 0.0, 57.0]));
    }

    #[test]
    #[ignore = "reads Black Ops II's common_zm.ff named by T6_COMMON_ZM"]
    fn bo2_climbs_carry_their_root_motion() {
        let path = std::env::var("T6_COMMON_ZM").expect("T6_COMMON_ZM");
        let capture = asset_t6::capture_zone(std::path::Path::new(&path)).expect("zone");
        let mut build = XAnimBuild::default();
        for anim in &capture.xanims {
            build.insert_t6(anim);
        }
        let catalog = build.publish();
        for name in [
            "mp_mantle_up_57",
            "mp_mantle_up_51",
            "mp_mantle_up_45",
            "mp_mantle_up_39",
            "mp_mantle_up_33",
            "mp_mantle_up_27",
            "mp_mantle_up_21",
            "mp_mantle_over_high",
            "mp_mantle_over_mid",
            "player_mantle_over_low",
        ] {
            let clip = catalog
                .clip_with_root_motion(AssetNamespace::T6, name)
                .expect(name);
            let end = clip.abs_delta_trans(1.0);
            println!("{name}: {} ms, ends {end:?}", clip.length_msec());
            // How far along the climb the body is at each tenth of its time.
            let curve: Vec<String> = (0..=10)
                .map(|i| {
                    let d = clip.abs_delta_trans(i as f32 / 10.0);
                    format!("{:.0}/{:.0}", d[0], d[2])
                })
                .collect();
            println!("  x/z by tenths: {}", curve.join(" "));
            assert!(clip.has_delta(), "{name} has no root motion");
            if let Some(height) = name.strip_prefix("mp_mantle_up_") {
                let height: f32 = height.parse().unwrap();
                assert!((end[2] - height).abs() < 1.0, "{name} ends {end:?}");
            } else {
                assert!(end[0] > 20.0, "{name} ends {end:?}");
            }
        }
    }
}
