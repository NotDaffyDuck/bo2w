//! Fingerprints of what a product list was built from.
//!
//! A product is rebuilt only when its fingerprint (a "stamp") differs from the
//! stamp of the frame before. Each stamp is a plain struct of the inputs that
//! matter; `digest` folds it into the single number the renderer compares to
//! tell whether a list changed.

use super::super::list::{CONTENT_KEY_SEED, fold_content_bytes, blend_content_id};
use super::super::kept_lanes::{HeldDrawItem, mix_identity};

/// Chained hash over the shared content-id mixer.
pub(super) struct Hash64(u64);

impl Hash64 {
    pub(super) fn new() -> Self {
        Self(CONTENT_KEY_SEED)
    }

    pub(super) fn word(mut self, word: u64) -> Self {
        blend_content_id(&mut self.0, word);
        self
    }

    pub(super) fn count(self, count: usize) -> Self {
        self.word(count as u64)
    }

    pub(super) fn flag(self, flag: bool) -> Self {
        self.word(u64::from(flag))
    }

    /// An optional value; "none" and "zero" stay distinct.
    pub(super) fn maybe(self, value: Option<u64>) -> Self {
        self.word(value.map_or(0, |v| v.wrapping_add(1)))
    }

    pub(super) fn finish(self) -> u64 {
        self.0
    }
}

/// Identity of a list of tech ids (for example the per-light type bytes).
pub(super) fn bytes_id(bytes: &[u8]) -> u64 {
    let mut id = CONTENT_KEY_SEED;
    fold_content_bytes(&mut id, bytes);
    id
}

/// Identity of the rows of a shadow-caster list and where its near cascade ends.
pub(super) fn caster_membership(row_list: &[HeldDrawItem], near_n: usize) -> u64 {
    let mut id = Hash64::new().count(near_n).count(row_list.len()).finish();
    for row in row_list {
        mix_identity(&mut id, row);
    }
    id
}

/// Everything the camera lists depend on. The two payload fields change every
/// frame a model moves without changing which draws exist; everything else is
/// the composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CameraStamp {
    pub(super) static_membership: u64,
    pub(super) xmodel_membership: u64,
    pub(super) fx_membership: u64,
    pub(super) xmodel_payload: u64,
    pub(super) fx_payload: u64,
    pub(super) catalog: u64,
    pub(super) level_generation: Option<u64>,
    pub(super) colour_tech: u8,
    pub(super) emissive_tech: u8,
    pub(super) depth_haze: bool,
    pub(super) daylight_present: bool,
    pub(super) distortion_enabled: bool,
    pub(super) distortion_sort_key: Option<u32>,
    pub(super) light_types_id: u64,
    pub(super) spot_lights_id: u64,
}

impl CameraStamp {
    /// Same draws in the same order, whatever their payload.
    pub(super) fn same_composition(&self, rhs: &Self) -> bool {
        let strip = |s: &Self| Self {
            xmodel_payload: 0,
            fx_payload: 0,
            ..*s
        };
        strip(self) == strip(rhs)
    }

    pub(super) fn same_payload(&self, that: &Self) -> bool {
        (self.xmodel_payload, self.fx_payload) == (that.xmodel_payload, that.fx_payload)
    }

    /// Fingerprint of the composition. The payload is left out on purpose: it
    /// is not part of what the list is.
    pub(super) fn digest(&self) -> u64 {
        Hash64::new()
            .word(self.static_membership)
            .word(self.xmodel_membership)
            .word(self.fx_membership)
            .word(self.catalog)
            .maybe(self.level_generation)
            .word(u64::from(self.colour_tech))
            .word(u64::from(self.emissive_tech))
            .flag(self.depth_haze)
            .flag(self.daylight_present)
            .flag(self.distortion_enabled)
            .maybe(self.distortion_sort_key.map(u64::from))
            .word(self.light_types_id)
            .word(self.spot_lights_id)
            .finish()
    }
}

/// Everything the packed sun-shadow lists depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SunStamp {
    pub(super) membership: u64,
    pub(super) catalog: u64,
    pub(super) level_generation: Option<u64>,
    pub(super) level_vertices: u32,
    pub(super) world_range_n: usize,
    pub(super) smodel_range_n: usize,
    pub(super) xmodel_topology: u64,
    pub(super) xmodel_range_n: usize,
}

impl SunStamp {
    pub(super) fn digest(&self) -> u64 {
        Hash64::new()
            .word(self.membership)
            .word(self.catalog)
            .maybe(self.level_generation)
            .word(u64::from(self.level_vertices))
            .count(self.world_range_n)
            .count(self.smodel_range_n)
            .word(self.xmodel_topology)
            .count(self.xmodel_range_n)
            .finish()
    }
}

/// Everything the spot-shadow list depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SpotStamp {
    pub(super) membership: u64,
    pub(super) catalog: u64,
    pub(super) slot_n: usize,
}

