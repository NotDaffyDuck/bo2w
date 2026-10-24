//! Making the rows match a snapshot that was just adopted into `SimState`.

use std::collections::HashSet;

use bevy_ecs::prelude::{Entity, World};
use playerstate_t6::game::PlayerState;

use super::rows::{DroppedItemRow, PlayerRow, ProjectileRow, ScriptMoverRow};
use super::store::{despawn_row, index, index_mut, spawn_player, upsert_row};
use crate::ProjectileState;
use crate::gentity::ScriptMoverGentity;
use crate::item::DroppedItem;
use crate::world::ClientId;

/// Brings the rows in line with the given payloads: rows for anything not listed go, listed ones are overwritten in
/// place, and new ones are created. Players are matched by client, everything else by gentity number.
pub(super) fn reconcile(
    world: &mut World,
    players: &[(ClientId, PlayerState)],
    projectiles: Vec<ProjectileState>,
    movers: Vec<ScriptMoverGentity>,
    items: Vec<DroppedItem>,
) {
    reconcile_players(world, players);

    let kept: HashSet<i32> = projectiles
        .iter()
        .map(|p| p.entnum)
        .chain(movers.iter().map(|m| m.state.number))
        .chain(items.iter().map(|i| i.state.number))
        .collect();
    drop_numbered_except(world, &kept);

    for projectile in projectiles {
        upsert_row::<ProjectileRow>(world, projectile);
    }
    for mover in movers {
        upsert_row::<ScriptMoverRow>(world, mover);
    }
    for item in items {
        upsert_row::<DroppedItemRow>(world, item);
    }
}

fn reconcile_players(world: &mut World, players: &[(ClientId, PlayerState)]) {
    let gone: Vec<(ClientId, Entity)> = index(world)
        .clients()
        .iter()
        .copied()
        .filter(|(held, _)| players.iter().all(|(wanted, _)| wanted != held))
        .collect();
    for (client, row) in gone {
        index_mut(world).release_client(client);
        despawn_row(world, row, "player");
    }

    for &(client, state) in players {
        if let Some(row) = index(world).client_row(client) {
            if let Some(mut held) = world.get_mut::<PlayerRow>(row) {
                held.state = state;
                continue;
            }
            // The bound entity is not a player row; start it over.
            index_mut(world).release_client(client);
            despawn_row(world, row, "player");
        }
        spawn_player(world, client, state);
    }
}

fn drop_numbered_except(world: &mut World, kept: &HashSet<i32>) {
    let gone: Vec<(i32, Entity)> = index(world).numbered_rows().filter(|(number, _)| !kept.contains(number)).collect();
    for (number, row) in gone {
        index_mut(world).release_numbered(number);
        despawn_row(world, row, "numbered payload");
    }
}
