//! Lets go of what a match loaded once the match is over.

use bevy::prelude::*;
use frame::{ClientStep, GameSessionClosed, ReturnedToMenu, MatchSwapDone};

use crate::{
    RoundTenthKindAudioClues, ReadiedCharacterBodies, ReadiedBreakableDeath, ReadiedFpvMeshes,
    ReadiedLocalText, ReadiedProjectileMeshes, StagedWeapons, ReadiedWorldWeapons,
    StagedXAnims, ReadiedXModelWalkCensus,
};
use asset_audio::SessionMapScriptSound;
use asset_game::SessionTeamSettings;
use asset_material::AssetPointerDumpTally;
use asset_world::LevelModelPlacementRoster;

/// Wires the clean-up into the client's load phase, after a swapped-in
/// session has been applied.
pub(crate) fn hook_round_cleanup(app: &mut App) {
    app.add_message::<ReturnedToMenu>().add_systems(
        Update,
        retire_when_match_ends
            .after(MatchSwapDone)
            .in_set(ClientStep::Load),
    );
}

/// A match ends by being torn down or by the player going back to the menu.
/// Both queues are drained every frame, so a message never lingers into the
/// next frame.
fn retire_when_match_ends(
    mut torn: MessageReader<GameSessionClosed>,
    mut returned: MessageReader<ReturnedToMenu>,
    mut commands: Commands,
) {
    let ended = torn.read().count() + returned.read().count() > 0;
    if ended {
        commands.queue(retire_match_resources);
    }
}

fn retire_match_resources(world: &mut World) {
    frame::retire::retire_resources(world, |batch| {
        batch
            .resource::<StagedWeapons>()
            .resource::<ReadiedWorldWeapons>()
            .resource::<ReadiedFpvMeshes>()
            .resource::<ReadiedCharacterBodies>()
            .resource::<ReadiedProjectileMeshes>()
            .resource::<StagedXAnims>()
            .resource::<ReadiedXModelWalkCensus>()
            .resource::<ReadiedBreakableDeath>()
            .resource::<ReadiedLocalText>()
            .resource::<RoundTenthKindAudioClues>()
            .resource::<SessionMapScriptSound>()
            .resource::<SessionTeamSettings>()
            .resource::<LevelModelPlacementRoster>()
            .resource::<AssetPointerDumpTally>();
    });
}
