//! Snapshots: the world laid out for the network and the replay, and a world taken back from a snapshot.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use entity_t6::EntityState;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState};

use super::state::{PredictionRemoteBody, SimState};
use super::{ClientId, Tick};
use crate::equipment::ProjectileState;
use crate::gentity::ScriptMoverGentity;
use crate::identities::ScriptModelId;
use crate::item::DroppedItem;
use crate::match_state::{ClientSnapshotMeta, SnapshotMeta};
use crate::snapshot::Snapshot;
use crate::{AdoptReport, ClientLifecycle, DroppedItemAmmo, EntityRunKind};

/// What taking a snapshot in leaves for the caller: the report, and the rows it keeps outside the world.
type Adopted = (AdoptReport, Vec<ProjectileState>, Vec<ScriptMoverGentity>, Vec<DroppedItem>);

const KERNEL_INVALID: &str = "authoritative snapshot carried an invalid EntityKernel state";
const DUPLICATE_NUMBER: &str = "authoritative snapshot duplicated a typed dynamic entity number";

/// The rows a dropped item adds to the snapshot's ammo table.
fn ammo_rows(items: &[DroppedItem]) -> Vec<DroppedItemAmmo> {
    items
        .iter()
        .map(|item| DroppedItemAmmo {
            entnum: item.state.number,
            clip_r: item.clip_r,
            clip_l: item.clip_l,
            stock: item.stock,
            stock2: item.stock2,
            scavenger: i32::from(item.scavenger),
            owner_clear_ms: item.owner_clear_ms,
        })
        .collect()
}

/// The dropped items a snapshot's item entities stand for, each with the ammo the snapshot kept for it.
fn dropped_items_of(meta: &SnapshotMeta) -> Vec<DroppedItem> {
    let level_time = meta.entity_kernel.level_time_ms;
    meta.entities
        .iter()
        .filter(|state| state.e_type == entity_t6::et::ITEM)
        .map(|state| {
            let ammo = meta
                .item_ammo
                .iter()
                .find(|ammo| ammo.entnum == state.number)
                .expect("authoritative ET_ITEM omitted ItemWeaponSetAmmo state");
            let pos = &state.lerp.pos;
            let path = entity_t6::Trajectory {
                tr_time: pos.tr_time,
                tr_type: pos.tr_type,
                tr_duration: pos.tr_duration,
                tr_delta: pos.tr_delta,
                tr_base: pos.tr_base,
            };
            DroppedItem {
                state: *state,
                origin: entity_t6::evaluate_trajectory(&path, level_time),
                falling: pos.tr_type == entity_t6::trajectory::tr_type::GRAVITY,
                clip_r: ammo.clip_r,
                clip_l: ammo.clip_l,
                stock: ammo.stock,
                stock2: ammo.stock2,
                scavenger: ammo.scavenger != 0,
                owner_clear_ms: ammo.owner_clear_ms,
            }
        })
        .collect()
}

/// The living players other than `local`, where the snapshot has them: a prediction world keeps only these.
fn remote_bodies(snapshot: &Snapshot, local: ClientId) -> Vec<PredictionRemoteBody> {
    snapshot
        .players
        .iter()
        .filter(|(id, _)| *id != local)
        .filter_map(|(id, ps)| {
            let meta = snapshot.meta.for_client(*id).expect("authoritative player row omitted client meta");
            (meta.lifecycle == ClientLifecycle::Alive).then_some(PredictionRemoteBody {
                client: *id,
                origin: ps.origin,
                life_sequence: meta.life_sequence,
            })
        })
        .collect()
}

