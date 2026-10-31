//! Player rows: looking one up, editing one, and keeping the collision world in step with where players are.

use core::ops::DerefMut;

use movement_t6::bridge::HullExtents;
use playerstate_t6::game::AvatarState;

use super::StepWorldView;
use super::rows::PlayerRow;
use super::store::{index, avatar_of, spawn_player, each_avatar_row};
use crate::shot_trace::{
    ShotRayRequest, RewindRequest, BODY_BOX_UPPER, BODY_BOX_LOWER, PlayerHitboxPose, RayResult,
};
use crate::sim_parts::{SeatNo, MatchSimulation, StepNo, empty_avatar};
use crate::PlayerSessionStage;

impl<'a> StepWorldView<'a> {
    pub fn avatar(&self, client: SeatNo) -> Option<&AvatarState> {
        avatar_of(self.ecs, client)
    }

    fn state_copy(&self, client: SeatNo) -> Option<AvatarState> {
        avatar_of(self.ecs, client).copied()
    }

    fn origin_of(&self, client: SeatNo) -> Option<[f32; 3]> {
        self.state_copy(client).map(|state| state.origin)
    }

    pub(crate) fn occupant_mut(&mut self, client: SeatNo) -> Option<&mut AvatarState> {
        let row = index(self.ecs).client_row(client)?;
        Some(&mut self.ecs.get_mut::<PlayerRow>(row)?.into_inner().state)
    }

    pub(crate) fn walk_avatars(&self, each: impl FnMut(SeatNo, &AvatarState)) {
        each_avatar_row(self.ecs, each);
    }

    pub(crate) fn for_each_player_mut(&mut self, mut visit: impl FnMut(SeatNo, &mut AvatarState)) {
        let rows: Vec<_> = index(self.ecs).clients().iter().map(|&(_, entry)| entry).collect();
        for entry in rows {
            let held = self.ecs.get_mut::<PlayerRow>(entry).expect("indexed player row vanished").into_inner();
            visit(held.client, &mut held.state);
        }
    }

    /// The player's state, adding a blank one (and the client's match record) first if there is none.
    pub(crate) fn seat_slot_for(&mut self, client: SeatNo) -> &mut AvatarState {
        let _ = self.client_meta_mut(client);
        if index(self.ecs).client_row(client).is_none() {
            let mut state = empty_avatar();
            state.body_index = -1;
            spawn_player(self.ecs, client, state);
        }
        self.occupant_mut(client).expect("ensure_player just inserted the row")
    }

    /// Drops everything held for the client: its script presence, its match membership and its player row.
    pub(crate) fn drop_player_record(&mut self, client: SeatNo) {
        crate::gsc_host::disconnect_player(self.ecs, client.0);
        self.deref_mut().drop_seat_links(client);
        let Some(row) = index(self.ecs).client_row(client) else {
            return;
        };
        super::store::index_mut(self.ecs).release_client(client);
        super::store::despawn_row(self.ecs, row, "player");
    }

    // ---- editing one field ----

    fn edit_player(&mut self, client: SeatNo, edit: impl FnOnce(&mut AvatarState)) -> bool {
        self.occupant_mut(client).map(edit).is_some()
    }

    pub fn write_stride_clip(&mut self, client: SeatNo, anim: i32) -> bool {
        self.edit_player(client, |ps| ps.legs_anim = anim)
    }

    pub fn write_aim(&mut self, client: SeatNo, aim: [f32; 3]) -> bool {
        self.edit_player(client, |ps| ps.look_angles = aim)
    }

    pub fn write_body_flags(&mut self, client: SeatNo, flags: u32) -> bool {
        self.edit_player(client, |ps| ps.e_flags = flags)
    }

    pub fn write_lean(&mut self, client: SeatNo, lean: f32) -> bool {
        self.edit_player(client, |ps| ps.tilt_sideways = lean)
    }

    pub fn aim_eye_height(&mut self, client: SeatNo, target: i32) -> bool {
        self.edit_player(client, |ps| ps.eye_height_goal = target)
    }

    // ---- moving a player ----

    /// Puts the player at `origin`, stopped, and drags their collision box along.
    pub fn place_at(&mut self, client: SeatNo, spot: [f32; 3]) -> bool {
        let Some(state) = self.occupant_mut(client) else {
            return false;
        };
        let was = core::mem::replace(&mut state.origin, spot);
        state.velocity = [0.0; 3];
        self.translate_player_area(client, core::array::from_fn(|axis| spot[axis] - was[axis]));
        true
    }

    /// Shifts the player by `delta` without touching their velocity, dragging their collision box along.
    pub fn nudge_gate(&mut self, client: SeatNo, diff: [f32; 3]) {
        let Some(state) = self.occupant_mut(client) else {
            return;
        };
        for (coord, shift) in state.origin.iter_mut().zip(diff) {
            *coord += shift;
        }
        self.translate_player_area(client, diff);
    }

    // ---- the collision world ----

    pub(crate) fn link_player_area(&mut self, client: SeatNo, extent: HullExtents) -> bool {
        let Some(origin) = self.origin_of(client) else {
            return false;
        };
        self.deref_mut().link_player_area(client, origin, extent.lo_corner, extent.hi_corner)
    }

    pub(crate) fn relink_seat_footing(&mut self, client: SeatNo) -> bool {
        let Some(origin) = self.origin_of(client) else {
            return false;
        };
        self.deref_mut().link_player_area(client, origin, BODY_BOX_LOWER, BODY_BOX_UPPER)
    }

    pub(crate) fn release_seat_area(&mut self, client: SeatNo) -> bool {
        self.deref_mut().release_seat_area(client)
    }

    pub(crate) fn alive_collision_poses(&self) -> Vec<PlayerHitboxPose> {
        MatchSimulation::alive_collision_poses(self, |client| self.state_copy(client))
    }

    /// `sensor_trace` as of now: players moved, spawned or killed earlier this tick are traced where they are, not
    /// where the last record left them.
    pub(crate) fn current_sensor_trace(&self, query: ShotRayRequest) -> RayResult {
        let moved = self.player_poses_since_record(|client| self.state_copy(client));
        self.trace_shot(query, moved.as_deref())
    }

    pub(crate) fn record_collision_history(&mut self, tick: StepNo, msec: i32, box_orders: Option<&[SeatNo]>) {
        let ordered = self.ordered_player_ids();
        let copies: Vec<(SeatNo, AvatarState)> = ordered
            .into_iter()
            .filter_map(|client| Some((client, self.state_copy(client)?)))
            .collect();
        let state_of = |client: SeatNo| copies.iter().find(|&&(owner, _)| owner == client).map(|&(_, ent_state)| ent_state);
        MatchSimulation::record_collision_history(self, tick, msec, state_of, box_orders);
    }

    pub fn rewind_request_for(&self, attacker: SeatNo, current: StepNo) -> RewindRequest {
        MatchSimulation::rewind_request_for(self, attacker, current, |client| self.state_copy(client))
    }

    /// Where each living client stands, in client order.
    pub(crate) fn living_positions(&self) -> Vec<[f32; 3]> {
        let ordered = self.ordered_player_ids();
        ordered
            .into_iter()
            .filter(|&client| self.client_meta(client).is_some_and(|meta| meta.lifecycle == PlayerSessionStage::Alive))
            .filter_map(|client| self.avatar(client).map(|ps| ps.origin))
            .collect()
    }
}
