use playerstate_t6::game::rebase_archived_timers;
use sim::{ClientId, Snapshot};

use crate::avatar_entity_row;

/// Move a time that is in use (non-zero) by `delta_ms`; a zero time means "not set" and stays.
fn shift_if_set(time: &mut i32, shift_ms: i32) {
    if *time != 0 {
        *time = time.wrapping_add(shift_ms);
    }
}

pub(super) fn rebase_entity_state(es: &mut entity_t6::EntitySnapshot, shift_ms: i32) {
    shift_if_set(&mut es.lerp.pos.began_at, shift_ms);
    shift_if_set(&mut es.lerp.apos.began_at, shift_ms);
    shift_if_set(&mut es.aux_time, shift_ms);
    // A general entity's first data word and a missile's launch time are the same storage.
    if matches!(es.e_type, entity_t6::et::GENERAL | entity_t6::et::MISSILE) {
        let mut missile = es.lerp.u.missile();
        missile.spawn_moment = missile.spawn_moment.wrapping_add(shift_ms);
        es.lerp.u.set_missile(&missile);
    }
}

/// Move everything in an archived world that carries a time forward by `delta_ms`, so it runs on
/// the live clock. The beholder's own row is left alone: it is the live one.
pub(super) fn rebase_archived_world(shown: &mut Snapshot, beholder: ClientId, shift_ms: i32) {
    for es in &mut shown.meta.entities {
        rebase_entity_state(es, shift_ms);
    }
    // The typed movers must still equal their entity rows when the client adopts them.
    for mover in &mut shown.meta.script_movers {
        rebase_entity_state(&mut mover.state, shift_ms);
    }
    for shot in &mut shown.projectiles {
        shift_if_set(&mut shot.at.began_at, shift_ms);
        shift_if_set(&mut shot.turn_path.began_at, shift_ms);
        shot.spawn_moment = shot.spawn_moment.wrapping_add(shift_ms);
        shot.spawn_time_ms = shot.spawn_time_ms.wrapping_add(shift_ms);
        shot.detonate_at_ms = shot.detonate_at_ms.map(|at| at.wrapping_add(shift_ms));
        shot.cleanup_at_ms = shot.cleanup_at_ms.wrapping_add(shift_ms);
    }
    // A body that went limp during the replay falls by the replay's clock; by the live one it
    // would already be seconds into its fall, far below the map.
    for slot in &mut shown.meta.corpses.slots {
        shift_if_set(&mut slot.tr_time, shift_ms);
    }
    for (id, state) in &mut shown.players {
        if *id != beholder {
            rebase_archived_timers(state, shift_ms);
        }
    }
}

/// The archived frame as the world of a killcam: its entities and players on the live clock, the
/// match state and HUD text of the live game, and only the beholder's own rows from the live game.
pub(super) fn overlay_archived_world(present: &Snapshot, recorded: &Snapshot, beholder: ClientId, focus: ClientId) -> Snapshot {
    let mut stage = recorded.clone();
    stage.tick = present.tick;
    let (meta, now) = (&mut stage.meta, &present.meta);

    // Only what the focused player could see, and now addressed to the beholder.
    meta.entity_events.retain(|record| record.audience.is_visible_to(focus));
    meta.entity_events.iter_mut().for_each(|record| record.audience = sim::EventRecipients::Client(beholder));

    // The match state and HUD text are the live game's.
    (meta.phase, meta.match_clock_ms, meta.prematch) = (now.phase, now.match_clock_ms, now.prematch);
    (meta.score_limit, meta.time_limit_ms, meta.kind) = (now.score_limit, now.time_limit_ms, now.kind);
    meta.hud_strings.clone_from(&now.hud_strings);
    meta.hud_materials.clone_from(&now.hud_materials);

    // The beholder stood somewhere in the archived frame; keep them in the world as an entity.
    if let Some(found) = recorded.players.iter().find(|(who, _)| *who == beholder) {
        let stand_in = avatar_entity_row(beholder, &found.1);
        let held = meta.entities.iter().any(|entity| entity.slot_index == stand_in.slot_index);
        if !held {
            meta.entities.push(stand_in);
        }
    }

    // The beholder's own player row and client row are the live ones.
    use_live_row(&mut stage.players, &present.players, beholder);
    use_live_row(&mut stage.meta.clients, &present.meta.clients, beholder);

    let shift_ms = sim::level_time_ms(present.tick).wrapping_sub(sim::level_time_ms(recorded.tick));
    rebase_archived_world(&mut stage, beholder, shift_ms);
    stage
}

