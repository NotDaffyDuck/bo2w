//! `WholeSim` operations on entity rows: script movers, projectiles and players.

use crate::EntitySlotShortage;
use crate::FlyingProjectile;
use crate::ent_slots::MoverSlot;
use crate::match_ids::ScriptedModel;
use crate::sim_parts::SeatNo;
use playerstate_t6::game::AvatarState;

impl super::WholeSim {
    // ---- script movers ----

    /// How many script movers currently exist.
    pub fn scripted_mover_total(&self) -> usize {
        crate::step_view::script_mover_row_count(&self.bevy)
    }

    /// Calls `each` once for every script mover.
    pub fn each_mover(&self, each: impl FnMut(&MoverSlot)) {
        crate::step_view::visit_script_movers(&self.bevy, each);
    }

    /// Looks a mover up by its entity number.
    pub fn script_mover_by_number(&self, entity_number: i32) -> Option<MoverSlot> {
        crate::step_view::script_mover_by_number(&self.bevy, entity_number)
    }

    /// Looks a mover up by the script-side model id it was spawned under.
    pub fn mover_for_model(&self, model: ScriptedModel) -> Option<MoverSlot> {
        crate::step_view::script_mover_by_id(&self.bevy, model)
    }

    /// The entity number of the mover for `model`, if it has been spawned.
    pub fn game_actor_number(&self, model: ScriptedModel) -> Option<i32> {
        self.mover_for_model(model).map(|found| found.state.slot_index)
    }

    /// Spawns the mover for `model` (or finds the one already there) and returns its entity number.
    pub fn create_scripted_mover(
        &mut self,
        model: ScriptedModel,
        pos: [f32; 3],
        rotation: [f32; 3],
    ) -> Result<i32, EntitySlotShortage> {
        self.frame().create_scripted_mover(model, pos, rotation)
    }

    /// A script mover that wears the map's brush model `collision_geometry` and blocks like a solid.
    pub fn spawn_brush_mover(
        &mut self,
        model: ScriptedModel,
        collision_geometry: u32,
        pos: [f32; 3],
        rotation: [f32; 3],
    ) -> Result<i32, EntitySlotShortage> {
        let mut frame = self.frame();
        let entity_number = frame.create_scripted_mover(model, pos, rotation)?;
        if let Some(spawned) = frame.script_mover_mut_by_number(entity_number) {
            spawned.state.index = collision_geometry as i16;
            spawned.state.solid = entity_t6::script_mover::SOLID_BMODEL;
        }
        Ok(entity_number)
    }

    /// Begins a timed spin of one mover at the given per-axis rates.
    pub fn start_scripted_spin(
        &mut self,
        model: ScriptedModel,
        rates: [f32; 3],
        duration_secs: f32,
        clock_millis: i32,
    ) -> bool {
        self.frame().start_scripted_spin(model, rates, duration_secs, clock_millis)
    }

    /// Starts every mover spinning about its own side axis at `rate`; returns how many started.
    pub fn start_rate_spins(
        &mut self,
        rate: f32,
        clock_millis: i32,
    ) -> usize {
        self.frame().start_rate_spins(rate, clock_millis)
    }

    /// Sets the bounding box (centre and half extents) of one mover.
    pub fn resize_scripted_mover_box(
        &mut self,
        entity_number: i32,
        centre: [f32; 3],
        half_extents: [f32; 3],
    ) -> bool {
        self.frame().resize_scripted_mover_box(entity_number, centre, half_extents)
    }

    /// Teleports one mover to `pos`.
    pub fn place_mover(&mut self, entity_number: i32, pos: [f32; 3]) -> bool {
        self.frame().place_mover(entity_number, pos)
    }

    // ---- projectiles ----

    /// Calls `each` once for every live projectile.
    pub fn visit_projectiles(&self, each: impl FnMut(&FlyingProjectile)) {
        crate::step_view::visit_projectiles(&self.bevy, each);
    }

    // ---- players ----

    /// A client's player state, if they are in the world.
    pub fn player(&self, client: SeatNo) -> Option<&AvatarState> {
        crate::step_view::avatar_of(&self.bevy, client)
    }

    /// Calls `each` once for every player.
    pub fn each_avatar(&self, each: impl FnMut(SeatNo, &AvatarState)) {
        crate::step_view::each_avatar_row(&self.bevy, each);
    }

    /// How many players are in the world.
    pub fn seated_player_total(&self) -> usize {
        crate::step_view::player_row_count(&self.bevy)
    }

    /// Takes a client out of the match. A client the scripts know about is queued to disconnect
    /// through them, so their disconnect handlers run; any other is removed at once.
    pub fn drop_player_record(&mut self, client: SeatNo) {
        let mut script_rt = self.bevy.resource_mut::<crate::gsc_host::Runtime>();
        if script_rt.players.contains_key(&client.0) {
            script_rt.disconnects.insert(client.0);
            return;
        }
        self.frame().drop_player_record(client);
    }

    pub fn set_origin(&mut self, client: SeatNo, pos: [f32; 3]) -> bool {
        self.frame().place_at(client, pos)
    }

    pub fn write_stride_clip(&mut self, client: SeatNo, anim: i32) -> bool {
        self.frame().write_stride_clip(client, anim)
    }

    pub fn write_aim(&mut self, client: SeatNo, view: [f32; 3]) -> bool {
        self.frame().write_aim(client, view)
    }

    pub fn write_body_flags(&mut self, client: SeatNo, flags: u32) -> bool {
        self.frame().write_body_flags(client, flags)
    }

    pub fn write_lean(&mut self, client: SeatNo, lean: f32) -> bool {
        self.frame().write_lean(client, lean)
    }

    pub fn aim_eye_height(&mut self, client: SeatNo, target: i32) -> bool {
        self.frame().aim_eye_height(client, target)
    }

    /// Moves a player by `offset` without touching their velocity (used by the gate's test moves).
    pub fn gate_nudge_origin(&mut self, client: SeatNo, offset: [f32; 3]) {
        self.frame().nudge_gate(client, offset);
    }
}
