//! The material-local half of a pass, packed once into flat constant banks
//! and a sampler lane list.

use super::args::{RunnableParamSource, PreparedParamStrips};
use crate::LiveShaderStage;
use crate::catalog::LiveTextureBinding;
use crate::catalog::hash_text_64;
use crate::catalog::fnv_extend;

/// Constant rows the material fixes for a pass, laid out like the shader
/// banks, with a flag per row saying whether the material wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSamplerBanks {
    pub vertex_rows: Vec<[u32; 4]>,
    pub pixel_rows: Vec<[u32; 4]>,
    pub vertex_filled: Vec<bool>,
    pub pixel_filled: Vec<bool>,
}

impl LocalSamplerBanks {
    fn zeroed(vertex_rows: usize, fragment_stage_length: usize) -> Self {
        Self {
            vertex_rows: vec![[0; 4]; vertex_rows],
            pixel_rows: vec![[0; 4]; fragment_stage_length],
            vertex_filled: vec![false; vertex_rows],
            pixel_filled: vec![false; fragment_stage_length],
        }
    }

    /// `None` when the register lies outside the bank.
    fn put_row(&mut self, stage: LiveShaderStage, reg: u16, value: [u32; 4]) -> Option<()> {
        let (bank, marks) = match stage {
            LiveShaderStage::Vertex => (&mut self.vertex_rows, &mut self.vertex_filled),
            LiveShaderStage::Pixel => (&mut self.pixel_rows, &mut self.pixel_filled),
        };
        let row = usize::from(reg);
        *bank.get_mut(row)? = value;
        *marks.get_mut(row)? = true;
        Some(())
    }
}

/// `None` when any of the material's constants does not fit the given bank
/// sizes; the pass then keeps no packed banks for that vertex type.
pub fn encode_local_register_banks(
    vertex_slots: usize,
    fragment_stage_length: usize,
    belts: &PreparedParamStrips,
) -> Option<LocalSamplerBanks> {
    let mut banks = LocalSamplerBanks::zeroed(vertex_slots, fragment_stage_length);
    let locals = belts
        .in_bank_order()
        .into_iter()
        .flat_map(|slice| slice.local.iter().flatten());
    for local in locals {
        match *local {
            RunnableParamSource::ShaderParameterValue {
                stage,
                slot_reg: dest,
                lane_bits: words,
                ..
            }
            | RunnableParamSource::InlineConstant {
                stage,
                slot_reg: dest,
                lane_bits: words,
            } => banks.put_row(stage, dest, words)?,
            RunnableParamSource::ShaderSamplerImage { .. }
            | RunnableParamSource::EngineSuppliedTexture { .. }
            | RunnableParamSource::EngineConstant { .. } => {}
        }
    }
    Some(banks)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CompactSamplerSlot {
    pub sampler_reg: u16,
    pub label_digest: u32,
    pub texture: LiveTextureBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalSamplerBundle {
    /// Content hash of `lanes`; equal ids mean equal sampler sets.
    pub id: u64,
    pub lanes: Vec<CompactSamplerSlot>,
}

impl Default for LocalSamplerBundle {
    fn default() -> Self {
        Self::assemble_from_channels(Vec::new())
    }
}

impl LocalSamplerBundle {
    fn assemble_from_channels(lanes: Vec<CompactSamplerSlot>) -> Self {
        Self {
            id: lanes_id(&lanes),
            lanes,
        }
    }

    pub fn binding_at(&self, reg: u16, label_digest: u32) -> Option<LiveTextureBinding> {
        let held = &self.lanes;
        held
            .iter()
            .find(|lane| lane.sampler_reg == reg && lane.label_digest == label_digest)
            .map(|slot| slot.texture)
    }
}

pub fn pack_local_samplers(belts: &PreparedParamStrips) -> LocalSamplerBundle {
    let lanes = belts
        .in_bank_order()
        .into_iter()
        .flat_map(|slice| slice.local.iter().flatten())
        .filter_map(|local| match *local {
            RunnableParamSource::ShaderSamplerImage {
                slot_reg: dest,
                label_digest,
                texture,
            } => Some(CompactSamplerSlot {
                sampler_reg: dest,
                label_digest,
                texture,
            }),
            _ => None,
        })
        .collect();
    LocalSamplerBundle::assemble_from_channels(lanes)
}

fn lanes_id(lanes: &[CompactSamplerSlot]) -> u64 {
    let seed = hash_text_64(&(lanes.len() as u64).to_le_bytes());
    lanes.iter().fold(seed, |acc, slot| {
        let acc = fnv_extend(acc, &slot.sampler_reg.to_le_bytes());
        let acc = fnv_extend(acc, &slot.label_digest.to_le_bytes());
        let acc = fnv_extend(acc, &slot.texture.image.0.to_le_bytes());
        fnv_extend(acc, &[slot.texture.texture_sampling_mode, slot.texture.semantic])
    })
}