impl SpotStamp {
    pub(super) fn digest(&self) -> u64 {
        Hash64::new()
            .word(self.membership)
            .word(self.catalog)
            .count(self.slot_n)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::super::lane_set::tests::row;
    use super::*;

    fn camera() -> CameraStamp {
        CameraStamp {
            static_membership: 1,
            xmodel_membership: 2,
            fx_membership: 3,
            xmodel_payload: 4,
            fx_payload: 5,
            catalog: 6,
            level_generation: Some(7),
            colour_tech: 8,
            emissive_tech: 9,
            depth_haze: false,
            daylight_present: true,
            distortion_enabled: true,
            distortion_sort_key: Some(10),
            light_types_id: 11,
            spot_lights_id: 12,
        }
    }

    #[test]
    fn the_payload_is_not_part_of_the_composition() {
        let a = camera();
        let b = CameraStamp {
            xmodel_payload: 40,
            fx_payload: 50,
            ..a
        };
        assert!(a.same_composition(&b));
        assert!(!a.same_payload(&b));
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn every_composition_field_moves_the_digest() {
        let base = camera();
        let variants = [
            CameraStamp { static_membership: 99, ..base },
            CameraStamp { xmodel_membership: 99, ..base },
            CameraStamp { fx_membership: 99, ..base },
            CameraStamp { catalog: 99, ..base },
            CameraStamp { level_generation: None, ..base },
            CameraStamp { level_generation: Some(8), ..base },
            CameraStamp { colour_tech: 99, ..base },
            CameraStamp { emissive_tech: 99, ..base },
            CameraStamp { depth_haze: true, ..base },
            CameraStamp { daylight_present: false, ..base },
            CameraStamp { distortion_enabled: false, ..base },
            CameraStamp { distortion_sort_key: None, ..base },
            CameraStamp { distortion_sort_key: Some(11), ..base },
            CameraStamp { light_types_id: 99, ..base },
            CameraStamp { spot_lights_id: 99, ..base },
        ];
        for (i, var) in variants.iter().enumerate() {
            assert!(!base.same_composition(var), "variant {i}");
            assert_ne!(base.digest(), var.digest(), "variant {i}");
        }
    }

    #[test]
    fn none_and_zero_are_different_optionals() {
        let some_zero = CameraStamp {
            level_generation: Some(0),
            ..camera()
        };
        let none = CameraStamp {
            level_generation: None,
            ..camera()
        };
        assert_ne!(some_zero.digest(), none.digest());
        let key_zero = CameraStamp {
            distortion_sort_key: Some(0),
            ..camera()
        };
        let key_none = CameraStamp {
            distortion_sort_key: None,
            ..camera()
        };
        assert_ne!(key_zero.digest(), key_none.digest());
    }

    #[test]
    fn sun_and_spot_digests_follow_every_field() {
        let sun = SunStamp {
            membership: 1,
            catalog: 2,
            level_generation: Some(3),
            level_vertices: 4,
            world_range_n: 5,
            smodel_range_n: 6,
            xmodel_topology: 7,
            xmodel_range_n: 8,
        };
        let variants = [
            SunStamp { membership: 0, ..sun },
            SunStamp { catalog: 0, ..sun },
            SunStamp { level_generation: None, ..sun },
            SunStamp { level_vertices: 0, ..sun },
            SunStamp { world_range_n: 0, ..sun },
            SunStamp { smodel_range_n: 0, ..sun },
            SunStamp { xmodel_topology: 0, ..sun },
            SunStamp { xmodel_range_n: 0, ..sun },
        ];
        for (i, var) in variants.iter().enumerate() {
            assert_ne!(sun.digest(), var.digest(), "sun variant {i}");
        }

        let spot = SpotStamp {
            membership: 1,
            catalog: 2,
            slot_n: 3,
        };
        for variant in [
            SpotStamp { membership: 0, ..spot },
            SpotStamp { catalog: 0, ..spot },
            SpotStamp { slot_n: 0, ..spot },
        ] {
            assert_ne!(spot.digest(), variant.digest());
        }
    }

    #[test]
    fn byte_lists_have_distinct_ids() {
        assert_ne!(bytes_id(&[1, 2]), bytes_id(&[2, 1]));
        assert_ne!(bytes_id(&[]), bytes_id(&[0]));
        assert_eq!(bytes_id(&[3, 4]), bytes_id(&[3, 4]));
    }

    #[test]
    fn caster_membership_sees_rows_order_and_the_split() {
        let rows = [row(1, 1), row(2, 2), row(3, 3)];
        let base = caster_membership(&rows, 1);
        assert_eq!(base, caster_membership(&rows, 1));
        assert_ne!(base, caster_membership(&rows, 2));
        assert_ne!(base, caster_membership(&rows[..2], 1));
        let swapped = [rows[1], rows[0], rows[2]];
        assert_ne!(base, caster_membership(&swapped, 1));
    }
}
