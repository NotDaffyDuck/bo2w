//! Player rows: looking one up, editing one, and keeping the collision world in step with where players are.

use core::ops::DerefMut;

use movement_t6::bridge::MoveBounds;
use playerstate_t6::game::PlayerState;

use super::FrameWorld;
use super::rows::PlayerRow;
use super::store::{index, player_ref, spawn_player, visit_players};
use crate::bullet_collision::{
    BulletTraceQuery, LagcompQuery, PLAYER_MAXS, PLAYER_MINS, PlayerCollisionPose, TraceOutcome,
};
use crate::world::{ClientId, SimState, Tick, blank_player_state};
use crate::ClientLifecycle;

impl FrameWorld<'_> {
    pub fn player(&self, id: ClientId) -> Option<&PlayerState> {
        player_ref(self.ecs, id)
    }

    pub(crate) fn player_mut(&mut self, id: ClientId) -> Option<&mut PlayerState> {
        let row = index(self.ecs).client_row(id)?;
        Some(&mut self.ecs.get_mut::<PlayerRow>(row)?.into_inner().state)
    }

    pub(crate) fn visit_players(&self, visit: impl FnMut(ClientId, &PlayerState)) {
        visit_players(self.ecs, visit);
    }

    pub(crate) fn for_each_player_mut(&mut self, mut visit: impl FnMut(ClientId, &mut PlayerState)) {
        let rows: Vec<_> = index(self.ecs).clients().iter().map(|&(_, row)| row).collect();
        for row in rows {
            let held = self.ecs.get_mut::<PlayerRow>(row).expect("indexed player row vanished").into_inner();
            visit(held.client, &mut held.state);
        }
    }

    /// The player's state, adding a blank one (and the client's match record) first if there is none.
    pub(crate) fn ensure_player(&mut self, id: ClientId) -> &mut PlayerState {
        let _ = self.client_meta_mut(id);
        if index(self.ecs).client_row(id).is_none() {
            let mut state = blank_player_state();
            state.corpse_index = -1;
            spawn_player(self.ecs, id, state);
        }
        self.player_mut(id).expect("ensure_player just inserted the row")
    }

    /// Drops everything held for the client: its script presence, its match membership and its player row.
    pub(crate) fn retire_client(&mut self, id: ClientId) {
        crate::script::disconnect_player(self.ecs, id.0);
        self.deref_mut().forget_client_membership(id);
        let Some(row) = index(self.ecs).client_row(id) else {
            return;
        };
        super::store::index_mut(self.ecs).release_client(id);
        super::store::despawn_row(self.ecs, row, "player");
    }

    // ---- editing one field ----

    fn edit_player(&mut self, id: ClientId, edit: impl FnOnce(&mut PlayerState)) -> bool {
        match self.player_mut(id) {
            Some(state) => {
                edit(state);
                true
            }
            None => false,
        }
    }

    pub fn set_legs_anim(&mut self, id: ClientId, legs_anim: i32) -> bool {
        self.edit_player(id, |ps| ps.legs_anim = legs_anim)
    }

    pub fn set_viewangles(&mut self, id: ClientId, viewangles: [f32; 3]) -> bool {
        self.edit_player(id, |ps| ps.viewangles = viewangles)
    }

    pub fn set_e_flags(&mut self, id: ClientId, e_flags: u32) -> bool {
        self.edit_player(id, |ps| ps.e_flags = e_flags)
    }

    pub fn set_leanf(&mut self, id: ClientId, leanf: f32) -> bool {
        self.edit_player(id, |ps| ps.leanf = leanf)
    }

    pub fn set_view_height_target(&mut self, id: ClientId, view_height_target: i32) -> bool {
        self.edit_player(id, |ps| ps.view_height_target = view_height_target)
    }

    // ---- moving a player ----

    /// Puts the player at `origin`, stopped, and drags their collision box along.
    pub fn set_origin(&mut self, id: ClientId, origin: [f32; 3]) -> bool {
        let Some(state) = self.player_mut(id) else {
            return false;
        };
        let was = core::mem::replace(&mut state.origin, origin);
        state.velocity = [0.0; 3];
        self.translate_player_area(id, core::array::from_fn(|axis| origin[axis] - was[axis]));
        true
    }

    /// Shifts the player by `delta` without touching their velocity, dragging their collision box along.
    pub fn gate_nudge_origin(&mut self, id: ClientId, delta: [f32; 3]) {
        let Some(state) = self.player_mut(id) else {
            return;
        };
        for axis in 0..3 {
            state.origin[axis] += delta[axis];
        }
        self.translate_player_area(id, delta);
    }

    // ---- the collision world ----

    pub(crate) fn link_player_area(&mut self, id: ClientId, bounds: MoveBounds) -> bool {
        let Some(origin) = self.player(id).map(|ps| ps.origin) else {
            return false;
        };
        self.deref_mut().link_player_area(id, origin, bounds.mins, bounds.maxs)
    }

    pub(crate) fn link_player_standing_area(&mut self, id: ClientId) -> bool {
        let Some(origin) = self.player(id).map(|ps| ps.origin) else {
            return false;
        };
        self.deref_mut().link_player_area(id, origin, PLAYER_MINS, PLAYER_MAXS)
    }

    pub(crate) fn unlink_player_area(&mut self, id: ClientId) -> bool {
        self.deref_mut().unlink_player_area(id)
    }

    pub(crate) fn alive_collision_poses(&self) -> Vec<PlayerCollisionPose> {
        SimState::alive_collision_poses(self, |id| player_ref(self.ecs, id).copied())
    }

    /// `sensor_trace` as of now: players moved, spawned or killed earlier this tick are traced where they are, not
    /// where the last record left them.
    pub(crate) fn current_sensor_trace(&self, query: BulletTraceQuery) -> TraceOutcome {
        let moved = self.player_poses_since_record(|id| player_ref(self.ecs, id).copied());
        self.bullet_trace(query, moved.as_deref())
    }

    pub(crate) fn record_collision_history(&mut self, tick: Tick, msec: i32, hitbox_cmds: Option<&[ClientId]>) {
        let copies: Vec<(ClientId, PlayerState)> = self
            .client_ids_sorted()
            .into_iter()
            .filter_map(|id| Some((id, player_ref(self.ecs, id).copied()?)))
            .collect();
        let state_of = |id: ClientId| copies.iter().find(|&&(client, _)| client == id).map(|&(_, state)| state);
        SimState::record_collision_history(self, tick, msec, state_of, hitbox_cmds);
    }

    pub fn lagcomp_query_for(&self, attacker: ClientId, current: Tick) -> LagcompQuery {
        SimState::lagcomp_query_for(self, attacker, current, |id| player_ref(self.ecs, id).copied())
    }

    /// Where each living client stands, in client order.
    pub(crate) fn alive_origins(&self) -> Vec<[f32; 3]> {
        self.client_ids_sorted()
            .into_iter()
            .filter(|&id| self.client_meta(id).is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive))
            .filter_map(|id| self.player(id).map(|ps| ps.origin))
            .collect()
    }
}
