//! The movement code the whole game shares: one step of a player's move and
//! the pieces it is built from (walk, air, ladder, climb, jump, stance and
//! the rest). Black Ops II's own modules sit above this and call into it.

mod accelerate;
mod ads_frac;
mod ads_intent;
mod air;
mod check_prone;
mod cmdscale;
mod collision;
mod correct_solid;
mod crash;
mod dive;
mod dmgtimer;
mod drop_timers;
mod events;
mod feel;
mod footstep;
mod gjk;
mod is_in_air;
pub mod jump;
mod ladder;
pub mod mantle;
mod melee_charge;
mod melee_charge_bo2;
mod pml;
mod single;
mod slide;
mod sprint;
mod stance;
pub mod ufo;
mod viewangles;
mod walk;

pub use accelerate::accelerate;
pub use ads_frac::{AdsFracContext, update_ads_frac};
pub use ads_intent::{AdsIntentContext, BUTTON_ADS, update_ads_intent};
pub use air::AirMoveContext;
pub use check_prone::{PRONE_CHECK_HEIGHT, PRONE_FEET_DIST, ProneRules, check_prone};
pub use cmdscale::{CmdScaleWalkContext, cmd_scale_walk};
pub use collision::CollisionBackend;
pub use correct_solid::BG_CORRECT_SOLID_DELTAS;
pub use crash::crash_land_fall_height;
pub use dive::{PMF_DIVE, dive_to_prone, in_air as dive_in_air, sliding as dive_sliding};
pub use dmgtimer::{damage_scale_walk, update_damage_timer, walk_move_drop_damage_timer};
pub use drop_timers::drop_timers;
pub use events::{add_event, add_predictable_event, consume_player_events};
pub use feel::{
    Bo2Feel, PERK_FALLHEIGHT, PERK_FASTADS, PERK_FLAKJACKET, PERK_LONGERSPRINT, PERK_MOVEFASTER,
    PERK_STALKER, PRONE_LERP_MS, player_speed, quickdraw_ads_rates, shellshock_walk_scale,
};
pub use footstep::{
    LADDER_SURFACE_TYPE, SURFACE_TYPE_NAMES, bob_cycle_wrapped, footsteps_anim_move_type,
    footsteps_bob_cycle, surface_type_index,
};
pub use is_in_air::is_in_air;
pub use jump::{JumpCheckContext, JumpCheckResult, JumpLaunchContext};
pub use ladder::{
    CheckLadderContext, LADDER_JUMP_BLOCK_MS, LADDER_TRACE_DIST_AIR, LADDER_TRACE_DIST_WALK,
    LadderAttachBackend, LadderTraceHit, SURF_LADDER, ladder_attract_velocity,
};
pub use mantle::{
    CONTENTS_MANTLE, CreateAnimsMantleRootDelta, FlatMantleAnimLength, MANTLE_XANIM_TREE_SIZE,
    MantleCapViewContext, MantleCapsuleTrace, MantleMoveContext, MantleRootDelta,
    MantleXAnimLength, PERK_FASTMANTLE, ZeroMantleRootDelta,
};
pub use melee_charge::{
    MeleeChargeWeaponDelays, PLAYER_MELEE_RANGE_DEFAULT as MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
    calc_melee_charge_time, melee_charge_move,
};
pub use melee_charge_bo2::{
    BO2_CHARGE_FRICTION, Bo2Lunge, LungeTarget, bo2_lunge_clear, bo2_lunge_friction,
    bo2_lunge_update,
};
pub use pml::Pml;
pub use single::{GroundTraceInput, MoveBounds, PmoveResult, PmoveSingleContext};
pub use slide::step_slide_move;
pub use sprint::{
    PERK_MARATHON, SprintContext, SprintResult, end_sprint, get_max_sprint_time,
    sprint_forward_below_minimum, sprint_time_remaining,
};
pub use stance::{
    CROUCH_MAXS_Z, PRONE_MAXS_Z, STAND_MAXS_Z, StanceChange, StanceSurface, stance_surface_type,
    view_height,
};
pub use viewangles::{ANGLE2SHORT, SHORT2ANGLE, ViewAngleClamp};
pub use walk::WalkMoveContext;

// Used inside this crate only.
pub(crate) use footstep::get_bob_max_speed;
pub(crate) use footstep::{footstep_event, should_make_footsteps};
pub(crate) use slide::project_velocity;
pub(crate) use stance::stance_speed_scale;