/// Panics when a snapshot's entity rows disagree with its kernel: every typed entity (mover, item, missile) must
/// be unique and sit in a kernel slot of its own kind, and an item must come with its ammo.
fn assert_entity_runtime_snapshot(snapshot: &Snapshot) {
    let meta = &snapshot.meta;
    meta.entity_kernel.validate().expect(KERNEL_INVALID);

    let mut seen = HashSet::new();
    let mut claim = |number, kind: EntityRunKind, message: &str| {
        assert!(seen.insert(number), "{DUPLICATE_NUMBER}");
        assert_eq!(meta.entity_kernel.occupied_kind(number), Some(kind), "{message}");
    };

    for mover in &meta.script_movers {
        claim(mover.state.number, EntityRunKind::ScriptMover, "script mover does not occupy a ScriptMover kernel slot");
        assert!(
            meta.entities.contains(&mover.state),
            "typed script mover is absent from entityState presentation rows"
        );
    }
    let of_type = |e_type| meta.entities.iter().filter(move |state| state.e_type == e_type);
    for state in of_type(entity_t6::et::ITEM) {
        claim(state.number, EntityRunKind::Item, "ET_ITEM does not occupy an Item kernel slot");
        assert!(
            meta.item_ammo.iter().any(|ammo| ammo.entnum == state.number),
            "ET_ITEM omitted ItemWeaponSetAmmo state"
        );
    }
    for state in of_type(entity_t6::et::MISSILE) {
        claim(state.number, EntityRunKind::Missile, "ET_MISSILE does not occupy a Missile kernel slot");
    }
}

impl SimState {
    // ---- out --------------------------------------------------------------------------------------------------

    /// One snapshot row per client, with the parts that live outside the client's own record filled in.
    fn client_rows(&self, players: &[(ClientId, PlayerState)]) -> Vec<(ClientId, ClientSnapshotMeta)> {
        self.roster
            .members
            .iter()
            .map(|(id, member)| {
                let mut meta = member.to_snapshot_meta();
                if let (Some(view), Some(gun), Some((_, ps))) =
                    (meta.t6_view.as_mut(), member.t6_gun.as_deref(), players.iter().find(|(p, _)| p == id))
                {
                    view.ever_had = gun.ever_had_bits(&ps.weapons);
                    view.needs_rechamber = gun.needs_rechamber_bits(&ps.weapons);
                }
                (meta.hud_archival, meta.hud_current) = crate::hudelem::hud_elem_update_client(
                    &self.hud.elems,
                    *id,
                    meta.client_state_team,
                    crate::hudelem::HUDELEM_UPDATE_BOTH,
                );
                (*id, meta)
            })
            .collect()
    }

    /// The entity rows clients draw: script movers, then the flying missiles, the ones just gone, and the items.
    fn entity_rows(
        &self,
        movers: &[ScriptMoverGentity],
        projectiles: &[ProjectileState],
        items: &[DroppedItem],
    ) -> Vec<EntityState> {
        let flying = projectiles.iter().filter(|p| p.entnum != ENTITYNUM_NONE).map(|p| {
            crate::gentity::init_missile_state(p.entnum, p.weapon, p.pos, p.apos, p.launch_time)
        });
        movers
            .iter()
            .map(|mover| mover.state)
            .chain(flying)
            .chain(self.space.dying_missiles.iter().copied())
            .chain(items.iter().map(|item| item.state))
            .collect()
    }

    /// The skeletons of the entities a client may see: all of them, but a hidden entity only once a script has
    /// shown it.
    fn visible_dobjs(
        &self,
        movers: &[ScriptMoverGentity],
    ) -> Vec<(crate::AuthorityModelOwner, xmodel_runtime::DObjSemanticState)> {
        let shown: BTreeSet<ScriptModelId> =
            movers.iter().filter(|mover| mover.shown_to != 0).map(|mover| mover.id).collect();
        self.space
            .caps
            .iter()
            .filter(|caps| !caps.hidden || caps.owner.script_model().is_some_and(|id| shown.contains(&id)))
            .filter_map(|caps| caps.dobj.as_ref().map(|dobj| (caps.owner, dobj.semantic_state.clone())))
            .collect()
    }

