//! The game-script runtime: the language (`lang`) and Black Ops II's side of
//! it (`bo2`). The match installs the start-up stubs through here and the step
//! loop calls the per-tick entry points below.

mod bo2;
mod lang;

pub use bo2::level::{KeyType, LevelData, StringTable, parse_entity_string, parse_radiant_keys};
pub use bo2::natives::NativeRegistry;
pub use bo2::startup::T6Startup;
pub use bo2::{EXIT_LEVEL, MAP_RESTART};
pub use lang::{Builtin, Catalog, Fault, Location, Namespace, Program, Realm, SourceResolver, Value, decode_source};

pub(crate) use bo2::controls::{Mechanics, advance_mechanics, select_location};
pub(crate) use bo2::damage::{
    EntityHit, HitTarget, ScriptBlast, ScriptHit, damage_entity, radius_targets,
};
// TODO(rw-zm): Zombies' t6/natives_fx.rs calls crate::gsc_host::push_earthquake; the engine now sends earthquakes as
// entity events (t6::killstreaks::shake) and bo2/ has no push_earthquake - route natives_fx's earthquake there.
pub(crate) use bo2::players::{
    apply_disconnects, choose_class, disconnect_player, force_death, grenade_suicide,
    personal_class, player_damage, sync_players,
};
pub(crate) use bo2::dof::depth_of_field_values;
pub(crate) use bo2::match_start::apply_level_notify;
pub(crate) use bo2::presence::sync_presence;
pub(crate) use bo2::runtime::{
    Runtime, advance_scheduler, copy_state, healthy, seat_program, preflight, reset, set_dvar, start,
    take_signals,
};
pub(crate) use bo2::volume::{Volume, t6_volume};
pub(crate) use bo2::weapons::sync_engine_events;
