//! What a Black Ops II map contributes to the script runtime: its compiled
//! scripts, tables, entity text, animation facts and the config files the
//! game logic reads.

use std::sync::Arc;

/// Everything script-side that one map's zones hold. Where two zones supply
/// the same thing, the zone loaded later replaces the earlier one.
#[derive(Clone, Debug, Default)]
pub struct T6ScriptSet {
    /// Compiled script objects, in zone load order.
    pub objects: Vec<Vec<u8>>,
    /// String tables as (name, columns, rows, cells row by row).
    pub tables: Vec<(String, usize, usize, Vec<String>)>,
    pub entities: Vec<String>,
    /// The map's AI path nodes, in node order.
    pub path_nodes: Vec<asset_t6::PathNodeRef>,
    /// The sight and smoothing tables baked for those nodes.
    pub path_tables: asset_t6::PathTablesRef,
    /// Timing, root motion and notetracks of every animation.
    pub anims: Vec<T6AnimFacts>,
    /// Animation state definitions (`animstatedefs/*.asd`) as (name, text).
    pub animstatedefs: Vec<(String, String)>,
    /// The language zones' text, such as `ZOMBIE_WEAPON_M14` to its English.
    pub strings: Vec<(String, String)>,
    /// Every sound alias the map's banks hold, lower case.
    pub sound_aliases: Vec<String>,
    /// Game type settings files (`mp/gamesettings_tdm.cfg` and the like) as
    /// (name, text).
    pub gamesettings: Vec<(String, String)>,
    /// Length in milliseconds of each sound alias, by lower-case name.
    pub sound_lengths: Vec<(String, u32)>,
    /// Weapons whose weapon file says they can be picked up again.
    pub retrievable_weapons: Vec<String>,
    /// Each vehicle's turret and gunner weapons and how it drives, such as
    /// the scorestreak helicopters.
    pub vehicles: Vec<asset_t6::VehicleRef>,
    /// Shellshock files (`shock/<name>.shock`) as (name, text).
    pub shocks: Vec<(String, String)>,
    /// The third-person player animation script and its list of anim types
    /// (`mp/playeranim.script`, `mp/playeranimtypes.txt`).
    pub playeranim: Option<(String, String)>,
    /// Props that break apart piece by piece when shot: mannequins, cars, a
    /// clock.
    pub destructibles: Vec<Arc<xmodel_runtime::BreakableModelDef>>,
    /// Barrier types (`ZBarrierDef`: window boards, the box), a later zone
    /// winning a name.
    pub zbarriers: Vec<asset_t6::ZBarrierDefRef>,
    /// Another map's special weapons loaded onto this one (the Hell's
    /// Retriever on Nuketown): the `level._effect` rows that map's fx
    /// script sets for them, as (key, effect).
    pub donor_effects: Vec<(String, String)>,
    /// How a dead body goes limp (`ragdoll.cfg`).
    pub ragdoll: Option<String>,
}

/// The parts of an animation the server needs.
#[derive(Clone, Debug, Default)]
pub struct T6AnimFacts {
    pub name: String,
    pub frame_total: u16,
    pub fps: f32,
    pub looping: bool,
    /// Root translation keys as (frame, position).
    pub delta_trans: Vec<(u16, [f32; 3])>,
    /// Root turn keys as (frame, quaternion x y z w).
    pub delta_quat: Vec<(u16, [f32; 4])>,
    /// Notetracks as (name, time from 0 to 1).
    pub notifies: Vec<(String, f32)>,
}