    /// The whole world at `tick`, given the rows of things the caller keeps outside it.
    pub(crate) fn snapshot_with_dynamic_rows(
        &self,
        tick: Tick,
        players: Vec<(ClientId, PlayerState)>,
        projectiles: Vec<ProjectileState>,
        script_movers: Vec<ScriptMoverGentity>,
        dropped_items: Vec<DroppedItem>,
    ) -> Snapshot {
        let elapsed = self.clock.elapsed_ms;
        let mut world_objects = self.world_objects.to_snapshot();
        world_objects.as_of_ms = i32::try_from(elapsed).unwrap_or(i32::MAX);
        let meta = SnapshotMeta {
            objectives: self.objectives.clone(),
            phase: self.clock.phase,
            match_elapsed_ms: elapsed,
            prematch: self.clock.prematch,
            score_limit: self.clock.bootstrap.score_limit,
            time_limit_ms: self.clock.bootstrap.time_limit_ms,
            kind: self.clock.bootstrap.kind,
            clients: self.client_rows(&players),
            journal: self.outbox.journal.clone(),
            entity_events: self.outbox.entity_events.clone(),
            pellet_fx: self.outbox.pellet_fx.clone(),
            debris: self.outbox.debris.clone(),
            oriented_blockers: self.blockers.oriented.clone(),
            enemy_models: self.blockers.enemy_models.clone(),
            feel: self.blockers.feel,
            sound_aliases: self.strings.sound_alias.occupied(),
            effect_names: self.strings.effect_name.occupied(),
            hud_materials: self.strings.hud_material.occupied(),
            hud_strings: self.strings.hud_string.occupied(),
            rng: self.rng_debug_meta(),
            world_objects,
            area_entities: self.space.area.as_ref().map(crate::AreaEntityWorldSnapshot::capture),
            entity_dobjs: self.visible_dobjs(&script_movers),
            entities: self.entity_rows(&script_movers, &projectiles, &dropped_items),
            item_ammo: ammo_rows(&dropped_items),
            entity_kernel: self.space.kernel.to_snapshot(),
            corpses: self.corpses,
            item_pickups: self.outbox.pickups.clone(),
            script_movers,
        };
        Snapshot { tick, players, projectiles, meta }
    }

    // ---- in ---------------------------------------------------------------------------------------------------

    /// Takes a full authoritative snapshot in.
    pub(crate) fn adopt_snapshot_state(&mut self, snapshot: &Snapshot) -> Adopted {
        self.adopt_from(snapshot, None)
    }

    /// Takes in what a prediction world of `local` keeps of an authoritative snapshot: his own row and shots.
    pub(crate) fn adopt_prediction_snapshot_state(&mut self, snapshot: &Snapshot, local: ClientId) -> Adopted {
        self.adopt_from(snapshot, Some(local))
    }

    /// Replaces the client rows with the snapshot's (only `local`'s when given) and returns how many it took.
    fn adopt_client_rows(&mut self, snapshot: &Snapshot, local: Option<ClientId>) -> usize {
        let rows: Vec<_> = snapshot
            .meta
            .clients
            .iter()
            .filter(|(id, _)| local.is_none_or(|only| *id == only))
            .map(|(id, meta)| {
                let mut row = self.client_meta(*id).cloned().unwrap_or_default();
                row.adopt_snapshot_meta(meta);
                if local.is_some()
                    && let Some((_, ps)) = snapshot.players.iter().find(|(c, _)| c == id)
                {
                    crate::t6_gun::rewind(&mut row, ps);
                }
                (*id, row)
            })
            .collect();
        let taken = rows.len();
        self.roster.members = rows;
        taken
    }

