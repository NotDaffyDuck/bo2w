//! Playing an effect: the entry points callers use to start one, one-shot or
//! held, free or bolted to a model bone.

use fx_t6::runtime::{
    BIRTH_UNATTACHED, bolt_centity_teleport_for_compare, bolt_pack, contact_facing_basis,
};

use crate::recipe::EmitterRecipeSummary;
use crate::birth::launch_fresh_effect;
use crate::host::{AttachPointFrame, EffectBoltTarget, LiveEffect, EffectSystemHost, SpawnRefusal};

#[derive(Clone, Copy, Debug)]
pub struct EffectStartFrame {
    pub anchor: [f32; 3],
    pub frame: [[f32; 3]; 3],
    pub ms_now: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackResult {
    /// Started, and the caller's claim on it is already given up.
    StartedLetGo { handle: u16 },
    /// Started, and the caller still holds it (it must stop or release it).
    Kept { handle: u16 },
    Refused(SpawnRefusal),
}

impl PlaybackResult {
    pub fn handle(self) -> Option<u16> {
        match self {
            Self::StartedLetGo { handle } | Self::Kept { handle } => Some(handle),
            Self::Refused(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EffectStartAsk<'a> {
    pub definition_name: &'a str,
    pub pose: EffectStartFrame,
    pub needs_cone_glow: bool,
    /// The definition to start parts from; without it only the slot is made.
    pub def: Option<EmitterRecipeSummary<'a>>,
    pub ledger_index: u16,
}

/// Plays a fire-and-forget effect at a pose.
pub fn start_aligned(host: &mut EffectSystemHost, req: EffectStartAsk<'_>) -> PlaybackResult {
    start(host, req, None, Claim::GiveUp)
}

/// Starts an effect the caller keeps holding.
pub fn play_facing(host: &mut EffectSystemHost, req: EffectStartAsk<'_>) -> PlaybackResult {
    start(host, req, None, Claim::Keep)
}

/// Plays a fire-and-forget effect that follows a model bone.
pub fn start_attached(
    host: &mut EffectSystemHost,
    req: EffectStartAsk<'_>,
    bolt: EffectBoltTarget,
) -> PlaybackResult {
    start(host, req, Some(bolt), Claim::GiveUp)
}

/// Where an effect faces when it comes from a surface hit.
pub fn basis_off_contact_facing(normal: [f32; 3]) -> [[f32; 3]; 3] {
    contact_facing_basis(normal)
}

/// An effect started by something hitting or dying is played like any other.
pub fn play_hit_or_death_effect(host: &mut EffectSystemHost, req: EffectStartAsk<'_>) -> PlaybackResult {
    start(host, req, None, Claim::GiveUp)
}

/// Whether the caller keeps its claim on a started effect.
#[derive(Clone, Copy)]
enum Claim {
    Keep,
    GiveUp,
}

/// Puts a freshly spawned effect on the bone it follows.
fn pin_to_bone(effect: &mut LiveEffect, bolt: EffectBoltTarget) {
    let teleport = bolt_centity_teleport_for_compare(bolt.dobj, bolt.client_actor_teleport);
    let AttachPointFrame { origin: pos, axis } = bolt.orientation;
    effect.tether_bits = bolt_pack(bolt.dobj, teleport, u32::from(bolt.bone));
    effect.bolt_teleported = teleport;
    effect.tether_pose = Some((pos, axis));
    (effect.where_at, effect.orient_rows) = (pos, axis);
}

/// Spawns the slot, pins it to its bone if it has one, starts its parts,
/// then keeps or gives up the caller's claim.
fn start(host: &mut EffectSystemHost, req: EffectStartAsk<'_>, bone: Option<EffectBoltTarget>, claim: Claim) -> PlaybackResult {
    let EffectStartAsk { definition_name, pose: pose_v, needs_cone_glow, def, ledger_index } = req;
    let dobj = bone.map_or(BIRTH_UNATTACHED, |b| b.dobj);
    let handle = match host.start_effect(definition_name, pose_v.anchor, pose_v.frame, pose_v.ms_now, dobj, needs_cone_glow, ledger_index) {
        Err(refusal) => return PlaybackResult::Refused(refusal),
        Ok(h) => h,
    };

    if let Some(b) = bone {
        if let Some(effect) = host.bin_of_ticket_editable(handle) {
            pin_to_bone(effect, b);
        }
    }
    if let Some(parts) = def {
        launch_fresh_effect(host, handle, parts);
    }

    match claim {
        Claim::Keep => PlaybackResult::Kept { handle },
        Claim::GiveUp => {
            host.release_play_ownership(handle);
            PlaybackResult::StartedLetGo { handle }
        }
    }
}