/// Drops `beholder`'s row from `rows` and appends its row from `live_rows`, if it has one.
fn use_live_row<T: Clone>(rows: &mut Vec<(ClientId, T)>, live_rows: &[(ClientId, T)], beholder: ClientId) {
    rows.retain(|row| row.0 != beholder);
    rows.extend(live_rows.iter().find(|row| row.0 == beholder).cloned());
}

#[cfg(test)]
mod tests {
    use playerstate_t6::game::AvatarState;
    use sim::Tick;

    use super::*;

    #[test]
    fn a_time_that_is_set_moves_and_one_that_is_not_stays() {
        let (mut set, mut unset) = (1000, 0);
        shift_if_set(&mut set, 250);
        shift_if_set(&mut unset, 250);
        assert_eq!((set, unset), (1250, 0));
    }

    #[test]
    fn a_time_at_the_end_of_the_range_wraps_instead_of_panicking() {
        let mut time = i32::MAX;
        shift_if_set(&mut time, 1);
        assert_eq!(time, i32::MIN);
    }

    #[test]
    fn an_entity_state_moves_each_of_its_clocks_that_is_set() {
        let mut es = entity_t6::EntitySnapshot::default();
        es.lerp.pos.began_at = 100;
        es.lerp.apos.began_at = 0;
        es.aux_time = 40;
        es.e_type = entity_t6::et::GENERAL;
        rebase_entity_state(&mut es, 500);
        assert_eq!(es.lerp.pos.began_at, 600);
        assert_eq!(es.lerp.apos.began_at, 0);
        assert_eq!(es.aux_time, 540);
        assert_eq!(es.lerp.u.missile().spawn_moment, 500);
    }

    #[test]
    fn only_general_and_missile_entities_have_a_launch_time_to_move() {
        let mut other = entity_t6::EntitySnapshot::default();
        other.e_type = entity_t6::et::GENERAL + 1;
        let before = other.lerp.u.missile().spawn_moment;
        rebase_entity_state(&mut other, 500);
        if other.e_type != entity_t6::et::MISSILE {
            assert_eq!(other.lerp.u.missile().spawn_moment, before);
        }
    }

    #[test]
    fn rebasing_a_world_spares_the_viewers_own_row() {
        let (beholder, other) = (ClientId(1), ClientId(2));
        let mut stage = Snapshot::unpublished(Tick(10));
        let mut moving = AvatarState::ZERO;
        moving.last_cmd_ms = 1000;
        stage.players.push((beholder, moving));
        stage.players.push((other, moving));
        rebase_archived_world(&mut stage, beholder, 300);
        assert_eq!(stage.players[0].1.last_cmd_ms, 1000);
        assert_eq!(stage.players[1].1.last_cmd_ms, 1300);
    }

    #[test]
    fn the_overlay_keeps_the_live_players_row_and_the_live_tick() {
        let (beholder, focus) = (ClientId(1), ClientId(2));
        let mut present = Snapshot::unpublished(Tick(60));
        let mut live_state = AvatarState::ZERO;
        live_state.last_cmd_ms = 777;
        present.players.push((beholder, live_state));

        let mut recorded = Snapshot::unpublished(Tick(20));
        let mut old_state = AvatarState::ZERO;
        old_state.last_cmd_ms = 1;
        recorded.players.push((beholder, old_state));
        recorded.players.push((focus, old_state));

        let shown = overlay_archived_world(&present, &recorded, beholder, focus);
        assert_eq!(shown.tick, present.tick);
        let beholder_rows: Vec<_> = shown.players.iter().filter(|(who, _)| *who == beholder).collect();
        assert_eq!(beholder_rows.len(), 1);
        assert_eq!(beholder_rows[0].1.last_cmd_ms, 777);
        assert!(shown.players.iter().any(|(who, _)| *who == focus));
    }
}
