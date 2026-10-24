//! The simulation: one match's state, advanced a tick at a time from client input.
//!
//! The modules below are grouped by what they hold; everything other crates use is re-exported from the crate root
//! further down.

// Modules other crates may reach into directly.
pub mod adopt;
pub mod bullet;
pub mod bullet_collision;
pub mod collision_census;
pub mod combat;
pub mod content;
pub mod guest;
pub mod hudelem;
pub mod identities;
pub mod input;
pub mod match_state;
pub mod rules;
pub mod script;
pub mod spawn;
pub mod t5_destructible;
pub mod t6_lunge;
pub mod t6_playeranim;
pub mod t6_ragdoll;
pub mod t6_viewmodel;
pub mod world_objects;

// Modules kept inside the crate.
mod area_world;
mod carrier;
mod clip_fallback;
mod corpse;
mod damage;
mod entity_local;
mod entity_run;
mod equipment;
mod frame;
mod gentity;
mod item;
mod item_t6;
mod mantle_xanim;
mod missile;
mod missile_decoy;
mod missile_guidance;
mod objectives;
mod presence;
mod remote_missile;
mod scene_effects;
mod score;
mod script_player;
mod snapshot;
mod sound_alias_cs;
mod step;
// bo2zm M3: Black Ops II Zombies' own scripts.
mod t6;
mod t6_gun;
mod t6_steer;
mod weapon_lock;
mod world;

// Bullets and collision.
pub use bullet_collision::{
    AuthorityDObjCollision, AuthorityDObjCollisionBone, AuthorityDObjState, AuthorityModelOwner, BulletPath,
    BulletTraceQuery, BulletTraceSegment, COLLISION_HISTORY_TICKS, CONTENTS_SOLID, ColliderId, CollisionHistory,
    CurrentAuthorityReason, EntityCollisionCapabilities, EntityCollisionEpoch, EntityCollisionFrame,
    EntityCollisionHistory, EntityCollisionSample, EntityCollisionTraceGeom, HistoryClampReason, HistoryFrame,
    HistoryPhase, HistoryRefusalReason, HistorySample, HistorySampleVerdict, HitVolumeKind,
    LAGCOMP_MAX_REWIND_TICKS, LagcompQuery, LinkedBrushCollisionBrush, MASK_BULLET_WORLD, MASK_PLAYER_SOLID,
    MASK_SHOT, PLAYER_MAXS, PLAYER_MINS, PlayerCollisionPose, ScriptModelPlayAnim, ShotSampleProvenance,
    ShotSampleQuality, TraceInvalidReason, TraceOutcome, bullet_trace, bullet_trace_segments,
    bullet_trace_segments_with_entity_models, bullet_trace_with_entity_models, contents_match_mask,
    glass_piece_from_hit,
};
pub use collision_census::{CollisionCensus, EntityClipCensus, ModelCollisionCensus, PlayerClipCensus, WorldClipCensus};
pub use combat::{AcceptedShot, Emission, MELEE_PARM_SHIELD, spread_direction_on_plane, spread_pellet_direction};
#[doc(hidden)]
pub use step::bo2_map_collision;

// The world, its clock and its content.
pub use adopt::AdoptReport;
pub use carrier::{SimWorld, StepReason, step, try_step};
pub use content::{CONTENT_DIGEST_SCHEME, ContentComponents, content_components, content_digest};
pub use score::{MATCH_TICK_MS, bootstrap_score_defaults};
pub use step::{phase_materialize_entity_dobjs, set_omni_movement};
pub use world::{
    ClientId, HitvolDumpRow, PendingLocalSound, PendingPlayerCardEvent, PendingPlayerCardKind, PendingPrint,
    PlayerKitCollision, SimBrush, SimClipBsp, SimClipCmodels, SimClipMesh, SimTriggerHull, Tick, WeaponScriptSounds,
    blank_player_state,
};
pub use world::{SimContent, SimContentBuilder, WeaponSetup};

