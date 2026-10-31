//! Combining the host's projectiles with the local player's predicted throws.

use std::collections::HashSet;

use sim::{ClientId, FlyingProjectile};

use super::ShownProjectile;

/// All of the host's projectiles, then the local player's predicted ones the host has not
/// shown yet. A prediction is dropped once the host reports a projectile of the same weapon
/// owned by the local player; other players' predictions are never shown.
pub fn combine_shown_missiles(
    authoritative: &[FlyingProjectile],
    predicted: &[FlyingProjectile],
    own_peer: ClientId,
) -> Vec<ShownProjectile> {
    let reported: HashSet<u32> = authoritative
        .iter()
        .filter_map(|host| (host.owner == own_peer).then_some(host.gun))
        .collect();
    let still_unreported =
        |guess: &&FlyingProjectile| guess.owner == own_peer && !reported.contains(&guess.gun);

    let from_host = authoritative
        .iter()
        .map(|host| ShownProjectile::HostDecided(*host));
    let from_guess = predicted.iter().filter(still_unreported).map(|guess| {
        let FlyingProjectile {
            owner,
            gun: weapon,
            origin,
            motion: velocity,
            at: pos,
            turn_path: apos,
            spawn_moment,
            ..
        } = *guess;
        ShownProjectile::Predicted {
            thrower: owner,
            kit: weapon,
            start_point: origin,
            motion: velocity,
            path: pos,
            spin: apos,
            spawn_moment,
        }
    });
    from_host.chain(from_guess).collect()
}

/// How many rows are `thrower`'s throws of `kit`, predicted or not.
pub fn throws_shown_for(rows: &[ShownProjectile], thrower: ClientId, kit: u32) -> usize {
    let mut tally = 0;
    for shown in rows {
        tally += usize::from(shown.owner() == thrower && shown.weapon() == kit);
    }
    tally
}

#[cfg(test)]
mod tests {
    use super::super::tests::projectile;
    use super::*;

    const LOCAL: ClientId = ClientId(1);

    #[test]
    fn the_hosts_projectiles_come_first_in_the_order_given() {
        let host = [projectile(5, 2, 10), projectile(4, 1, 11)];
        let rows = combine_shown_missiles(&host, &[], LOCAL);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].host_assigned_id().map(|id| id.0), Some(5));
        assert_eq!(rows[1].host_assigned_id().map(|id| id.0), Some(4));
    }

    #[test]
    fn a_local_prediction_is_shown_until_the_host_reports_that_weapon() {
        let predicted = [projectile(0, 1, 20)];
        let rows = combine_shown_missiles(&[], &predicted, LOCAL);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].host_assigned_id(), None);
        assert_eq!((rows[0].owner(), rows[0].weapon()), (LOCAL, 20));
        assert_eq!(rows[0].spawn_moment(), predicted[0].spawn_moment);
        assert_eq!(rows[0].path(), predicted[0].at);

        let host = [projectile(9, 1, 20)];
        let rows = combine_shown_missiles(&host, &predicted, LOCAL);
        assert_eq!(rows.len(), 1, "the host's own row replaces the prediction");
        assert_eq!(rows[0].host_assigned_id().map(|id| id.0), Some(9));
    }

    #[test]
    fn a_different_weapon_or_owner_does_not_confirm_a_prediction() {
        let predicted = [projectile(0, 1, 20)];
        let host = [projectile(9, 1, 21), projectile(8, 2, 20)];
        let rows = combine_shown_missiles(&host, &predicted, LOCAL);
        assert_eq!(rows.len(), 3);
        assert_eq!(throws_shown_for(&rows, LOCAL, 20), 1);
    }

    #[test]
    fn other_players_predictions_are_never_shown() {
        let predicted = [projectile(0, 2, 20)];
        assert!(combine_shown_missiles(&[], &predicted, LOCAL).is_empty());
    }

    #[test]
    fn counting_matches_owner_and_weapon_over_both_kinds_of_row() {
        let host = [
            projectile(1, 1, 20),
            projectile(2, 1, 20),
            projectile(3, 1, 21),
            projectile(4, 2, 20),
        ];
        let predicted = [projectile(0, 1, 22)];
        let rows = combine_shown_missiles(&host, &predicted, LOCAL);
        assert_eq!(throws_shown_for(&rows, LOCAL, 20), 2);
        assert_eq!(throws_shown_for(&rows, LOCAL, 22), 1);
        assert_eq!(throws_shown_for(&rows, ClientId(2), 20), 1);
        assert_eq!(throws_shown_for(&rows, ClientId(9), 20), 0);
        assert_eq!(throws_shown_for(&[], LOCAL, 20), 0);
    }
}
