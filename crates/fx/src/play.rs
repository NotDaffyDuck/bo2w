//! Playing an effect: the entry points callers use to start one, one-shot or
//! held, free or bolted to a model bone.

use fx_t6::runtime::{
    FX_SPAWN_BOLT_NONE, bolt_centity_teleport_for_compare, bolt_pack, vector_vectors,
};

use crate::def::FxEffectDefInfo;
use crate::spawn::start_new_effect;
use crate::system::{FxBoltTarget, FxSystemHost, SpawnFail};

#[derive(Clone, Copy, Debug)]
pub struct FxPlayPose {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub msec: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayResult {
    /// Started, and the caller's claim on it is already given up.
    PlayedReleased { handle: u16 },
    /// Started, and the caller still holds it (it must stop or release it).
    Held { handle: u16 },
    Failed(SpawnFail),
}

impl PlayResult {
    pub fn handle(self) -> Option<u16> {
        match self {
            Self::PlayedReleased { handle } | Self::Held { handle } => Some(handle),
            Self::Failed(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FxPlayRequest<'a> {
    pub def_name: &'a str,
    pub pose: FxPlayPose,
    pub wants_spotlight: bool,
    /// The definition to start parts from; without it only the slot is made.
    pub def: Option<FxEffectDefInfo<'a>>,
    pub catalog_index: u16,
}

/// Plays a fire-and-forget effect at a pose.
pub fn play_oriented(host: &mut FxSystemHost, req: FxPlayRequest<'_>) -> PlayResult {
    start(host, req, None, true)
}

/// Starts an effect the caller keeps holding.
pub fn spawn_oriented(host: &mut FxSystemHost, req: FxPlayRequest<'_>) -> PlayResult {
    start(host, req, None, false)
}

/// Plays a fire-and-forget effect that follows a model bone.
pub fn play_bolted(
    host: &mut FxSystemHost,
    req: FxPlayRequest<'_>,
    target: FxBoltTarget,
) -> PlayResult {
    start(host, req, Some(target), true)
}

/// Where an effect faces when it comes from a surface hit.
pub fn axis_from_hit_normal(normal: [f32; 3]) -> [[f32; 3]; 3] {
    vector_vectors(normal)
}

/// An effect started by something hitting or dying is played like any other.
pub fn spawn_impact_or_death_effect(host: &mut FxSystemHost, req: FxPlayRequest<'_>) -> PlayResult {
    play_oriented(host, req)
}

fn start(
    host: &mut FxSystemHost,
    req: FxPlayRequest<'_>,
    target: Option<FxBoltTarget>,
    release: bool,
) -> PlayResult {
    let bolt = target.map_or(FX_SPAWN_BOLT_NONE, |t| t.dobj);
    let handle = match host.spawn_effect(
        req.def_name,
        req.pose.origin,
        req.pose.axis,
        req.pose.msec,
        bolt,
        req.wants_spotlight,
        req.catalog_index,
    ) {
        Ok(handle) => handle,
        Err(fail) => return PlayResult::Failed(fail),
    };

    if let Some(target) = target
        && let Some(effect) = host.slot_for_handle_mut(handle)
    {
        let teleport = bolt_centity_teleport_for_compare(target.dobj, target.centity_teleport);
        let pose = target.orientation;
        effect.bolt_packed = bolt_pack(target.dobj, teleport, u32::from(target.bone));
        effect.bolt_centity_teleport = teleport;
        effect.bolt_bone_pose = Some((pose.origin, pose.axis));
        effect.origin = pose.origin;
        effect.axis = pose.axis;
    }
    if let Some(def) = req.def {
        start_new_effect(host, handle, def);
    }

    if release {
        host.play_release_ownership(handle);
        PlayResult::PlayedReleased { handle }
    } else {
        PlayResult::Held { handle }
    }
}
