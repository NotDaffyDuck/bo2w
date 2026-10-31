//! Moving a whole world in and out of a [`TickSnapshot`]: laying it out for the network and the replay, and taking an
//! authority's snapshot back in (fully, or only the parts a client's prediction world keeps).

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use entity_t6::EntitySnapshot;
use playerstate_t6::game::{AvatarState, NO_ENTITY_SLOT};

use super::state::{MatchSimulation, PredictedRemotePlayer};
use super::{SeatNo, StepNo};
use crate::gear::FlyingProjectile;
use crate::ent_slots::MoverSlot;
use crate::match_ids::ScriptedModel;
use crate::floor_guns::LootOnGround;
use crate::player_ledger::{SeatTickNotes, TickLedgerNotes};
use crate::tick_publish::TickSnapshot;
use crate::{DroppedRounds, EntityBehaviorClass, PlayerSessionStage, TakeoverReport};

const KERNEL_INVALID: &str = "authoritative snapshot carried an invalid EntityKernel state";
const DUPLICATE_NUMBER: &str = "authoritative snapshot duplicated a typed dynamic entity number";
const ITEM_WITHOUT_AMMO: &str = "authoritative item entity came without its dropped-gun ammo row";

fn is_kind(e_type: i16) -> impl Fn(&&EntitySnapshot) -> bool {
    move |row| row.e_type == e_type
}

/// The ammo row the snapshot stores for one dropped item.
fn ammo_row(item: &LootOnGround) -> DroppedRounds {
    let &LootOnGround { state, right_mag: clip_r, left_mag: clip_l, spare: stock, stock2, scavenge: scavenger, owner_clear_ms, .. } = item;
    DroppedRounds { entnum: state.slot_index, clip_r, clip_l, stock, stock2, scavenger: scavenger.into(), owner_clear_ms }
}

/// Rebuilds the dropped items from a snapshot: each item entity joined with its ammo row, placed where its
/// trajectory has it at the kernel's level time.
fn dropped_items_of(meta: &TickLedgerNotes) -> Vec<LootOnGround> {
    let level_ms = meta.entity_kernel.level_time_ms;
    let ammo_for =
        |number: i32| meta.item_ammo.iter().find(|row| row.entnum == number).copied().expect(ITEM_WITHOUT_AMMO);
    let mut floor_rows = Vec::new();
    for row in meta.entities.iter().filter(is_kind(entity_t6::et::ITEM)) {
        let DroppedRounds { clip_r, clip_l, stock, stock2, scavenger, owner_clear_ms, .. } = ammo_for(row.slot_index);
        let lerp = &row.lerp.pos;
        let path = entity_t6::Trajectory {
            tr_type: lerp.tr_type,
            began_at: lerp.began_at,
            span_ms: lerp.span_ms,
            base_point: lerp.base_point,
            rate: lerp.rate,
        };
        floor_rows.push(LootOnGround {
            state: *row,
            origin: entity_t6::evaluate_trajectory(&path, level_ms),
            dropping: lerp.tr_type == entity_t6::flight::path_kind::FALLING,
            right_mag: clip_r,
            left_mag: clip_l,
            spare: stock,
            stock2,
            scavenge: scavenger != 0,
            owner_clear_ms,
        });
    }
    floor_rows
}

/// Every living player in the snapshot except `local`, as a prediction world stores them.
fn remote_bodies(snapshot: &TickSnapshot, local: SeatNo) -> Vec<PredictedRemotePlayer> {
    let mut bodies = Vec::new();
    for (who, avatar) in &snapshot.players {
        if *who == local {
            continue;
        }
        let row = snapshot.meta.for_client(*who).expect("authoritative player row omitted client meta");
        if row.lifecycle == PlayerSessionStage::Alive {
            bodies.push(PredictedRemotePlayer { client: *who, spot: avatar.origin, life_sequence: row.life_sequence });
        }
    }
    bodies
}

