//! A compact identity for the textures a draw binds, so draws that bind the same
//! things can be recognised without comparing the bindings themselves.

use render_material::{PackedCodeSamplerLane, PackedCodeSamplers, PackedLocalSamplers, PortId};

use crate::SurfaceSamplerInputs;

/// What distinguishes one texture binding set from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureBindIdentity {
    pub port: PortId,
    /// Id of the material-local samplers; 0 when there are none.
    pub local: u64,
    pub code: u64,
    pub surface: SurfaceSamplerInputs,
}

/// 64-bit FNV-1a, fed incrementally.
struct Fnv(u64);

impl Fnv {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::BASIS)
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.0 = bytes
            .iter()
            .fold(self.0, |state, &b| (state ^ u64::from(b)).wrapping_mul(Self::PRIME));
    }
}

fn local_id(local: Option<&PackedLocalSamplers>) -> u64 {
    local.map_or(0, |packed| packed.id)
}

impl TextureBindIdentity {
    /// From samplers already packed under a single id.
    pub fn from_packed(
        port: PortId,
        local: Option<&PackedLocalSamplers>,
        code: &PackedCodeSamplers,
        surface: SurfaceSamplerInputs,
    ) -> Self {
        Self { port, local: local_id(local), code: code.id, surface }
    }

    /// From the individual code-sampler lanes: the `code` field is a digest of
    /// their count and of each lane's register, index, sampler state and image.
    pub fn from_lanes(
        port: PortId,
        local: Option<&PackedLocalSamplers>,
        code: &[PackedCodeSamplerLane],
        surface: SurfaceSamplerInputs,
    ) -> Self {
        let mut digest = Fnv::new();
        digest.feed(&(code.len() as u64).to_le_bytes());
        for lane in code {
            digest.feed(&lane.register.to_le_bytes());
            digest.feed(&lane.index.to_le_bytes());
            digest.feed(&[lane.sampler_state]);
            match lane.image {
                Some(image) => {
                    digest.feed(&[1]);
                    digest.feed(&image.0.to_le_bytes());
                }
                None => digest.feed(&[0]),
            }
        }
        Self { port, local: local_id(local), code: digest.0, surface }
    }
}
