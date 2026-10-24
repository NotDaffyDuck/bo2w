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
mod check_prone;
mod crash;
mod dead;
pub mod dive;
mod feel;
mod foliage;
mod footstep;
mod frame;
mod friction;
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
pub use frame::{BodyAnims, holdrand, pmove, pmove_with_body};
pub use ground::PMF_SLIDING;
pub use gjk::Bo2Collision;
pub use ladder::PERK_FASTLADDERCLIMB;
pub use mantle::MANTLE_FORCE_CROUCH;
pub use crate::bridge::{PmoveResult, PmoveSingleContext};

#[cfg(test)]
pub(crate) mod testing {
    use crate::bridge::Pml;

    /// A 50 ms frame, on flat ground or in the air.
    pub(crate) fn pml(walking: bool) -> Pml {
        let mut ground_trace = [0; 11];
        ground_trace[3] = 1.0_f32.to_bits();
        Pml {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            frametime: 0.05,
            msec: 50,
            walking: u32::from(walking),
            ground_plane: u32::from(walking),
            almost_ground_plane: u32::from(walking),
            ground_trace,
            previous_origin: [0.0; 3],
            previous_velocity: [0.0; 3],
            holdrand: 0,
        }
    }
}

mod ads_rates;
mod bob;
pub use ads_rates::{quickdraw_ads_rates, shellshock_walk_scale};
pub use bob::bob_cycle_wrapped;
