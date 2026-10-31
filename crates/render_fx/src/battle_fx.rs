use assets::StagedWeapons;
use bevy::prelude::*;
use fx::{EffectClockMs, EffectSystemHost, PlaybackResult, basis_off_contact_facing};
use fx_t6::runtime::{
    HIT_ON_WAY_OUT_BIT, MATERIAL_KIND_BODY, flesh_effect_index, impact_table_row,
};
use net::EntityTableCells;
use weapon_t6::surface::T6_SURFACE_TYPE_NAMES;

use crate::effect_play::{EffectElementInfoMemo, EffectStage};
use crate::effect_play::{spawn_named_attached_world_effect, play_named_oriented_in_level};
use crate::{BattleEffectsSnapshot, EffectJournalCursor, EffectsSystemHandle, JournalCount};
use crate::{ReadyFxBook, ReadyHitFx, StagedStreaks};
use crate::{TracerRenderGuard, TracerSpawnRefusal, StreakLevel, attempt_bullet_streak};

pub use fx::UNTURNED_BASIS;

/// True when the effect actually started (released or held); counts it in `played`.
fn started(result: Option<PlaybackResult>, count: &mut u32) -> bool {
    let did_start = matches!(
        result,
        Some(PlaybackResult::StartedLetGo { .. } | PlaybackResult::Kept { .. })
    );
    if did_start {
        *count = count.saturating_add(1);
    }
    did_start
}

pub fn try_play_gun_fx_at_origin(
    effect_host: &mut EffectSystemHost,
    defs: &asset_game::FxDefinitions,
    info_cache: &mut EffectElementInfoMemo,
    name: Option<asset_game::FxName<'_>>,
    at: [f32; 3],
    frame: [[f32; 3]; 3],
    count: &mut u32,
    world_scene: Option<&dyn EffectStage>,
) -> bool {
    let Some(name) = name else {
        return false;
    };
    info_cache.restock(defs);
    let result = play_named_oriented_in_level(effect_host, defs, info_cache, name, at, frame, world_scene);
    started(result, count)
}

pub fn try_play_gun_fx_bolted(
    effect_host: &mut EffectSystemHost,
    defs: &asset_game::FxDefinitions,
    info_cache: &mut EffectElementInfoMemo,
    name: Option<asset_game::FxName<'_>>,
    target: Option<fx::EffectBoltTarget>,
    count: &mut u32,
    world_scene: Option<&dyn EffectStage>,
) -> bool {
    let Some((name, target)) = name.zip(target) else {
        return false;
    };
    info_cache.restock(defs);
    let result = spawn_named_attached_world_effect(effect_host, defs, info_cache, name, target, world_scene);
    started(result, count)
}

pub fn spawn_casing_ejection(
    effect_host: &mut EffectSystemHost,
    defs: &asset_game::FxDefinitions,
    info_cache: &mut EffectElementInfoMemo,
    combat_fx: Option<&asset_game::WeaponBattleEffects>,
    first_person: bool,
    prior_shot: bool,
    target: Option<fx::EffectBoltTarget>,
    journal: &mut EffectJournalCursor,
    world_scene: Option<&dyn EffectStage>,
) {
    let brass = combat_fx.and_then(|fx| fx.brass_present_for_event(first_person, prior_shot));
    let started = try_play_gun_fx_bolted(
        effect_host,
        defs,
        info_cache,
        brass,
        target,
        journal.slot(JournalCount::BrassPlayed),
        world_scene,
    );
    if !started {
        journal.bump(JournalCount::BrassMissed);
    }
}

/// Where a missile's `tag_fx` bone currently is, if its pose is live this frame.
pub fn rocket_arrow_victim(
    poses: Option<&render_anim::HostSkeletonFrame>,
    meshes: &asset_model::MissileMeshRoster,
    namespace: asset_core::AssetCategoryScope,
    prop: &str,
    entnum: u32,
) -> Option<fx::EffectBoltTarget> {
    let bone = meshes
        .by_label(namespace, prop)?
        .bone_rig
        .joint_labels
        .iter()
        .position(|name| name == "tag_fx")
        .and_then(|index| u16::try_from(index).ok())?;
    let live = poses?.find_live_mount(entnum, bone)?;
    let orientation = live.orientation?;
    Some(fx::EffectBoltTarget {
        dobj: entnum,
        bone,
        client_actor_teleport: live.client_actor_teleport,
        orientation,
    })
}

