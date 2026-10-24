//! Reads a BO2 FxElemDef (292 bytes) and its FxElemAtlas from the zone's
//! bytes, at fastfile_t6's offsets.

use fastfile_t6::layout::{FxElemAtlas as a, FxElemDef as e};

use crate::elem_type::FxElemType;

pub const FX_ELEM_DEF_SIZE: usize = e::SIZE;

pub const FX_ELEM_ATLAS_SIZE: usize = a::SIZE;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxFloatRange {
    pub base: f32,
    pub amplitude: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxIntRange {
    pub base: i32,
    pub amplitude: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxElemAtlasT6 {
    pub behavior: u8,
    pub index: u8,
    pub fps: u8,
    pub loop_count: u8,
    pub col_index_bits: u8,
    pub row_index_bits: u8,
    pub entry_count_and_index_range: u16,
}

impl FxElemAtlasT6 {
    /// The entry count is the low byte (measured: 516 = 4 entries on a 2x2
    /// atlas).
    #[inline]
    pub fn entry_count(&self) -> u8 {
        self.entry_count_and_index_range as u8
    }
}

/// A view of one FxElemDef's bytes.
#[derive(Clone, Copy, Debug)]
pub struct FxElemDefT6<'a> {
    raw: &'a [u8],
}

impl<'a> FxElemDefT6<'a> {
    /// None when fewer than 292 bytes are given.
    pub fn new(raw: &'a [u8]) -> Option<Self> {
        raw.get(..e::SIZE).map(|raw| Self { raw })
    }

    #[inline]
    fn u8(&self, off: usize) -> u8 {
        self.raw[off]
    }

    #[inline]
    fn u16(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.raw[off], self.raw[off + 1]])
    }

    #[inline]
    fn u32(&self, off: usize) -> u32 {
        u32::from_le_bytes([
            self.raw[off],
            self.raw[off + 1],
            self.raw[off + 2],
            self.raw[off + 3],
        ])
    }

    #[inline]
    fn i32(&self, off: usize) -> i32 {
        self.u32(off) as i32
    }

    #[inline]
    fn f32(&self, off: usize) -> f32 {
        f32::from_bits(self.u32(off))
    }

    #[inline]
    fn float_range(&self, off: usize) -> FxFloatRange {
        FxFloatRange {
            base: self.f32(off),
            amplitude: self.f32(off + 4),
        }
    }

    #[inline]
    fn int_range(&self, off: usize) -> FxIntRange {
        FxIntRange {
            base: self.i32(off),
            amplitude: self.i32(off + 4),
        }
    }

    #[inline]
    fn vec3(&self, off: usize) -> [f32; 3] {
        [self.f32(off), self.f32(off + 4), self.f32(off + 8)]
    }

    #[inline]
    fn range3(&self, off: usize) -> [FxFloatRange; 3] {
        [
            self.float_range(off),
            self.float_range(off + 8),
            self.float_range(off + 16),
        ]
    }

    pub fn flags(&self) -> u32 {
        self.u32(e::flags)
    }

    /// The spawn union read as looping: (interval msec, count).
    pub fn spawn_looping(&self) -> (i32, i32) {
        (self.i32(e::spawn), self.i32(e::spawn + 4))
    }

    /// The spawn union read as one-shot: the count range.
    pub fn spawn_one_shot(&self) -> FxIntRange {
        self.int_range(e::spawn)
    }

    pub fn spawn_range(&self) -> FxFloatRange {
        self.float_range(e::spawnRange)
    }

    pub fn fade_in_range(&self) -> FxFloatRange {
        self.float_range(e::fadeInRange)
    }

    pub fn fade_out_range(&self) -> FxFloatRange {
        self.float_range(e::fadeOutRange)
    }

    pub fn spawn_frustum_cull_radius(&self) -> f32 {
        self.f32(e::spawnFrustumCullRadius)
    }

    pub fn spawn_delay_msec(&self) -> FxIntRange {
        self.int_range(e::spawnDelayMsec)
    }

    pub fn life_span_msec(&self) -> FxIntRange {
        self.int_range(e::lifeSpanMsec)
    }

    pub fn spawn_origin(&self) -> [FxFloatRange; 3] {
        self.range3(e::spawnOrigin)
    }

    pub fn spawn_offset_radius(&self) -> FxFloatRange {
        self.float_range(e::spawnOffsetRadius)
    }

    pub fn spawn_offset_height(&self) -> FxFloatRange {
        self.float_range(e::spawnOffsetHeight)
    }

    pub fn spawn_angles(&self) -> [FxFloatRange; 3] {
        self.range3(e::spawnAngles)
    }

    pub fn angular_velocity(&self) -> [FxFloatRange; 3] {
        self.range3(e::angularVelocity)
    }

    pub fn initial_rotation(&self) -> FxFloatRange {
        self.float_range(e::initialRotation)
    }

    /// The packed rotation axis, decoded at draw with flag 0x800000.
    // BO2 unknown: the packing (see facts/QUESTIONS.md); the raw word is given.
    pub fn rotation_axis_packed(&self) -> u32 {
        self.u32(e::rotationAxis)
    }

    pub fn gravity(&self) -> FxFloatRange {
        self.float_range(e::gravity)
    }

    pub fn reflection_factor(&self) -> FxFloatRange {
        self.float_range(e::reflectionFactor)
    }

    pub fn atlas(&self) -> FxElemAtlasT6 {
        let o = e::atlas;
        FxElemAtlasT6 {
            behavior: self.u8(o + a::behavior),
            index: self.u8(o + a::index),
            fps: self.u8(o + a::fps),
            loop_count: self.u8(o + a::loopCount),
            col_index_bits: self.u8(o + a::colIndexBits),
            row_index_bits: self.u8(o + a::rowIndexBits),
            entry_count_and_index_range: self.u16(o + a::entryCountAndIndexRange),
        }
    }

    pub fn wind_influence(&self) -> f32 {
        self.f32(e::windInfluence)
    }

    pub fn elem_type_raw(&self) -> u8 {
        self.u8(e::elemType)
    }

    pub fn elem_type(&self) -> Option<FxElemType> {
        FxElemType::from_u8(self.elem_type_raw())
    }

    pub fn visual_count(&self) -> u8 {
        self.u8(e::visualCount)
    }

    pub fn vel_interval_count(&self) -> u8 {
        self.u8(e::velIntervalCount)
    }

    pub fn vis_state_interval_count(&self) -> u8 {
        self.u8(e::visStateIntervalCount)
    }

    pub fn coll_mins(&self) -> [f32; 3] {
        self.vec3(e::collMins)
    }

    pub fn coll_maxs(&self) -> [f32; 3] {
        self.vec3(e::collMaxs)
    }

    pub fn emit_dist(&self) -> FxFloatRange {
        self.float_range(e::emitDist)
    }

    pub fn emit_dist_variance(&self) -> FxFloatRange {
        self.float_range(e::emitDistVariance)
    }

    pub fn sort_order(&self) -> u8 {
        self.u8(e::sortOrder)
    }

    pub fn lighting_frac(&self) -> u8 {
        self.u8(e::lightingFrac)
    }

    // The four wind and fade fields are 2 bytes each in the layout; read
    // unsigned.
    pub fn alpha_fade_time_msec(&self) -> u16 {
        self.u16(e::alphaFadeTimeMsec)
    }

    pub fn max_wind_strength(&self) -> u16 {
        self.u16(e::maxWindStrength)
    }

    pub fn spawn_interval_at_max_wind(&self) -> u16 {
        self.u16(e::spawnIntervalAtMaxWind)
    }

    pub fn lifespan_at_max_wind(&self) -> u16 {
        self.u16(e::lifespanAtMaxWind)
    }

    pub fn billboard_pivot(&self) -> [f32; 2] {
        [self.f32(e::billboardPivot), self.f32(e::billboardPivot + 4)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(raw: &mut [u8], off: usize, bytes: &[u8]) {
        raw[off..off + bytes.len()].copy_from_slice(bytes);
    }

    #[test]
    fn needs_a_whole_def() {
        assert!(FxElemDefT6::new(&[0u8; 291]).is_none());
        assert!(FxElemDefT6::new(&[0u8; 300]).is_some());
    }

    #[test]
    fn reads_fields_at_bo2_offsets() {
        let mut raw = [0u8; FX_ELEM_DEF_SIZE];
        put(&mut raw, e::flags, &0x8000_0400u32.to_le_bytes());
        put(&mut raw, e::lifeSpanMsec, &500i32.to_le_bytes());
        put(&mut raw, e::lifeSpanMsec + 4, &250i32.to_le_bytes());
        put(&mut raw, e::gravity + 4, &1.5f32.to_le_bytes());
        put(&mut raw, e::spawnAngles + 16, &(-2.0f32).to_le_bytes());
        put(
            &mut raw,
            e::atlas + a::entryCountAndIndexRange,
            &516u16.to_le_bytes(),
        );
        raw[e::elemType] = 4;
        raw[e::sortOrder] = 9;
        put(&mut raw, e::lifespanAtMaxWind, &1200u16.to_le_bytes());
        put(&mut raw, e::billboardPivot + 4, &0.25f32.to_le_bytes());
        let d = FxElemDefT6::new(&raw).unwrap();
        assert_eq!(d.flags(), 0x8000_0400);
        assert_eq!(
            d.life_span_msec(),
            FxIntRange {
                base: 500,
                amplitude: 250
            }
        );
        assert_eq!(d.gravity().amplitude, 1.5);
        assert_eq!(d.spawn_angles()[2].base, -2.0);
        assert_eq!(d.atlas().entry_count(), 4);
        assert_eq!(d.elem_type(), Some(FxElemType::Line));
        assert_eq!(d.sort_order(), 9);
        assert_eq!(d.lifespan_at_max_wind(), 1200);
        assert_eq!(d.billboard_pivot(), [0.0, 0.25]);
    }
}
