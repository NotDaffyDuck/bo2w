//! bo2mp: Black Ops II's player movement, one frame at a time (PmoveSingle
//! and the moves it runs), written from BO2's own order and numbers. It
//! takes the player state, command and context and gives back the result.
//! A Black Ops II game always runs this one.
//!
//! The pieces BO2 shares with the legacy code (acceleration, the slide and step,
//! jumping, mantle, aiming) come from the legacy movement. The frame itself, the
//! view, sprint, stance, walking, the air, friction, the slick and sliding
//! ground, ladders, footsteps, the dead move and the movement direction are
//! BO2's own and live here. This crate is always Black Ops II: `feel.on` is
//! forced on.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod bridge;
mod air;
mod prone_room;
mod crash;
mod dead;
pub mod dive;
mod feel;
mod foliage;
mod gait_cycle;
mod frame;
mod ground_drag;
pub mod gjk;
mod ground;
mod gun;
pub mod heli;
mod ladder;
mod mantle;
mod movement_dir;
mod mover;
mod project;
mod prone_turn;
mod slide_adjust;
mod slope;
mod snap;
mod sprint;
mod stance;
mod view;
mod walk;
pub mod water;

pub use foliage::{CONTENTS_FOLIAGE, EV_FOLIAGE_SOUND};
pub use frame::BodyAnims;
pub use frame::holdrand;
pub use frame::pmove;
pub use frame::pmove_with_body;
pub use ground::PMF_SLIDING;
pub use gjk::Bo2Collision;
pub use ladder::PERK_FASTLADDERCLIMB;
pub use mantle::MANTLE_FORCE_CROUCH;
pub use crate::bridge::{MotionTickOutcome, OneTickMotionArgs};

#[cfg(test)]
pub(crate) mod testing {
    use crate::bridge::TickLocals;

    /// A 50 ms frame, on flat ground or in the air.
    pub(crate) fn tick_vars(walking: bool) -> TickLocals {
        let mut floor_probe = [0; 11];
        floor_probe[3] = 1.0_f32.to_bits();
        TickLocals {
            fwd_axis: [1.0, 0.0, 0.0],
            right_axis: [0.0_f32, -1.0, 0.0],
            up_axis: [0.0, 0.0, 1.0],
            step_millis_elapsed: 0.05,
            frame_ms: 50,
            walking: u32::from(walking),
            standing_on_floor: u32::from(walking),
            near_floor_contact: u32::from(walking),
            floor_probe,
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        }
    }

    /// A world with nothing in it: every sweep runs its whole length.
    pub(crate) struct Open;

    impl crate::Bo2Collision for Open {}

    impl crate::bridge::SolidProbe for Open {
        fn trace(&self, _: crate::bridge::SweepAsk) -> trace_t6::Trace {
            trace_t6::Trace::new()
        }
    }

    /// bo2mp's multiplayer walking rules, nothing held last frame, no prone
    /// blend.
    pub(crate) fn walk_context() -> crate::bridge::FootedStepArgs {
        use crate::bridge::{AirborneStepArgs, Bo2Feel, StrideScaleArgs, LeapKickoffArgs};
        let air = AirborneStepArgs {
            observer_pace_ratio: 1.0,
            concussed_fall_ratio: 1.0,
            concussed_fall_offset: 0.0,
        };
        let cmd_scale = StrideScaleArgs {
            retreat_pace_ratio: 0.7,
            sidestep_pace_ratio: 0.8,
            dash_pace_ratio: 1.5,
            downed_crawl_pace_ratio: 0.15,
            gun_stride_ratio: 1.0,
            scoped_carry_pace: 1.0,
            shellshock_movement_scale: 1.0,
            prone_lerp_ms: 0,
        };
        let jump = LeapKickoffArgs {
            leap_height: 39.0_f32,
            dive: !true,
            squat_leap_ratio: 1.0,
            rung_kickoff_speed: 128.0,
        };
        crate::bridge::FootedStepArgs {
            cmd_scale,
            gun_carry_pace: 1.0,
            last_tick_buttons: 0,
            jump,
            air,
            feel: Bo2Feel::BO2_MP,
            lunge: None,
        }
    }
}

mod ads_rates;
mod bob;
pub use ads_rates::{quickdraw_ads_rates, shellshock_walk_scale};
pub use bob::sway_phase_looped;
