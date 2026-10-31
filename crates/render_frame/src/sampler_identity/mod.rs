//! A compact identity for the textures a draw binds, so draws that bind the same
//! things can be recognised without comparing the bindings themselves.

use render_material::{CodeSamplerChannel, CompactEngineSamplers, LocalSamplerBundle, ShaderPortKey};

use crate::FacetSamplerInputs;

/// What distinguishes one texture binding set from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureBindingKey {
    pub port: ShaderPortKey,
    /// Id of the material-local samplers; 0 when there are none.
    pub local: u64,
    pub code: u64,
    pub surf: FacetSamplerInputs,
}

/// 64-bit FNV-1a, fed incrementally.
struct Fnv(u64);

impl Fnv {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::BASIS)
    }

    fn feed(&mut self, chunk: &[u8]) {
        self.0 = chunk
            .iter()
            .fold(self.0, |state, &b| (state ^ u64::from(b)).wrapping_mul(Self::PRIME));
    }
}

fn local_id(local: Option<&LocalSamplerBundle>) -> u64 {
    local.map_or(0, |packed| packed.id)
}

impl TextureBindingKey {
    /// From samplers already packed under a single id.
    pub fn unpack_from_bits(
        port: ShaderPortKey,
        local: Option<&LocalSamplerBundle>,
        code: &CompactEngineSamplers,
        surf: FacetSamplerInputs,
    ) -> Self {
        Self { port, local: local_id(local), code: code.id, surf }
    }

    /// From the individual code-sampler lanes: the `code` field is a digest of
    /// their count and of each lane's register, index, sampler state and image.
    pub fn assemble_from_channels(
        port: ShaderPortKey,
        local: Option<&LocalSamplerBundle>,
        code: &[CodeSamplerChannel],
        surf: FacetSamplerInputs,
    ) -> Self {
        let mut digest = Fnv::new();
        digest.feed(&(code.len() as u64).to_le_bytes());
        for bind_lane in code {
            digest.feed(&bind_lane.sampler_reg.to_le_bytes());
            digest.feed(&bind_lane.index.to_le_bytes());
            digest.feed(&[bind_lane.texture_sampling_mode]);
            match bind_lane.image {
                Some(bound_image) => {
                    digest.feed(&[1]);
                    digest.feed(&bound_image.0.to_le_bytes());
                }
                None => digest.feed(&[0]),
            }
        }
        Self { port, local: local_id(local), code: digest.0, surf }
    }
}
