//! The match as a whole: starting it, its clock and phase, random streams, id counters, the content digest, and the
//! tables of HUD names.

use std::sync::Arc;

use super::catalog::SimContent;
use super::state::{Dice, SimState};
use super::{ClientId, PendingLocalSound, PendingPlayerCardEvent, PendingPlayerCardKind, PendingPrint};
use crate::identities::{MatchPhase, MatchRng, ProjectileId, RNG_DOMAIN_SCHEME, ShotId};
use crate::match_state::{ClientMatchState, RngDebugMeta};
use crate::spawn::MatchBootstrap;
use crate::world_objects::WorldObjectState;

impl SimState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a match from `bootstrap`. Refused while one is running, and when a class names a weapon the loaded
    /// weapon table does not have (an empty table skips that check).
    pub fn bootstrap(&mut self, bootstrap: MatchBootstrap) -> Result<(), &'static str> {
        if self.clock.running {
            return Err("MatchBootstrap refused: world already Running");
        }
        let weapon_count = self.content.weapon_combat().len();
        if weapon_count != 0 {
            let names_unknown_weapon = bootstrap
                .classes
                .iter()
                .flat_map(|class| class.weapon_slot_ids())
                .any(|id| id != 0 && id as usize >= weapon_count);
            if names_unknown_weapon {
                return Err("MatchBootstrap refused: class references unknown weapon id");
            }
        }

        self.dice = Dice::seeded(bootstrap.seed);
        self.roster.holdrand = bootstrap.seed as u32;
        self.clock.bootstrap = bootstrap;
        self.clock.phase = MatchPhase::Warmup;
        self.clock.elapsed_ms = 0;
        self.clock.prematch = gamemode_t6::PrematchStep::default();
        self.serials.shot = ShotId(1);

        self.replay.history.clear();
        self.replay.entity_history.clear();
        self.replay.sample.clear();
        self.replay.commands.clear();
        self.replay.lunge_seen.clear();

        self.hud.elems.clear();
        self.strings.hud_material = crate::HudMaterialCs::default();
        self.strings.hud_string = crate::HudStringCs::default();
        self.bind_required_hud_materials();

        self.weapon_notes.clear();
        self.recent_kills.clear();
        self.outbox.cards.clear();
        self.outbox.prints.clear();
        self.outbox.sounds.clear();
        self.refresh_digest();
        Ok(())
    }

    /// Throws the whole match away and starts over empty.
    pub fn shutdown_game(&mut self) {
        *self = Self::new();
    }

    // ---- clock and phase --------------------------------------------------------------------------------------

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.clock.running
    }

    pub(crate) fn mark_running(&mut self) {
        self.clock.running = true;
    }

    #[must_use]
    pub fn phase(&self) -> MatchPhase {
        self.clock.phase
    }

    /// Moves to `phase`. Leaving warm-up for play ends the pre-match countdown.
    pub(crate) fn set_phase(&mut self, phase: MatchPhase) {
        if phase == MatchPhase::Playing && self.clock.phase == MatchPhase::Warmup {
            self.clock.prematch = gamemode_t6::PrematchStep::Done;
        }
        self.clock.phase = phase;
    }

    pub(crate) fn restart_level_phase(&mut self) {
        self.clock.phase = MatchPhase::Warmup;
        self.clock.elapsed_ms = 0;
        self.clock.prematch = gamemode_t6::PrematchStep::default();
    }

    #[must_use]
    pub fn match_elapsed_ms(&self) -> u32 {
        self.clock.elapsed_ms
    }

    #[must_use]
    pub fn prematch(&self) -> gamemode_t6::PrematchStep {
        self.clock.prematch
    }

    #[must_use]
    pub(crate) fn bootstrap_ref(&self) -> &MatchBootstrap {
        &self.clock.bootstrap
    }

    #[must_use]
    pub fn cheats_enabled(&self) -> bool {
        self.clock.bootstrap.allow_debug_actions
    }

    // ---- random streams and id counters -----------------------------------------------------------------------

    #[must_use]
    pub fn root_seed(&self) -> u64 {
        self.dice.seed
    }

    /// The general-purpose stream is the spawn stream.
    #[must_use]
    pub fn rng(&self) -> &MatchRng {
        &self.dice.spawn
    }

    #[must_use]
    pub fn spawn_rng(&self) -> &MatchRng {
        &self.dice.spawn
    }

    #[must_use]
    pub fn combat_rng(&self) -> &MatchRng {
        &self.dice.combat
    }

    #[must_use]
    pub fn bot_rng(&self) -> &MatchRng {
        &self.dice.bot
    }

    pub(crate) fn combat_rng_mut(&mut self) -> &mut MatchRng {
        &mut self.dice.combat
    }

    #[must_use]
    pub fn rng_debug_meta(&self) -> RngDebugMeta {
        RngDebugMeta {
            root_seed: self.dice.seed,
            scheme: RNG_DOMAIN_SCHEME,
            spawn_draws: self.dice.spawn.draws(),
            combat_draws: self.dice.combat.draws(),
            bot_draws: self.dice.bot.draws(),
        }
    }

    pub(crate) fn alloc_shot_id(&mut self) -> ShotId {
        let id = self.serials.shot;
        self.serials.shot = id.next();
        id
    }

    /// Projectile ids wrap but never come out as zero.
    pub(crate) fn allocate_projectile_id(&mut self) -> ProjectileId {
        let id = self.serials.projectile;
        self.serials.projectile = ProjectileId(id.0.wrapping_add(1).max(1));
        id
    }

    // ---- the content digest -----------------------------------------------------------------------------------

    #[must_use]
    pub fn content_digest(&self) -> u64 {
        self.digest.value
    }

    #[must_use]
    pub fn content_components(&self) -> crate::ContentComponents {
        self.digest.parts
    }

    /// Hashes what the match plays on (weapon tables, bootstrap, clip brushes, collision owners) again.
    pub(super) fn refresh_digest(&mut self) {
        let content = &self.content;
        let (combat, runnable) = (content.weapon_combat(), content.weapon_runnable_table());
        let (groups, equipment) = (content.weapon_transition_groups(), content.equipment_table());
        let (bootstrap, brushes, owners) = (&self.clock.bootstrap, content.clip_brushes(), &self.space.caps);
        self.digest.value = crate::content::content_digest(combat, runnable, groups, equipment, bootstrap, brushes, owners);
        self.digest.parts =
            crate::content::content_components(combat, runnable, groups, equipment, bootstrap, brushes, owners);
    }

    /// Swaps in other content (a new map). The area grid is rebuilt for it.
    pub fn install_content(&mut self, content: Arc<SimContent>) {
        self.content = content;
        self.bodies.materialize_error = None;
        self.reset_area_world();
        self.refresh_digest();
    }

    // ---- HUD names and script HUD slots -----------------------------------------------------------------------

    pub(crate) fn hud_elem_slots_mut(&mut self) -> &mut Vec<crate::hudelem::GameHudElemSlot> {
        &mut self.hud.elems
    }

    #[must_use]
    pub(crate) fn hud_elem_sound_ids(&self) -> crate::hudelem::PulseFxSoundIds {
        self.hud.pulse_sounds
    }

    pub(crate) fn set_hud_elem_sound_ids(&mut self, ids: crate::hudelem::PulseFxSoundIds) {
        self.hud.pulse_sounds = ids;
    }

    pub fn sound_alias_index(&mut self, name: &str) -> u8 {
        self.strings.sound_alias.index(name)
    }

    pub fn effect_name_index(&mut self, name: &str) -> u8 {
        self.strings.effect_name.index(name)
    }

    pub fn hud_material_index(&mut self, name: &str) -> u8 {
        self.strings.hud_material.index(name)
    }

    pub(crate) fn hud_string_index(&mut self, text: &str) -> Option<i32> {
        self.strings.hud_string.index(text)
    }

    /// Gives every material the HUD cannot do without its slot.
    pub fn bind_required_hud_materials(&mut self) {
        for name in crate::REQUIRED_HUD_MATERIALS {
            let slot = self.strings.hud_material.index(name);
            debug_assert_ne!(slot, 0, "required HUD material `{name}` has no slot");
        }
    }

    // ---- things scripts reach through the state ---------------------------------------------------------------

    #[must_use]
    pub fn world_objects(&self) -> &WorldObjectState {
        &self.world_objects
    }

    pub fn world_objects_mut(&mut self) -> &mut WorldObjectState {
        &mut self.world_objects
    }

    pub fn script_destroy_glass(&mut self, id: crate::GlassPieceId, at_time_ms: i32) -> crate::GlassPieceState {
        self.world_objects.script_destroy_glass(id, at_time_ms)
    }

    // ---- player cards, prints, local sounds -------------------------------------------------------------------

    fn push_card(&mut self, recipient: ClientId, kind: PendingPlayerCardKind) {
        self.outbox.cards.push(PendingPlayerCardEvent { recipient, kind });
    }

    pub fn push_hud_splash(&mut self, recipient: ClientId, key: String, slot: i32, optional: i32) {
        self.push_card(recipient, PendingPlayerCardKind::Splash { key, slot, optional });
    }

    pub fn push_player_card_slot(&mut self, recipient: ClientId, source: ClientId, slot: i32) {
        self.push_card(recipient, PendingPlayerCardKind::SetSlot { source, slot });
    }

    pub fn push_player_card_open(&mut self, recipient: ClientId, cs_index: i32) {
        self.push_card(recipient, PendingPlayerCardKind::OpenMenu { cs_index });
    }

    pub fn take_pending_player_cards(&mut self) -> Vec<PendingPlayerCardEvent> {
        core::mem::take(&mut self.outbox.cards)
    }

    pub(crate) fn push_print(&mut self, print: PendingPrint) {
        self.outbox.prints.push(print);
    }

    pub fn take_pending_prints(&mut self) -> Vec<PendingPrint> {
        core::mem::take(&mut self.outbox.prints)
    }

    pub(crate) fn push_local_sound(&mut self, sound: PendingLocalSound) {
        self.outbox.sounds.push(sound);
    }

    pub fn take_pending_local_sounds(&mut self) -> Vec<PendingLocalSound> {
        core::mem::take(&mut self.outbox.sounds)
    }

    /// The last kill of a match is not announced separately in this build, so there is never one pending.
    pub fn take_pending_final_kill(&mut self) -> Option<(ClientId, ClientId)> {
        None
    }

    #[must_use]
    pub fn pending_final_kill(&self) -> Option<(ClientId, ClientId)> {
        None
    }

    /// Every client with its match row, for the scoreboard.
    #[must_use]
    pub fn clients_scoreboard(&self) -> Vec<(ClientId, ClientMatchState)> {
        self.roster.members.clone()
    }
}
