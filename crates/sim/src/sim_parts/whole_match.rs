//! The match as a whole: starting it, its clock and phase, random streams, id counters, the content digest, and the
//! tables of HUD names.

use std::sync::Arc;

use super::content_book::SimContent;
use super::state::Dice;
use super::{SeatNo, PendingLocalSound, QueuedCallingCardEvent, QueuedCallingCardKind, PendingPrint};
use crate::match_ids::{GameRoundStage, MatchRandom, ThrownId, BulletNo};
use crate::player_ledger::{PlayerMatchRecord, DiceTrace};
use crate::spawn_pick::MatchStartSetup;
use crate::glass_and_props::{Blast, Blow, MapPropState};

impl super::state::MatchSimulation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a match from `bootstrap`. Declined while one is running, and when a class names a weapon the loaded
    /// weapon table does not have (an empty table skips that check).
    pub fn bootstrap(
        &mut self,
        config: MatchStartSetup,
    ) -> Result<(), &'static str> {
        if self.clock.going {
            return Err("MatchBootstrap refused: a match is already running");
        }
        let weapon_count = self.content.weapon_combat().len();
        if weapon_count != 0 {
            let names_unknown_weapon = config
                .classes
                .iter()
                .flat_map(|class| class.gun_slot_ids())
                .any(|id| id != 0 && id as usize >= weapon_count);
            if names_unknown_weapon {
                return Err("MatchBootstrap refused: a class lists a weapon id outside the table");
            }
        }

        self.dice = Dice::seeded(config.root);
        self.roster.rand_state = config.root as u32;
        self.clock.bootstrap = config;
        self.clock.phase = GameRoundStage::Warmup;
        self.clock.elapsed_ms = 0;
        self.clock.prematch = gamemode_t6::WarmupStage::default();
        self.serials.shot = BulletNo(1);

        self.replay.history.clear();
        self.replay.entity_history.clear();
        self.replay.sample.clear();
        self.replay.commands.clear();
        self.replay.lunge_seen.clear();

        self.hud.elems.clear();
        self.strings.hud_material = crate::SoundAliasConfigTable::default();
        self.strings.hud_string = crate::HudStringCs::default();
        Self::bind_required_hud_materials(self);

        self.weapon_notes.clear();
        self.latest_frag_log = Vec::new();
        self.outbox.cards.clear();
        self.outbox.prints.clear();
        self.outbox.sounds.clear();
        self.refresh_digest();
        Ok(())
    }

    /// Throws the whole match away and starts over empty.
    pub fn end_game_session(
        &mut self,
    ) {
        *self = Self::default();
    }

    // ---- clock and phase --------------------------------------------------------------------------------------

    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.clock.going
    }

    pub(crate) const fn mark_running(&mut self) {
        self.clock.going = true;
    }

    #[must_use]
    pub const fn phase(&self) -> GameRoundStage {
        self.clock.phase
    }

    /// Moves to `phase`. Leaving warm-up for play ends the pre-match countdown.
    pub(crate) fn set_phase(
        &mut self,
        next: GameRoundStage,
    ) {
        if next == GameRoundStage::Playing && self.clock.phase == GameRoundStage::Warmup {
            self.clock.prematch = gamemode_t6::WarmupStage::Done;
        }
        self.clock.phase = next;
    }

    pub(crate) fn restart_level_phase(&mut self) {
        self.clock.phase = GameRoundStage::Warmup;
        self.clock.elapsed_ms = 0;
        self.clock.prematch = gamemode_t6::WarmupStage::default();
    }

    #[must_use]
    pub const fn match_elapsed_ms(&self) -> u32 {
        self.clock.elapsed_ms
    }

    #[must_use]
    pub fn prematch(&self) -> gamemode_t6::WarmupStage {
        self.clock.prematch
    }

    #[must_use]
    pub(crate) const fn bootstrap_ref(&self) -> &MatchStartSetup {
        &self.clock.bootstrap
    }

    #[must_use]
    pub const fn cheats_enabled(&self) -> bool {
        self.clock.bootstrap.permit_cheat_requests
    }

    // ---- random streams and id counters -----------------------------------------------------------------------

    #[must_use]
    pub const fn root_seed(&self) -> u64 {
        self.dice.seed
    }

    /// The general-purpose stream is the spawn stream.
    #[must_use]
    pub const fn rng(&self) -> &MatchRandom {
        &self.dice.spawn
    }

    #[must_use]
    pub const fn spawn_rng(&self) -> &MatchRandom {
        &self.dice.spawn
    }

    #[must_use]
    pub const fn combat_rng(&self) -> &MatchRandom {
        &self.dice.combat
    }

    #[must_use]
    pub const fn bot_rng(&self) -> &MatchRandom {
        &self.dice.bot
    }

    pub(crate) const fn combat_dice_mut(&mut self) -> &mut MatchRandom {
        &mut self.dice.combat
    }

    #[must_use]
    pub fn random_debug_info(
        &self,
    ) -> DiceTrace {
        let dice = &self.dice;
        let meta = DiceTrace {
            spawn_draws: dice.spawn.draws(),
            combat_draws: dice.combat.draws(),
            bot_draws: dice.bot.draws(),
            scheme: crate::match_ids::RANDOM_STREAM_LAYOUT_VERSION,
            root_seed: dice.seed,
        };
        meta
    }

    pub(crate) fn alloc_shot_id(
        &mut self,
    ) -> BulletNo {
        let id = self.serials.shot;
        self.serials.shot = id.next();
        id
    }

    /// Projectile ids wrap but never come out as zero.
    pub(crate) fn reserve_projectile_slot(
        &mut self,
    ) -> ThrownId {
        let id = self.serials.projectile;
        self.serials.projectile = ThrownId(id.0.wrapping_add(1).max(1));
        id
    }

    // ---- the content digest -----------------------------------------------------------------------------------

    #[must_use]
    pub const fn content_fingerprint(&self) -> u64 {
        self.digest.value
    }

    #[must_use]
    pub const fn content_part_fingerprints(&self) -> crate::ContentPartDigests {
        self.digest.parts
    }

    /// Hashes what the match plays on (weapon tables, bootstrap, clip brushes, collision owners) again.
    pub(super) fn refresh_digest(&mut self) {
        let content = &self.content;
        let (combat, runnable) = (content.weapon_combat(), content.weapon_runnable_table());
        let (groups, equipment) = (content.weapon_transition_groups(), content.equipment_table());
        let (bootstrap, brushes, owners) = (&self.clock.bootstrap, content.collision_solids(), &self.space.caps);
        self.digest.value = crate::content_print::content_fingerprint(combat, runnable, groups, equipment, bootstrap, brushes, owners);
        self.digest.parts =
            crate::content_print::content_part_fingerprints(combat, runnable, groups, equipment, bootstrap, brushes, owners);
    }

    /// Swaps in other content (a new map). The area grid is rebuilt for it.
    pub fn install_content(&mut self, content: Arc<SimContent>) {
        self.content = content;
        self.bodies.bake_failure = None;
        self.reset_area_world();
        self.refresh_digest();
    }

    // ---- HUD names and script HUD slots -----------------------------------------------------------------------

    pub(crate) fn hud_elem_slots_mut(
        &mut self,
    ) -> &mut Vec<crate::script_overlay::ScriptHudSlot> {
        &mut self.hud.elems
    }

    #[must_use]
    pub(crate) const fn hud_elem_sound_ids(&self) -> crate::script_overlay::PulseEffectSoundSet {
        self.hud.pulse_sounds
    }

    pub(crate) const fn set_hud_elem_sound_ids(&mut self, ids: crate::script_overlay::PulseEffectSoundSet) {
        self.hud.pulse_sounds = ids;
    }

    pub fn sound_alias_index(
        &mut self,
        name: &str,
    ) -> u8 {
        self.strings.sound_alias.number_of(name)
    }

    pub fn effect_name_index(
        &mut self,
        name: &str,
    ) -> u8 {
        self.strings.effect_label.number_of(name)
    }

    pub fn hud_material_index(
        &mut self,
        name: &str,
    ) -> u8 {
        self.strings.hud_material.number_of(name)
    }

    pub(crate) fn hud_string_index(&mut self, text: &str) -> Option<i32> {
        self.strings.hud_string.number_of(text)
    }

    /// Gives every material the HUD cannot do without its slot.
    pub fn bind_required_hud_materials(
        &mut self,
    ) {
        let table = &mut self.strings.hud_material;
        crate::MANDATORY_HUD_IMAGES.iter().for_each(|&material| {
            debug_assert_ne!(table.number_of(material), 0, "required HUD material `{material}` has no slot");
        });
    }

    // ---- things scripts reach through the state ---------------------------------------------------------------

    #[must_use]
    pub const fn world_objects(&self) -> &MapPropState {
        let Self { world_objects: placed_things, .. } = self;
        placed_things
    }

    pub const fn world_objects_mut(&mut self) -> &mut MapPropState {
        let Self { world_objects: placed_things, .. } = self;
        placed_things
    }

    /// Hits a pane, its crack rate rolled from the world's shared random stream.
    pub(crate) fn strike_glass(&mut self, blow: Blow) -> crate::PaneShardCondition {
        let mut holdrand = *self.stuck_holdrand_mut();
        let state = self.world_objects.strike_glass(blow, &mut || crate::floor_guns::random_unit(&mut holdrand));
        *self.stuck_holdrand_mut() = holdrand;
        state
    }

    /// An explosion over the glass, rolled from the same stream.
    pub(crate) fn blast_glass(&mut self, blast: &Blast, at_ms: i32) {
        let mut holdrand = *self.stuck_holdrand_mut();
        self.world_objects.blast_glass(blast, at_ms, &mut || crate::floor_guns::random_unit(&mut holdrand));
        *self.stuck_holdrand_mut() = holdrand;
    }

    // ---- player cards, prints, local sounds -------------------------------------------------------------------

    fn push_card(&mut self, recipient: SeatNo, kind: QueuedCallingCardKind) {
        self.outbox.cards.push(QueuedCallingCardEvent { recipient, kind });
    }

    pub fn push_hud_splash(&mut self, recipient: SeatNo, field: String, slot: i32, optional: i32) {
        self.push_card(recipient, QueuedCallingCardKind::Splash { card_key: field, slot, spare: optional });
    }

    pub fn push_player_card_slot(
        &mut self,
        to: SeatNo,
        from: SeatNo,
        slot: i32,
    ) {
        self.push_card(to, QueuedCallingCardKind::SetSlot { source: from, slot });
    }

    pub fn push_player_card_open(
        &mut self,
        to: SeatNo,
        menu_index: i32,
    ) {
        self.push_card(to, QueuedCallingCardKind::OpenMenu { config_slot_number: menu_index });
    }

    pub fn take_pending_player_cards(
        &mut self,
    ) -> Vec<QueuedCallingCardEvent> {
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
    pub const fn take_pending_final_kill(
        &mut self,
    ) -> Option<(SeatNo, SeatNo)> {
        None
    }

    #[must_use]
    pub const fn queued_last_kill(
        &self,
    ) -> Option<(SeatNo, SeatNo)> {
        None
    }

    /// Every client with its match row, for the scoreboard.
    #[must_use]
    pub fn clients_scoreboard(
        &self,
    ) -> Vec<(SeatNo, PlayerMatchRecord)> {
        self.roster.members.clone()
    }
}