// Entities, bodies and damage.
pub use corpse::{PlayerCorpsePool, PlayerCorpseSlot, level_time_ms};
pub use damage::{DamageAttempt, DamageOutcome, DamageRefusal, DeathCommit};
pub use equipment::{EquipmentRuntimeFacts, ProjectileState, StuckRider, projectile_birth_ms};
pub use gentity::{
    EntityAllocError, EntityKernel, EntityKernelOccupiedSnapshot, EntityKernelSlotSnapshot, EntityKernelSnapshot,
    EntityKernelSnapshotError, EntityRef, EntityRefError, EntityRelations, EntityRunKind, EntitySlotView,
    GENTITY_RESERVED_COUNT, GENTITY_REUSE_QUARANTINE_MS, GENTITY_TEMP_EVENT_LIFETIME_MS, KERNEL_PHASE_ORDER,
    KernelPhase, ScriptMoverGentity, ThinkEnter, UsePress, apos_from_entity_state,
    begin_script_mover_rotate_velocity, collect_use_presses, gentity_spawn_base, init_item_state, init_missile_state,
    init_script_mover_state, pos_from_entity_state, rotate_velocity_apos,
};
pub use mantle_xanim::{
    MANTLE_XANIM_NAMES_T6, MantleXAnimBind, PERK_MANTLE_REDUCTION_BO2MP, PERK_MANTLE_REDUCTION_DEFAULT,
};
pub use missile_guidance::{MissileGuide, MissileTarget};
pub use weapon_lock::WeaponLock;
pub use world_objects::{
    DestructibleLoopSound, GLASS_DAMAGE_TO_DESTROY, GLASS_DAMAGE_TO_WEAKEN, GlassPieceId, GlassPieceState,
    T6GlassPane, WorldObjectSnapshot, WorldObjectState,
};

// Players, input and the match.
pub use hudelem::{
    GameHudElemSlot, HUDELEM_UPDATE_ARCHIVAL, HUDELEM_UPDATE_BOTH, HUDELEM_UPDATE_CURRENT, hud_elem_update_client,
    rebase_hud_archival,
};
pub use identities::{
    ActionSequence, DamageSource, EventSequence, LifeSequence, MatchPhase, MatchRng, PelletId, ProjectileId,
    RNG_DOMAIN_SCHEME, RngDomain, ScriptModelId, ShotId,
};
pub use input::{
    ActionRequestId, ClassId, ClientAction, MENU_RESPONSE_BYTES, SpawnPick, TickInput, action_request_id,
    menu_response_field, menu_response_text,
};
pub use match_state::{
    ClassDef, ClassRejectReason, ClientLifecycle, ClientSnapshotMeta, ConfigurationChangeRejectReason, DebrisRecord,
    DroppedItemAmmo, EnemyModel, EntityEventPayload, EntityEventRecord, EventAudience, EventRecord, GiveRejectReason,
    InputReceipt, ItemPickupRecord, KillcamHud, LoadoutSpec, LocationSelection, MENU_COMMAND_TAIL, MatchEndReason,
    MenuCommand, MenuCommandKind, OrientedBlocker, PelletFxRecord, PersonalClass, RadarMode, RemoteMissile,
    RngDebugMeta, SIM_EVENT_ROSTER, ScriptBlur, ScriptControls, ScriptDepthOfField, ScriptDvars, ScriptSeat,
    SimEvent, SimEventRow, SnapshotMeta, T6ViewInputs, ViewEffects, VisionChange, is_postfx_dvar,
    sim_event_is_reliable,
};
pub use rules::{FFA, FreeForAllRules};
pub use spawn::{
    AuthoredSpawnPoint, HostGameModeSelection, MatchBootstrap, SpawnAttemptReport, SpawnDecision, SpawnReject,
    host_game_mode_kind, spawn_candidate_indices_for, team_from_client_state,
};

// What clients are sent.
pub use objectives::{CompassObjective, CompassVehicle, ObjectiveMatch, ObjectiveState, ScriptEffect};
pub use scene_effects::{MAX_SCRIPT_EARTHQUAKES, ScriptEarthquake, ScriptFog, ScriptFogParams, ScriptSunFog};
pub use snapshot::{AreaEntityLinkSnapshot, AreaEntityWorldSnapshot, AreaEntityWorldSnapshotError, AreaSectorSnapshot, Snapshot};
pub use sound_alias_cs::{
    CS_LOCALIZED_STRINGS_SLOTS, CS_SOUNDALIASES_SLOTS, EffectNameCs, EffectNameCsOccupied, HUD_PRINT_ARG_SEPARATOR,
    HUD_STRING_PLAIN, HudMaterialCs, HudMaterialCsOccupied, HudStringCs, HudStringCsOccupied, REQUIRED_HUD_MATERIALS,
    SoundAliasCs, SoundAliasCsOccupied, name_in_occupied,
};

// Black Ops II Zombies and menu glue.
pub use t6::melee_weapon;
pub use t6::menu::{
    MenuMapChange, advance_menu_timescale, reset_menu_timescale, take_menu_kicks, take_menu_map_change,
}; // bo2mp mod menu
pub use t6::stats::{ranked as local_match_ranked, set_local_match_ranked, set_local_stats_path}; // bo2mp lane C
pub use t6::{T6Anim, T6HeliDrive, T6Install, T6PathNode, T6PathTables, T6Table, T6Vehicle, T6VehicleDrive};
pub use t6_gun::{
    NOAMMO_PARM_LEFT, NOAMMO_PARM_SILENT, alt_mode_cycle_base, alt_toggle_allowed_by_state, alt_toggle_walk,
    is_melee_knife, out_of_ammo_pick, remember_mode, remembered_mode,
};
