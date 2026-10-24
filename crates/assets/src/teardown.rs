//! Lets go of what a match loaded once the match is over.

use bevy::prelude::*;
use frame::{ClientSet, MatchTornDown, ReturnedToMenu, SessionSwapApplied};

use crate::{
    MatchType10SoundHints, PreparedBodies, PreparedDestructibleDeath, PreparedFpvMeshes,
    PreparedLocalizedStrings, PreparedProjectileMeshes, PreparedWeapons, PreparedWorldWeapons,
    PreparedXAnims, PreparedXModelWalkCensus,
};
use asset_audio::SessionMapScriptSound;
use asset_game::SessionTeamSettings;
use asset_material::AssetRefDumpCensus;
use asset_world::MapXModelSceneCatalog;

/// Wires the clean-up into the client's load phase, after a swapped-in
/// session has been applied.
pub(crate) fn register_match_teardown(app: &mut App) {
    app.add_message::<ReturnedToMenu>().add_systems(
        Update,
        retire_when_match_ends
            .after(SessionSwapApplied)
            .in_set(ClientSet::Load),
    );
}

/// A match ends by being torn down or by the player going back to the menu.
/// Both queues are drained every frame, so a message never lingers into the
/// next frame.
fn retire_when_match_ends(
    mut torn: MessageReader<MatchTornDown>,
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
            .resource::<PreparedWeapons>()
            .resource::<PreparedWorldWeapons>()
            .resource::<PreparedFpvMeshes>()
            .resource::<PreparedBodies>()
            .resource::<PreparedProjectileMeshes>()
            .resource::<PreparedXAnims>()
            .resource::<PreparedXModelWalkCensus>()
            .resource::<PreparedDestructibleDeath>()
            .resource::<PreparedLocalizedStrings>()
            .resource::<MatchType10SoundHints>()
            .resource::<SessionMapScriptSound>()
            .resource::<SessionTeamSettings>()
            .resource::<MapXModelSceneCatalog>()
            .resource::<AssetRefDumpCensus>();
    });
}