    /// The match clock, the random streams, and every table the snapshot carries by value.
    fn adopt_tables(&mut self, meta: &SnapshotMeta) {
        self.clock.phase = meta.phase;
        self.clock.elapsed_ms = meta.match_elapsed_ms;
        self.clock.prematch = meta.prematch;
        self.clock.running = true;
        self.dice.spawn.restore_draws(meta.rng.spawn_draws);
        self.dice.combat.restore_draws(meta.rng.combat_draws);
        self.dice.bot.restore_draws(meta.rng.bot_draws);
        self.world_objects.adopt_snapshot(&meta.world_objects);
        self.space.area = meta
            .area_entities
            .as_ref()
            .map(|saved| saved.restore().expect("authoritative snapshot carried invalid CM area-sector state"));
        self.strings.sound_alias.adopt_occupied(&meta.sound_aliases);
        self.strings.effect_name.adopt_occupied(&meta.effect_names);
        self.strings.hud_material.adopt_occupied(&meta.hud_materials);
        self.strings.hud_string.adopt_occupied(&meta.hud_strings);
        self.corpses = meta.corpses;
    }

    fn adopt_from(&mut self, snapshot: &Snapshot, local: Option<ClientId>) -> Adopted {
        assert_entity_runtime_snapshot(snapshot);
        let meta = &snapshot.meta;
        let level_time = crate::level_time_ms(snapshot.tick);

        let adopted_players = local.map_or(snapshot.players.len(), |only| {
            usize::from(snapshot.players.iter().any(|(id, _)| *id == only))
        });
        let before = adopted_players + self.roster.members.len();

        self.blockers.oriented = meta.oriented_blockers.clone();
        self.blockers.enemy_models = meta.enemy_models.clone();
        self.blockers.feel = meta.feel;
        self.bodies.remote = local.map_or_else(Vec::new, |only| remote_bodies(snapshot, only));
        let clients = self.adopt_client_rows(snapshot, local);

        self.space.kernel = crate::EntityKernel::from_snapshot(&meta.entity_kernel).expect(KERNEL_INVALID);

        let projectiles: Vec<ProjectileState> = snapshot
            .projectiles
            .iter()
            .filter(|projectile| local.is_none_or(|only| projectile.owner == only))
            .copied()
            .collect();
        self.objectives = meta.objectives.clone();
        crate::presence::adopt_brush_movers(&mut self.space.caps, &meta.script_movers, level_time);
        crate::presence::follow_movers(&mut self.space.caps, &meta.script_movers, level_time);

        let flying: HashSet<i32> = projectiles.iter().map(|projectile| projectile.entnum).collect();
        self.space.dying_missiles = meta
            .entities
            .iter()
            .filter(|state| state.e_type == entity_t6::et::MISSILE && !flying.contains(&state.number))
            .copied()
            .collect();
        self.adopt_tables(meta);

        self.roster.prev_buttons.clear();
        self.roster.prev_angles.clear();
        self.roster.prev_moves.clear();

        let report = AdoptReport {
            players: adopted_players,
            clients,
            projectiles: projectiles.len(),
            dropped: before.saturating_sub(adopted_players + self.roster.members.len()),
            content_mismatch: meta.rng.root_seed != self.dice.seed,
        };
        (report, projectiles, meta.script_movers.clone(), dropped_items_of(meta))
    }

    // ---- prediction worlds ------------------------------------------------------------------------------------

    /// Makes this world play on what `other` plays on, so a prediction world agrees with the authority about the
    /// map without having loaded it.
    pub fn initialize_prediction_from(&mut self, other: &SimState) {
        self.content = Arc::clone(&other.content);
        self.reset_area_world();
        self.space.caps = other.space.caps.clone();
        self.space.models = Arc::clone(&other.space.models);
        self.clock.bootstrap = other.clock.bootstrap.clone();
        self.dice.seed = other.dice.seed;
        self.world_objects.clone_glass_panes_from(&other.world_objects);
        self.refresh_digest();
    }

    /// Stops this world from publishing snapshots and counting deaths; a prediction world calls this.
    pub fn suppress_snapshot_publish(&mut self) {
        self.publish_snapshot = false;
    }

    #[must_use]
    pub fn publishes_snapshot(&self) -> bool {
        self.publish_snapshot
    }
}