/// Panics unless the snapshot's typed entities (script movers, items, missiles) agree with its entity kernel:
/// each number used once, each in a kernel slot of its own kind, each mover also among the drawn rows, each item
/// with an ammo row.
fn verify_entity_snapshot_shape(snapshot: &TickSnapshot) {
    let meta = &snapshot.meta;
    let kernel = &meta.entity_kernel;
    kernel.validate().expect(KERNEL_INVALID);

    let mut spent = HashSet::new();
    let mut expect_slot = |number, kind: EntityBehaviorClass, complaint: &str| {
        assert!(spent.insert(number), "{DUPLICATE_NUMBER}");
        assert_eq!(kernel.holder_class(number), Some(kind), "{complaint}");
    };

    for mover in &meta.script_movers {
        expect_slot(mover.state.slot_index, EntityBehaviorClass::ScriptedMover, "script mover sits outside a MoverSlot slot");
        assert!(meta.entities.contains(&mover.state), "script mover has no matching drawn entity row");
    }
    for row in meta.entities.iter().filter(is_kind(entity_t6::et::ITEM)) {
        expect_slot(row.slot_index, EntityBehaviorClass::Item, "ET_ITEM does not occupy an Item kernel slot");
        assert!(meta.item_ammo.iter().any(|ammo| ammo.entnum == row.slot_index), "{ITEM_WITHOUT_AMMO}");
    }
    for row in meta.entities.iter().filter(is_kind(entity_t6::et::MISSILE)) {
        expect_slot(row.slot_index, EntityBehaviorClass::Missile, "ET_MISSILE does not occupy a Missile kernel slot");
    }
}

impl MatchSimulation {
    // Writing a snapshot.

    /// Each roster member's snapshot row, completed with what lives outside the member record: the gun bits
    /// worked out against the player's weapons, and the HUD element banks that client may see.
    fn client_rows(&self, players: &[(SeatNo, AvatarState)]) -> Vec<(SeatNo, SeatTickNotes)> {
        let mut rows = Vec::with_capacity(self.roster.members.len());
        for (who, member) in &self.roster.members {
            let mut row = member.as_wire_meta();
            let avatar = players.iter().find(|(owner, _)| owner == who).map(|(_, avatar)| avatar);
            if let (Some(view), Some(gun), Some(avatar)) = (row.t6_view.as_mut(), member.t6_gun.as_deref(), avatar) {
                view.ever_had = gun.ever_had_bits(&avatar.weapons);
                view.needs_rechamber = gun.needs_rechamber_bits(&avatar.weapons);
            }
            let team = row.client_state_team;
            let banks =
                crate::script_overlay::hud_sync_bank_for_player(&self.hud.elems, *who, team, crate::script_overlay::HUD_SYNC_ALL_BANKS);
            (row.hud_archival, row.hud_current) = banks;
            rows.push((*who, row));
        }
        rows
    }

    /// The drawn entity rows, in order: script movers, missiles in flight, missiles just gone, dropped items.
    fn entity_rows(
        &self,
        movers: &[MoverSlot],
        projectiles: &[FlyingProjectile],
        items: &[LootOnGround],
    ) -> Vec<EntitySnapshot> {
        let mut rows: Vec<EntitySnapshot> = movers.iter().map(|mover| mover.state).collect();
        for shot in projectiles.iter().filter(|shot| shot.ent_no != NO_ENTITY_SLOT) {
            rows.push(crate::ent_slots::shell_state_at(shot.ent_no, shot.gun, shot.at, shot.turn_path, shot.spawn_moment));
        }
        rows.extend_from_slice(&self.space.dying_missiles);
        rows.extend(items.iter().map(|item| item.state));
        rows
    }

    /// The posed models clients may see. A hidden one is left out unless it belongs to a script mover that a
    /// script has shown to someone.
    fn visible_dobjs(
        &self,
        movers: &[MoverSlot],
    ) -> Vec<(crate::ServerModelSource, xmodel_runtime::RigPoseRecord)> {
        let revealed: BTreeSet<ScriptedModel> =
            movers.iter().filter(|mover| mover.shown_to != 0).map(|mover| mover.id).collect();
        let may_see = |hidden: bool, owner: crate::ServerModelSource| {
            !hidden || owner.scripted_model().is_some_and(|id| revealed.contains(&id))
        };
        let mut out = Vec::new();
        for entry in &self.space.caps {
            if let Some(model) = entry.dobj.as_ref().filter(|_| may_see(entry.unseen, entry.owner)) {
                out.push((entry.owner, model.bone_role_state.clone()));
            }
        }
        out
    }

