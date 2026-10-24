//! `SimWorld` operations on entity rows: script movers, projectiles and players.

use super::SimWorld;
use crate::EntityAllocError;
use crate::ProjectileState;
use crate::gentity::ScriptMoverGentity;
use crate::identities::ScriptModelId;
use crate::world::ClientId;
use playerstate_t6::game::PlayerState;

// ---- script movers ----

impl SimWorld {
    pub fn script_mover_count(&self) -> usize {
        crate::frame::script_mover_row_count(&self.ecs)
    }

    pub fn visit_script_movers(&self, visit: impl FnMut(&ScriptMoverGentity)) {
        crate::frame::visit_script_movers(&self.ecs, visit);
    }

    pub fn script_mover_by_number(&self, number: i32) -> Option<ScriptMoverGentity> {
        crate::frame::script_mover_by_number(&self.ecs, number)
    }

    pub fn script_mover_by_id(&self, id: ScriptModelId) -> Option<ScriptMoverGentity> {
        crate::frame::script_mover_by_id(&self.ecs, id)
    }

    /// The entity number of the mover for `id`, if it has been spawned.
    pub fn gentity_number(&self, id: ScriptModelId) -> Option<i32> {
        self.script_mover_by_id(id).map(|mover| mover.state.number)
    }

    /// Spawns the mover for `id` (or finds the one already there) and returns its entity number.
    pub fn spawn_script_mover(
        &mut self,
        id: ScriptModelId,
        origin: [f32; 3],
        angles: [f32; 3],
    ) -> Result<i32, EntityAllocError> {
        self.frame().spawn_script_mover(id, origin, angles)
    }

    /// A script mover that wears the map's brush model `cmodel` and blocks like a solid.
    pub fn spawn_brush_mover(
        &mut self,
        id: ScriptModelId,
        cmodel: u32,
        origin: [f32; 3],
        angles: [f32; 3],
    ) -> Result<i32, EntityAllocError> {
        let mut frame = self.frame();
        let number = frame.spawn_script_mover(id, origin, angles)?;
        if let Some(mover) = frame.script_mover_mut_by_number(number) {
            mover.state.index = cmodel as i16;
            mover.state.solid = entity_t6::script_mover::SOLID_BMODEL;
        }
        Ok(number)
    }

    pub fn begin_script_mover_rotate_velocity(
        &mut self,
        id: ScriptModelId,
        speed: [f32; 3],
        total_time_seconds: f32,
        level_time_ms: i32,
    ) -> bool {
        self.frame().begin_script_mover_rotate_velocity(id, speed, total_time_seconds, level_time_ms)
    }

    /// Starts every mover spinning about its own side axis at `speed`; returns how many started.
    pub fn begin_script_movers_rotate_velocity_supplied(
        &mut self,
        speed: f32,
        level_time_ms: i32,
    ) -> usize {
        self.frame().begin_script_movers_rotate_velocity_supplied(speed, level_time_ms)
    }

    pub fn set_script_mover_r_box(
        &mut self,
        number: i32,
        box_mid: [f32; 3],
        box_half: [f32; 3],
    ) -> bool {
        self.frame().set_script_mover_r_box(number, box_mid, box_half)
    }

    pub fn set_script_mover_origin(&mut self, number: i32, origin: [f32; 3]) -> bool {
        self.frame().set_script_mover_origin(number, origin)
    }
}

// ---- projectiles ----

impl SimWorld {
    pub fn visit_projectiles(&self, visit: impl FnMut(&ProjectileState)) {
        crate::frame::visit_projectiles(&self.ecs, visit);
    }
}

// ---- players ----

impl SimWorld {
    pub fn player(&self, id: ClientId) -> Option<&PlayerState> {
        crate::frame::player_ref(&self.ecs, id)
    }

    pub fn visit_players(&self, visit: impl FnMut(ClientId, &PlayerState)) {
        crate::frame::visit_players(&self.ecs, visit);
    }

    pub fn player_count(&self) -> usize {
        crate::frame::player_row_count(&self.ecs)
    }

    /// Takes a client out of the match. A client the scripts know about is queued to disconnect
    /// through them, so their disconnect handlers run; any other is removed at once.
    pub fn retire_client(&mut self, id: ClientId) {
        let mut runtime = self.ecs.resource_mut::<crate::script::Runtime>();
        if runtime.players.contains_key(&id.0) {
            runtime.disconnects.insert(id.0);
        } else {
            self.frame().retire_client(id);
        }
    }

    pub fn set_origin(&mut self, id: ClientId, origin: [f32; 3]) -> bool {
        self.frame().set_origin(id, origin)
    }

    pub fn set_legs_anim(&mut self, id: ClientId, legs_anim: i32) -> bool {
        self.frame().set_legs_anim(id, legs_anim)
    }

    pub fn set_viewangles(&mut self, id: ClientId, viewangles: [f32; 3]) -> bool {
        self.frame().set_viewangles(id, viewangles)
    }

    pub fn set_e_flags(&mut self, id: ClientId, e_flags: u32) -> bool {
        self.frame().set_e_flags(id, e_flags)
    }

    pub fn set_leanf(&mut self, id: ClientId, leanf: f32) -> bool {
        self.frame().set_leanf(id, leanf)
    }

    pub fn set_view_height_target(&mut self, id: ClientId, view_height_target: i32) -> bool {
        self.frame().set_view_height_target(id, view_height_target)
    }

    /// Moves a player by `delta` without touching their velocity (used by the gate's test moves).
    pub fn gate_nudge_origin(&mut self, id: ClientId, delta: [f32; 3]) {
        self.frame().gate_nudge_origin(id, delta);
    }
}
