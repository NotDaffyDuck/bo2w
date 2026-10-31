//! bo2zm: a player who bled out in co-op sees through the teammate he follows. BO2 copies that
//! player's state into his (`SpectatorClientEndFrame`); his client draws it as a seat (no
//! prediction), with no killcam.

use playerstate_t6::game::{NO_ENTITY_SLOT, MOVE_KIND_DEAD, SpectatorTarget, apply_killcam_seat};
use sim::{ClientId, Snapshot};

/// The client dvars that follow the watched teammate: his grenades and action slots.
const FOLLOWED_DVARS: [&str; 2] = ["bo2zm_offhand", "bo2zm_actionslots"];

pub(super) fn follow_teammate(shown: &mut Snapshot, beholder: ClientId) {
    let Some(&(_, followed)) = shown.meta.follows.iter().find(|(v, _)| *v == beholder) else {
        return;
    };
    // Not a dead or third-person teammate: a killcam seat's third-person camera has no archive
    // clock.
    let spectating = playerstate_t6::flags::secondary_state_bits::SPECTATING as u32;
    let Some(seen) = shown
        .players
        .iter()
        .find(|(who, _)| *who == followed)
        .map(|(_, row)| *row)
        .filter(|ps| ps.movement_mode < MOVE_KIND_DEAD && ps.secondary_state_bits & spectating == 0)
    else {
        return;
    };
    let focus = SpectatorTarget {
        death_replay_target: NO_ENTITY_SLOT,
        death_replay_focus: NO_ENTITY_SLOT,
        death_replay_player: followed.0 as i32,
    };
    if let Some(entry) = shown.players.iter_mut().find(|(who, _)| *who == beholder) {
        entry.1 = apply_killcam_seat(&entry.1, &seen, focus);
    }
    // The copy carries his ammo and grenades too, so the watcher's HUD counts the teammate's
    // clip, stock and grenades. His points and the hint stay the watcher's own (BO2 rebuilds
    // the current hud elems for him).
    let Some((mag, reserve, by_weapon, dvars)) = shown.meta.for_client(followed).map(|m| {
        let dvars: Vec<(String, String)> = m
            .client_dvars
            .iter()
            .filter(|(k, _)| FOLLOWED_DVARS.contains(&k.as_str()))
            .cloned()
            .collect();
        (m.mag_rounds, m.ammo_stock, m.rounds_per_gun.clone(), dvars)
    }) else {
        return;
    };
    let Some(slot) = shown.meta.clients.iter_mut().find(|(who, _)| *who == beholder) else {
        return;
    };
    let own = &mut slot.1;
    own.mag_rounds = mag;
    own.ammo_stock = reserve;
    own.rounds_per_gun = by_weapon;
    for (name, value) in dvars {
        match own.client_dvars.iter_mut().find(|(k, _)| *k == name) {
            Some(row) => row.1 = value,
            None => own.client_dvars.push((name, value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playerstate_t6::game::AvatarState;
    use sim::{ClientSnapshotMeta, Tick};

    fn meta(rounds: i32, offhand: &str) -> ClientSnapshotMeta {
        let mut row = ClientSnapshotMeta::default();
        row.mag_rounds = rounds;
        row.ammo_stock = rounds * 10;
        row.score = rounds * 100;
        row.client_dvars = vec![("bo2zm_offhand".to_owned(), offhand.to_owned())];
        row
    }

    #[test]
    fn a_watcher_counts_the_teammates_ammo_but_keeps_his_points() {
        let (host, watcher) = (ClientId(0), ClientId(1));
        let mut shown = Snapshot::unpublished(Tick(1));
        shown.players = vec![(host, AvatarState::ZERO), (watcher, AvatarState::ZERO)];
        shown.meta.clients = vec![(host, meta(30, "frag_grenade_zm:2")), (watcher, meta(8, ""))];
        shown.meta.follows = vec![(watcher, host)];
        follow_teammate(&mut shown, watcher);
        let mine = shown.meta.for_client(watcher).unwrap();
        assert_eq!((mine.mag_rounds, mine.ammo_stock, mine.score), (30, 300, 800));
        assert_eq!(shown.meta.script_dvars(watcher).string("bo2zm_offhand"), Some("frag_grenade_zm:2"));
    }
}