    /// The whole world as of `tick`. The players, shells, movers and loot on the floor come from the caller,
    /// which keeps them outside this struct.
    pub(crate) fn snapshot_plus_live_rows(
        &self,
        tick: StepNo,
        players: Vec<(SeatNo, AvatarState)>,
        shells: Vec<FlyingProjectile>,
        script_movers: Vec<MoverSlot>,
        dropped_items: Vec<LootOnGround>,
    ) -> TickSnapshot {
        let elapsed = self.clock.elapsed_ms;
        let bootstrap = &self.clock.bootstrap;
        let outbox = &self.outbox;
        let strings = &self.strings;
        let mut props = self.world_objects.export_snapshot();
        props.as_of_ms = i32::try_from(elapsed).unwrap_or(i32::MAX);

        let meta = TickLedgerNotes {
            phase: self.clock.phase,
            match_clock_ms: elapsed,
            prematch: self.clock.prematch,
            score_limit: bootstrap.win_score,
            time_limit_ms: bootstrap.round_ms,
            kind: bootstrap.kind,
            objectives: self.objectives.clone(),
            clients: self.client_rows(&players),

            journal: outbox.journal.clone(),
            entity_events: outbox.entity_events.clone(),
            pellet_fx: outbox.pellet_fx.clone(),
            debris: outbox.debris.clone(),
            item_pickups: outbox.pickups.clone(),

            oriented_blockers: self.blockers.oriented.clone(),
            follows: self.blockers.follows.clone(),
            enemy_models: self.blockers.enemy_models.clone(),
            feel: self.blockers.feel,

            sound_aliases: strings.sound_alias.occupied(),
            effect_names: strings.effect_label.occupied(),
            hud_materials: strings.hud_material.occupied(),
            hud_strings: strings.hud_string.occupied(),

            rng: self.random_debug_info(),
            world_objects: props,
            corpses: self.corpses,

            area_entities: self.space.area.as_ref().map(crate::ZoneThingTable::capture),
            entity_kernel: self.space.kernel.export_snapshot(),
            entity_dobjs: self.visible_dobjs(&script_movers),
            entities: self.entity_rows(&script_movers, &shells, &dropped_items),
            item_ammo: dropped_items.iter().map(ammo_row).collect(),
            script_movers,
        };
        TickSnapshot { tick, players, projectiles: shells, meta }
    }

    // Reading a snapshot.

    /// Takes in an authoritative snapshot whole.
    pub(crate) fn adopt_snapshot_state(&mut self, snapshot: &TickSnapshot) -> (TakeoverReport, Vec<FlyingProjectile>, Vec<MoverSlot>, Vec<LootOnGround>) {
        self.adopt_from(snapshot, None)
    }

    /// Takes in the part of an authoritative snapshot that `local`'s prediction world keeps: his own client row
    /// and his own projectiles.
    pub(crate) fn adopt_prediction_snapshot_state(&mut self, snap: &TickSnapshot, local: SeatNo) -> (TakeoverReport, Vec<FlyingProjectile>, Vec<MoverSlot>, Vec<LootOnGround>) {
        self.adopt_from(snap, Some(local))
    }

    /// Swaps the roster for the snapshot's client rows (only `local`'s, when given), each merged into what this
    /// world already had for that client. A prediction world also rewinds `local`'s gun to the snapshot's avatar.
    /// Returns the number of rows kept.
    fn adopt_client_rows(&mut self, snap: &TickSnapshot, local: Option<SeatNo>) -> usize {
        let mut members = Vec::new();
        for (who, incoming) in &snap.meta.clients {
            if local.is_some_and(|only| only != *who) {
                continue;
            }
            let mut row = self.client_meta(*who).cloned().unwrap_or_default();
            row.take_over_snapshot_info(incoming);
            if local.is_some() {
                let avatar = snap.players.iter().find(|(owner, _)| owner == who);
                if let Some((_, avatar)) = avatar {
                    crate::gun_brain::rewind(&mut row, avatar);
                }
            }
            members.push((*who, row));
        }
        let kept = members.len();
        self.roster.members = members;
        kept
    }

