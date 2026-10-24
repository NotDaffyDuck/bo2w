//! Per-pass code constant and sampler lanes, each set carrying a content id.

use std::sync::Arc;

use crate::catalog::{fnv1a64, fnv1a64_more};
use crate::{RuntimeImageId, RuntimeShaderStage};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackedCodeSamplerLane {
    pub register: u16,
    pub index: u32,
    pub sampler_state: u8,
    pub image: Option<RuntimeImageId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackedCodeSamplers {
    pub id: u64,
    pub lanes: Vec<PackedCodeSamplerLane>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedCodeConstantLane {
    pub stage: RuntimeShaderStage,
    pub destination: u16,
    pub index: u16,
    pub first_row: u8,
    pub row_count: u8,
    pub rows: Arc<[[u32; 4]]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedCodeConstants {
    pub id: u64,
    pub lanes: Vec<PackedCodeConstantLane>,
}

impl Default for PackedCodeSamplers {
    fn default() -> Self {
        Self::from_lanes(Vec::new())
    }
}

impl PackedCodeSamplers {
    pub fn from_lanes(lanes: Vec<PackedCodeSamplerLane>) -> Self {
        let id = sampler_set_id(&lanes);
        Self { id, lanes }
    }

    pub fn texture(&self, register: u16, index: u32) -> Option<PackedCodeSamplerLane> {
        self.lanes
            .iter()
            .find(|lane| lane.register == register && lane.index == index)
            .copied()
    }

    pub(super) fn rehash(&mut self) {
        self.id = sampler_set_id(&self.lanes);
    }
}

impl Default for PackedCodeConstants {
    fn default() -> Self {
        Self::from_lanes(Vec::new())
    }
}

impl PackedCodeConstants {
    pub fn from_lanes(lanes: Vec<PackedCodeConstantLane>) -> Self {
        let id = Self::hash_lanes(&lanes);
        Self { id, lanes }
    }

    pub fn hash_lanes(lanes: &[PackedCodeConstantLane]) -> u64 {
        constant_set_id(lanes)
    }

    pub(super) fn rehash(&mut self) {
        self.id = constant_set_id(&self.lanes);
    }
}

const MIX_SEED: u64 = 0x243f_6a88_85a3_08d3;
const MIX_FACTOR: u64 = 0x9e37_79b9_7f4a_7c15;

/// One multiply-fold step. The constant ids key caches with no second
/// comparison, so every word goes through the full 128-bit product.
fn mix(state: u64, word: u64) -> u64 {
    let wide = u128::from(state ^ word) * u128::from(MIX_FACTOR);
    (wide as u64) ^ (wide >> 64) as u64
}

fn constant_set_id(lanes: &[PackedCodeConstantLane]) -> u64 {
    let mut state = mix(MIX_SEED, lanes.len() as u64);
    for lane in lanes {
        let stage_bit = match lane.stage {
            RuntimeShaderStage::Vertex => 0,
            RuntimeShaderStage::Pixel => 1,
        };
        state = mix(
            state,
            stage_bit
                | u64::from(lane.destination) << 8
                | u64::from(lane.index) << 24
                | u64::from(lane.first_row) << 40
                | u64::from(lane.row_count) << 48,
        );
        for [a, b, c, d] in lane.rows.iter().copied() {
            state = mix(state, u64::from(a) | u64::from(b) << 32);
            state = mix(state, u64::from(c) | u64::from(d) << 32);
        }
    }
    mix(state, MIX_SEED)
}

fn sampler_set_id(lanes: &[PackedCodeSamplerLane]) -> u64 {
    let mut state = fnv1a64(&(lanes.len() as u64).to_le_bytes());
    for lane in lanes {
        state = fnv1a64_more(state, &lane.register.to_le_bytes());
        state = fnv1a64_more(state, &lane.index.to_le_bytes());
        state = fnv1a64_more(state, &[lane.sampler_state]);
        state = match lane.image {
            Some(image) => fnv1a64_more(fnv1a64_more(state, &[1]), &image.0.to_le_bytes()),
            None => fnv1a64_more(state, &[0]),
        };
    }
    state
}
