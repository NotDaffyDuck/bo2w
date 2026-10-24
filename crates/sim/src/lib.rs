pub mod adopt;
mod area_world;
pub mod bullet;
pub mod bullet_collision;
mod carrier;
mod clip_fallback;
pub mod collision_census;
pub mod combat;
pub mod content;
pub mod guest;
mod corpse;
mod damage;
mod entity_local;
mod entity_run;
mod equipment;
mod frame;
mod gentity;
pub mod hudelem;
pub mod identities;
pub mod input;
mod item;
mod item_t6;
mod mantle_xanim;
pub mod match_state;
mod missile;
mod missile_guidance;
mod missile_decoy;
pub use missile_guidance::{MissileGuide, MissileTarget};
mod presence;
mod remote_missile;
pub mod script;
mod weapon_lock;
pub use weapon_lock::WeaponLock;
pub mod rules;
mod score;
pub mod script_gaps;
mod script_gap_ids;
mod script_player;
mod snapshot;
mod sound_alias_cs;
pub mod spawn;
mod step;
// bo2zm M3: Black Ops II Zombies' own scripts.
mod t6;
mod t6_gun;
mod t6_steer;
pub use t6_gun::{
    NOAMMO_PARM_LEFT, NOAMMO_PARM_SILENT, alt_mode_cycle_base, alt_toggle_allowed_by_state, is_melee_knife, alt_toggle_walk,
    out_of_ammo_pick, remember_mode, remembered_mode,
};
pub mod t6_lunge;
pub mod t6_playeranim;
pub mod t6_ragdoll;
pub mod t6_viewmodel;
pub use t6::stats::{ranked as local_match_ranked, set_local_match_ranked, set_local_stats_path}; // bo2mp lane C
pub use t6::melee_weapon;
pub use t6::menu::{MenuMapChange, advance_menu_timescale, reset_menu_timescale, take_menu_kicks, take_menu_map_change}; // bo2mp mod menu
pub use t6::{T6Anim, T6HeliDrive, T6Install, T6PathNode, T6PathTables, T6Table, T6Vehicle, T6VehicleDrive};
pub mod t5_destructible;
mod world;
pub mod world_objects;

