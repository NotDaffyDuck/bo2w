//! BO2's trace result, `trace_t`.

use crate::hit::TraceHitType;

/// The surface type sits in bits 20..26 of a surface's flags. BO2 reads six
/// bits (MW2 read five).
pub const SURF_TYPE_SHIFT: u32 = 20;
pub const SURF_TYPE_MASK: u32 = 0x3f;

/// The surface type of a trace's `sflags`.
#[must_use]
pub const fn surface_kind_of_flags(sflags: u32) -> u8 {
    ((sflags >> SURF_TYPE_SHIFT) & SURF_TYPE_MASK) as u8
}

/// BO2's `trace_t`. It has no end point: callers lerp start to end by
/// `fraction` ([`TraceResult::finish_location`]). `static_model` is the hit static model's
/// index in the clip map (BO2 keeps a pointer).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TraceResult {
    pub normal: [f32; 3],
    pub fraction: f32,
    pub sflags: u32,
    pub cflags: u32,
    pub impact_category: TraceHitType,
    pub struck_entity_num: u16,
    pub mesh_slot_number: u16,
    pub hit_part_label: u16,
    pub bone_index: u16,
    pub hit_body_group: u16,
    pub fully_embedded: bool,
    pub starts_inside: bool,
    pub walkable: bool,
    pub static_model: Option<u32>,
    pub hit_partition: i32,
}

/// The name the rest of the workspace still uses; the rename sweep moves it to `TraceResult`.
pub type Trace = TraceResult;

impl TraceResult {
    /// A fresh result: nothing hit, the whole way.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fraction: 1.0,
            ..Self::default()
        }
    }

    /// Where the trace stopped (BO2's callers do this `Vec3Lerp`).
    #[must_use]
    pub fn finish_location(&self, from: [f32; 3], till: [f32; 3]) -> [f32; 3] {
        let share = self.fraction;
        core::array::from_fn(|axis| from[axis] + (till[axis] - from[axis]) * share)
    }

    /// The surface type of what the trace hit.
    #[must_use]
    pub const fn surface_type(&self) -> u8 {
        surface_kind_of_flags(self.sflags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_type_reads_six_bits() {
        assert_eq!(surface_kind_of_flags(0x1f0_0000), 31);
        assert_eq!(surface_kind_of_flags(0x200_0000), 32);
        assert_eq!(surface_kind_of_flags(0x3f0_0000 | 0xfffff), 63);
    }

    #[test]
    fn end_pos_lerps() {
        let t = TraceResult {
            fraction: 0.25,
            ..TraceResult::new()
        };
        assert_eq!(t.finish_location([0.0, 0.0, 0.0], [8.0, -4.0, 16.0]), [2.0, -1.0, 4.0]);
    }
}
