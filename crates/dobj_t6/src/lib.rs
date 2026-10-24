//! Black Ops II's DObj (the posed model instance a player, gun or script
//! model draws as), from `facts/xanim-dobj-t6.md` sections 1, 3 and 5:
//!
//! - [`PartBits`]: the 5-word, most-significant-bit-first part sets a DObj
//!   keeps (at most 160 parts over all its models);
//! - [`SkelPartBits::begin_calc`]: how `DObjCalcAnim` turns the wanted parts
//!   into the skip and ignore masks, and [`touch_leaf`] for each anim leaf;
//! - [`AnimMat`]: the per-part sum the sample decoders blend into, and
//!   [`reset_part`] for parts no anim touched;
//! - [`LocalPose`]: the part poses between the calc and the skeleton, with
//!   the control bits `DObjSetControlTagAngles` sets;
//! - [`compute_radius`] / [`bounds`]: `DObjComputeBounds` and `DObjGetBounds`;
//! - [`ModelPoseSrc`], [`Bone`], [`Attach`] and [`HidePartBits`]: the models a
//!   DObj is built from, its bones and its hidden parts, and the surface
//!   tests that read them ([`surface_hidden_whole`]).

mod bounds;
mod calc;
mod dobj;
mod hide;
mod part_bits;
mod pose;

pub use bounds::*;
pub use calc::*;
pub use dobj::*;
pub use hide::*;
pub use part_bits::*;
pub use pose::*;
