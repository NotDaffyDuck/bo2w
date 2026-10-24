//! Game rules read from the zones: weapons, impact tables, strings, teams and
//! tracers.

use asset_anim::XAnimCatalog;
use asset_audio::SoundCatalog;
use asset_core::*;
use asset_material::{MaterialCatalog, MaterialDefinitions};
use asset_model::{FpvMeshCatalog, ModelSkel, WorldWeaponCatalog, WorldWeaponEntry};

mod cac;
mod families;
mod font_def;
mod fpv_clip_tracks;
mod fpv_view_join;
mod fpv_view_types;
mod fx;
mod fx_model_set;
mod hidden_bones;
mod hit_location_damage;
mod impact_fx_table;
mod localize_catalog;
mod map_sun;
mod match_teams;
mod pen_table;
mod string_table;
mod tracer_set;
mod ui_root;
mod weapon;
mod weapon_anim;
mod weapon_anim_dispatch;

pub use asset_transport::read_iwd_named;
pub use cac::*;
pub use families::*;
pub use font_def::*;
pub use fpv_clip_tracks::*;
pub use fpv_view_types::*;
pub use fx::*;
pub use fx_model_set::*;
pub use hidden_bones::*;
pub use hit_location_damage::*;
pub use impact_fx_table::*;
pub use localize_catalog::*;
pub use map_sun::*;
pub use match_teams::*;
pub use pen_table::*;
pub use string_table::*;
pub use tracer_set::*;
pub use ui_root::*;
pub use weapon::*;
pub use weapon_anim::*;
pub use weapon_anim_dispatch::*;
