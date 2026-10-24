//! The material-local half of a pass, packed once into flat constant banks
//! and a sampler lane list.

use super::args::{ExecutableArgumentBinding, PreparedArgBelts};
use crate::RuntimeShaderStage;
use crate::catalog::{RuntimeTextureBinding, fnv1a64, fnv1a64_more};

/// Constant rows the material fixes for a pass, laid out like the shader
/// banks, with a flag per row saying whether the material wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedLocalBanks {
    pub vertex: Vec<[u32; 4]>,
    pub pixel: Vec<[u32; 4]>,
    pub written_vertex: Vec<bool>,
    pub written_pixel: Vec<bool>,
}

impl PackedLocalBanks {
    fn zeroed(vertex_len: usize, pixel_len: usize) -> Self {
        Self {
            vertex: vec![[0; 4]; vertex_len],
            pixel: vec![[0; 4]; pixel_len],
            written_vertex: vec![false; vertex_len],
            written_pixel: vec![false; pixel_len],
        }
    }

    /// `None` when the register lies outside the bank.
    fn write(&mut self, stage: RuntimeShaderStage, register: u16, words: [u32; 4]) -> Option<()> {
        let (rows, written) = match stage {
            RuntimeShaderStage::Vertex => (&mut self.vertex, &mut self.written_vertex),
            RuntimeShaderStage::Pixel => (&mut self.pixel, &mut self.written_pixel),
        };
        let row = usize::from(register);
        *rows.get_mut(row)? = words;
        *written.get_mut(row)? = true;
        Some(())
    }
}

/// `None` when any of the material's constants does not fit the given bank
/// sizes; the pass then keeps no packed banks for that vertex type.
pub fn pack_local_banks(
    vertex_len: usize,
    pixel_len: usize,
    belts: &PreparedArgBelts,
) -> Option<PackedLocalBanks> {
    let mut banks = PackedLocalBanks::zeroed(vertex_len, pixel_len);
    let locals = belts
        .in_bank_order()
        .into_iter()
        .flat_map(|slice| slice.local.iter().flatten());
    for local in locals {
        match *local {
            ExecutableArgumentBinding::MaterialConstant {
                stage,
                destination,
                words,
                ..
            }
            | ExecutableArgumentBinding::LiteralConstant {
                stage,
                destination,
                words,
            } => banks.write(stage, destination, words)?,
            ExecutableArgumentBinding::MaterialTexture { .. }
            | ExecutableArgumentBinding::CodeTexture { .. }
            | ExecutableArgumentBinding::CodeConstant { .. } => {}
        }
    }
    Some(banks)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackedLocalSamplerLane {
    pub register: u16,
    pub name_hash: u32,
    pub texture: RuntimeTextureBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackedLocalSamplers {
    /// Content hash of `lanes`; equal ids mean equal sampler sets.
    pub id: u64,
    pub lanes: Vec<PackedLocalSamplerLane>,
}

impl Default for PackedLocalSamplers {
    fn default() -> Self {
        Self::from_lanes(Vec::new())
    }
}

impl PackedLocalSamplers {
    fn from_lanes(lanes: Vec<PackedLocalSamplerLane>) -> Self {
        Self {
            id: lanes_id(&lanes),
            lanes,
        }
    }

    pub fn texture(&self, register: u16, name_hash: u32) -> Option<RuntimeTextureBinding> {
        self.lanes
            .iter()
            .find(|lane| lane.register == register && lane.name_hash == name_hash)
            .map(|lane| lane.texture)
    }
}

pub fn pack_local_samplers(belts: &PreparedArgBelts) -> PackedLocalSamplers {
    let lanes = belts
        .in_bank_order()
        .into_iter()
        .flat_map(|slice| slice.local.iter().flatten())
        .filter_map(|local| match *local {
            ExecutableArgumentBinding::MaterialTexture {
                destination,
                name_hash,
                texture,
            } => Some(PackedLocalSamplerLane {
                register: destination,
                name_hash,
                texture,
            }),
            _ => None,
        })
        .collect();
    PackedLocalSamplers::from_lanes(lanes)
}

fn lanes_id(lanes: &[PackedLocalSamplerLane]) -> u64 {
    let seed = fnv1a64(&(lanes.len() as u64).to_le_bytes());
    lanes.iter().fold(seed, |hash, lane| {
        let hash = fnv1a64_more(hash, &lane.register.to_le_bytes());
        let hash = fnv1a64_more(hash, &lane.name_hash.to_le_bytes());
        let hash = fnv1a64_more(hash, &lane.texture.image.0.to_le_bytes());
        fnv1a64_more(hash, &[lane.texture.sampler_state, lane.texture.semantic])
    })
}
