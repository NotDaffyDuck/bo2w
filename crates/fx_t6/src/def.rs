//! Reads a BO2 FxElemDef (292 bytes) and its FxElemAtlas from the zone's
//! bytes, at fastfile_t6's offsets.

use fastfile_t6::layout::{FxElemAtlas, FxElemDef};

use crate::look_kind::ParticleShape;

pub const FX_ELEM_DEF_SIZE: usize = FxElemDef::SIZE;

pub const FX_ELEM_ATLAS_SIZE: usize = FxElemAtlas::SIZE;

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
    pub fn entry_tally(&self) -> u8 {
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
        raw.get(..FxElemDef::SIZE).map(|raw| Self { raw })
    }

    #[inline]
    fn u8(&self, at: usize) -> u8 {
        self.raw[at]
    }

    #[inline]
    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.raw[at], self.raw[at + 1]])
    }

    #[inline]
    fn u32(&self, at: usize) -> u32 {
        let low = u32::from(self.u16(at));
        low | u32::from(self.u16(at + 2)) << 16
    }

    #[inline]
    fn i32(&self, at: usize) -> i32 {
        self.u32(at) as i32
    }

    #[inline]
    fn f32(&self, at: usize) -> f32 {
        f32::from_bits(self.u32(at))
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
        self.u32(FxElemDef::flags)
    }

    /// The spawn union read as looping: (interval msec, count).
    pub fn spawn_looping(&self) -> (i32, i32) {
        (self.i32(FxElemDef::spawn), self.i32(FxElemDef::spawn + 4))
    }

    /// The spawn union read as one-shot: the count range.
    pub fn spawn_one_shot(&self) -> FxIntRange {
        self.int_range(FxElemDef::spawn)
    }

    pub fn birth_span(&self) -> FxFloatRange {
        self.float_range(FxElemDef::spawnRange)
    }

    pub fn appear_blend_reach(&self) -> FxFloatRange {
        self.float_range(FxElemDef::fadeInRange)
    }

    pub fn vanish_blend_reach(&self) -> FxFloatRange {
        self.float_range(FxElemDef::fadeOutRange)
    }

    pub fn birth_view_reject_reach(&self) -> f32 {
        self.f32(FxElemDef::spawnFrustumCullRadius)
    }

    pub fn birth_wait_ms(&self) -> FxIntRange {
        self.int_range(FxElemDef::spawnDelayMsec)
    }

    pub fn lifespan_window_ms(&self) -> FxIntRange {
        self.int_range(FxElemDef::lifeSpanMsec)
    }

    pub fn birth_point(&self) -> [FxFloatRange; 3] {
        self.range3(FxElemDef::spawnOrigin)
    }

    pub fn birth_ring_reach(&self) -> FxFloatRange {
        self.float_range(FxElemDef::spawnOffsetRadius)
    }

    pub fn birth_lift(&self) -> FxFloatRange {
        self.float_range(FxElemDef::spawnOffsetHeight)
    }

    pub fn birth_orientation(&self) -> [FxFloatRange; 3] {
        self.range3(FxElemDef::spawnAngles)
    }

    pub fn twirl_rate(&self) -> [FxFloatRange; 3] {
        self.range3(FxElemDef::angularVelocity)
    }

    pub fn birth_spin_angle(&self) -> FxFloatRange {
        self.float_range(FxElemDef::initialRotation)
    }

    /// The packed rotation axis, decoded at draw with flag 0x800000.
    // BO2 unknown: the packing (see facts/QUESTIONS.md); the raw word is given.
    pub fn rotation_axis_packed(&self) -> u32 {
        self.u32(FxElemDef::rotationAxis)
    }

    pub fn gravity(&self) -> FxFloatRange {
        self.float_range(FxElemDef::gravity)
    }

    pub fn mirror_strength(&self) -> FxFloatRange {
        self.float_range(FxElemDef::reflectionFactor)
    }

    pub fn atlas(&self) -> FxElemAtlasT6 {
        let o = FxElemDef::atlas;
        FxElemAtlasT6 {
            behavior: self.u8(o + FxElemAtlas::behavior),
            index: self.u8(o + FxElemAtlas::index),
            fps: self.u8(o + FxElemAtlas::fps),
            loop_count: self.u8(o + FxElemAtlas::loopCount),
            col_index_bits: self.u8(o + FxElemAtlas::colIndexBits),
            row_index_bits: self.u8(o + FxElemAtlas::rowIndexBits),
            entry_count_and_index_range: self.u16(o + FxElemAtlas::entryCountAndIndexRange),
        }
    }

    pub fn wind_influence(&self) -> f32 {
        self.f32(FxElemDef::windInfluence)
    }

    pub fn elem_type_raw(&self) -> u8 {
        self.u8(FxElemDef::elemType)
    }

    pub fn elem_type(&self) -> Option<ParticleShape> {
        ParticleShape::from_u8(self.elem_type_raw())
    }

    pub fn look_variant_total(&self) -> u8 {
        self.u8(FxElemDef::visualCount)
    }

    pub fn speed_keyframe_total(&self) -> u8 {
        self.u8(FxElemDef::velIntervalCount)
    }

    pub fn look_keyframe_total(&self) -> u8 {
        self.u8(FxElemDef::visStateIntervalCount)
    }

    pub fn bounce_box_bottom(&self) -> [f32; 3] {
        self.vec3(FxElemDef::collMins)
    }

    pub fn bounce_box_top(&self) -> [f32; 3] {
        self.vec3(FxElemDef::collMaxs)
    }

    pub fn emission_spacing(&self) -> FxFloatRange {
        self.float_range(FxElemDef::emitDist)
    }

    pub fn emission_spacing_spread(&self) -> FxFloatRange {
        self.float_range(FxElemDef::emitDistVariance)
    }

    pub fn paint_rank(&self) -> u8 {
        self.u8(FxElemDef::sortOrder)
    }

    pub fn shading_blend(&self) -> u8 {
        self.u8(FxElemDef::lightingFrac)
    }

    // The four wind and fade fields are 2 bytes each in the layout; read
    // unsigned.
    pub fn alpha_fade_time_msec(&self) -> u16 {
        self.u16(FxElemDef::alphaFadeTimeMsec)
    }

    pub fn max_wind_strength(&self) -> u16 {
        self.u16(FxElemDef::maxWindStrength)
    }

    pub fn spawn_interval_at_max_wind(&self) -> u16 {
        self.u16(FxElemDef::spawnIntervalAtMaxWind)
    }

    pub fn lifespan_at_max_wind(&self) -> u16 {
        self.u16(FxElemDef::lifespanAtMaxWind)
    }

    pub fn billboard_pivot(&self) -> [f32; 2] {
        [self.f32(FxElemDef::billboardPivot), self.f32(FxElemDef::billboardPivot + 4)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(raw: &mut [u8], off: usize, buf: &[u8]) {
        raw[off..off + buf.len()].copy_from_slice(buf);
    }

    #[test]
    fn needs_a_whole_def() {
        assert!(FxElemDefT6::new(&[0u8; 291]).is_none());
        assert!(FxElemDefT6::new(&[0u8; 300]).is_some());
    }

    #[test]
    fn reads_fields_at_bo2_offsets() {
        let mut raw = [0u8; FX_ELEM_DEF_SIZE];
        put(&mut raw, FxElemDef::flags, &0x8000_0400u32.to_le_bytes());
        put(&mut raw, FxElemDef::lifeSpanMsec, &500i32.to_le_bytes());
        put(&mut raw, FxElemDef::lifeSpanMsec + 4, &250i32.to_le_bytes());
        put(&mut raw, FxElemDef::gravity + 4, &1.5f32.to_le_bytes());
        put(&mut raw, FxElemDef::spawnAngles + 16, &(-2.0f32).to_le_bytes());
        put(
            &mut raw,
            FxElemDef::atlas + FxElemAtlas::entryCountAndIndexRange,
            &516u16.to_le_bytes(),
        );
        raw[FxElemDef::elemType] = 4;
        raw[FxElemDef::sortOrder] = 9;
        put(&mut raw, FxElemDef::lifespanAtMaxWind, &1200u16.to_le_bytes());
        put(&mut raw, FxElemDef::billboardPivot + 4, &0.25f32.to_le_bytes());
        let d = FxElemDefT6::new(&raw).unwrap();
        assert_eq!(d.flags(), 0x8000_0400);
        assert_eq!(
            d.lifespan_window_ms(),
            FxIntRange {
                base: 500,
                amplitude: 250
            }
        );
        assert_eq!(d.gravity().amplitude, 1.5);
        assert_eq!(d.birth_orientation()[2].base, -2.0);
        assert_eq!(d.atlas().entry_tally(), 4);
        assert_eq!(d.elem_type(), Some(ParticleShape::Line));
        assert_eq!(d.paint_rank(), 9);
        assert_eq!(d.lifespan_at_max_wind(), 1200);
        assert_eq!(d.billboard_pivot(), [0.0, 0.25]);
    }
}
