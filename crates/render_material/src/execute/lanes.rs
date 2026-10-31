//! Per-pass code constant and sampler lanes, each set carrying a content id.

use std::sync::Arc;

use crate::catalog::hash_text_64;
use crate::catalog::fnv_extend;
use crate::{LiveImageId, LiveShaderStage};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CodeSamplerChannel {
    pub sampler_reg: u16,
    pub index: u32,
    pub texture_sampling_mode: u8,
    pub image: Option<LiveImageId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompactEngineSamplers {
    pub id: u64,
    pub lanes: Vec<CodeSamplerChannel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeConstantChannel {
    pub stage: LiveShaderStage,
    pub slot_reg: u16,
    pub index: u16,
    pub row_base: u8,
    pub line_total: u8,
    pub row_data: Arc<[[u32; 4]]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeConstantBundle {
    pub id: u64,
    pub lanes: Vec<CodeConstantChannel>,
}

impl Default for CompactEngineSamplers {
    fn default() -> Self {
        Self::assemble_from_channels(Vec::new())
    }
}

impl CompactEngineSamplers {
    pub fn assemble_from_channels(lanes: Vec<CodeSamplerChannel>) -> Self {
        let id = sampler_set_id(&lanes);
        Self { id, lanes }
    }

    pub fn lane_at(&self, reg: u16, index: u32) -> Option<CodeSamplerChannel> {
        let held = &self.lanes;
        held
            .iter()
            .find(|lane| lane.index == index && lane.sampler_reg == reg)
            .copied()
    }

    pub(super) fn rehash(&mut self) {
        self.id = sampler_set_id(&self.lanes);
    }
}

impl Default for CodeConstantBundle {
    fn default() -> Self {
        Self::assemble_from_channels(Vec::new())
    }
}

impl CodeConstantBundle {
    pub fn assemble_from_channels(lanes: Vec<CodeConstantChannel>) -> Self {
        let id = Self::hash_lanes(&lanes);
        Self { id, lanes }
    }

    pub fn hash_lanes(lanes: &[CodeConstantChannel]) -> u64 {
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

fn constant_set_id(lanes: &[CodeConstantChannel]) -> u64 {
    let mut state = mix(MIX_SEED, lanes.len() as u64);
    for chan in lanes {
        let stage_bit = match chan.stage {
            LiveShaderStage::Vertex => 0,
            LiveShaderStage::Pixel => 1,
        };
        state = mix(
            state,
            stage_bit
                | u64::from(chan.slot_reg) << 8
                | u64::from(chan.index) << 24
                | u64::from(chan.row_base) << 40
                | u64::from(chan.line_total) << 48,
        );
        for [a, b, c, d] in chan.row_data.iter().copied() {
            state = mix(state, u64::from(a) | u64::from(b) << 32);
            state = mix(state, u64::from(c) | u64::from(d) << 32);
        }
    }
    mix(state, MIX_SEED)
}

fn sampler_set_id(lanes: &[CodeSamplerChannel]) -> u64 {
    let mut state = hash_text_64(&(lanes.len() as u64).to_le_bytes());
    for chan in lanes {
        state = fnv_extend(state, &chan.sampler_reg.to_le_bytes());
        state = fnv_extend(state, &chan.index.to_le_bytes());
        state = fnv_extend(state, &[chan.texture_sampling_mode]);
        state = match chan.image {
            Some(img) => fnv_extend(fnv_extend(state, &[1]), &img.0.to_le_bytes()),
            None => fnv_extend(state, &[0]),
        };
    }
    state
}
