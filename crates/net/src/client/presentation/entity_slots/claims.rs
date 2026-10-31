//! Who wants which entity number in one snapshot. Players come first, then corpses, then the
//! entities the server replicates; when two want the same number the first keeps it and the
//! clash is recorded as a gap.

use playerstate_t6::game::{BODY_ENTITY_FIRST_SLOT, MOVE_MODE_INTERLUDE, AvatarState};
use sim::{ClientId, PlayerCorpsePool, PlayerCorpseSlot};

use super::ENTITY_NUMBER_COUNT;
use crate::client::presentation::entity_records::{
    cadaver_slot_to_object_state, avatar_entity_row,
};
use crate::identity_slips::{LinkDeficiencyOrigin, LinkIdentityDeficits};

/// What stands in a slot: its owner (if any) and the state it should be in.
#[derive(Clone, Copy)]
pub(super) struct Occupant {
    pub(super) owner: Option<ClientId>,
    pub(super) state: entity_t6::EntitySnapshot,
}

/// The numbers queued so far, in the order they were claimed.
pub(super) struct Claims {
    taken: [bool; ENTITY_NUMBER_COUNT],
    pub(super) queued: Vec<(u16, Occupant)>,
}

impl Claims {
    fn empty() -> Self {
        Claims {
            taken: [false; ENTITY_NUMBER_COUNT],
            queued: Vec::with_capacity(64),
        }
    }

    pub(super) fn is_taken(&self, number: usize) -> bool {
        self.taken.get(number).copied().unwrap_or(false)
    }

    /// Records the claim; when the number already has a holder the newcomer is dropped and the
    /// clash is raised as a gap.
    fn claim(&mut self, slot_no: u16, holder: Occupant, deficits: &mut LinkIdentityDeficits) {
        if std::mem::replace(&mut self.taken[usize::from(slot_no)], true) {
            deficits.raise(LinkDeficiencyOrigin::NumberClaimedTwice { number: slot_no });
        } else {
            self.queued.push((slot_no, holder));
        }
    }
}

/// A player's entity number is their client number, as long as the table has room for it.
fn participant_object_index(peer: ClientId) -> Option<u16> {
    slot_number(i32::try_from(peer.0).ok()?)
}

/// The entity types the server replicates as table entries.
fn occupies_a_slot(entity: &entity_t6::EntitySnapshot) -> bool {
    matches!(
        entity.e_type,
        entity_t6::et::SCRIPTMOVER
            | entity_t6::et::MISSILE
            | entity_t6::et::ITEM
            | entity_t6::et::PLAYER
    )
}

/// The number if it is one the table has.
fn slot_number(raw: i32) -> Option<u16> {
    u16::try_from(raw)
        .ok()
        .filter(|slot_no| usize::from(*slot_no) < ENTITY_NUMBER_COUNT)
}

fn corpse_is_shown(slot: &PlayerCorpseSlot) -> bool {
    slot.occupied && slot.entnum >= BODY_ENTITY_FIRST_SLOT
}

