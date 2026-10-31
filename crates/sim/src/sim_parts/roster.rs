//! The connected clients and what the simulation remembers about each between commands.

use super::SeatNo;
use crate::player_ledger::PlayerMatchRecord;

/// Finds the value stored for `id` in a small id-keyed table.
fn find<T: Copy>(rows: &[(SeatNo, T)], id: SeatNo) -> Option<T> {
    rows.iter().find(|row| row.0 == id).map(|row| row.1)
}

/// Stores `value` for `id`, replacing an earlier one.
fn store<T>(rows: &mut Vec<(SeatNo, T)>, id: SeatNo, value: T) {
    match rows.iter_mut().find(|row| row.0 == id) {
        Some(row) => row.1 = value,
        None => rows.push((id, value)),
    }
}

impl super::state::MatchSimulation {
    // ---- the roster itself ------------------------------------------------------------------------------------

    #[must_use]
    pub(crate) fn ordered_player_ids(
        &self,
    ) -> Vec<super::SeatNo> {
        let mut ids: Vec<SeatNo> = self.roster.members.iter().map(|member| member.0).collect();
        ids.sort_unstable_by_key(|id| id.0);
        ids
    }

    #[must_use]
    pub fn client_meta(
        &self,
        id: SeatNo,
    ) -> Option<&PlayerMatchRecord> {
        self.roster.members.iter().find(|member| member.0 == id).map(|member| &member.1)
    }

    /// The row for `id`; a client the roster has not seen gets a blank one.
    pub(crate) fn client_meta_mut(
        &mut self,
        id: SeatNo,
    ) -> &mut PlayerMatchRecord {
        let at = match self.roster.members.iter().position(|member| member.0 == id) {
            Some(at) => at,
            None => {
                self.roster.members.push((id, PlayerMatchRecord::default()));
                self.roster.members.len() - 1
            }
        };
        &mut self.roster.members[at].1
    }

    #[must_use]
    pub fn packed_client_name(
        &self,
        id: SeatNo,
    ) -> [u8; 16] {
        self.client_meta(id).map_or([0; 16], |meta| meta.name)
    }

    #[must_use]
    pub const fn client_count(&self) -> usize {
        self.roster.members.len()
    }

    /// The free-for-all team number a client was given, if any.
    #[must_use]
    pub fn ffa_team(
        &self,
        client: u32,
    ) -> Option<u8> {
        self.client_meta(SeatNo(client)).and_then(|meta| meta.solo_side)
    }

    // ---- what was last seen of each client --------------------------------------------------------------------

    pub fn note_stride_kind(&mut self, client: SeatNo, kind: u8) {
        self.roster.movetype.insert(client, kind);
    }

    #[must_use]
    pub fn last_anim_movetype(
        &self,
        id: SeatNo,
    ) -> Option<u8> {
        self.roster.movetype.get(&id).copied()
    }

    #[must_use]
    pub const fn anim_event_seed(&self) -> u32 {
        self.roster.anim_seed
    }

    pub const fn set_anim_event_seed(&mut self, seed: u32) {
        self.roster.anim_seed = seed;
    }

    #[must_use]
    pub(crate) fn footing_of(&self, client: SeatNo) -> Option<i32> {
        self.roster.walking.get(&client).copied()
    }

    pub(crate) fn set_pmove_walking(
        &mut self,
        id: SeatNo,
        walking: i32,
    ) {
        self.roster.walking.insert(id, walking);
    }

    /// How far a stair step lifted this player on their last move (0 for none). The client's camera eases over it.
    #[must_use]
    pub fn pmove_view_change(&self, id: SeatNo) -> f32 {
        self.roster.view_change.get(&id).copied().unwrap_or(0.0)
    }

    pub(crate) fn set_pmove_view_change(&mut self, id: SeatNo, view_change: f32) {
        self.roster.view_change.insert(id, view_change);
    }

    /// The running random number the "stuck inside each other" eject uses.
    pub(crate) const fn stuck_holdrand_mut(&mut self) -> &mut u32 {
        &mut self.roster.rand_state
    }

    // ---- the previous command of each client ------------------------------------------------------------------

    pub fn set_old_buttons(
        &mut self,
        id: SeatNo,
        held: u32,
    ) {
        store(&mut self.roster.prev_buttons, id, held);
    }

    pub fn set_old_cmd_angles(
        &mut self,
        id: SeatNo,
        aim: [i32; 3],
    ) {
        store(&mut self.roster.prev_angles, id, aim);
    }

    /// The forward and side keys of the previous command.
    pub fn set_old_cmd_move(&mut self, id: SeatNo, moves: [i8; 2]) {
        store(&mut self.roster.prev_moves, id, moves);
    }

    pub fn store_prior_input(
        &mut self,
        id: SeatNo,
        held: u32,
        aim: [i32; 3],
    ) {
        store(&mut self.roster.prev_buttons, id, held);
        store(&mut self.roster.prev_angles, id, aim);
    }

    #[must_use]
    pub fn last_tick_buttons(
        &self,
        id: SeatNo,
    ) -> Option<u32> {
        find(&self.roster.prev_buttons, id)
    }

    #[must_use]
    pub fn old_cmd_angles(
        &self,
        id: SeatNo,
    ) -> Option<[i32; 3]> {
        find(&self.roster.prev_angles, id)
    }

    #[must_use]
    pub fn old_cmd_move(&self, id: SeatNo) -> Option<[i8; 2]> {
        find(&self.roster.prev_moves, id)
    }

    pub(crate) const fn prev_button_state_mut(
        &mut self,
    ) -> &mut Vec<(SeatNo, u32)> {
        &mut self.roster.prev_buttons
    }

    pub(crate) const fn prev_input_angles_mut(
        &mut self,
    ) -> &mut Vec<(SeatNo, [i32; 3])> {
        &mut self.roster.prev_angles
    }

    pub(crate) fn old_cmd_moves_mut(&mut self) -> &mut Vec<(SeatNo, [i8; 2])> {
        &mut self.roster.prev_moves
    }

    // ---- bodies and pickups that belong to the roster ---------------------------------------------------------

    #[must_use]
    pub(crate) fn prediction_remote_bodies(&self) -> &[super::state::PredictedRemotePlayer] {
        &self.bodies.remote
    }

    #[must_use]
    pub const fn body_pool(&self) -> &crate::CorpseYard {
        let Self { corpses, .. } = self;
        corpses
    }

    pub(crate) const fn corpses_mut(&mut self) -> &mut crate::CorpseYard {
        let Self { corpses, .. } = self;
        corpses
    }

    pub(crate) const fn item_pickups_mut(
        &mut self,
    ) -> &mut Vec<crate::GrabbedItemNote> {
        &mut self.outbox.pickups
    }
}