/// The effect names an explosion could play, and the impact-table row they came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlastEffectNameSet<'a> {
    pub entry: Option<usize>,
    pub table: Option<asset_game::FxName<'a>>,
    pub slot: Option<asset_game::FxName<'a>>,
}

pub fn explosion_effect_names<'a>(
    impact_kind: Option<i32>,
    surface_type: u8,
    table: Option<&'a asset_game::SelfContainedImpactFxTable>,
    slot: Option<asset_game::FxName<'a>>,
) -> BlastEffectNameSet<'a> {
    let row = impact_kind.and_then(|kind| match table {
        Some(table) => table.hit_fx_row(kind, false),
        None => impact_table_row(kind, false),
    });
    let surface = usize::from(surface_type);
    let body_material_slot = (surface == MATERIAL_KIND_BODY).then_some(0);
    BlastEffectNameSet {
        table: row
            .zip(table)
            .and_then(|(row, table)| table.effect_label(row, surface, body_material_slot)),
        slot,
        entry: row,
    }
}

fn shot_log_enabled() -> bool {
    static LOG_ONCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOG_ONCE.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some())
}

/// Origin of the weapon's flash tag for the first segment of a pellet: the local player's
/// viewmodel tag for their own shots, the remote body's tag for everyone else's.
fn flash_tag_origin(
    fired_by_me: bool,
    hand: u8,
    attacker: i32,
    remote_bolts: &Query<&render_anim::OtherBodyEffectTags>,
    view_bolts: &render_anim::AttachTagGoals,
    entity_slots: &EntityTableCells,
) -> Option<[f32; 3]> {
    let target = if fired_by_me {
        let last = view_bolts.flash.len() - 1;
        view_bolts.flash[usize::from(hand).min(last)]
    } else {
        let number = u16::try_from(attacker).ok()?;
        let entity = entity_slots.object_at_index(number)?;
        remote_bolts.get(entity).ok()?.flash
    };
    target.map(|t| t.orientation.origin)
}

/// Books the outcome of a tracer spawn attempt: spawned, or the reason it was skipped.
fn count_tracer(
    journal: &mut EffectJournalCursor,
    spawned: Option<Result<(), TracerSpawnRefusal>>,
    from_tag: bool,
) {
    let outcome = match spawned {
        Some(Ok(())) => JournalCount::TracerSpawned,
        Some(Err(TracerSpawnRefusal::Interval)) => JournalCount::TracerSkippedInterval,
        Some(Err(TracerSpawnRefusal::Short)) => JournalCount::TracerSkippedShort,
        _ => JournalCount::TracerSkippedNoDef,
    };
    if from_tag && outcome == JournalCount::TracerSpawned {
        journal.bump(JournalCount::TracerFromTag);
    }
    journal.bump(outcome);
}

