//! bo2zm: his own dive and voice. Black Ops II's player plays the
//! dive-to-prone launch whoosh when the dive starts, the landing thud
//! plus a grunt (the client script's `dtplandsoundplayer` exert) when he
//! touches down, and a pain grunt when he is hurt. Our movement keeps the
//! dive in his playerstate flags, so this watches those flags (and his
//! health) and plays BO2's own aliases, 2D, as the player-only `_plr`
//! sounds are.

use bevy::prelude::*;
use net::{HostViewClient, ShownWorldFrame};

use crate::{AliasCommand, TriggerCueMsg};

use movement_t6::dive::{PMF_DIVE, sliding};

/// A pain grunt at most this often, as BO2's exert cooldown.
const PAIN_GAP_SECS: f32 = 0.6;

#[derive(Default)]
pub(crate) struct LastSeen {
    client: Option<u32>,
    flags: u32,
    sliding: bool,
    hp: i32,
    pain_at: f32,
}

fn t6(cue_id: &str) -> AliasCommand {
    AliasCommand::Play(TriggerCueMsg {
        cue_ns: asset_core::AssetCategoryScope::T6,
        alias: cue_id.to_owned(),
        fallback: None,
        position_in_inches: None,
        source_entity_no: None,
    })
}

pub(crate) fn sync_dive_vox(
    presented: Option<Res<ShownWorldFrame>>,
    viewer: Option<Res<HostViewClient>>,
    time: Res<Time>,
    mut last: Local<LastSeen>,
    mut play: MessageWriter<AliasCommand>,
) {
    let (Some(shown), Some(viewer)) = (presented, viewer) else {
        last.client = None;
        return;
    };
    let Some(pstate) = shown.player(viewer.0) else {
        last.client = None;
        return;
    };
    let now = time.elapsed_secs();
    if last.client != Some(viewer.0.0) {
        // A new match or a new view: take the state as it is, no sounds.
        *last = LastSeen {
            client: Some(viewer.0.0),
            flags: pstate.movement_state_bits as u32,
            sliding: sliding(pstate),
            hp: pstate.health,
            pain_at: now,
        };
        return;
    }
    let rose = |flag: u32| (pstate.movement_state_bits as u32) & flag != 0 && last.flags & flag == 0;
    let slid = sliding(pstate);
    if rose(PMF_DIVE) {
        // Its secondary is the character's launch effort.
        play.write(t6("fly_dtp_launch_plr"));
        diag::info!(Audio, "sound: dive start vox");
    }
    if slid && !last.sliding {
        play.write(t6("fly_dtp_land_plr_default"));
        play.write(t6("vox_exert_generic_grunt"));
        diag::info!(Audio, "sound: dive landing vox");
    }
    if pstate.health < last.hp && last.hp > 0 && now - last.pain_at >= PAIN_GAP_SECS {
        play.write(t6("vox_exert_generic_pain"));
        diag::info!(Audio, "sound: hurt grunt, hp {} to {}", last.hp, pstate.health);
        last.pain_at = now;
    }
    last.flags = pstate.movement_state_bits as u32;
    last.sliding = slid;
    last.hp = pstate.health;
}
