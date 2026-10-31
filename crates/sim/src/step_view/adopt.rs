//! Making the rows match a snapshot that was just adopted into `MatchSimulation`.

use std::collections::HashSet;

use bevy_ecs::prelude::{Entity, World};
use playerstate_t6::game::AvatarState;

use super::rows::{DroppedItemRow, PlayerRow, ProjectileRow, ScriptMoverRow};
use super::store::{despawn_row, index, index_mut, spawn_player, upsert_row};
use crate::FlyingProjectile;
use crate::ent_slots::MoverSlot;
use crate::floor_guns::LootOnGround;
use crate::sim_parts::SeatNo;

/// Brings the rows in line with the given payloads: rows for anything not listed go, listed ones are overwritten in
/// place, and new ones are created. Players are matched by client, everything else by gentity number.
pub(super) fn reconcile(
    rows: &mut World,
    seated: &[(SeatNo, AvatarState)],
    shots: Vec<FlyingProjectile>,
    script_movers: Vec<MoverSlot>,
    ground_items: Vec<LootOnGround>,
) {
    reconcile_players(rows, seated);

    let kept: HashSet<i32> = shots
        .iter()
        .map(|p| p.ent_no)
        .chain(script_movers.iter().map(|m| m.state.slot_index))
        .chain(ground_items.iter().map(|i| i.state.slot_index))
        .collect();
    drop_numbered_except(rows, &kept);

    shots.into_iter().for_each(|shot| upsert_row::<ProjectileRow>(rows, shot));
    script_movers.into_iter().for_each(|found| upsert_row::<ScriptMoverRow>(rows, found));
    ground_items.into_iter().for_each(|dropped| upsert_row::<DroppedItemRow>(rows, dropped));
}

fn reconcile_players(world: &mut World, avatars: &[(SeatNo, AvatarState)]) {
    let gone: Vec<(SeatNo, Entity)> = index(world)
        .clients()
        .iter()
        .copied()
        .filter(|(held, _)| avatars.iter().all(|(wanted, _)| wanted != held))
        .collect();
    for (who, row) in gone {
        index_mut(world).release_client(who);
        despawn_row(world, row, "player");
    }

    for &(who, ent_state) in avatars {
        if let Some(row) = index(world).client_row(who) {
            if let Some(mut held) = world.get_mut::<PlayerRow>(row) {
                held.state = ent_state;
                continue;
            }
            // The bound entity is not a player row; start it over.
            index_mut(world).release_client(who);
            despawn_row(world, row, "player");
        }
        spawn_player(world, who, ent_state);
    }
}

fn drop_numbered_except(world: &mut World, kept: &HashSet<i32>) {
    let gone: Vec<(Entity, i32)> = index(world).numbered_rows().filter(|(_, slot_no)| !kept.contains(slot_no)).collect();
    for (row, slot_no) in gone {
        index_mut(world).release_numbered(slot_no);
        despawn_row(world, row, "numbered payload");
    }
}
