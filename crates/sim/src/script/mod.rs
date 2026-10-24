//! The game-script runtime: the language (`lang`) and Black Ops II's side of
//! it (`bo2`). The match installs the start-up stubs through here and the step
//! loop calls the per-tick entry points below.

mod bo2;
mod lang;

/// Old paths the BO2 side of the sim still names.
pub(crate) mod host {
    pub(crate) use crate::script::bo2::match_start;

    pub(crate) mod triggers {
        pub(crate) use crate::script::bo2::volume::{Volume, t6_volume};
    }

    pub(crate) mod client_effects {
        pub(crate) use crate::script::bo2::dof::depth_of_field_values;
    }
}

pub use bo2::level::{KeyType, LevelData, StringTable, parse_entity_string, parse_radiant_keys};
pub use bo2::natives::NativeRegistry;
pub use bo2::startup::T6Startup;
pub use bo2::{EXIT_LEVEL, MAP_RESTART};
pub use lang::{Builtin, Catalog, Fault, Location, Namespace, Program, Realm, SourceResolver, Value, decode_source};

pub(crate) use bo2::controls::{
    Mechanics, action_slot_command, advance_mechanics, player_commands, restart_level,
    select_location,
};
pub(crate) use bo2::damage::{
    EntityHit, HitTarget, ScriptBlast, ScriptHit, damage_entity, radius_targets,
};
pub(crate) use bo2::players::{
    answer_join, answer_menu, apply_disconnects, choose_class, choose_default_class,
    disconnect_player, flashbang, force_death, give_killstreak, grenade_suicide, is_t5,
    note_team_answer, personal_class, player_damage, script_seats, store_seat_archive,
    sync_players,
};
pub(crate) use bo2::presence::sync_presence;
pub(crate) use bo2::runtime::{
    Runtime, advance_scheduler, copy_state, healthy, install, preflight, reset, set_dvar, start,
    take_signals,
};
pub(crate) use bo2::weapons::sync_engine_events;
