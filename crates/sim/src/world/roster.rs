//! The connected clients and what the simulation remembers about each between commands.

use super::state::SimState;
use super::ClientId;
use crate::match_state::ClientMatchState;

/// Finds the value stored for `id` in a small id-keyed table.
fn find<T: Copy>(rows: &[(ClientId, T)], id: ClientId) -> Option<T> {
    rows.iter().find(|row| row.0 == id).map(|row| row.1)
}

/// Stores `value` for `id`, replacing an earlier one.
fn store<T>(rows: &mut Vec<(ClientId, T)>, id: ClientId, value: T) {
    match rows.iter_mut().find(|row| row.0 == id) {
        Some(row) => row.1 = value,
        None => rows.push((id, value)),
    }
}

impl SimState {
    // ---- the roster itself ------------------------------------------------------------------------------------

    #[must_use]
    pub(crate) fn client_ids_sorted(&self) -> Vec<ClientId> {
        let mut ids: Vec<ClientId> = self.roster.members.iter().map(|member| member.0).collect();
        ids.sort_unstable_by_key(|id| id.0);
        ids
    }

    #[must_use]
    pub fn client_meta(&self, id: ClientId) -> Option<&ClientMatchState> {
        self.roster.members.iter().find(|member| member.0 == id).map(|member| &member.1)
    }

    /// The row for `id`; a client the roster has not seen gets a blank one.
    pub(crate) fn client_meta_mut(&mut self, id: ClientId) -> &mut ClientMatchState {
        let at = match self.roster.members.iter().position(|member| member.0 == id) {
            Some(at) => at,
            None => {
                self.roster.members.push((id, ClientMatchState::default()));
                self.roster.members.len() - 1
            }
        };
        &mut self.roster.members[at].1
    }

    #[must_use]
    pub fn packed_client_name(&self, id: ClientId) -> [u8; 16] {
        self.client_meta(id).map_or([0; 16], |meta| meta.name)
    }

    #[must_use]
    pub fn client_count(&self) -> usize {
        self.roster.members.len()
    }

    /// The free-for-all team number a client was given, if any.
    #[must_use]
    pub fn ffa_team(&self, client: u32) -> Option<u8> {
        self.client_meta(ClientId(client)).and_then(|meta| meta.ffa_team)
    }

    // ---- what was last seen of each client --------------------------------------------------------------------

    pub fn set_last_anim_movetype(&mut self, id: ClientId, movetype: u8) {
        self.roster.movetype.insert(id, movetype);
    }

    #[must_use]
    pub fn last_anim_movetype(&self, id: ClientId) -> Option<u8> {
        self.roster.movetype.get(&id).copied()
    }

    #[must_use]
    pub fn anim_event_seed(&self) -> u32 {
        self.roster.anim_seed
    }

    pub fn set_anim_event_seed(&mut self, seed: u32) {
        self.roster.anim_seed = seed;
    }

    #[must_use]
    pub(crate) fn pmove_walking(&self, id: ClientId) -> Option<i32> {
        self.roster.walking.get(&id).copied()
    }

    pub(crate) fn set_pmove_walking(&mut self, id: ClientId, walking: i32) {
        self.roster.walking.insert(id, walking);
    }

    /// How far a stair step lifted this player on their last move (0 for none). The client's camera eases over it.
    #[must_use]
    pub fn pmove_view_change(&self, id: ClientId) -> f32 {
        self.roster.view_change.get(&id).copied().unwrap_or(0.0)
    }

    pub(crate) fn set_pmove_view_change(&mut self, id: ClientId, view_change: f32) {
        self.roster.view_change.insert(id, view_change);
    }

    /// The running random number the "stuck inside each other" eject uses.
    pub(crate) fn stuck_holdrand_mut(&mut self) -> &mut u32 {
        &mut self.roster.holdrand
    }

    // ---- the previous command of each client ------------------------------------------------------------------

    pub fn set_old_buttons(&mut self, id: ClientId, buttons: u32) {
        store(&mut self.roster.prev_buttons, id, buttons);
    }

    pub fn set_old_cmd_angles(&mut self, id: ClientId, angles: [i32; 3]) {
        store(&mut self.roster.prev_angles, id, angles);
    }

    /// The forward and side keys of the previous command.
    pub fn set_old_cmd_move(&mut self, id: ClientId, moves: [i8; 2]) {
        store(&mut self.roster.prev_moves, id, moves);
    }

    pub fn set_old_cmd(&mut self, id: ClientId, buttons: u32, angles: [i32; 3]) {
        self.set_old_buttons(id, buttons);
        self.set_old_cmd_angles(id, angles);
    }

    #[must_use]
    pub fn old_buttons(&self, id: ClientId) -> Option<u32> {
        find(&self.roster.prev_buttons, id)
    }

    #[must_use]
    pub fn old_cmd_angles(&self, id: ClientId) -> Option<[i32; 3]> {
        find(&self.roster.prev_angles, id)
    }

    #[must_use]
    pub fn old_cmd_move(&self, id: ClientId) -> Option<[i8; 2]> {
        find(&self.roster.prev_moves, id)
    }

    pub(crate) fn old_buttons_mut(&mut self) -> &mut Vec<(ClientId, u32)> {
        &mut self.roster.prev_buttons
    }

    pub(crate) fn old_cmd_angles_mut(&mut self) -> &mut Vec<(ClientId, [i32; 3])> {
        &mut self.roster.prev_angles
    }

    pub(crate) fn old_cmd_moves_mut(&mut self) -> &mut Vec<(ClientId, [i8; 2])> {
        &mut self.roster.prev_moves
    }

    // ---- bodies and pickups that belong to the roster ---------------------------------------------------------

    #[must_use]
    pub(crate) fn prediction_remote_bodies(&self) -> &[super::state::PredictionRemoteBody] {
        &self.bodies.remote
    }

    #[must_use]
    pub fn corpses(&self) -> &crate::PlayerCorpsePool {
        &self.corpses
    }

    pub(crate) fn corpses_mut(&mut self) -> &mut crate::PlayerCorpsePool {
        &mut self.corpses
    }

    pub(crate) fn item_pickups_mut(&mut self) -> &mut Vec<crate::ItemPickupRecord> {
        &mut self.outbox.pickups
    }
}