    /// Restores the match clock, the random draws and every table the snapshot carries whole.
    fn adopt_tables(&mut self, meta: &TickLedgerNotes) {
        let clock = &mut self.clock;
        clock.phase = meta.phase;
        clock.elapsed_ms = meta.match_clock_ms;
        clock.prematch = meta.prematch;
        clock.going = true;

        let draws = &meta.rng;
        self.dice.spawn.set_drawn(draws.spawn_draws);
        self.dice.combat.set_drawn(draws.combat_draws);
        self.dice.bot.set_drawn(draws.bot_draws);

        self.world_objects.load_props(&meta.world_objects);
        self.space.area = match &meta.area_entities {
            Some(saved) => Some(saved.restore().expect("authoritative snapshot carried invalid CM area-sector state")),
            None => None,
        };

        let strings = &mut self.strings;
        strings.sound_alias.take_listed(&meta.sound_aliases);
        strings.effect_label.take_listed(&meta.effect_names);
        strings.hud_material.take_listed(&meta.hud_materials);
        strings.hud_string.take_listed(&meta.hud_strings);

        self.corpses = meta.corpses;
    }

    fn adopt_from(&mut self, snap: &TickSnapshot, local: Option<SeatNo>) -> (TakeoverReport, Vec<FlyingProjectile>, Vec<MoverSlot>, Vec<LootOnGround>) {
        verify_entity_snapshot_shape(snap);
        let meta = &snap.meta;
        let level_ms = crate::level_time_ms(snap.tick);
        let keeps_client = |client: SeatNo| local.is_none_or(|only| only == client);

        // A prediction world counts its one local player, if the snapshot has him.
        let player_count = match local {
            Some(only) => usize::from(snap.players.iter().any(|(client, _)| *client == only)),
            None => snap.players.len(),
        };
        let rows_before = player_count + self.roster.members.len();

        self.blockers.oriented = meta.oriented_blockers.clone();
        self.blockers.enemy_models = meta.enemy_models.clone();
        self.blockers.feel = meta.feel;
        self.bodies.remote = match local {
            Some(only) => remote_bodies(snap, only),
            None => Vec::new(),
        };
        let clients = self.adopt_client_rows(snap, local);

        self.space.kernel = crate::EntitySlotArena::from_world_state(&meta.entity_kernel).expect(KERNEL_INVALID);

        let kept_shots: Vec<FlyingProjectile> =
            snap.projectiles.iter().filter(|shot| keeps_client(shot.owner)).copied().collect();
        self.objectives = meta.objectives.clone();
        crate::mover_clip::adopt_brush_movers(&mut self.space.caps, &meta.script_movers, level_ms);
        crate::mover_clip::follow_movers(&mut self.space.caps, &meta.script_movers, level_ms);

        // A missile row with no live projectile behind it is one that just ended; clients still draw its end.
        let in_flight: HashSet<i32> = kept_shots.iter().map(|shot| shot.ent_no).collect();
        self.space.dying_missiles = meta
            .entities
            .iter()
            .filter(is_kind(entity_t6::et::MISSILE))
            .filter(|row| !in_flight.contains(&row.slot_index))
            .copied()
            .collect();
        self.adopt_tables(meta);

        let roster = &mut self.roster;
        roster.prev_buttons.clear();
        roster.prev_angles.clear();
        roster.prev_moves.clear();

        let rows_after = player_count + self.roster.members.len();
        let report = TakeoverReport {
            people: player_count,
            seats: clients,
            thrown: kept_shots.len(),
            let_go: rows_before.saturating_sub(rows_after),
            content_differs: meta.rng.root_seed != self.dice.seed,
        };
        (report, kept_shots, meta.script_movers.clone(), dropped_items_of(meta))
    }

    // Prediction worlds.

    /// Points this world at the same map content, models and match settings as `other`, so a client's prediction
    /// world agrees with the authority about the map without loading it itself.
    pub fn initialize_prediction_from(&mut self, other: &MatchSimulation) {
        self.content = Arc::clone(&other.content);
        self.reset_area_world();
        self.space.caps = other.space.caps.clone();
        self.space.models = Arc::clone(&other.space.models);
        self.clock.bootstrap = other.clock.bootstrap.clone();
        self.dice.seed = other.dice.seed;
        self.world_objects.fresh_panes_like(&other.world_objects);
        self.refresh_digest();
    }

    /// Makes this a prediction world: it stops publishing snapshots and stops counting deaths.
    pub fn hold_wire_meta(&mut self) {
        self.is_authority = false;
    }

    /// True for the authority's world, false for a prediction world.
    #[must_use]
    pub fn emits_state_snapshot(&self) -> bool {
        self.is_authority
    }
}
