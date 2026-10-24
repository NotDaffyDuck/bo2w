//! BO2 (T6) weapon brain, rebuilt in our own Rust from facts about BO2's behaviour.
//!
//! - [`state`], [`anim`], [`events`]: BO2's weapon states, viewmodel anims and events.
//! - [`player`]: BO2's per-player weapon data (both hands), [`env`] and [`cmd`]: what a tick reads.
//! - [`facts`]: per-weapon facts from the fastfiles.
//! - [`tick`]: PM_Weapon in BO2's order; [`timing`]: the timer step; [`base`]: shared helpers.
//! - [`pm`]: one file of hooks per mechanic window; [`switch`]: weapon switching (window 2).
//! - [`fire`], [`reload`]: windows 3 and 4's standalone timelines, wired in through `pm`.
//! - [`trace`]: the frame trace and scripted input runner.
//! - [`legacy`]: maps T6 states onto the old weapon_iw4 ones while both brains exist.

pub mod anim;
pub mod base;
pub mod cmd;
pub mod env;
pub mod events;
pub mod facts;
pub mod fire;
pub mod legacy;
pub mod player;
pub mod pm;
pub mod recoil;
pub mod reload;
pub mod script;
pub mod state;
pub mod switch;
pub mod third;
pub mod tick;
pub mod timing;
pub mod trace;
pub mod viewmodel;
