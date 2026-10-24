//! The products a frame is split into and whether each was produced.

use std::sync::Arc;

use render_material::{MaterialGenerationId, TechType};

use crate::{PackedFrontendLists, RetainedDrawItem};

/// The passes a frame is divided into, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameProductKind {
    Depth,
    FloatZ,
    SunShadow,
    SpotShadow,
    Colour,
    Emissive,
    Distortion,
    Light,
    TwoD,
}

impl FrameProductKind {
    pub const ALL: [Self; 9] = [
        Self::Depth,
        Self::FloatZ,
        Self::SunShadow,
        Self::SpotShadow,
        Self::Colour,
        Self::Emissive,
        Self::Distortion,
        Self::Light,
        Self::TwoD,
    ];

    /// Why a product of this kind has nothing in it before anyone fills it.
    pub(super) const fn default_gap(self) -> MissingProductCause {
        match self {
            Self::SunShadow => MissingProductCause::MissingSunShadowProducer,
            Self::SpotShadow => MissingProductCause::MissingSpotShadowProducer,
            Self::TwoD => MissingProductCause::HostOwnedTwoD,
            _ => MissingProductCause::MissingFrontendEmitter,
        }
    }
}

/// Render target a finished product lands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductTarget {
    Core3dViewColour,
    ResolvedPostSun,
    SunShadowFallbackAtlas,
    SpotShadowMaps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingProductCause {
    MissingFrontendEmitter,
    MissingSunShadowProducer,
    MissingSpotShadowProducer,
    MissingDistortionSortKey,
    HostOwnedTwoD,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameProductStatus {
    /// Drawn by a resolve step; there is no technique to name.
    ResolveReady { target: ProductTarget },
    Ready { tech_type: TechType, target: ProductTarget },
    Missing(MissingProductCause),
}

/// One spot light's shadow pass: which light slot it is and its packed lists.
#[derive(Clone, Debug)]
pub struct SpotShadowFrameSlot {
    pub emitted: lighting_t6::SpotShadowEmittedSlot,
    pub packed: Arc<PackedFrontendLists>,
}

#[derive(Clone, Debug)]
pub struct FrameProduct {
    pub kind: FrameProductKind,
    pub generation_id: MaterialGenerationId,
    pub status: FrameProductStatus,
    pub ordered_draws: Vec<RetainedDrawItem>,
    /// Draws at the front of `ordered_draws` that belong to the near cascade.
    pub sun_near_n: usize,
    /// Per-cascade packed lists, present for the sun-shadow product only.
    pub sun_packed: Option<Arc<[PackedFrontendLists; 2]>>,
    pub spot_slots: Vec<SpotShadowFrameSlot>,
    pub list_digest: u64,
    pub world_pretess_id: u64,
    /// Technique of each ordered draw, parallel to `ordered_draws`.
    pub draw_tech: Vec<TechType>,
    /// Bit `n` set: some draw binds code texture `n`.
    pub code_sampler_mask: u64,
    pub has_codemesh: bool,
}

impl FrameProduct {
    pub(super) fn vacant(kind: FrameProductKind) -> Self {
        Self {
            kind,
            generation_id: MaterialGenerationId::default(),
            status: FrameProductStatus::Missing(kind.default_gap()),
            ordered_draws: Vec::new(),
            sun_near_n: 0,
            sun_packed: None,
            spot_slots: Vec::new(),
            list_digest: 0,
            world_pretess_id: 0,
            draw_tech: Vec::new(),
            code_sampler_mask: 0,
            has_codemesh: false,
        }
    }

    /// Empties the product for a new fill, leaving it marked missing for `cause`
    /// until a producer says otherwise. The generation id is kept.
    pub fn begin_fill(&mut self, cause: MissingProductCause) {
        let generation_id = self.generation_id;
        *self = Self { generation_id, status: FrameProductStatus::Missing(cause), ..Self::vacant(self.kind) };
    }

    pub fn binds_code_texture(&self, index: u32) -> bool {
        self.code_sampler_mask.checked_shr(index).is_some_and(|rest| rest & 1 == 1)
    }

    pub fn tech_type(&self) -> Option<TechType> {
        if let FrameProductStatus::Ready { tech_type, .. } = self.status {
            Some(tech_type)
        } else {
            None
        }
    }
}
