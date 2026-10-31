//! The products a frame is split into and whether each was produced.

use std::sync::Arc;

use render_material::{MaterialEpochId, MethodType};

use crate::{CompactFrontendLists, HeldDrawItem};

/// The passes a frame is divided into, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameOutputKind {
    DepthOnly,
    FloatDepth,
    SunDepth,
    SpotLightShadowPass,
    Colour,
    Emissive,
    RefractionPass,
    Light,
    Planar,
}

impl FrameOutputKind {
    pub const ALL_KINDS: [Self; 9] = [
        Self::DepthOnly,
        Self::FloatDepth,
        Self::SunDepth,
        Self::SpotLightShadowPass,
        Self::Colour,
        Self::Emissive,
        Self::RefractionPass,
        Self::Light,
        Self::Planar,
    ];

    /// Why a product of this kind has nothing in it before anyone fills it.
    pub(super) const fn default_gap(self) -> AbsentOutputReason {
        match self {
            Self::SunDepth => AbsentOutputReason::SunShadowSourceAbsent,
            Self::SpotLightShadowPass => AbsentOutputReason::ConeShadowSourceAbsent,
            Self::Planar => AbsentOutputReason::HostDrivenFlatLayer,
            _ => AbsentOutputReason::DrawlistWriterAbsent,
        }
    }
}

/// Render target a finished product lands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputTarget {
    MainViewColor,
    FinishedAfterSun,
    SunShadowSubstituteAtlas,
    SpotLightShadowAtlas,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbsentOutputReason {
    DrawlistWriterAbsent,
    SunShadowSourceAbsent,
    ConeShadowSourceAbsent,
    RefractionOrderKeyAbsent,
    HostDrivenFlatLayer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameOutputState {
    /// Drawn by a resolve step; there is no technique to name.
    ResolveReady { target: OutputTarget },
    Ready { shading_technique_kind: MethodType, target: OutputTarget },
    Missing(AbsentOutputReason),
}

/// One spot light's shadow pass: which light slot it is and its packed lists.
#[derive(Clone, Debug)]
pub struct ConeShadowFrameSlot {
    pub emitted: lighting_t6::SpotShadowSubmittedSlot,
    pub packed: Arc<CompactFrontendLists>,
}

#[derive(Clone, Debug)]
pub struct FrameOutputItem {
    pub kind: FrameOutputKind,
    pub revision_stamp: MaterialEpochId,
    pub state: FrameOutputState,
    pub draw_sequence: Vec<HeldDrawItem>,
    /// Draws at the front of `draw_sequence` that belong to the near cascade.
    pub daylight_near_tally: usize,
    /// Per-cascade packed lists, present for the sun-shadow product only.
    pub sun_pack: Option<Arc<[CompactFrontendLists; 2]>>,
    pub spot_light_frame_slots: Vec<ConeShadowFrameSlot>,
    pub list_fingerprint: u64,
    pub world_geometry_stamp: u64,
    /// Technique of each ordered draw, parallel to `draw_sequence`.
    pub per_draw_technique_kinds: Vec<MethodType>,
    /// Bit `n` set: some draw binds code texture `n`.
    pub engine_sampler_bitmask: u64,
    pub has_engine_mesh: bool,
}

impl FrameOutputItem {
    pub(super) fn vacant(kind: FrameOutputKind) -> Self {
        Self {
            kind,
            revision_stamp: MaterialEpochId::default(),
            state: FrameOutputState::Missing(kind.default_gap()),
            draw_sequence: Vec::new(),
            daylight_near_tally: 0,
            sun_pack: None,
            spot_light_frame_slots: Vec::new(),
            list_fingerprint: 0,
            world_geometry_stamp: 0,
            per_draw_technique_kinds: Vec::new(),
            engine_sampler_bitmask: 0,
            has_engine_mesh: false,
        }
    }

    /// Empties the product for a new fill, leaving it marked missing for `cause`
    /// until a producer says otherwise. The generation id is kept.
    pub fn start_fill(&mut self, cause: AbsentOutputReason) {
        let revision_stamp = self.revision_stamp;
        *self = Self { revision_stamp, state: FrameOutputState::Missing(cause), ..Self::vacant(self.kind) };
    }

    pub fn uses_engine_texture(&self, index: u32) -> bool {
        self.engine_sampler_bitmask.checked_shr(index).is_some_and(|rest| rest & 1 == 1)
    }

    pub fn shading_technique_kind(&self) -> Option<MethodType> {
        if let FrameOutputState::Ready { shading_technique_kind, .. } = self.state {
            Some(shading_technique_kind)
        } else {
            None
        }
    }
}
