//! The simulation: one match's state, advanced a tick at a time from client input.
//!
//! Modules come first, split by who may reach into them; the `pub use` lists after them are the crate's surface,
//! grouped by subject: collision, the world, entities, the match and its players, and what clients are sent.
//
// Reachable by path from other crates.
pub mod host_overlay; // laying the host state over a predicting client
pub mod shot_falloff; // falloff and view-angle maths
pub mod shot_trace; // traces, whole bullet flights, glass, shots through walls
pub mod clip_tally; // what the collision world holds
pub mod gunfire; // accepted shots into bullets; melee
pub mod content_print; // fingerprints of the content a match runs on
pub mod guest_arms; // guest weapons in the Wonder Weapons tab
pub mod script_overlay; // script HUD elements and who gets which
pub mod match_ids; // ids and the match seeds
pub mod tick_feed; // what the match is fed each tick
pub mod player_ledger; // per-client match state and what a snapshot publishes
pub mod ruleset; // the free-for-all rules
pub mod gsc_host; // the game-script runtime
pub mod spawn_pick; // spawn points and match setup
pub mod staged_props; // props that break in stages
pub mod knife_lunge; // the knife lunge
pub mod body_anim_script; // the player animation script
pub mod ragdoll_defs; // ragdoll definitions
pub mod gun_view_facts; // per-weapon viewmodel facts
pub mod glass_and_props; // glass and destructible loop sounds
//
// Private to the crate.
mod sector_links; // area entities in sectors
mod sim_whole; // WholeSim, the whole simulation as one value
mod brush_clip; // a clip map built from the brush list
mod bodies; // bodies of dead players
mod harm; // damage to players and things around them
mod ent_links; // link bounds, parents, grenade helpers
mod ent_think; // each live entity thinks once a tick
mod gear; // thrown, launched and planted gear
mod step_view; // StepWorldView, the view one system works through
mod ent_slots; // entity numbers and the kernel
mod floor_guns; // guns lying on the ground
mod floor_gun_facts; // dropped guns with BO2 weapon facts
mod climb_anim; // climb animations for mantling
mod rocketry; // rockets, launched grenades, bolts
mod rocket_wobble; // rocket wobble and decoys
mod homing; // homing state and steering
mod hud_markers; // compass markers and other match state on the HUD
mod mover_clip; // collision rows of script movers
mod rider_missile; // a missile its rider steers
mod fog_quake; // fog and earthquakes
mod round_clock; // the match clock and opening limits
mod client_script; // what a script can do to a client
mod tick_publish; // what the sim publishes each tick
mod sound_names; // numbered names for snapshots
mod frame_step; // one simulation frame
mod zombie_rt; // Zombies scripts and their engine side
mod gun_brain; // the BO2 weapon brain
mod lock_steer; // steering for lock-on rockets
mod lock_on; // lock-on targeting state
mod sim_parts; // the world state and its parts
//
// Collision and bullets.
pub use self::shot_trace::{
    trace_shot, trace_shot_legs, shot_legs_against_models,
    trace_shot_against_models, contents_match_mask, pane_from_hit, ServerModelCollision,
    ServerModelState, ServerModelSource, ShotRouteKind, ShotRayRequest,
    ShotPathLeg, HITBOX_TIMELINE_LENGTH, HARD_SURFACE_FLAG, HitTargetId, HitboxTimeline,
    LiveStateCause, EntityHitAbilities, EntityCollisionMoment, EntityHitboxSnapshot,
    EntityHitboxTimeline, EntityHitboxReading, EntityRayShape, RewindLimitCause,
    RewindFrame, RewindStage, RewindDenialCause, RewindSample, RewindSampleRuling,
    HitboxShapeKind, MAX_COMPENSATION_STEPS, RewindRequest, AttachedSolidShape, SHOT_WORLD_FILTER,
    PLAYER_BODY_MASK, MASK_SHOT, BODY_BOX_UPPER, BODY_BOX_LOWER, PlayerHitboxPose, ScriptedModelAnimPlay,
    ShotRecordOrigin, ShotRecordGrade, RayRejectCause, RayResult,
}; // bullet_collision
pub use self::clip_tally::{
    CollisionCensus, EntityClipCensus, ModelCollisionCensus, PlayerClipCensus, WorldClipCensus,
}; // collision_census
pub use self::gunfire::{
    spread_direction_on_plane, pellet_heading, ValidatedShot, PelletShot, MELEE_PARM_SHIELD,
}; // combat
#[doc(hidden)]
pub use self::frame_step::bo2_map_collision;
//
// The world, its clock and its content.
pub use self::host_overlay::TakeoverReport;
pub use self::sim_whole::{step, try_step, WholeSim, SimWorld, AdvanceCause};
pub use self::content_print::{content_part_fingerprints, content_fingerprint, CONTENT_PRINT_REVISION, ContentPartDigests};
pub use self::round_clock::{opening_limits, SIM_STEP_MILLIS};
pub use self::sim_parts::{
    empty_avatar, SeatNo, HitboxReportRow, PendingLocalSound, QueuedCallingCardEvent,
    QueuedCallingCardKind, PendingPrint, PlayerOutfitHitbox, SolidBrush, CollisionTreeData, CollisionSubmodels,
    CollisionTriangleData, SimTriggerHull, StepNo, WeaponRawKinds, WeaponScriptSounds,
}; // world
pub use self::sim_parts::{SimContent, SimContentBuilder, WeaponPart, WeaponSetup};
pub use self::frame_step::{stage_build_entity_models, set_omni_movement};
//
// Entities, bodies and damage.
pub use self::ent_slots::{
    turn_path_of, start_scripted_spin, gather_presses, first_spawnable_slot,
    loot_state_at, shell_state_at, build_scripted_mover, move_path_of,
    spin_rate_pose, EntitySlotShortage, EntitySlotArena, BayHeldRow, EntityKernelOccupiedSnapshot,
    BayFrozenRow, EntityKernelSlotSnapshot, FrozenSlotTable, EntityKernelSnapshot, EntityTableSnapshotFault, EntHandle,
    EntityHandleFault, ThingTies, EntityRelations, EntityBehaviorClass, EntitySlotReadout, PROTECTED_ENTITY_SLOTS,
    SLOT_COOLDOWN_BEFORE_REUSE_MS, EVENT_ENTITY_LIFESPAN_MS, PASS_STAGE_SEQUENCE, EntityPassStage,
    MoverSlot, ThinkEntryResult, InteractTap,
}; // gentity
pub use self::bodies::{level_time_ms, CorpseYard, PlayerCorpsePool, CorpseBay, PlayerCorpseSlot};
pub use self::harm::{HarmRequest, HarmResult, HarmDenial, FatalityRecord};
pub use self::gear::{shell_born_ms, GadgetLiveStats, FlyingProjectile, StuckRider};
pub use self::climb_anim::{
    MANTLE_XANIM_NAMES_T6, LedgeClimbAnimSet, PERK_MANTLE_REDUCTION_BO2MP, PERK_MANTLE_REDUCTION_DEFAULT,
}; // mantle_xanim
pub use self::homing::{MissileGuide, MissileTarget};
pub use self::lock_on::WeaponLock;
pub use self::glass_and_props::{
    DestructibleLoopSound, PANE_SHATTER_HARM, PANE_CRACK_HARM,
    PaneShardCondition, PropsSnapshot, MapPropState,
}; // world_objects
//
// The match and its players.
pub use self::script_overlay::{
    hud_sync_bank_for_player, shift_replayed_clock, ScriptHudSlot, HUD_SYNC_SAVED_BANK,
    HUD_SYNC_ALL_BANKS, HUD_SYNC_LIVE_BANK,
}; // hudelem
pub use self::match_ids::{
    ActionStamp, HarmOrigin, EventStamp, LifeCount, GameRoundStage, MatchRandom, BuckshotId,
    ThrownId, RANDOM_STREAM_LAYOUT_VERSION, RandomStreamKind, ScriptedModel, BulletNo,
}; // ids and stamps
pub use self::tick_feed::{
    intent_serial, menu_response_field, menu_response_text, LoadoutClass, ClassId, PlayerIntent,
    MENU_RESPONSE_BYTES, SpawnPick, StepInputs,
}; // input
pub use self::player_ledger::{
    is_postfx_dvar, occurrence_needs_delivery, LoadoutClassSpec, LoadoutRefusal, ClassRejectReason, PlayerSessionStage,
    SeatTickNotes, ClientSnapshotMeta, ConfigurationChangeRejectReason, DebrisRecord, DroppedRounds, DroppedItemAmmo, EnemyModel,
    HappeningBody, EntityEventPayload, ThingHappening, EntityEventRecord, EventRecipients, OccurrenceEntry, EventRecord, HandoutRefusal, GiveRejectReason, InputReceipt,
    GrabbedItemNote, ItemPickupRecord, DeathReelBoard, KillcamHud, KitPlan, LoadoutSpec, LocationSelection, MENU_COMMAND_TAIL, MatchOverCause,
    MenuOrder, MenuCommandKind, OrientedBlocker, ScatterPuff, PelletFxRecord, PersonalClass, RadarMode,
    SteeredMissile, DiceTrace, RngDebugMeta, OCCURRENCE_TABLE, ScriptBlur, ScriptControls, ScriptDepthOfField,
    ScriptDvars, ScriptSeat, MatchOccurrence, OccurrenceTableRow, TickLedgerNotes, SnapshotMeta, T6ViewInputs, ViewEffects,
    VisionChange,
}; // match_state
pub use self::ruleset::{FFA, DeathmatchRuleset};
pub use self::spawn_pick::{
    hosted_ruleset, eligible_spawn_rows, team_from_client_state, MapSpawnSpot,
    HostChosenMode, MatchStartSetup, SpawnTryOutcome, SpawnVerdict, SpawnRefusalCause,
}; // spawn
//
// What clients are sent.
pub use self::hud_markers::{
    CompassMark, CompassRide, MatchBoard, ObjectiveState, PlacedEffect,
}; // compass marks and the scoreboard
pub use self::fog_quake::{
    MAX_SCRIPT_EARTHQUAKES, ScriptEarthquake, ScriptFog, ScriptFogParams, ScriptSunFog,
}; // scene_effects
pub use self::tick_publish::{
    ZoneLinkRow, AreaEntityLinkSnapshot, ZoneThingTable, AreaEntityWorldSnapshot, RegionSnapshotFault, ZoneCellRow, AreaSectorSnapshot,
    TickSnapshot, Snapshot,
}; // snapshot
pub use self::sound_names::{
    label_in_listed, CS_LOCALIZED_STRINGS_SLOTS, SOUND_ALIAS_TABLE_CAPACITY,
    HUD_PRINT_ARG_SEPARATOR, HUD_STRING_PLAIN,
    HudStringCs, MANDATORY_HUD_IMAGES, SoundAliasConfigTable,
}; // sound_alias_cs
//
// Black Ops II Zombies, menus and guns.
pub use self::zombie_rt::{
    melee_weapon, T6Anim, T6HeliDrive, T6Install, T6PathNode, T6PathTables, T6Table, T6Vehicle,
    T6VehicleDrive,
}; // t6
pub use self::zombie_rt::{
    add_special_items, add_special_weapons, cheat_on, cheat_pick, cheats_allowed, client_frame_heartbeat,
    platform_ride_box, platform_rides_on_ground, set_t6_game_settings, set_t6_party_size, set_t6_profile,
    special_pick, special_weapons, zm_gametype, Cheat, T6ZBarrierDef,
}; // bo2zm: Zombies' re-exports
pub use self::zombie_rt::menu::{
    advance_menu_timescale, reset_menu_timescale, take_menu_kicks, take_menu_map_change, MenuMapChange,
}; // t6::menu
pub use self::zombie_rt::stats::{ranked, set_local_match_ranked, set_local_stats_path};
/// Older spelling local_match_ranked still calls; the rename sweep moves it over.
/// Older spelling IntentSerial still calls; the rename sweep moves it over.
pub use core::primitive::u32 as IntentSerial;
/// Older spelling T6GlassPane still calls; the rename sweep moves it over.
pub use glass_t6::PaneDef as T6GlassPane;
pub use self::gun_brain::{
    alt_mode_cycle_base, alt_toggle_allowed_by_state, alt_toggle_walk, is_melee_knife, out_of_ammo_pick,
    remember_mode, remembered_mode, NOAMMO_PARM_LEFT, NOAMMO_PARM_SILENT,
}; // t6_gun

/// Older module names other crates still use; the rename sweep moves them over.

pub use self::match_ids::EventStamp as EventSequence;
pub use self::match_ids::LifeCount as LifeSequence;
pub use self::match_ids::ThrownId as ProjectileId;
pub use self::match_ids::ScriptedModel as ScriptModelId;
pub use self::PropsSnapshot as WorldObjectSnapshot;
pub use self::SeatNo as ClientId;
pub use self::StepNo as Tick;
pub use self::BulletNo as ShotId;