pub use adopt::{ADOPT_GAP_COUNT, ADOPT_GAPS, AdoptGap, AdoptReport};
pub use bullet_collision::{
    AuthorityDObjCollision, AuthorityDObjCollisionBone, AuthorityDObjState, AuthorityModelOwner,
    BulletHitKind, BulletPath, BulletTraceQuery, BulletTraceSegment, COLLISION_COVERAGE,
    COLLISION_HISTORY_TICKS, CONTENTS_SOLID, ColliderId, CollisionCoverageRow, CollisionHistory,
    CoverageSupport, CurrentAuthorityReason, EntityCollisionCapabilities, EntityCollisionEpoch,
    EntityCollisionFrame, EntityCollisionHistory, EntityCollisionSample, EntityCollisionTraceGeom,
    HistoryClampReason, HistoryFrame, HistoryPhase, HistoryRefusalReason, HistorySample,
    HistorySampleVerdict, HitVolumeKind, LAGCOMP_MAX_REWIND_TICKS, LagcompQuery,
    LinkedBrushCollisionBrush, MASK_BULLET_WORLD, MASK_PLAYER_SOLID, MASK_SHOT, PLAYER_MAXS,
    PLAYER_MINS, PlayerCollisionPose, ScriptModelPlayAnim, ShotSampleProvenance, ShotSampleQuality,
    TraceInvalidReason, TraceOutcome, bullet_trace, bullet_trace_segments,
    bullet_trace_segments_with_entity_models, bullet_trace_with_entity_models, contents_match_mask,
    glass_piece_from_hit, lagcomp_rewind_ticks,
};
pub use carrier::{SimWorld, StepReason, step, try_step};
pub use collision_census::{
    CollisionCensus, EntityClipCensus, ModelCollisionCensus, PlayerClipCensus, WorldClipCensus,
};
pub use combat::{
    AcceptedShot, Emission, EntityClipKind, MELEE_PARM_SHIELD, PlayerCollisionRepresentation,
    ShotCollisionGeometry, ShotCollisionVerdict, TracePhaseOutput, spread_direction_on_plane,
    spread_pellet_direction,
};
pub use content::{CONTENT_DIGEST_SCHEME, ContentComponents, content_components, content_digest};
pub use corpse::{PlayerCorpsePool, PlayerCorpseSlot, level_time_ms};
pub use damage::{DamageAttempt, DamageOutcome, DamageRefusal, DeathCommit};
pub use equipment::{
    EquipmentRuntimeFacts, ProjectileHitGeometry, ProjectileImpact, ProjectileState,
    StuckRider, projectile_birth_ms,
};
pub use gentity::{
    EntityAllocError, EntityKernel, EntityKernelOccupiedSnapshot, EntityKernelSlotSnapshot,
    EntityKernelSnapshot, EntityKernelSnapshotError, EntityRef, EntityRefError, EntityRelations,
    EntityRunKind, EntitySlotView, GENTITY_RESERVED_COUNT, GENTITY_REUSE_QUARANTINE_MS,
    GENTITY_TEMP_EVENT_LIFETIME_MS, KERNEL_PHASE_ORDER, KernelPhase, ScriptMoverGentity,
    ThinkEnter, UsePress, apos_from_entity_state, begin_script_mover_rotate_velocity,
    collect_use_presses, gentity_spawn_base, init_item_state, init_missile_state,
    init_script_mover_state, pos_from_entity_state, rotate_velocity_apos,
};
pub use hudelem::{
    GameHudElemSlot, HUDELEM_UPDATE_ARCHIVAL, HUDELEM_UPDATE_BOTH, HUDELEM_UPDATE_CURRENT,
    hud_elem_update_client, rebase_hud_archival,
};
pub use identities::{
    ActionSequence, DamageSource, EventSequence, LifeSequence, MatchPhase, MatchRng, PelletId,
    ProjectileId, RNG_DOMAIN_SCHEME, RngDomain, ScriptModelId, ShotId,
};
pub use input::{
    ActionRequestId, ClassId, ClientAction, MENU_RESPONSE_BYTES, SpawnPick, TickInput,
    action_request_id, menu_response_field, menu_response_text,
};
pub use mantle_xanim::{MANTLE_XANIM_NAMES_T6, MantleXAnimBind, PERK_MANTLE_REDUCTION_BO2MP, PERK_MANTLE_REDUCTION_DEFAULT};
pub use match_state::{
    ClassDef, ClassRejectReason, ClientLifecycle, ClientSnapshotMeta, T6ViewInputs,
    ConfigurationChangeRejectReason, DebrisRecord, DroppedItemAmmo, EntityEventPayload, EntityEventRecord,
    EventAudience, EventRecord, GiveRejectReason, HealthRegenCensus, InputReceipt,
    ItemPickupRecord, KillcamHud, LoadoutSpec, LocationSelection, MENU_COMMAND_TAIL,
    MatchEndReason, MenuCommand, MenuCommandKind, PelletFxRecord, PersonalClass, RadarMode,
    RemoteMissile, RngDebugMeta, SIM_EVENT_ROSTER, ScriptBlur, ScriptControls, ScriptDepthOfField,
    EnemyModel, OrientedBlocker, ScriptDvars, ScriptSeat, SimEvent, SimEventRow, SnapshotMeta, UNRELIABLE_SIM_EVENT_COUNT,
    ViewEffects, VisionChange, is_postfx_dvar, sim_event_is_reliable,
};
pub use rules::{FFA, FreeForAllRules};
pub use score::{MATCH_TICK_MS, bootstrap_score_defaults};
pub use script_gaps::ScriptGaps;
pub use snapshot::{
    AreaEntityLinkSnapshot, AreaEntityWorldSnapshot, AreaEntityWorldSnapshotError,
    AreaSectorSnapshot, Snapshot,
};
pub use sound_alias_cs::{
    CS_LOCALIZED_STRINGS_SLOTS, CS_SOUNDALIASES_SLOTS, EffectNameCs, EffectNameCsOccupied,
    HUD_PRINT_ARG_SEPARATOR, HUD_STRING_PLAIN, HudMaterialCs, HudMaterialCsOccupied, HudStringCs,
    HudStringCsOccupied, REQUIRED_HUD_MATERIALS, SoundAliasCs, SoundAliasCsOccupied,
    hud_string_in_occupied, name_in_occupied,
};
pub use spawn::{
    AuthoredSpawnPoint, HostGameModeSelection, MatchBootstrap, SPAWN_BAD_DIST, SPAWN_IDEAL_DIST,
    SpawnAttemptReport, SpawnDecision, SpawnReject, host_game_mode_kind, pick_ffa_spawn,
    spawn_candidate_indices, spawn_candidate_indices_for, team_from_client_state,
};
pub use step::phase_materialize_entity_dobjs;
pub use step::set_omni_movement;
#[doc(hidden)]
pub use step::bo2_map_collision;
pub use world::{
    ClientId, HitvolDumpRow, PendingLocalSound, PendingPlayerCardEvent, PendingPlayerCardKind,
    PendingPrint, PlayerKitCollision, SimBrush, SimClipBsp, SimClipCmodels, SimClipMesh,
    SimTriggerHull, Tick, WeaponScriptSounds, blank_player_state,
};
pub use world_objects::{
    DestructibleLoopSound, GLASS_DAMAGE_TO_DESTROY, GLASS_DAMAGE_TO_WEAKEN, GlassPieceId,
    GlassPieceState, T6GlassPane, WorldObjectSnapshot, WorldObjectState,
};

mod scene_effects;
pub use scene_effects::{
    MAX_SCRIPT_EARTHQUAKES, ScriptEarthquake, ScriptFog, ScriptFogParams, ScriptSunFog,
};
mod objectives;
pub use objectives::{
    CompassObjective, CompassVehicle, ObjectiveMatch, ObjectiveState, ScriptEffect,
};

pub use world::{SimContent, SimContentBuilder, WeaponSetup};
