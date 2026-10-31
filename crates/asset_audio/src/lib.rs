//! The sound side of a loaded match.
//!
//! - alias tables and their variants: `sound_alias`, `sound_pcm`, `variant_pick`
//! - the catalog that finds an alias by name: `sound_bank`, `sound_bank_load`
//! - Black Ops II bank files and clip decoding: `sound_load_t6`
//! - what a map's scripts place and say: `map_fx`, `map_sound_facts`

mod sound_alias;
mod sound_pcm;
mod variant_pick;

mod sound_bank;
mod sound_bank_load;

mod sound_load_t6;

mod map_fx;
mod map_sound_facts;

pub use sound_alias::*;
pub use sound_pcm::*;
pub use variant_pick::*;

pub use sound_bank::*;
pub use sound_bank_load::*;

pub use sound_load_t6::*;

pub use map_fx::*;
pub use map_sound_facts::*;