/// Warns the first time the impact table names no effect to play, then only counts.
fn warn_impact_gap_once(journal: &mut EffectJournalCursor, what: impl FnOnce() -> String) {
    if journal.claim_impact_warning() {
        diag::warn!(World, "{}", what());
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_pellet_leg_effect(
    attacker: i32,
    gun_catalog_no: u32,
    shot_serial: u32,
    pellet_no: u16,
    hand: u8,
    ray_from: [f32; 3],
    ray_to: [f32; 3],
    hit_normal: [f32; 3],
    surface_type: u8,
    face_property_bits: u32,
    flesh_bits: u32,
    // bo2mp: false for the hit on the local player's own body: Black Ops
    // II's own-hit event only sounds; his game never draws the puff on
    // himself.
    impact: bool,
    remote_bolts: &Query<&render_anim::OtherBodyEffectTags>,
    view_bolts: &render_anim::AttachTagGoals,
    entity_slots: &EntityTableCells,
    defs: Option<&ReadyFxBook>,
    elem_cache: &mut EffectElementInfoMemo,
    impact_defs: Option<&ReadyHitFx>,
    weapon_db: Option<&StagedWeapons>,
    tracer_defs: Option<&StagedStreaks>,
    streak_level: &mut StreakLevel,
    draw_gate: &mut TracerRenderGuard,
    is_own_player: i32,
    effect_host: &mut EffectsSystemHandle,
    journal: &mut EffectJournalCursor,
    dump: &mut BattleEffectsSnapshot,
    world_scene: Option<&dyn EffectStage>,
) {
    // One glass key per pellet: shot, shooter and pellet number packed so no two collide.
    let pellet_key =
        u64::from(shot_serial) << 32 ^ u64::from(attacker as u32) << 16 ^ u64::from(pellet_no);
    effect_host.0.glass.hit_segment(ray_from, ray_to, pellet_key);

    let fired_by_me = attacker == is_own_player;
    let emitter_key = attacker.max(0) as u32;
    let streak_edge = weapon_db
        .and_then(|w| w.0.effects_for(gun_catalog_no))
        .map_or(assets::AssetDependencyLink::NothingThere, |fx| fx.tracer);

    // Only a pellet's first segment starts at the weapon's flash tag; later ones continue the ray.
    let tag_start = draw_gate
        .opening_leg_of_pellet(emitter_key, shot_serial, pellet_no)
        .then(|| {
            flash_tag_origin(fired_by_me, hand, attacker, remote_bolts, view_bolts, entity_slots)
        })
        .flatten();
    let start = tag_start.unwrap_or(ray_from);
    let logging = shot_log_enabled();
    if logging {
        let surface_name = T6_SURFACE_TYPE_NAMES.get(usize::from(surface_type)).copied();
        diag::info!(
            World,
            "shot fx: weapon {gun_catalog_no} pellet {pellet_no} own {fired_by_me} tracer {:?} tag {} start {:?} seg {:?} end {:?} surf {} ({})",
            streak_edge.dependency_cause_of(),
            tag_start.is_some(),
            start.map(|v| v.round()),
            ray_from.map(|v| v.round()),
            ray_to.map(|v| v.round()),
            surface_type,
            surface_name.unwrap_or("?")
        );
    }
    let clock = EffectClockMs::from_host(&effect_host.0);
    let spawned = tracer_defs.map(|recipe_book| {
        attempt_bullet_streak(
            draw_gate,
            streak_level,
            &recipe_book.0,
            streak_edge,
            emitter_key,
            start,
            ray_to,
            fired_by_me,
            clock,
            dump,
        )
    });
    if logging {
        diag::info!(
            World,
            "shot fx: tracer spawn {spawned:?} live {}",
            streak_level.pool.active_total()
        );
    }
    count_tracer(journal, spawned, tag_start.is_some());

    // Nothing more to draw: the local player's own body takes sound only, and a hit with no
    // normal has no surface to orient the puff on.
    if !impact {
        return;
    }
    if hit_normal == [0.0; 3] {
        journal.bump(JournalCount::ImpactTableMisses);
        report_combat_fx_gaps(journal);
        return;
    }
    let Some(facts) = weapon_db.and_then(|w| w.0.summary_for(gun_catalog_no)) else {
        journal.bump(JournalCount::ImpactTableMisses);
        return;
    };
    play_impact_table_room(
        facts.impact_kind_id,
        ray_to,
        hit_normal,
        surface_type,
        face_property_bits,
        flesh_bits,
        defs,
        impact_defs,
        elem_cache,
        effect_host,
        journal,
        world_scene,
    );
}

pub fn play_impact_table_room(
    impact_kind: i32,
    at: [f32; 3],
    hit_normal: [f32; 3],
    surface_type: u8,
    face_property_bits: u32,
    flesh_bits: u32,
    defs: Option<&ReadyFxBook>,
    impact_defs: Option<&ReadyHitFx>,
    info_cache: &mut EffectElementInfoMemo,
    effect_host: &mut EffectsSystemHandle,
    journal: &mut EffectJournalCursor,
    world_scene: Option<&dyn EffectStage>,
) {
    let leaving = face_property_bits & HIT_ON_WAY_OUT_BIT != 0;
    let table = impact_defs.and_then(|fx| fx.0.as_ref());
    let row = table.map_or_else(
        || impact_table_row(impact_kind, leaving),
        |table| table.hit_fx_row(impact_kind, leaving),
    );
    let Some(entry) = row else {
        journal.bump(JournalCount::ImpactTableMisses);
        return;
    };
    let surface = usize::from(surface_type);
    let body_material_slot = (surface == MATERIAL_KIND_BODY).then(|| flesh_effect_index(flesh_bits));

    let Some(cell_name) = table.and_then(|t| t.effect_label(entry, surface, body_material_slot)) else {
        journal.bump(JournalCount::ImpactMissed);
        warn_impact_gap_once(journal, || {
            format!(
                "fx: BulletImpact row={entry} surf={surface} — no ImpactFx cell name; play_oriented skipped (further misses counted)"
            )
        });
        return;
    };
    let Some(defs) = defs else {
        return;
    };
    info_cache.restock(&defs.0);
    let axis = basis_off_contact_facing(hit_normal);
    let started = play_named_oriented_in_level(
        &mut effect_host.0,
        &defs.0,
        info_cache,
        cell_name,
        at,
        axis,
        world_scene,
    );
    match started {
        Some(PlaybackResult::StartedLetGo { .. } | PlaybackResult::Kept { .. }) => {
            journal.bump(JournalCount::ImpactPlayed);
        }
        Some(PlaybackResult::Refused(why)) => {
            journal.bump(JournalCount::ImpactMissed);
            diag::warn!(World, "fx: play_oriented `{cell_name}` failed: {why:?}");
        }
        None => {
            journal.bump(JournalCount::ImpactMissed);
            warn_impact_gap_once(journal, || {
                format!("fx: BulletImpact cell `{cell_name}` not in FxCatalog — play_oriented skipped")
            });
            return;
        }
    }
    report_combat_fx_gaps(journal);
}

/// Logs the journal counters once, the first time any of them shows activity.
pub fn report_combat_fx_gaps(journal: &mut EffectJournalCursor) {
    use JournalCount::*;
    let quiet = journal.tracers_skipped() == 0
        && [
            MuzzlePlayed,
            MuzzleMissed,
            TracerSpawned,
            BrassPlayed,
            Pellets,
            ImpactMissed,
            BlastPlayed,
            BlastMissed,
            ImpactPlayed,
        ]
        .into_iter()
        .all(|kind| journal.tally_of(kind) == 0);
    if quiet || !journal.claim_summary() {
        return;
    }
    let n = |kind| journal.tally_of(kind);
    diag::info!(
        World,
        "fx: muzzle[played={} missed={} bolted={}] tracer[spawned={} from_tag={} skipped_interval={} skipped_short={} skipped_no_def={}] brass[played={} missed={}] pellets={} impact[played={} missed={} table_misses={}] blast[played={} missed={}] sound_missed[fire={} blast={}]",
        n(MuzzlePlayed),
        n(MuzzleMissed),
        n(MuzzleBolted),
        n(TracerSpawned),
        n(TracerFromTag),
        n(TracerSkippedInterval),
        n(TracerSkippedShort),
        n(TracerSkippedNoDef),
        n(BrassPlayed),
        n(BrassMissed),
        n(Pellets),
        n(ImpactPlayed),
        n(ImpactMissed),
        n(ImpactTableMisses),
        n(BlastPlayed),
        n(BlastMissed),
        n(FireSoundMissed),
        n(BlastSoundMissed),
    );
}