/// Collect the claims of one snapshot. `seated_viewer`, when given, is a player whose body is not
/// shown because the view sits inside it.
pub(super) fn collect(
    roster: &[(ClientId, AvatarState)],
    fallen: &PlayerCorpsePool,
    entities: &[entity_t6::EntitySnapshot],
    seated_viewer: Option<ClientId>,
    deficits: &mut LinkIdentityDeficits,
) -> Claims {
    let mut claims = Claims::empty();

    // A client at the intermission is not in the world: the end scoreboard's camera stands where
    // they all stand, inside their bodies.
    let in_world = |(peer, avatar): &&(ClientId, AvatarState)| {
        Some(*peer) != seated_viewer && avatar.movement_mode != MOVE_MODE_INTERLUDE
    };
    for (peer, avatar) in roster.iter().filter(in_world) {
        match participant_object_index(*peer) {
            Some(slot_no) => {
                let state = avatar_entity_row(*peer, avatar);
                claims.claim(slot_no, Occupant { owner: Some(*peer), state }, deficits);
            }
            None => deficits.raise(LinkDeficiencyOrigin::ClientOutOfNumberSpace { client: *peer }),
        }
    }

    let shown_corpses = fallen.slots.iter().filter(|slot| corpse_is_shown(slot));
    for (slot_no, slot) in shown_corpses.filter_map(|s| Some((slot_number(s.entnum)?, s))) {
        let occupant = Occupant {
            state: cadaver_slot_to_object_state(slot),
            owner: Some(slot.victim),
        };
        claims.claim(slot_no, occupant, deficits);
    }

    let replicated = entities.iter().filter(|e| occupies_a_slot(e));
    for (slot_no, entity) in replicated.filter_map(|e| Some((slot_number(e.slot_index)?, e))) {
        let owner = ClientId(u32::from(entity.client_num));
        let holder = (entity.e_type == entity_t6::et::PLAYER).then_some(owner);
        claims.claim(
            slot_no,
            Occupant {
                owner: holder,
                state: *entity,
            },
            deficits,
        );
    }

    claims
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity_slips::LinkDeficiency;

    fn alive(movement_mode: i32) -> AvatarState {
        let mut ps = AvatarState::ZERO;
        ps.movement_mode = movement_mode;
        ps
    }

    fn numbers(claims: &Claims) -> Vec<u16> {
        claims.queued.iter().map(|(slot_no, _)| *slot_no).collect()
    }

    fn entity(number: i32, e_type: i16) -> entity_t6::EntitySnapshot {
        entity_t6::EntitySnapshot {
            slot_index: number,
            e_type,
            ..Default::default()
        }
    }

    fn corpse(entnum: i32, victim: u32) -> PlayerCorpsePool {
        let mut pool = PlayerCorpsePool::default();
        pool.slots[0].occupied = true;
        pool.slots[0].entnum = entnum;
        pool.slots[0].victim = ClientId(victim);
        pool
    }

    fn collect_players(
        roster: &[(ClientId, AvatarState)],
        seated: Option<ClientId>,
        deficits: &mut LinkIdentityDeficits,
    ) -> Claims {
        collect(roster, &PlayerCorpsePool::default(), &[], seated, deficits)
    }

    #[test]
    fn a_player_takes_the_number_of_their_client() {
        let mut deficits = LinkIdentityDeficits::default();
        let claims = collect_players(&[(ClientId(5), alive(0))], None, &mut deficits);
        assert_eq!(numbers(&claims), [5]);
        assert_eq!(claims.queued[0].1.owner, Some(ClientId(5)));
        assert!(claims.is_taken(5));
        assert!(!claims.is_taken(4));
        assert_eq!(
            deficits.count(),
            LinkIdentityDeficits::default().count(),
            "only the standing gap"
        );
    }

    #[test]
    fn the_seated_viewer_and_intermission_players_are_left_out() {
        let mut deficits = LinkIdentityDeficits::default();
        let players = [
            (ClientId(1), alive(0)),
            (ClientId(2), alive(0)),
            (ClientId(3), alive(MOVE_MODE_INTERLUDE)),
        ];
        let claims = collect_players(&players, Some(ClientId(1)), &mut deficits);
        assert_eq!(numbers(&claims), [2]);
        assert_eq!(
            deficits.count(),
            LinkIdentityDeficits::default().count(),
            "only the standing gap"
        );
    }

    #[test]
    fn a_client_beyond_the_number_space_is_a_gap_not_a_claim() {
        let mut deficits = LinkIdentityDeficits::default();
        let far = ClientId(ENTITY_NUMBER_COUNT as u32);
        let claims = collect_players(&[(far, alive(0))], None, &mut deficits);
        assert!(claims.queued.is_empty());
        assert!(deficits.is_live(LinkDeficiency::ClientEntityIndex));
        assert_eq!(
            deficits.cause(LinkDeficiency::ClientEntityIndex),
            Some(&LinkDeficiencyOrigin::ClientOutOfNumberSpace { client: far })
        );
    }

    #[test]
    fn corpses_count_only_when_occupied_and_above_the_corpse_base() {
        let mut deficits = LinkIdentityDeficits::default();
        let base = BODY_ENTITY_FIRST_SLOT;
        let shown = collect(&[], &corpse(base + 2, 4), &[], None, &mut deficits);
        assert_eq!(numbers(&shown), [(base + 2) as u16]);
        assert_eq!(shown.queued[0].1.owner, Some(ClientId(4)));

        let below = collect(&[], &corpse(base - 1, 4), &[], None, &mut deficits);
        assert!(below.queued.is_empty());
        let mut empty = corpse(base + 2, 4);
        empty.slots[0].occupied = false;
        assert!(collect(&[], &empty, &[], None, &mut deficits).queued.is_empty());
        let out_of_range = corpse(ENTITY_NUMBER_COUNT as i32, 4);
        assert!(
            collect(&[], &out_of_range, &[], None, &mut deficits)
                .queued
                .is_empty()
        );
    }

    #[test]
    fn only_the_listed_entity_types_take_a_slot() {
        let mut deficits = LinkIdentityDeficits::default();
        let entities = [
            entity(100, entity_t6::et::SCRIPTMOVER),
            entity(101, entity_t6::et::MISSILE),
            entity(102, entity_t6::et::ITEM),
            entity(103, entity_t6::et::GENERAL),
            entity(-1, entity_t6::et::MISSILE),
        ];
        let claims = collect(
            &[],
            &PlayerCorpsePool::default(),
            &entities,
            None,
            &mut deficits,
        );
        assert_eq!(numbers(&claims), [100, 101, 102]);
        assert!(
            claims
                .queued
                .iter()
                .all(|(_, occupant)| occupant.owner.is_none())
        );
    }

    #[test]
    fn a_replicated_player_is_owned_by_its_client_number() {
        let mut deficits = LinkIdentityDeficits::default();
        let mut replicated = entity(7, entity_t6::et::PLAYER);
        replicated.client_num = 7;
        let claims = collect(
            &[],
            &PlayerCorpsePool::default(),
            &[replicated],
            None,
            &mut deficits,
        );
        assert_eq!(claims.queued[0].1.owner, Some(ClientId(7)));
    }

    #[test]
    fn the_first_claim_keeps_a_number_and_the_clash_is_a_gap() {
        let mut deficits = LinkIdentityDeficits::default();
        let missile = entity(5, entity_t6::et::MISSILE);
        let claims = collect(
            &[(ClientId(5), alive(0))],
            &PlayerCorpsePool::default(),
            &[missile],
            None,
            &mut deficits,
        );
        assert_eq!(numbers(&claims), [5]);
        assert_eq!(claims.queued[0].1.owner, Some(ClientId(5)));
        assert_eq!(
            deficits.cause(LinkDeficiency::ClientEntityIndex),
            Some(&LinkDeficiencyOrigin::NumberClaimedTwice { number: 5 })
        );
    }
}
