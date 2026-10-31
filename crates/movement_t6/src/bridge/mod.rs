//! The movement code the whole game shares: one step of a player's move and
//! the pieces it is built from (walk, air, ladder, climb, jump, stance and
//! the rest). Black Ops II's own modules sit above this and call into it.

mod speed_up;
mod aim_blend;
mod aim_wish;
mod air;
mod prone_fit;
mod stick_scale;
mod hull_hits;
mod solid_escape;
mod crash;
mod dive;
mod hurt_slow;
mod timer_decay;
mod event_feed;
mod feel;
mod stride_sfx;
mod off_ground;
pub mod leap;
mod rungs;
pub mod mantle;
mod melee_reach;
mod melee_charge_bo2;
mod pml;
mod one_sweep;
mod slide;
mod dash;
mod posture;
pub mod ufo;
mod look_angles;
mod walk;

pub use speed_up::accelerate;
pub use aim_blend::{ScopeBlendArgs, update_ads_frac};
pub use aim_wish::{ScopeWishArgs, INTENT_SCOPE_BIT, update_ads_intent};
pub use air::AirborneStepArgs;
pub use prone_fit::{BELLY_CLEARANCE_PROBE, BELLY_FOOT_REACH, ProneRules, check_prone};
pub use stick_scale::{StrideScaleArgs, cmd_scale_walk};
pub use hull_hits::SolidProbe;
pub use solid_escape::UNSTICK_NUDGES;
pub use crash::hard_landing_drop;
pub use dive::{PMF_DIVE, dive_in_air, dive_sliding, dive_to_prone};
pub use hurt_slow::{damage_scale_walk, update_damage_timer, walk_move_drop_damage_timer};
pub use timer_decay::drop_timers;
pub use event_feed::{add_event, queue_forecast_happening, drain_player_happenings};
pub use feel::{
    Bo2Feel, PERK_FALLHEIGHT, PERK_FASTADS, PERK_FLAKJACKET, PERK_LONGERSPRINT, PERK_MOVEFASTER,
    PERK_STALKER, PRONE_LERP_MS, player_speed, quickdraw_ads_rates, shellshock_walk_scale,
};
pub use stride_sfx::{
    RUNG_MATERIAL, sway_phase_looped, footsteps_anim_move_type,
    footsteps_bob_cycle, material_kind_number,
};
pub use off_ground::is_in_air;
pub use leap::{LeapGateArgs, LeapGateVerdict, LeapKickoffArgs};
pub use rungs::{
    RungGrabArgs, RUNG_LEAP_LOCKOUT_MS, RUNG_PROBE_REACH_AIRBORNE, RUNG_PROBE_REACH_FOOTED,
    RungGripSource, RungProbeContact, FLOOR_IS_RUNGS, ladder_attract_velocity,
};
pub use mantle::{
    SOLID_CLIMBABLE, ClipDrivenClimbOffset, FixedClimbClipDuration, CLIMB_CLIP_SLOTS,
    ClimbLookLimitArgs, ClimbPillSweep, ClimbStepArgs, ClimbBoneOffsetSource,
    ClimbClipDuration, PERK_FASTMANTLE, StillClimbOffset,
};
pub use melee_reach::{
    LungeGunTimings, STOCK_KNIFE_REACH as MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
    calc_melee_charge_time, melee_charge_move,
};
pub use melee_charge_bo2::{
    BO2_CHARGE_FRICTION, Bo2Lunge, LungeTarget, bo2_lunge_clear, bo2_lunge_friction,
    bo2_lunge_update,
};
pub use pml::TickLocals;
pub use one_sweep::{SweepAsk, HullExtents, MotionTickOutcome, OneTickMotionArgs};
pub use slide::step_slide_move;
pub use dash::{
    PERK_UNLIMITEDSPRINT, DashArgs, DashVerdict, end_sprint, get_max_sprint_time,
};
pub use posture::{
    SQUAT_HULL_TOP, BELLY_HULL_TOP, UPRIGHT_HULL_TOP, PostureShift, PostureFooting, posture_footing_kind,
};
pub use stride_sfx::SURFACE_KIND_LABELS;
pub use dash::sprint_forward_below_minimum;
pub use dash::sprint_time_remaining;
pub use posture::eye_level;

pub use look_angles::{DEGREES_TO_WORD16, WORD16_TO_DEGREES, LookRotationLimit};
pub use walk::FootedStepArgs;

// Used inside this crate only.
pub(crate) use stride_sfx::get_bob_max_speed;
pub(crate) use stride_sfx::{footstep_event, should_make_footsteps};
pub(crate) use slide::project_velocity;
pub(crate) use posture::posture_pace_ratio;
