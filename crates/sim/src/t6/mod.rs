//! bo2zm M3: Black Ops II Zombies' own scripts and the engine side they call.
//!
//! A BO2 map hands its zones' compiled scripts to [`install`]; the `gsc_t6`
//! VM runs them, here, beside IW4L's GSC runtime (which then only keeps its
//! stub start-up: dvar `bo2zm_t6` set makes its connect callback step aside).
//! The level start follows the game's order: map entities and structs, the
//! game type's `main`, the map's `main`, `codecallback_startgametype`; then
//! every authority frame connects players and runs the scheduler.
//!
//! Engine state the scripts see lives in [`Zm`]: entities (map and spawned),
//! players, movers, tables. Natives are `fn(&mut Vm<World>, &mut World, self,
//! args)`. Not part of upstream IW4L.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Hooks, ObjKind, ObjRef, Program, ScriptObject, Strings, Value, Vm};

use crate::frame::FrameWorld;
use crate::world::ClientId;

mod actors;
mod asd;
mod audio_triggers;
mod autoplay;
mod autoplay_prison;
mod brushes;
mod buried_egg;
mod cheats; // bo2zm: the solo cheat menu (Num 1)
mod clientfields;
mod darts;
mod declassified;
mod fields;
mod flight;
mod blackhole_fx;
mod freezegun_fx;
mod foreign_weapons;
mod fxanims;
mod globallogic;
mod grenades;
// bo2mp gun brain window 13: the gun builtins and notifies on the T6 gun state.
mod gun_script;
mod hud;
mod movers;
mod natives_ai;
mod natives_ent;
mod highrise;
mod natives_fx;
mod natives_game;
mod natives_player;
mod nav;
mod offhand_visible;
mod players;
mod presence;
mod prison_box;
mod prison_plane;
mod prison_retriever;
mod prison_wolves;
mod tomb_craft;
mod tomb_biplane;
mod tomb_mechz;
mod tomb_portal;
mod tomb_robot;
mod tomb_staffs;
mod tags;
mod tomb_beacon;
mod tomb_staff_charge;
mod tomb_staff_fx;
mod tomb_tank;
mod tomb_test;
mod riders;
pub(crate) mod riotshield;
pub(crate) mod charge_shot;
pub(crate) mod jetgun;
pub(crate) mod mc_specials;
mod limits;
mod stats;
mod offhand;
mod transit;
mod semtex_light;
mod triggers;
mod turrets;
pub(crate) use grenades::note_detonation;
pub(crate) use triggers::{DamageKind, damage_triggers};
mod use_test;
mod vehicles;
mod wallbuys;
mod zbarrier;

/// A string table (`mp/zombiemode.csv`, ...).
#[derive(Clone, Debug, Default)]
pub struct T6Table {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<String>,
}

/// What a BO2 map gives the script runtime.
#[derive(Clone, Debug, Default)]
pub struct T6Install {
    pub map: String,
    pub gametype: String,
    /// The start location when chosen (Tranzit's `town`); empty = the map's own.
    pub location: String,
    /// Compiled script objects in zone load order (later replace earlier).
    pub objects: Vec<Vec<u8>>,
    /// String tables by lower-case name.
    pub tables: BTreeMap<String, T6Table>,
    /// The map's entity string(s).
    pub entities: Vec<String>,
    /// The map's AI path nodes, in node order.
    pub path_nodes: Vec<T6PathNode>,
    /// Animations the server times and moves actors by.
    pub anims: Vec<T6Anim>,
    /// Animation state definitions: (name, text).
    pub animstatedefs: Vec<(String, String)>,
    /// Skeletons and hit boxes of the models entities can wear, by name.
    pub models: Vec<(String, Arc<xmodel_runtime::RetainedModelCapability>)>,
    /// Clips actors are posed with for hits, by (lowercase) name.
    pub clips: Vec<(String, Arc<xmodel_runtime::AnimClip>)>,
    /// The English strings (`ZOMBIE_WEAPON_M14` -> its text).
    pub strings: std::collections::HashMap<String, String>,
    /// Every sound alias the banks hold (lower case).
    pub sound_aliases: BTreeSet<String>,
    /// Props that break in stages (`destructibledef`), by name.
    pub destructibles: Vec<Arc<xmodel_runtime::T5DestructibleDef>>,
    /// Barrier types (`ZBarrierDef`) the map's zbarriers name by `type`.
    pub zbarrier_defs: Vec<T6ZBarrierDef>,
    /// Another map's special weapons loaded onto this one: the
    /// `level._effect` rows that map's fx script sets for them, (key,
    /// effect), set at the start where the map leaves them unset.
    pub donor_effects: Vec<(String, String)>,
}

/// A barrier type: the zombie animation states its windows ask for.
#[derive(Clone, Debug, Default)]
pub struct T6ZBarrierDef {
    pub name: String,
    pub taunts: bool,
    pub reach_through: bool,
    /// Pieces torn off (state "open") stop drawing (`autoHideOpenPieces`).
    pub auto_hide_open_pieces: bool,
    pub taunt_state: String,
    pub reach_through_state: String,
    pub num_attack_slots: u32,
    pub attack_spot_horz_offset: f32,
    /// Per board: (tear anim state, tear anim substate).
    pub boards: Vec<(String, String)>,
    /// Per board: (model, opening clip, closing clip), as the type has them.
    pub looks: Vec<(String, String, String)>,
}

/// An AI path node and its links (node, distance, negotiation).
#[derive(Clone, Debug, Default)]
pub struct T6PathNode {
    pub ty: u32,
    pub spawnflags: u32,
    pub targetname: String,
    pub target: String,
    pub script_noteworthy: String,
    pub script_linkname: String,
    pub animscript: String,
    pub origin: [f32; 3],
    pub angle: f32,
    pub radius: f32,
    pub links: Vec<(u16, f32, bool)>,
}

/// An animation's timing, root motion and notetracks.
#[derive(Clone, Debug, Default)]
pub struct T6Anim {
    pub name: String,
    pub numframes: u16,
    pub framerate: f32,
    pub looping: bool,
    pub delta_trans: Vec<(u16, [f32; 3])>,
    /// The root's turn keys (frame, quaternion x y z w); empty: no turn.
    pub delta_quat: Vec<(u16, [f32; 4])>,
    pub notifies: Vec<(String, f32)>,
}

#[derive(Resource)]
pub(crate) struct T6Runtime {
    pub vm: Vm<World>,
    /// His "Omni Movement" setting when the game started.
    pub omni: bool,
}

/// An entity the scripts see (not a player).
#[derive(Clone, Debug, Default)]
pub(crate) struct Ent {
    pub obj: Option<ObjRef>,
    pub classname: String,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub model: String,
    pub hidden: bool,
    /// Hidden by `ghost`: not drawn, but still shot and hit (Origins' wind
    /// staff smoke detectors are ghosted brushes the staff's shots turn).
    pub ghosted: bool,
    pub solid: bool,
    /// From the map (not spawned by a script).
    pub map: bool,
    /// Brush model (`*N`) a map entity uses.
    pub brush: Option<String>,
    /// A brush entity's angles as the map placed it (its brushes are built
    /// that way; a trigger turned since turns them, `triggers::touches`).
    pub spawn_angles: Option<[f32; 3]>,
    /// The map's `destructibledef`: the prop breaks in stages as it is hit.
    pub destructible: String,
    /// Trigger radius/height (`trigger_radius*`) or brush extents.
    pub radius: f32,
    pub height: f32,
    /// A zbarrier's pieces (the magic box).
    pub zbarrier: Option<zbarrier::ZBarrier>,
    /// A box trigger (`trigger_box`, `trigger_box_use`): width (along its
    /// forward), length (right), height, centred on the origin and turned
    /// by its angles.
    pub box_dims: Option<[f32; 3]>,
    /// `usetriggerrequirelookat`: the player must also face the trigger's
    /// centre (`triggers::box_use_reach`), not only stand in it.
    pub look_at: bool,
    /// `sethintlowpriority(1)`: its hint yields to any other trigger he can
    /// use there (every buyable door, `_zm_blockers` door_think).
    pub hint_low: bool,
    pub hint: Option<String>,
    pub cursor_hint: Option<String>,
    pub link: Option<(u32, [f32; 3], [f32; 3])>,
    /// The parent's tag it rides (`linkto` with a tag; `tags`).
    pub link_tag: Option<String>,
    pub attached: Vec<(String, String)>,
    /// The parts Black Ops II hangs on a weapon's world model
    /// (`useweaponmodel`, a wall or box gun): magazine, scope, rocket, each
    /// placed at its offset from a tag of the gun.
    pub weapon_parts: Vec<(String, String, xmodel_runtime::TagOffset)>,
    pub loop_sound: Option<String>,
    pub can_damage: bool,
    /// Players a trigger (or model) is hidden from.
    pub invisible_to: BTreeSet<u32>,
    pub invisible_to_all: bool,
    pub trigger_off: bool,
    /// `setteamfortrigger` (the team, false) or `setexcludeteamfortrigger`
    /// (the team, true): which zombies a radius, box or use trigger answers
    /// (`triggers::actor_touches`).
    pub trigger_team: Option<(String, bool)>,
    /// `setmovingplatformenabled(1)`: players standing on it ride it
    /// (`riders`).
    pub platform: bool,
    /// Tags the scripts hid on its model (`hidepart`), lower case.
    pub hidden_parts: Vec<String>,
    /// A script model's animation (`setanim`): clip and when it started.
    pub anim: Option<(String, i64)>,
    /// Gone limp (`startragdoll`, thrown by `launchragdoll`): the clients
    /// drop the body under gravity.
    pub ragdoll: Option<xmodel_runtime::DObjRagdoll>,
    /// bo2zm round 1: its eye brightness (`mapshaderconstant` scriptVector2.y
    /// from zombie_eyes_clientfield_cb): 2 alive (Origins 3), 0.25 out.
    pub eye_luminance: Option<f32>,
}

#[derive(Clone, Debug)]
pub(crate) struct Player {
    pub obj: ObjRef,
    pub begun: bool,
    pub sessionstate: String,
    pub perks: BTreeSet<String>,
    /// The entity the player is linked to (`playerlinkto`): he goes where
    /// it goes.
    pub linked: Option<u32>,
    /// The knife the scripts gave him (named in a knife hit).
    pub melee_weapon: Option<String>,
    /// Down (last stand): he crawls until `reviveplayer` / `undolaststand`.
    pub laststand: bool,
    /// His Jet Gun's spin and heat.
    pub jet: jetgun::JetState,
    /// `forcegrenadethrow`: let go of his grenade for him (t6::offhand).
    pub force_throw: bool,
    /// Whether he was diving / sprinting last tick (`dtp_start` /
    /// `dtp_end`, `sprint_begin` / `sprint_end`).
    pub last_dive: bool,
    pub last_sprint: bool,
    /// His scoreboard counts the engine keeps (headshots, downs, revives...).
    pub stats: BTreeMap<String, i32>,
    /// His HUD's client fields that are set (perks, power-ups, Mob's
    /// Afterlife lives and mana), in the order they were set.
    pub hud_fields: Vec<(String, f32)>,
    /// His points counters (`score_cf_damage`, ...: each hit and kill
    /// steps one; his HUD flies one popup per step).
    pub score_fields: Vec<(String, i32)>,
    /// A script camera (`camerasetposition` + `cameraactivate`): his view
    /// rides that entity (Nuketown's game-over rocket shot).
    pub camera: Option<u32>,
    pub camera_on: bool,
    /// His body model (`setmodel`; not drawn in third person yet): Mob's
    /// Afterlife copies it onto the body he leaves behind.
    pub model: String,
    /// `startpoisoning` / `stoppoisoning`: BO2's player-state poisoned flag
    /// (TranZit's Denizen grab and the fog's gas; the client draws it).
    pub poisoned: bool,
}

/// Engine state for the scripts.
#[derive(Resource, Default)]
pub(crate) struct Zm {
    pub map: String,
    pub ents: BTreeMap<u32, Ent>,
    pub next_entnum: u32,
    pub players: BTreeMap<u32, Player>,
    pub tables: Arc<BTreeMap<String, T6Table>>,
    pub fx: Vec<String>,
    /// Client systems by id (`clientsysregister`): "musicCmd", ...
    pub client_sys: Vec<String>,
    /// `spawnfx` effects: name, origin, forward, up (when given).
    pub fx_ents: BTreeMap<u32, (String, [f32; 3], [f32; 3], Option<[f32; 3]>)>,
    pub unbound: BTreeMap<String, u64>,
    pub started: bool,
    pub movers: movers::Movers,
    /// Notifies to deliver later: (server ms, object, name).
    pub timers: Vec<(i64, ObjRef, String)>,
    /// Script movers showing entities' models.
    pub presences: presence::Presences,
    /// The map's AI navigation graph.
    pub nav: nav::Nav,
    /// Brush entities' collision as last sent.
    pub brush_rows: BTreeMap<u32, brushes::BrushRow>,
    /// Path node script objects, by node index.
    pub node_objs: Vec<ObjRef>,
    /// Animations by lower-case name.
    pub anims: std::collections::HashMap<String, T6Anim>,
    pub clips: std::collections::HashMap<String, Arc<xmodel_runtime::AnimClip>>,
    pub strings: std::collections::HashMap<String, String>,
    pub sound_aliases: BTreeSet<String>,
    /// Props that break in stages, by lower-case name.
    pub destructibles: std::collections::HashMap<String, Arc<xmodel_runtime::T5DestructibleDef>>,
    /// Barrier types by lower-case name.
    pub zbarrier_defs: std::collections::HashMap<String, Arc<T6ZBarrierDef>>,
    /// Other maps' special weapon effects, (`level._effect` key, effect).
    pub donor_effects: Vec<(String, String)>,
    /// Script HUD elements (Game Over, Max Ammo...).
    pub huds: hud::Huds,
    /// Flying limbs and when they go.
    pub gibs: Vec<(u32, i64)>,
    /// Client-field effects waiting their time.
    pub client_fx: clientfields::ClientFx,
    /// Script vehicles on their paths (Nuketown's perk arrival).
    pub vehicles: vehicles::Vehicles,
    /// Players standing on moving platforms (Tranzit's bus).
    pub riders: riders::Riders,
    /// Wall buys' chalk and bought guns (BO2's client-script visuals).
    pub wallbuys: wallbuys::WallBuys,
    pub wallbuy_since: Option<i64>,
    /// The debris piles' path cuts are made (`brushes::cut_debris_paths`).
    pub debris_paths_cut: bool,
    /// `disablegrenadesuicide` (BO2's level.disable_grenade_suicide): a grenade held past
    /// its fuse goes off in his hand.
    pub grenade_suicide_off: bool,
    /// Origins' `tu14_bg_chargeShotExponentialAmmoPerChargeLevel` (charge_shot::sync_dvars).
    pub charge_ammo_exponential: bool,
    /// What each player's HUD was last sent (round, hint, offhand,
    /// action slots).
    pub hud_sent: BTreeMap<u32, (String, String, String, String, String)>,
    /// bo2zm M4: each player's d-pad slots (`setactionslot(slot, "weapon",
    /// name)`): (slot, weapon or kind).
    pub action_slots: BTreeMap<u32, Vec<(i32, String)>>,
    /// The test player's fire button, pressed every other tick.
    pub autoplay_fire: bool,
    /// When every expected player of a party was first playing (the test
    /// aids' party clock, `autoplay::party_clock`).
    pub party_in_at: Option<i64>,
    /// The round the scripts last reported (`setroundsplayed`).
    pub rounds_played: i32,
    /// Animation state definitions by file stem (`zm_nuked_basic`).
    pub asds: std::collections::HashMap<String, asd::AnimStateDef>,
    /// Actors (zombies) by entity number.
    pub actors: BTreeMap<u32, actors::Actor>,
    /// Server time of the last scheduler run.
    pub now_ms: i64,
    pub next_thread_dump: i64,
    /// Players holding use (for the press edge).
    pub use_held: BTreeSet<u32>,
    /// The hint of the use trigger each player faces.
    pub hints: BTreeMap<u32, String>,
    /// The map's moving props (washing lines, shutters, wires, dust devils).
    pub fxprops: fxanims::FxProps,
    /// Mob's wolf heads (the client script's half of feeding them).
    pub wolves: prison_wolves::Wolves,
    /// The map's sound triggers (bumping a car, the truck bed).
    pub audio_trigs: audio_triggers::AudioTrigs,
    /// Players' thrown grenades the scripts watch (the Monkey Bomb).
    pub thrown: Vec<grenades::Thrown>,
    /// Gone-off grenades freed a tick later.
    pub thrown_free: Vec<(ObjRef, i64)>,
    /// Where tracked shots went off this tick (their `projectile_impact`).
    pub detonations: Vec<(crate::ProjectileId, [f32; 3])>,
}

/// Entity numbers: players are their client number; the rest start here.
pub(crate) const FIRST_ENTNUM: u32 = 64;

impl Zm {
    pub(crate) fn alloc_entnum(&mut self) -> u32 {
        if self.next_entnum < FIRST_ENTNUM {
            self.next_entnum = FIRST_ENTNUM;
        }
        let n = self.next_entnum;
        self.next_entnum += 1;
        n
    }
}

/// Install a BO2 map's scripts (before [`start`]).
/// bo2zm: Black Ops II's hit locations by its models' part classes (T6
/// added torso_mid at 5, so from there on IW4's names were one off: a lower
/// torso hit reached the scripts as right_arm_upper, a left foot as gun).
pub(crate) const T6_HITLOC_NAMES: [&str; 21] = [
    "none",
    "helmet",
    "head",
    "neck",
    "torso_upper",
    "torso_mid",
    "torso_lower",
    "right_arm_upper",
    "left_arm_upper",
    "right_arm_lower",
    "left_arm_lower",
    "right_hand",
    "left_hand",
    "right_leg_upper",
    "left_leg_upper",
    "right_leg_lower",
    "left_leg_lower",
    "right_foot",
    "left_foot",
    "gun",
    "riotshield",
];

/// bo2zm: his profile's choices the scripts and the engine read (BO2's
/// Settings menu sets them): mature content (`getlocalprofileint
/// ("cg_mature")`: heads and limbs come off) and how many corpses stay
/// (`ai_corpseCount`: past it the oldest goes).
static MATURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static CORPSES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(16);

/// The melee weapon each client holds as his knife (0: none given), for
/// his first-person swing: BO2 stabs with the Bowie Knife, the
/// Galvaknuckles or the Spork he bought, drawn as itself, not the knife.
static MELEE_WEAPONS: [std::sync::atomic::AtomicU32; 18] =
    [const { std::sync::atomic::AtomicU32::new(0) }; 18];

pub fn melee_weapon(client: u32) -> u32 {
    MELEE_WEAPONS
        .get(client as usize)
        .map_or(0, |w| w.load(std::sync::atomic::Ordering::Relaxed))
}

pub(super) fn set_melee_weapon(client: u32, weapon: u32) {
    if let Some(w) = MELEE_WEAPONS.get(client as usize) {
        w.store(weapon, std::sync::atomic::Ordering::Relaxed);
    }
}

pub use natives_game::set_t6_game_settings;
pub use cheats::{
    Cheat, add_special_items, add_special_weapons, cheat_on, cheat_pick, cheats_allowed, special_pick,
    special_weapons,
};
pub use highrise::platform_rides_on_ground;
pub use transit::platform_ride_box;

pub fn set_t6_profile(mature: bool, corpses: u8) {
    MATURE.store(mature, std::sync::atomic::Ordering::Relaxed);
    CORPSES.store(corpses.max(1), std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn profile_mature() -> bool {
    MATURE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether a Black Ops II Zombies player has a perk (`hasperk`); false
/// outside Black Ops II Zombies.
pub(crate) fn has_perk(world: &crate::frame::FrameWorld, id: crate::world::ClientId, perk: &str) -> bool {
    world
        .ecs_ref()
        .get_resource::<Zm>()
        .and_then(|zm| zm.players.get(&id.0))
        .is_some_and(|pl| pl.perks.contains(perk))
}

pub(crate) fn corpse_limit() -> usize {
    usize::from(CORPSES.load(std::sync::atomic::Ordering::Relaxed))
}

/// The party a Public Match lobby started with (0 = none): wins over
/// IW4L_T6_PLAYERS.
static PARTY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// bo2zm H28: the players in the lobby that starts the next game (the host
/// and those who joined it), 1-4; 0 = no lobby (his own game).
pub fn set_t6_party_size(n: usize) {
    PARTY.store(n.min(4) as i32, std::sync::atomic::Ordering::Relaxed);
}

/// How many players the host's party is (IW4L_T6_PLAYERS, 1-4, default 1):
/// BO2 waits for them all before the first round, and a party of one is a
/// solo game.
pub(crate) fn expected_players() -> i32 {
    let party = PARTY.load(std::sync::atomic::Ordering::Relaxed);
    if party > 0 {
        return party;
    }
    static N: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("IW4L_T6_PLAYERS")
            .ok()
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(1)
            .clamp(1, 4)
    })
}

pub(crate) fn install(world: &mut World, inst: T6Install) -> Result<(), String> {
    cheats::reset();
    let mut objects = Vec::with_capacity(inst.objects.len());
    for bytes in &inst.objects {
        objects.push(ScriptObject::parse(bytes)?);
    }
    // IW4L_T6_FDIS=name,name: write those script functions' decoded code to
    // the log (reading a map's own script when no source is on disk).
    if let Ok(names) = std::env::var("IW4L_T6_FDIS") {
        let names: Vec<&str> = names.split(',').map(str::trim).collect();
        for o in &objects {
            for f in o.functions.iter().filter(|f| names.contains(&f.name.as_str())) {
                diag::info!(Sim, "bo2zm t6 fdis: {}::{} params {}", o.name, f.name, f.params);
                for (at, insn) in &f.code {
                    diag::info!(Sim, "bo2zm t6 fdis:   {at:#06x} {insn:?}");
                }
            }
        }
    }
    let mut strings = Strings::default();
    let program = Program::link(objects, &mut strings)?;
    let functions = program.functions.len();
    let builtins = program.builtins.len();
    let mut vm: Vm<World> = Vm::new(program, strings);
    // A new game rolls new dice (which perk falls next, box guns, drops): the
    // scripts' random numbers start from the clock. IW4L_T6_SEED=<n> fixes
    // them for a test that must repeat.
    vm.rng = std::env::var("IW4L_T6_SEED")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as u64)
        })
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        | 1;
    gsc_t6::natives::bind_core(&mut vm);
    natives_game::bind(&mut vm);
    natives_ent::bind(&mut vm);
    natives_player::bind(&mut vm);
    globallogic::bind(&mut vm);
    // Sounds and effects replace the quiet stubs bound above.
    natives_fx::bind(&mut vm);
    natives_ai::bind(&mut vm);
    zbarrier::bind(&mut vm);
    darts::bind(&mut vm);
    clientfields::bind(&mut vm);
    brushes::bind(&mut vm);
    vehicles::bind(&mut vm);
    turrets::bind(&mut vm);
    riotshield::bind(&mut vm);
    jetgun::bind(&mut vm);
    mc_specials::add_to_cheats();
    offhand::bind(&mut vm);
    limits::bind(&mut vm);
    stats::bind(&mut vm);
    hud::bind(&mut vm);
    // bo2mp gun brain, window 5: BO2's setoffhandvisible (the equipment in his hand).
    offhand_visible::bind(&mut vm);
    // bo2mp gun brain: the gun builtins on BO2's gun state, last.
    gun_script::bind(&mut vm);
    // IW4L_T6_TRACE=name,name: log those builtins' calls (debugging).
    if let Ok(names) = std::env::var("IW4L_T6_TRACE") {
        vm.trace_builtins(&names, 5000);
    }
    // IW4L_T6_FTRACE=name,name: log those script functions' calls.
    if let Ok(names) = std::env::var("IW4L_T6_FTRACE") {
        vm.trace_functions(&names);
    }
    vm.hooks = Hooks {
        get_field: Some(fields::get),
        set_field: Some(fields::set),
        unbound: Some(unbound),
    };
    for (k, v) in [
        ("mapname", inst.map.as_str()),
        ("g_gametype", inst.gametype.as_str()),
        ("ui_gametype", inst.gametype.as_str()),
        ("ui_zm_gamemodegroup", zm_gamemodegroup(&inst.gametype)),
        (
            "ui_zm_mapstartlocation",
            if inst.location.is_empty() {
                location(&inst.map)
            } else {
                inst.location.as_str()
            },
        ),
        ("zm_gamemodegroup", zm_gamemodegroup(&inst.gametype)),
        ("sv_maxclients", "4"),
        ("party_maxplayers", "4"),
        ("onlinegame", "0"),
        ("systemlink", "0"),
        ("splitscreen", "0"),
        ("xblive_privatematch", "0"),
        ("developer", "0"),
        ("developer_script", "0"),
        ("zombie_cheat", "0"),
        ("sv_cheats", "0"),
        ("g_gameskill", "1"),
        ("scr_zm_enable_bots", "0"),
        // The box may move (teddy bear): the magic box reads it and no
        // script or config of his sets it, so the engine's own default.
        ("magic_chest_movable", "1"),
        // Client scripts own the map's placed effects and exploders (our
        // client draws the placed effects itself).
        ("cg_usingclientscripts", "1"),
        // The engine's default gravity: Ascension's Matryoshka dolls divide
        // by it at init (their launch peak time).
        ("bg_gravity", "800"),
    ] {
        vm.dvars.insert(k.to_owned(), v.to_owned());
    }
    diag::info!(
        Sim,
        "bo2zm t6 scripts: {} objects, {functions} functions, {builtins} builtins, {} unbound",
        inst.objects.len(),
        vm.unbound_builtins().len()
    );
    // IW4L_T6_UNBOUND=1: every builtin the scripts name that has no native.
    if std::env::var("IW4L_T6_UNBOUND").is_ok() {
        let mut names: Vec<String> = vm
            .unbound_builtins()
            .into_iter()
            .map(|(n, m)| if m { format!(".{n}") } else { n })
            .collect();
        names.sort();
        diag::info!(Sim, "bo2zm t6 unbound: {}", names.join(" "));
    }
    // Path nodes are stored where the designer placed them (Nuketown's
    // town grid floats at z 0, the ground is near -60); BO2 drops them to
    // the floor. Ours did not: every "can it walk straight there" sweep ran
    // 60 units up, over low walls, fences and cars, and zombies took those
    // routes into them and stood there (M4 retest 1).
    let mut nodes = inst.path_nodes.clone();
    actors::drop_nodes_to_floor(world, &mut nodes);
    let mut node_objs = Vec::with_capacity(nodes.len());
    for n in &nodes {
        let o = vm.alloc_object(ObjKind::Struct);
        let kind = match n.ty {
            nav::NODE_PATH => "Path",
            nav::NODE_NEGOTIATION_BEGIN => "Begin",
            nav::NODE_NEGOTIATION_END => "End",
            _ => "Path",
        };
        let fields: [(&str, Value); 4] = [
            ("origin", Value::Vec3(n.origin)),
            ("angles", Value::Vec3([0.0, n.angle, 0.0])),
            ("spawnflags", Value::Int(n.spawnflags as i32)),
            ("radius", Value::Float(n.radius)),
        ];
        for (k, v) in fields {
            let f = vm.intern(k);
            vm.set_raw_field(o, f, v);
        }
        for (k, v) in [
            ("type", kind),
            ("targetname", n.targetname.as_str()),
            ("target", n.target.as_str()),
            ("script_noteworthy", n.script_noteworthy.as_str()),
            ("script_linkname", n.script_linkname.as_str()),
            ("animscript", n.animscript.as_str()),
        ] {
            if !v.is_empty() {
                let f = vm.intern(k);
                let s = vm.string(v);
                vm.set_raw_field(o, f, s);
            }
        }
        node_objs.push(o);
    }
    let anims: std::collections::HashMap<String, T6Anim> = inst
        .anims
        .into_iter()
        .map(|a| (a.name.to_ascii_lowercase(), a))
        .collect();
    let asds = inst
        .animstatedefs
        .iter()
        .map(|(name, text)| {
            let stem = name
                .rsplit('/')
                .next()
                .unwrap_or(name)
                .trim_end_matches(".asd")
                .to_ascii_lowercase();
            (stem, asd::AnimStateDef::parse(text))
        })
        .collect::<std::collections::HashMap<String, asd::AnimStateDef>>();
    // IW4L_T6_ASDLOG=<part of a def name>: each state of those animstatedefs,
    // its notify and animations (test aid).
    if let Ok(want) = std::env::var("IW4L_T6_ASDLOG") {
        for (stem, def) in asds.iter().filter(|(k, _)| k.contains(want.as_str())) {
            let mut names: Vec<&String> = def.states.keys().collect();
            names.sort();
            for k in names {
                let st = &def.states[k];
                diag::info!(
                    Sim,
                    "bo2zm t6 asd {stem} {k}: notify {} flags {:?} anims {:?}",
                    st.notify,
                    st.flags,
                    st.substates
                );
            }
        }
    }
    // IW4L_T6_ANIM=<part of a name>: those animations' timing and root motion.
    if let Ok(want) = std::env::var("IW4L_T6_ANIM") {
        let mut names: Vec<&String> = anims.keys().filter(|k| k.contains(want.as_str())).collect();
        names.sort();
        for k in names.into_iter().take(40) {
            let a = &anims[k];
            diag::info!(
                Sim,
                "bo2zm t6 anim {k}: {} frames at {} fps, looping {}, {} root keys {:?} .. {:?}, speed {:.1}, notes {:?}",
                a.numframes,
                a.framerate,
                a.looping,
                a.delta_trans.len(),
                a.delta_trans.first(),
                a.delta_trans.last(),
                actors::anim_speed(a),
                a.notifies
            );
        }
    }
    diag::info!(
        Sim,
        "bo2zm t6: {} path nodes, {} animations, {} animstatedefs",
        nodes.len(),
        anims.len(),
        inst.animstatedefs.len()
    );
    diag::info!(
        Sim,
        "bo2zm t6: {} models with hit boxes, {} actor clips",
        inst.models
            .iter()
            .filter(|(_, c)| c.bone_collision.iter().any(Option::is_some))
            .count(),
        inst.clips.len()
    );
    frame(world).add_model_capabilities(inst.models);
    world.insert_resource(Zm {
        map: inst.map.clone(),
        clips: inst.clips.into_iter().collect(),
        // Keys upper-cased: BO2 looks them up without case (the B23R's
        // chalk asks for "ZOMBIE_WEAPON_BERETTA93r", fix list 3).
        strings: inst
            .strings
            .into_iter()
            .map(|(k, v)| (k.to_ascii_uppercase(), v))
            .collect(),
        sound_aliases: inst.sound_aliases,
        destructibles: inst
            .destructibles
            .into_iter()
            .map(|d| (d.name.to_ascii_lowercase(), d))
            .collect(),
        zbarrier_defs: inst
            .zbarrier_defs
            .into_iter()
            .map(|d| (d.name.to_ascii_lowercase(), Arc::new(d)))
            .collect(),
        donor_effects: inst.donor_effects,
        tables: Arc::new(inst.tables),
        next_entnum: FIRST_ENTNUM,
        nav: nav::Nav::new(nodes),
        node_objs,
        anims,
        asds,
        ..Default::default()
    });
    world.insert_resource(T6Runtime { vm, omni: crate::step::omni_movement_setting() });
    world.insert_resource(PendingEntities(inst.entities));
    Ok(())
}

#[derive(Resource)]
struct PendingEntities(Vec<String>);

/// The zombies game type a map starts in from Solo: Nuketown is
/// `zstandard` only; the other Black Ops II maps play their full mode,
/// `zclassic` (each map's `gamemode_callback_setup` registers it).
pub fn zm_gametype(map: &str) -> &'static str {
    match map {
        "zm_nuked" => "zstandard",
        _ => "zclassic",
    }
}

/// The game mode group of a game type: `_zm_utility::is_classic` reads
/// `ui_zm_gamemodegroup`, so a `zclassic` map must say `zclassic`; and
/// `is_encounter` wants `zencounter` (Grief and Turned: without it no
/// player gets a team, and Grief says "another chance" at the start).
fn zm_gamemodegroup(gametype: &str) -> &'static str {
    match gametype {
        "zclassic" => "zclassic",
        "zgrief" | "zcleansed" | "zmeat" => "zencounter",
        _ => "zsurvival",
    }
}

fn location(map: &str) -> &'static str {
    match map {
        "zm_nuked" => "nuked",
        "zm_transit" => "transit",
        "zm_highrise" => "rooftop",
        "zm_prison" => "prison",
        "zm_buried" => "processing",
        "zm_tomb" => "tomb",
        // Declassified: the map's own default_start_location (its Mule Kick
        // is "zclassic_perks_default"; zzz_zm_location.gsc).
        "zm_prototype" => "default",
        _ => "",
    }
}

/// Run `f` with the VM out of the world.
pub(crate) fn with_vm<R>(
    world: &mut World,
    f: impl FnOnce(&mut Vm<World>, &mut World) -> R,
) -> Option<R> {
    if !world.contains_resource::<T6Runtime>() {
        return None;
    }
    Some(
        world.resource_scope(|world, mut rt: bevy_ecs::prelude::Mut<T6Runtime>| {
            f(&mut rt.vm, world)
        }),
    )
}

/// bo2zm M4: a player's answer from a BO2 menu (`Engine.SendMenuResponse`:
/// "popup_leavegame" "endround", ...) reaches the scripts as his
/// `menuresponse` notify. False when no BO2 map runs.
pub(crate) fn menu_response(world: &mut World, client: u32, menu: &str, response: &str) -> bool {
    with_vm(world, |vm, world| {
        let Some(p) = world.resource::<Zm>().players.get(&client).map(|p| p.obj) else {
            return;
        };
        let (m, r) = (vm.string(menu), vm.string(response));
        diag::info!(
            Sim,
            "bo2zm t6 menuresponse from {client}: {menu} {response}"
        );
        vm.notify_str(world, p, "menuresponse", &[m, r]);
    })
    .is_some()
}

/// The level start: map entities and structs, then the game type's and
/// the map's `main` and `codecallback_startgametype`.
pub(crate) fn start(world: &mut World) {
    let Some(PendingEntities(texts)) = world.remove_resource::<PendingEntities>() else {
        return;
    };
    let map = world.resource::<Zm>().map.clone();
    let gametype = with_vm(world, |vm, _| {
        vm.dvars.get("g_gametype").cloned().unwrap_or_default()
    })
    .unwrap_or_default();
    with_vm(world, |vm, world| {
        let level = Value::Object(vm.level);
        vm.spawn_named(
            world,
            "codescripts/struct",
            "initstructs",
            level.clone(),
            vec![],
        );
        natives_ent::spawn_map_entities(vm, world, &texts);
        // The engine's calls, in order. Functions no script calls are the
        // engine's (measured: zm_nuked::gamemode_callback_setup registers
        // the map's game modes, which the game type's start needs;
        // _zm::post_main; codecallback_finalizeinitialization). A map main
        // that calls _zm::post_main itself (Buried) gets it once: twice ran
        // init_custom_ai_type twice (two Leroys, doubled spawn functions).
        let post_main_scripted = vm
            .program
            .find(&vm.strings, "maps/mp/zombies/_zm", "post_main")
            .is_some_and(|fi| vm.program.script_refers_to(&format!("maps/mp/{map}"), fi));
        for (script, func, required) in [
            (format!("maps/mp/{map}"), "gamemode_callback_setup", false),
            (format!("maps/mp/gametypes_zm/{gametype}"), "main", true),
            (format!("maps/mp/{map}"), "main", true),
            ("maps/mp/zombies/_zm".to_owned(), "post_main", false),
            (
                "maps/mp/gametypes_zm/_callbacksetup".to_owned(),
                "codecallback_startgametype",
                true,
            ),
            (
                "maps/mp/gametypes_zm/_callbacksetup".to_owned(),
                "codecallback_finalizeinitialization",
                false,
            ),
        ] {
            if func == "post_main" && post_main_scripted {
                continue;
            }
            if vm
                .spawn_named(world, &script, func, level.clone(), vec![])
                .is_none()
                && required
            {
                diag::warn!(Sim, "bo2zm t6: no {script}::{func}");
            }
        }
        foreign_weapons::set_effects(vm, world);
        diag::info!(
            Sim,
            "bo2zm t6: level started, {} threads",
            vm.thread_count()
        );
    });
    world.resource_mut::<Zm>().started = true;
    declassified::start(world, &map);
    report(world);
}

/// Each authority frame: connect players, move movers, run the scheduler.
/// IW4L_T6_PROF=1: where a tick's time goes (milliseconds per tick, every
/// 5 s of game).
#[derive(Default)]
struct Prof {
    ticks: u32,
    parts: Vec<(&'static str, f64)>,
}

fn prof_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("IW4L_T6_PROF").is_some())
}

fn prof_add(name: &'static str, since: std::time::Instant) -> std::time::Instant {
    static PROF: std::sync::Mutex<Option<Prof>> = std::sync::Mutex::new(None);
    let now = std::time::Instant::now();
    if let Ok(mut p) = PROF.lock() {
        let p = p.get_or_insert_with(Prof::default);
        if name == "tick" {
            p.ticks += 1;
            if p.ticks >= 100 {
                let line: Vec<String> = p
                    .parts
                    .iter()
                    .map(|(n, ms)| format!("{n} {:.2}", ms / f64::from(p.ticks)))
                    .collect();
                diag::info!(Sim, "bo2zm t6 prof (ms per tick): {}", line.join(", "));
                *p = Prof::default();
            }
        } else {
            let ms = (now - since).as_secs_f64() * 1000.0;
            match p.parts.iter_mut().find(|(n, _)| *n == name) {
                Some(row) => row.1 += ms,
                None => p.parts.push((name, ms)),
            }
        }
    }
    now
}

pub(crate) fn advance(world: &mut World) {
    if !world.contains_resource::<T6Runtime>() {
        return;
    }
    if prof_on() {
        let t0 = std::time::Instant::now();
        advance_inner(world);
        prof_add("total", t0);
        prof_add("tick", t0);
        return;
    }
    advance_inner(world);
}

/// One step's sub-part, timed when profiling.
/// bo2zm: the hang tracer. Each sim phase names itself; a watcher thread
/// prints the phase (to stderr, which a stuck main thread cannot hold up)
/// when no tick or client frame has gone by for 5 s.
static PHASE: std::sync::Mutex<&'static str> = std::sync::Mutex::new("idle");
static TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn phase(name: &'static str) {
    if let Ok(mut p) = PHASE.lock() {
        *p = name;
    }
}

/// His client drew a frame (the hang tracer's other clock).
pub fn client_frame_heartbeat() {
    FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn start_hang_tracer() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = std::thread::Builder::new().name("bo2zm-hang-tracer".into()).spawn(|| {
            use std::sync::atomic::Ordering::Relaxed;
            let (mut ticks, mut frames, mut told) = (0, 0, 0);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let (t, f) = (TICKS.load(Relaxed), FRAMES.load(Relaxed));
                let stuck_sim = t == ticks && t > 0;
                let stuck_client = f == frames && f > 0;
                // Told up to 6 times (every 5 s), so a script loop shows its
                // instruction count moving and a stuck builtin shows not.
                if (stuck_sim || stuck_client) && told < 6 {
                    let p = PHASE.lock().map(|p| *p).unwrap_or("?");
                    eprintln!(
                        "bo2zm hang: sim ticks {} ({t}), client frames {} ({f}); sim phase {p}; {}",
                        if stuck_sim { "STOPPED" } else { "going" },
                        if stuck_client { "STOPPED" } else { "going" },
                        gsc_t6::hang_report(),
                    );
                    diag::info!(Sim, "bo2zm hang: sim stuck {stuck_sim}, client stuck {stuck_client}, phase {p}");
                    told += 1;
                } else if !(stuck_sim || stuck_client) {
                    told = 0;
                }
                (ticks, frames) = (t, f);
            }
        });
    });
}

fn timed(name: &'static str, world: &mut World, f: impl FnOnce(&mut World)) {
    phase(name);
    if prof_on() {
        let t0 = std::time::Instant::now();
        f(world);
        prof_add(name, t0);
    } else {
        f(world);
    }
}

fn advance_inner(world: &mut World) {
    let (advances, tick) = {
        let request = world.resource::<crate::step::StepRequest>();
        (request.reason.advances_authority_world(), request.tick)
    };
    if !advances {
        return;
    }
    if world.contains_resource::<PendingEntities>() {
        start(world);
    }
    let now = i64::from(tick.0) * i64::from(crate::MATCH_TICK_MS);
    declassified::advance(world, now);
    world.resource_mut::<Zm>().now_ms = now;
    start_hang_tracer();
    TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    phase("test aids");
    players::sync(world);
    // bo2mp gun brain: BO2's client events from the usercmds (reload_start, weapon_fired,
    // weapon_melee, detonate, ...), on the T6 gun state.
    gun_script::frame_notifies(world);
    charge_shot::sync_dvars(world);
    players::movement_events(world);
    if !world.resource::<Zm>().debris_paths_cut {
        brushes::cut_debris_paths(world);
    }
    // The test player's buttons go in before the triggers read them.
    autoplay::drive(world, now);
    cheats::apply(world);
    cheats::hold(world);
    autoplay::buy_test(world, now);
    autoplay::open_all_test(world, now);
    autoplay::census_test(world, now);
    highrise::tick(world, now);
    autoplay::roam_test(world, now);
    autoplay::glide_test(world, now);
    autoplay::only_spawn_test(world);
    autoplay::box_test(world, now);
    autoplay_prison::shock_test(world, now);
    autoplay_prison::door_sweep(world, now);
    autoplay_prison::lives_watch(world, now);
    autoplay_prison::brutus_test(world, now);
    autoplay_prison::gondola_test(world, now);
    autoplay_prison::zone_probe(world, now);
    autoplay_prison::ghost_door_test(world, now);
    prison_plane::plane_test(world, now);
    prison_retriever::retriever_test(world, now);
    autoplay::box_move_test(world, now);
    autoplay::dual_wield_test(world, now);
    autoplay::mannequin_test(world, now);
    autoplay::knife_at_test(world, now);
    autoplay::view_test(world, now);
    autoplay::nohud_test(world);
    autoplay::bears_test(world, now);
    autoplay::perk_drop_test(world, now);
    autoplay::egg1_test(world, now);
    buried_egg::lantern_test(world, now);
    autoplay::game_over_test(world, now);
    autoplay::down_test(world, now);
    autoplay::corevive_test(world, now);
    autoplay::nades_test(world, now);
    autoplay::press_test(world, now);
    autoplay::give_test(world, now);
    autoplay::sprint_log(world, now);
    autoplay::door_test(world, now);
    autoplay::use_test(world, now);
    autoplay::walk_test(world, now);
    autoplay::ent_log(world, now);
    autoplay::floor_map(world, now);
    autoplay::kite_test(world, now);
    autoplay::face_test(world);
    autoplay::zombie_block_test(world, now);
    autoplay::solid_walk_test(world, now);
    autoplay::perk_test(world, now);
    use_test::use_test(world, now);
    use_test::build_test(world, now);
    use_test::place_test(world, now);
    autoplay::pap_test(world, now);
    autoplay::powerup_test(world, now);
    autoplay::fx_test(world, now);
    autoplay::look_test(world);
    autoplay::gib_test(world, now);
    tomb_test::generator_test(world, now);
    tomb_test::dig_test(world, now);
    tomb_test::recapture_test(world, now);
    tomb_mechz::mechz_test(world, now);
    tomb_robot::robot_test(world, now);
    tomb_biplane::biplane_test(world, now);
    tomb_portal::portal_test(world, now);
    tomb_craft::craft_test(world, now);
    tomb_staffs::staff_puzzle_test(world, now);
    tomb_tank::tank_test(world, now);
    darts::advance(world, now);
    darts::log(world, now);
    grenades::advance(world, now);
    // limits first: its IW4L_T6_PRESS holds are test aids too.
    limits::tick(world);
    // After the test aids: their held fire spins it too.
    jetgun::tick(world, crate::MATCH_TICK_MS as f32);
    mc_specials::tick(world);
    offhand::tick(world);
    phase("vehicles+movers");
    vehicles::advance(world, crate::MATCH_TICK_MS as f32 / 1000.0);
    movers::advance(world, now);
    riders::advance(world);
    turrets::advance(world);
    let moved = {
        let mut zm = world.resource_mut::<Zm>();
        let zm = &mut *zm;
        let ents = &zm.ents;
        zm.nav
            .follow_movers(|n| ents.get(&n).map(|e| (e.origin, e.angles)))
    };
    // Their script `origin` too (Die Rise's escape pod links its door
    // nodes to the nodes near node.origin once it has crashed).
    if !moved.is_empty() {
        let objs: Vec<(ObjRef, [f32; 3])> = {
            let zm = world.resource::<Zm>();
            moved
                .iter()
                .filter_map(|&(n, p)| zm.node_objs.get(n as usize).map(|&o| (o, p)))
                .collect()
        };
        with_vm(world, |vm, _| {
            let f = vm.intern("origin");
            for (o, p) in objs {
                vm.set_raw_field(o, f, Value::Vec3(p));
            }
        });
    }
    zbarrier::advance(world, now);
    wallbuys::advance(world, now);
    timed("triggers", world, triggers::dispatch);
    timed("hud", world, |world| {
        publish_hud(world);
        hud::publish(world);
    });
    timed("scripts", world, |world| {
        with_vm(world, |vm, world| {
            if vm.time_ms < now {
                vm.time_ms = now;
            }
            natives_game::deliver_timers(vm, world, now);
            vm.run_due(world);
        });
        // bo2mp gun brain: BO2's per-frame weapon notifies come after the threads.
        gun_script::frame_watch(world);
    });
    timed("actors", world, |world| actors::think(world, now));
    phase("gibs");
    // Limbs that have lain long enough go.
    let gone: Vec<u32> = {
        let mut zm = world.resource_mut::<Zm>();
        let (old, keep): (Vec<(u32, i64)>, Vec<(u32, i64)>) =
            zm.gibs.iter().partition(|(_, t)| *t <= now);
        zm.gibs = keep;
        old.into_iter().map(|(n, _)| n).collect()
    };
    for n in gone {
        let obj = world
            .resource_mut::<Zm>()
            .ents
            .remove(&n)
            .and_then(|e| e.obj);
        if let Some(o) = obj {
            with_vm(world, |vm, world| vm.free_object(world, o));
        }
    }
    timed("presence", world, |world| {
        presence::sync(world, now);
        fxanims::advance(world, now);
        prison_wolves::advance(world, now);
        audio_triggers::advance(world, now);
        brushes::sync(world, now);
        players::follow_sync(world);
        publish_melee_targets(world);
    });
    timed("loops+fx", world, |world| {
        presence::publish_loops(world);
        clientfields::advance(world, now);
        tomb_beacon::advance(world, now);
        tomb_staff_fx::advance(world, now);
        tomb_staff_charge::advance(world, now);
        semtex_light::advance(world, now);
    });
    phase("report");
    report(world);
    phase("idle");
}

/// A living zombie's feet by its entity number: a knife lunge's target (BO2 lunges at AI
/// the same as at players).
pub(crate) fn zombie_feet(world: &World, ent: u32) -> Option<[f32; 3]> {
    let zm = world.get_resource::<Zm>()?;
    zm.actors.get(&ent).filter(|a| a.alive)?;
    zm.ents.get(&ent).map(|e| e.origin)
}

/// A client dvar for one player's client (its HUD, its music).
/// bo2zm: the living zombies near each player (`n x y z;...`: entity
/// number and feet, nearest first, at most 24 within 2000 units), for his
/// client's knife lunge (BO2 lunges at a zombie in reach and stabs; IW4's
/// lunge only looked for players) and the pad's aim assist (BO2 assists onto
/// zombies; IW4's only onto players).
fn publish_melee_targets(world: &mut World) {
    const REACH: f32 = 2000.0;
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for n in players {
        let Some(me) = frame(world).player(ClientId(n)).map(|ps| ps.origin) else {
            continue;
        };
        let list = {
            let zm = world.resource::<Zm>();
            let mut near: Vec<(f32, u32, [f32; 3])> = zm
                .actors
                .iter()
                .filter(|(_, a)| a.alive)
                .filter_map(|(k, _)| zm.ents.get(k).map(|e| (*k, e.origin)))
                .map(|(k, o)| {
                    let d: [f32; 3] = std::array::from_fn(|i| o[i] - me[i]);
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2], k, o)
                })
                .filter(|(d2, _, _)| *d2 <= REACH * REACH)
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            near.iter()
                .take(24)
                .map(|(_, k, o)| format!("{k} {:.0} {:.0} {:.0}", o[0], o[1], o[2]))
                .collect::<Vec<_>>()
                .join(";")
        };
        set_client_dvar(world, n, "bo2zm_zombies", &list);
    }
}

pub(crate) fn set_client_dvar(world: &mut World, client: u32, key: &str, value: &str) {
    let mut f = frame(world);
    if f.client_meta(crate::world::ClientId(client)).is_none() {
        return;
    }
    let dvars = &mut f
        .client_meta_mut(crate::world::ClientId(client))
        .client_dvars;
    match dvars.iter_mut().find(|(k, _)| k == key) {
        Some(row) => value.clone_into(&mut row.1),
        None => dvars.push((key.to_owned(), value.to_owned())),
    }
}

fn report(world: &mut World) {
    let msgs = with_vm(world, |vm, _| std::mem::take(&mut vm.messages)).unwrap_or_default();
    for m in msgs {
        diag::warn!(Sim, "bo2zm t6: {m}");
    }
    // IW4L_T6_THREADS=<seconds>: list every waiting thread that often.
    let every = std::env::var("IW4L_T6_THREADS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok());
    if let Some(every) = every {
        let now = world.resource::<Zm>().now_ms;
        let due = {
            let mut zm = world.resource_mut::<Zm>();
            if now >= zm.next_thread_dump {
                zm.next_thread_dump = now + every.max(1) * 1000;
                true
            } else {
                false
            }
        };
        if due {
            let lines = with_vm(world, |vm, _| vm.describe_threads()).unwrap_or_default();
            diag::info!(Sim, "bo2zm t6 threads at {}s: {}", now / 1000, lines.len());
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for l in lines {
                *counts.entry(l).or_default() += 1;
            }
            for (l, n) in counts {
                diag::info!(Sim, "bo2zm t6 thread x{n}: {l}");
            }
        }
    }
    // IW4L_T6_ROUND_JUMP=<n>[@<secs>]: 10 s in (or then), set
    // level.round_number to n (a test aid for things that only start in a
    // round-1 game, like the Nuketown transmission and blue eyes; or that
    // wait rounds, like TranZit's Avogadro in his cloud: `5@300`).
    static ROUND_JUMPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if let Some((n, at)) = std::env::var("IW4L_T6_ROUND_JUMP").ok().and_then(|v| {
        let (n, at) = v.split_once('@').unwrap_or((&v, "10"));
        Some((n.trim().parse::<i32>().ok()?, at.trim().parse::<f32>().ok()?))
    }) && world.resource::<Zm>().now_ms >= (at * 1000.0) as i64
        && !ROUND_JUMPED.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        with_vm(world, |vm, world| {
            let f = vm.intern("round_number");
            let level = vm.level;
            vm.set_field(world, level, f, Value::Int(n));
        });
        diag::info!(Sim, "bo2zm t6: round jumped to {n}");
    }
    // IW4L_T6_KILLS_ADD=<n>: 10 s in, n more kills on the books
    // (level.total_zombies_killed): the population sign counts them down.
    // IW4L_T6_DOOR_GRAB_AT=<n>: when the sign reads n, the player steps onto
    // the power-up behind the door (Nuketown's first song egg).
    static KILLS_ADDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if let Some(n) = std::env::var("IW4L_T6_KILLS_ADD").ok().and_then(|v| v.parse::<i32>().ok())
        && world.resource::<Zm>().now_ms >= 10_000
        && !KILLS_ADDED.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        with_vm(world, |vm, world| {
            let f = vm.intern("total_zombies_killed");
            let level = vm.level;
            let had = match vm.get_field(world, level, f) {
                Value::Int(i) => i,
                _ => 0,
            };
            vm.set_field(world, level, f, Value::Int(had + n));
        });
        diag::info!(Sim, "bo2zm t6: {n} kills added");
    }
    if let Some(want) = std::env::var("IW4L_T6_DOOR_GRAB_AT").ok().and_then(|v| v.parse::<i32>().ok()) {
        let at = with_vm(world, |vm, world| {
            let level = vm.level;
            let f = vm.intern("population_count");
            let pop = vm.get_field(world, level, f);
            let f = vm.intern("door_powerup");
            let pu = match vm.get_field(world, level, f) {
                Value::Object(o) if vm.alive(o) => Some(o),
                _ => None,
            };
            let f = vm.intern("origin");
            let origin = pu.map(|o| vm.get_field(world, o, f));
            (pop, origin)
        });
        if let Some((Value::Int(pop), Some(Value::Vec3(o)))) = at
            && pop == want
        {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                diag::info!(Sim, "bo2zm t6 test: sign at {pop}, grabbing the door power-up at {o:?}");
            }
            teleport_player(world, crate::world::ClientId(0), [o[0], o[1], o[2] - 40.0]);
        }
    }
    // IW4L_T6_SETLEVEL=field=int,...: set on level at 15 s (a test aid:
    // Die Rise's next_leaper_round=2 brings a Jumping Jack round early).
    if let Ok(sets) = std::env::var("IW4L_T6_SETLEVEL") {
        let now = world.resource::<Zm>().now_ms;
        if (15000..15000 + i64::from(crate::MATCH_TICK_MS)).contains(&now) {
            with_vm(world, |vm, _| {
                for (k, v) in sets.split(',').filter_map(|kv| kv.split_once('=')) {
                    let Ok(v) = v.parse::<i32>() else {
                        continue;
                    };
                    let f = vm.intern(k);
                    vm.set_raw_field(vm.level, f, Value::Int(v));
                    diag::info!(Sim, "bo2zm t6 setlevel {k} = {v}");
                }
            });
        }
    }
    // IW4L_T6_EVAL=level.a.b,...: those values every 5 s (debugging; roots level, anim, player).
    if let Ok(paths) = std::env::var("IW4L_T6_EVAL") {
        let now = world.resource::<Zm>().now_ms;
        if now % 5000 < i64::from(crate::MATCH_TICK_MS) {
            let lines = with_vm(world, |vm, world| {
                let mut out = Vec::new();
                for path in paths.split(',') {
                    let mut parts = path.split('.');
                    let mut v = match parts.next() {
                        Some("level") => Value::Object(vm.level),
                        Some("anim") => Value::Object(vm.anim),
                        Some("player") => match world.resource::<Zm>().players.get(&0) {
                            Some(p) => Value::Object(p.obj),
                            None => continue,
                        },
                        _ => continue,
                    };
                    for p in parts {
                        v = match &v {
                            Value::Object(o) => {
                                let f = vm.intern(p);
                                vm.get_field(world, *o, f)
                            }
                            Value::Array(a) => {
                                let k = match p.parse::<i32>() {
                                    Ok(i) => gsc_t6::Key::Int(i),
                                    Err(_) => gsc_t6::Key::Str(vm.intern(p)),
                                };
                                a.get(&k).unwrap_or_default()
                            }
                            _ => Value::Undefined,
                        };
                    }
                    let text = match &v {
                        Value::Array(a) => {
                            let snap = a.snapshot();
                            let items: Vec<String> = snap
                                .keys()
                                .map(|k| {
                                    let val = snap.get(&k).cloned().unwrap_or_default();
                                    format!("{}={}", vm.to_text(&k.value()), vm.to_text(&val))
                                })
                                .collect();
                            format!("[{}]", items.join(", "))
                        }
                        other => vm.to_text(other),
                    };
                    out.push(format!("{path} = {text}"));
                }
                out
            })
            .unwrap_or_default();
            for l in lines {
                diag::info!(Sim, "bo2zm t6 eval at {}s: {l}", now / 1000);
            }
        }
    }
    // IW4L_T6_ACTORS=<ms>: every actor's place, animscript and animation.
    let every = std::env::var("IW4L_T6_ACTORS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok());
    if let Some(every) = every {
        let zm = world.resource::<Zm>();
        let now = zm.now_ms;
        if now % every.max(50) < i64::from(crate::MATCH_TICK_MS) {
            for (n, a) in &zm.actors {
                let e = zm.ents.get(n);
                let o = e.map_or([0.0; 3], |e| e.origin);
                let anim = a.playing.as_ref().map_or("-".to_owned(), |p| {
                    format!("{}/{}:{}", p.state, p.substate, p.anim)
                });
                let next = a.path.get(a.path_i).copied().unwrap_or([0.0; 3]);
                let goal = a.goal.unwrap_or([0.0; 3]);
                diag::info!(
                    Sim,
                    "bo2zm t6 actor {n} at {}ms: ({:.0} {:.0} {:.0}) yaw {:.0} script {} anim {} hp {} alive {} path {}/{} next ({:.0} {:.0} {:.0}) goal ({:.0} {:.0} {:.0}) scripted {} traverse {} model {}",
                    now,
                    o[0],
                    o[1],
                    o[2],
                    e.map_or(0.0, |e| e.angles[1]),
                    a.script,
                    anim,
                    a.health,
                    a.alive,
                    a.path_i,
                    a.path.len(),
                    next[0],
                    next[1],
                    next[2],
                    goal[0],
                    goal[1],
                    goal[2],
                    a.scripted.is_some(),
                    a.traverse.is_some(),
                    e.map_or("", |e| e.model.as_str())
                );
            }
            // IW4L_T6_ACTOR_FIELDS=a,b,...: those script fields of each actor too.
            if let Ok(fields) = std::env::var("IW4L_T6_ACTOR_FIELDS") {
                let objs: Vec<(u32, ObjRef)> = zm
                    .actors
                    .iter()
                    .filter_map(|(n, a)| a.obj.map(|o| (*n, o)))
                    .collect();
                let lines = with_vm(world, |vm, world| {
                    objs.iter()
                        .map(|(n, o)| {
                            let vals: Vec<String> = fields
                                .split(',')
                                .map(|f| {
                                    let id = vm.intern(f);
                                    let v = vm.get_field(world, *o, id);
                                    format!("{f}={}", vm.to_text(&v))
                                })
                                .collect();
                            format!("{n}: {}", vals.join(" "))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
                for l in lines {
                    diag::info!(Sim, "bo2zm t6 actor fields at {now}ms {l}");
                }
            }
        }
    }
}

fn unbound(vm: &mut Vm<World>, world: &mut World, id: u32, _self: &Value, args: &[Value]) -> Value {
    let (name, method) = vm.program.builtins[id as usize];
    let n = format!("{}{}", if method { "." } else { "" }, vm.str(name));
    let mut zm = world.resource_mut::<Zm>();
    let count = zm.unbound.entry(n.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        let a: Vec<String> = args.iter().map(|a| vm.to_text(a)).collect();
        diag::warn!(Sim, "bo2zm t6: unbound builtin {n}({})", a.join(", "));
    }
    Value::Undefined
}

/// bo2zm M3: engine damage on a player (fall, explosion, ...) goes to
/// the BO2 scripts' CodeCallback_PlayerDamage; false when they don't run.
pub(crate) fn player_hit(world: &mut World, hit: &crate::script_player::Hit) -> bool {
    if !world.contains_resource::<T6Runtime>() {
        return false;
    }
    let hitloc = weapon_iw4::HITLOC_NAMES
        .get(usize::from(hit.hitloc))
        .copied()
        .unwrap_or("none");
    let weapon = weapon_text(world, hit.weapon);
    with_vm(world, |vm, world| {
        let attacker = hit
            .attacker
            .and_then(|a| {
                world
                    .resource::<Zm>()
                    .players
                    .get(&a.0)
                    .map(|p| Value::Object(p.obj))
            })
            .unwrap_or(Value::Undefined);
        natives_ai::player_damage(
            vm,
            world,
            hit.victim.0,
            attacker.clone(),
            attacker,
            hit.amount,
            hit.flags,
            hit.means,
            &weapon,
            hit.point,
            hit.dir,
            hitloc,
        );
    });
    true
}

/// A broken prop's piece (a mannequin's head, BO2's stage spawn model)
/// flies off its bone, tumbling, lands on the floor below and lies there
/// 20 s, as BO2's physics debris does.
pub(crate) fn throw_debris(
    world: &mut World,
    model: &str,
    from: [f32; 3],
    direction: [f32; 3],
    floor: f32,
) {
    if !world.contains_resource::<T6Runtime>() {
        return;
    }
    let now = world.resource::<Zm>().now_ms;
    let g = world.resource_mut::<Zm>().alloc_entnum();
    // Out that way, a little to one side (by its number), and up: a short
    // hop that lands within about 40 units.
    let (fx, fy) = (direction[0], direction[1]);
    let len = (fx * fx + fy * fy).sqrt().max(0.001);
    let side = (g % 7) as f32 / 3.0 - 1.0;
    let (hx, hy) = (fx / len - fy / len * side * 0.3, fy / len + fx / len * side * 0.3);
    let vel = [hx * 55.0, hy * 55.0, 160.0];
    // Flight time to the floor: z(t) = z0 + vz*t - 400*t^2.
    let drop = (from[2] - floor).max(0.0);
    let secs = ((vel[2] + (vel[2] * vel[2] + 1600.0 * drop).sqrt()) / 800.0).clamp(0.2, 3.0);
    let yaw = fy.atan2(fx).to_degrees();
    let obj = with_vm(world, |vm, _| vm.alloc_object(ObjKind::Entity(g)));
    let Some(obj) = obj else {
        return;
    };
    let spin = if g % 2 == 0 { 1.0 } else { -1.0 };
    let mut zm = world.resource_mut::<Zm>();
    zm.ents.insert(
        g,
        Ent {
            obj: Some(obj),
            classname: "script_model".into(),
            origin: from,
            angles: [0.0, yaw, 0.0],
            model: model.to_owned(),
            ..Default::default()
        },
    );
    zm.movers.gravity(g, from, vel, now, secs);
    zm.movers.rotate_to(g, [0.0, yaw, 0.0], [90.0 * spin, yaw + 200.0 * spin, 0.0], now, secs, 0.0, 0.0);
    zm.gibs.push((g, now + (secs * 1000.0) as i64 + 20000));
    let at = [from[0] + vel[0] * secs, from[1] + vel[1] * secs, floor];
    diag::info!(Sim, "bo2zm t6 debris {model} thrown from {from:?}, lands in {secs:.2}s at {at:?}");
}

/// bo2zm M3: a bullet (or a knife, a blast) on a script model that shows a
/// BO2 entity: an actor takes it through CodeCallback_ActorDamage with the
/// hit location of the bone it struck (the weapon's location multiplier
/// applied, as the engine does); false when the model isn't one of ours.
/// A map prop's breakable piece broke (`destructibledef` stage with a
/// break notify): the engine's `codecallback_destructibleevent("broken",
/// notify, attacker, weapon)` on the prop (the Nuketown mannequin heads
/// are "headless").
pub(crate) fn destructible_broken(
    world: &mut World,
    target: crate::ScriptModelId,
    notify: &str,
    attacker: Option<ClientId>,
    weapon: u32,
) {
    if !world.contains_resource::<T6Runtime>() {
        return;
    }
    let Some(n) = world
        .resource::<Zm>()
        .presences
        .by_ent
        .iter()
        .find(|(_, s)| s.id == target)
        .map(|(n, _)| *n)
    else {
        return;
    };
    let Some(obj) = world.resource::<Zm>().ents.get(&n).and_then(|e| e.obj) else {
        return;
    };
    let w = weapon_text(world, weapon);
    diag::info!(Sim, "bo2zm t6 destructible ent{n} broken {notify}");
    with_vm(world, |vm, world| {
        let att = attacker
            .and_then(|a| {
                world
                    .resource::<Zm>()
                    .players
                    .get(&a.0)
                    .map(|p| Value::Object(p.obj))
            })
            .unwrap_or(Value::Undefined);
        let args = vec![vm.string("broken"), vm.string(notify), att, vm.string(&w)];
        vm.spawn_named(
            world,
            "maps/mp/zombies/_zm",
            "codecallback_destructibleevent",
            Value::Object(obj),
            args,
        );
    });
}

pub(crate) fn entity_hit(world: &mut World, hit: &crate::script::EntityHit) -> bool {
    if !world.contains_resource::<T6Runtime>() {
        return false;
    }
    let Some(n) = world
        .resource::<Zm>()
        .presences
        .by_ent
        .iter()
        .find(|(_, s)| s.id == hit.target)
        .map(|(n, _)| *n)
    else {
        return false;
    };
    let alive = world
        .resource::<Zm>()
        .actors
        .get(&n)
        .is_some_and(|a| a.alive);
    // BO2's G_DamageActor takes no damage below 1 (a Paralyzer shot: its
    // script does the harm), so no callback and no "damage".
    if !alive || hit.amount <= 0 {
        return true;
    }
    // The struck bone's hit location.
    let part = hit.bone.and_then(|bone| {
        let f = frame(world);
        f.entity_collision_capabilities()
            .iter()
            .find(|row| row.owner.script_model() == Some(hit.target))
            .and_then(|row| row.dobj.as_ref())
            .and_then(|d| d.current_collision.as_ref())
            .and_then(|c| c.bones.iter().find(|b| usize::from(b.bone) == bone))
            .map(|b| b.part_classification)
    });
    // A blast strikes no bone: BO2 calls its location "none" and its point
    // is the blast's centre, so the scripts tear off the limb nearest the
    // blast (a grenade at the feet takes the legs: a crawler).
    let blast = part.is_none() && hit.flags & 1 != 0;
    let part = part.unwrap_or(4);
    let hitloc = if blast {
        "none"
    } else {
        T6_HITLOC_NAMES
            .get(usize::from(part))
            .copied()
            .unwrap_or("none")
    };
    let point = if blast {
        std::array::from_fn(|i| hit.point[i] - hit.dir[i])
    } else {
        hit.point
    };
    // The weapon's location multiplier (not for a knife).
    let melee = hit.means == "MOD_MELEE";
    let scale = if melee {
        1.0
    } else {
        let f = frame(world);
        f.combat_facts_for(hit.weapon)
            .map_or(1.0, |facts| facts.location_scale(part))
    };
    let amount = ((hit.amount as f32) * scale).round().max(1.0) as i32;
    // A knife hit names the knife the scripts gave him, and cuts as hard as
    // that knife: BO2 stabs with the melee weapon he holds (the Bowie Knife
    // harder than the knife). A gun that is its own knife keeps its own.
    let held_knife = if melee {
        hit.attacker.and_then(|a| {
            world
                .resource::<Zm>()
                .players
                .get(&a.0)
                .and_then(|p| p.melee_weapon.clone())
        })
    } else {
        None
    };
    let amount = match held_knife.as_deref().map(|k| weapon(world, k)) {
        Some(Ok(knife)) if knife != 0 => {
            let f = frame(world);
            let own = f.weapon_melee_damage(knife);
            match f.combat_facts_for(hit.weapon) {
                Some(gun) if gun.knife_model != 0 && own > 0 => {
                    (amount - gun.melee_damage + own).max(1)
                }
                _ => amount,
            }
        }
        _ => amount,
    };
    let own_melee = melee
        && frame(world)
            .combat_facts_for(hit.weapon)
            .is_some_and(|f| f.knife_model == 0);
    let weapon = if melee && !own_melee {
        held_knife.unwrap_or_else(|| "knife_zm".to_owned())
    } else {
        weapon_text(world, hit.weapon)
    };
    if melee || std::env::var("IW4L_T6_HITLOG").is_ok() {
        diag::info!(
            Sim,
            "bo2zm t6 hit ent{n}: {} {weapon} {hitloc} x{scale:.2} = {amount}",
            hit.means
        );
    }
    with_vm(world, |vm, world| {
        let attacker = hit
            .attacker
            .and_then(|a| {
                world
                    .resource::<Zm>()
                    .players
                    .get(&a.0)
                    .map(|p| Value::Object(p.obj))
            })
            .unwrap_or(Value::Undefined);
        actors::damage(
            vm,
            world,
            n,
            attacker.clone(),
            attacker,
            amount,
            hit.flags,
            hit.means,
            &weapon,
            point,
            hit.dir,
            hitloc,
        );
    });
    true
}

/// A script string as the player reads it: a localized key (`&"KEY"` or a
/// bare `KEY`) becomes its English text with `&&1`.. filled from `args`
/// (themselves localized when they are keys); other values print as is.
pub(crate) fn localize(vm: &Vm<World>, world: &World, v: &Value, args: &[Value]) -> String {
    let zm = world.resource::<Zm>();
    let raw = vm.to_text(v);
    let key = raw.trim_start_matches('&');
    let mut out = match zm.strings.get(&key.to_ascii_uppercase()) {
        Some(t) => t.clone(),
        None if matches!(v, Value::IStr(_)) => key.to_owned(),
        None => raw.clone(),
    };
    for (i, a) in args.iter().enumerate() {
        let text = match a {
            Value::IStr(_) => {
                let k = vm.to_text(a);
                zm.strings
                    .get(&k.trim_start_matches('&').to_ascii_uppercase())
                    .cloned()
                    .unwrap_or(k)
            }
            other => vm.to_text(other),
        };
        out = out.replace(&format!("&&{}", i + 1), &text);
    }
    out
}

/// The arguments of a localized string with each player swapped for his
/// name, as BO2 prints one (Scr_ConstructMessageString: a player entity
/// becomes its display name), e.g. "&&1 is reviving you".
pub(crate) fn name_players(vm: &mut Vm<World>, world: &mut World, args: &[Value]) -> Vec<Value> {
    args.iter()
        .map(|a| match client(vm, world, a) {
            Ok(id) => {
                let name = frame(world)
                    .client_meta(id)
                    .map(|m| {
                        let end = m.name.iter().position(|&b| b == 0).unwrap_or(m.name.len());
                        String::from_utf8_lossy(&m.name[..end]).into_owned()
                    })
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "Player".to_owned());
                vm.string(&name)
            }
            Err(_) => a.clone(),
        })
        .collect()
}

/// The grenades and mines a player carries, as the HUD shows them (BO2's
/// offhand icons, one per grenade): `name:count` by `;`, lethal first.
fn offhand_counts(world: &mut World, c: u32) -> String {
    const OFFHANDS: [&str; 4] = [
        "frag_grenade_zm",
        "sticky_grenade_zm",
        "cymbal_monkey_zm",
        "claymore_zm",
    ];
    let f = frame(world);
    let id = crate::world::ClientId(c);
    let held = crate::script_player::weapons(&f, id, crate::script_player::WeaponList::All);
    let mut out: Vec<String> = Vec::new();
    for want in OFFHANDS {
        for &w in &held {
            let name = crate::script_player::weapon_name(&f, w);
            if name.trim_end_matches("_mp") == want {
                let (clip, stock) = (
                    crate::script_player::ammo_clip(&f, id, w),
                    crate::script_player::ammo_stock(&f, id, w),
                );
                if std::env::var_os("IW4L_T6_HINTLOG").is_some() {
                    diag::info!(Sim, "bo2zm t6 offhand {want}: clip {clip} stock {stock}");
                }
                out.push(format!("{want}:{}", clip + stock));
            }
        }
    }
    out.join(";")
}

/// bo2zm: the held gun's heat (`bo2zm_heat`, `percent:overheated`; empty for a gun that does
/// not overheat): BO2's HUD shows the Paralyzer's heat as a percentage where the ammo goes.
/// Sent on every change, apart from the other HUD values.
fn publish_heat(world: &mut World, c: u32) {
    let id = crate::world::ClientId(c);
    let mut f = frame(world);
    if f.client_meta(id).is_none() {
        return;
    }
    let heat = crate::t6_gun::script_read(&f, id, |ps, facts| {
        let h = ps.held.iter().find(|h| h.weapon != 0 && h.weapon == ps.weapon)?;
        facts.get(h.weapon).fire.overheat.then(|| {
            format!("{}:{}", h.heat_percent.round() as i32, u8::from(h.over_heating))
        })
    })
    .flatten()
    .unwrap_or_default();
    let dvars = &mut f.client_meta_mut(id).client_dvars;
    match dvars.iter_mut().find(|(k, _)| k == "bo2zm_heat") {
        Some(row) if row.1 == heat => {}
        Some(row) => row.1 = heat,
        None => dvars.push(("bo2zm_heat".to_owned(), heat)),
    }
}

/// bo2zm: the scoreboard BO2's menus show (Tab, and the game over): every
/// player as `clientnum:name:score:kills:downs:revives:headshots`, by `;`
/// (the columns Zombies' `setscoreboardcolumns` names, in its order).
fn scoreboard_rows(world: &mut World) -> String {
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let mut rows = Vec::new();
    for c in clients {
        let (name, score, kills) = {
            let f = frame(world);
            let Some(m) = f.client_meta(crate::world::ClientId(c)) else {
                continue;
            };
            let end = m.name.iter().position(|&b| b == 0).unwrap_or(m.name.len());
            let name = String::from_utf8_lossy(&m.name[..end]).replace([':', ';'], " ");
            (
                if name.is_empty() {
                    "Player".to_owned()
                } else {
                    name
                },
                m.score,
                m.kills,
            )
        };
        let zm = world.resource::<Zm>();
        let stat = |k: &str| {
            zm.players
                .get(&c)
                .and_then(|p| p.stats.get(k).copied())
                .unwrap_or(0)
        };
        rows.push(format!(
            "{c}:{name}:{score}:{kills}:{}:{}:{}",
            stat("downs"),
            stat("revives"),
            stat("headshots")
        ));
    }
    rows.join(";")
}

/// Each player's HUD values the scripts own go out as client dvars: the
/// round (`bo2zm_round`), the hint of the use trigger he faces
/// (`bo2zm_hint`), his grenades (`bo2zm_offhand`) and the scoreboard
/// (`bo2zm_scoreboard`); sent when they change.
fn publish_hud(world: &mut World) {
    let round = with_vm(world, |vm, _| {
        let f = vm.intern("round_number");
        vm.raw_field(vm.level, f).as_int().unwrap_or(0)
    })
    .unwrap_or(0)
    .to_string();
    // The game type (the HUD's Turned and Grief checks read it).
    let gametype = with_vm(world, |vm, _| vm.dvars.get("g_gametype").cloned())
        .flatten()
        .unwrap_or_default();
    let board = scoreboard_rows(world);
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        publish_heat(world, c);
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&c)
            .cloned()
            .unwrap_or_default();
        let offhand = offhand_counts(world, c);
        let slots = world
            .resource::<Zm>()
            .action_slots
            .get(&c)
            .map(|v| {
                v.iter()
                    .map(|(n, w)| format!("{n}:{w}"))
                    .collect::<Vec<_>>()
                    .join(";")
            })
            .unwrap_or_default();
        let sent = world.resource::<Zm>().hud_sent.get(&c).cloned();
        if sent.as_ref().is_some_and(|(r, h, o, a, b)| {
            *r == round && *h == hint && *o == offhand && *a == slots && *b == board
        }) {
            continue;
        }
        let mut f = frame(world);
        if f.client_meta(crate::world::ClientId(c)).is_none() {
            continue;
        }
        if std::env::var_os("IW4L_T6_HINTLOG").is_some() {
            diag::info!(
                Sim,
                "bo2zm t6 hud out for {c}: round {round:?} hint {hint:?} offhand {offhand:?} scoreboard {board:?}"
            );
        }
        let dvars = &mut f.client_meta_mut(crate::world::ClientId(c)).client_dvars;
        for (name, value) in [
            ("bo2zm_round", round.clone()),
            ("bo2zm_hint", hint.clone()),
            ("bo2zm_offhand", offhand.clone()),
            ("bo2zm_actionslots", slots.clone()),
            ("bo2zm_scoreboard", board.clone()),
            ("bo2zm_gametype", gametype.clone()),
        ] {
            match dvars.iter_mut().find(|(k, _)| k == name) {
                Some(row) => row.1 = value,
                None => dvars.push((name.to_owned(), value)),
            }
        }
        world
            .resource_mut::<Zm>()
            .hud_sent
            .insert(c, (round.clone(), hint, offhand, slots, board.clone()));
    }
}

/// Put a player somewhere from the server (a script's `setorigin`, a
/// link): the teleport bit flips so his client jumps there too.
pub(crate) fn teleport_player(world: &mut World, client: crate::world::ClientId, origin: [f32; 3]) {
    let mut f = frame(world);
    f.set_origin(client, origin);
    if let Some(ps) = f.player_mut(client) {
        ps.e_flags ^= playerstate_iw4::eflags::TELEPORT;
    }
}

/// Turn a player's view from the server (`setplayerangles`): the delta
/// under his last command's angles, so his client's view turns too.
pub(crate) fn set_player_view(world: &mut World, client: crate::world::ClientId, view: [f32; 3]) {
    let mut f = frame(world);
    let cmd = f
        .old_cmd_angles_mut()
        .iter()
        .find(|(c, _)| *c == client)
        .map(|(_, a)| *a);
    if let Some(ps) = f.player_mut(client) {
        if let Some(cmd) = cmd {
            ps.delta_angles =
                std::array::from_fn(|i| view[i] - (cmd[i] as u16) as f32 * (360.0 / 65536.0));
        }
        ps.viewangles = view;
    }
}

/// bo2zm M3: the living zombies a blast at `origin` reaches: (their shown
/// model, their middle, distance), nearest first. As BO2's engine does, the
/// distance is to the nearest point of each one's box, not its middle: the
/// Sliquifier's bolt (radius 25) goes off on the floor at a zombie's feet.
pub(crate) fn radius_targets(
    world: &mut World,
    origin: [f32; 3],
    radius: f32,
) -> Vec<(crate::ScriptModelId, [f32; 3], f32)> {
    if !world.contains_resource::<T6Runtime>() {
        return Vec::new();
    }
    let zm = world.resource::<Zm>();
    let mut out: Vec<(crate::ScriptModelId, [f32; 3], f32)> = zm
        .actors
        .iter()
        .filter(|(_, a)| a.alive)
        .filter_map(|(n, a)| {
            let e = zm.ents.get(n)?;
            let id = zm.presences.by_ent.get(n)?.id;
            let o = e.origin;
            let mid = [o[0], o[1], o[2] + a.height * 0.5];
            let lo = [o[0] - a.radius, o[1] - a.radius, o[2]];
            let hi = [o[0] + a.radius, o[1] + a.radius, o[2] + a.height];
            let near: [f32; 3] = std::array::from_fn(|i| origin[i].clamp(lo[i], hi[i]));
            let d = gsc_t6::math::length(gsc_t6::math::sub(near, origin));
            (d < radius).then_some((id, mid, d))
        })
        .collect();
    out.sort_by(|a, b| a.2.total_cmp(&b.2));
    out
}

// ---- helpers for natives -------------------------------------------------

pub(crate) fn arg(a: &[Value], i: usize) -> &Value {
    a.get(i).unwrap_or(&Value::Undefined)
}

pub(crate) fn num(a: &[Value], i: usize) -> Result<f32, String> {
    arg(a, i).as_float().ok_or_else(|| {
        format!(
            "argument {} is {}, not a number",
            i + 1,
            arg(a, i).type_name()
        )
    })
}

pub(crate) fn int(a: &[Value], i: usize) -> Result<i32, String> {
    arg(a, i).as_int().ok_or_else(|| {
        format!(
            "argument {} is {}, not a number",
            i + 1,
            arg(a, i).type_name()
        )
    })
}

pub(crate) fn vec3(a: &[Value], i: usize) -> Result<[f32; 3], String> {
    arg(a, i).as_vec3().ok_or_else(|| {
        format!(
            "argument {} is {}, not a vector",
            i + 1,
            arg(a, i).type_name()
        )
    })
}

pub(crate) fn text(vm: &Vm<World>, a: &[Value], i: usize) -> String {
    match arg(a, i) {
        Value::Undefined => String::new(),
        v => vm.to_text(v),
    }
}

pub(crate) fn flag(a: &[Value], i: usize, default: bool) -> bool {
    match arg(a, i) {
        Value::Undefined => default,
        v => gsc_t6::truthy(v),
    }
}

pub(crate) fn list(vals: Vec<Value>) -> Value {
    let mut a = gsc_t6::Array::new();
    for v in vals {
        a.push(v);
    }
    Value::array(a)
}

/// The entity number of an entity object.
pub(crate) fn entnum(vm: &Vm<World>, v: &Value) -> Option<u32> {
    match v {
        Value::Object(o) => match vm.kind(*o)? {
            ObjKind::Entity(n) => Some(n),
            _ => None,
        },
        _ => None,
    }
}

/// The client of a player object.
pub(crate) fn client(vm: &Vm<World>, world: &World, v: &Value) -> Result<ClientId, String> {
    let n = entnum(vm, v).ok_or("not an entity")?;
    if world.resource::<Zm>().players.contains_key(&n) {
        Ok(ClientId(n))
    } else {
        Err("not a player".into())
    }
}

pub(crate) fn is_player(vm: &Vm<World>, world: &World, v: &Value) -> bool {
    entnum(vm, v).is_some_and(|n| world.resource::<Zm>().players.contains_key(&n))
}

pub(crate) fn tick(world: &World) -> crate::Tick {
    world.resource::<crate::step::StepRequest>().tick
}

pub(crate) fn frame(world: &mut World) -> FrameWorld<'_> {
    FrameWorld::from_world(world)
}

/// An entity's origin (players from the sim).
pub(crate) fn origin_of(vm: &Vm<World>, world: &mut World, v: &Value) -> Option<[f32; 3]> {
    let n = entnum(vm, v)?;
    if world.resource::<Zm>().players.contains_key(&n) {
        return frame(world).player(ClientId(n)).map(|ps| ps.origin);
    }
    world.resource::<Zm>().ents.get(&n).map(|e| e.origin)
}

/// Weapon index by script name (`m1911_zm`; IW4L registers `_mp` names).
pub(crate) fn weapon(world: &mut World, name: &str) -> Result<u32, String> {
    if name == "none" || name.is_empty() {
        return Ok(0);
    }
    let f = frame(world);
    let by = |n: &str| {
        crate::script_player::weapon_named(&f, &format!("{n}_mp"))
            .or_else(|_| crate::script_player::weapon_named(&f, n))
    };
    // `base+attachment` (Pack-a-Punch gives `tar21_upgraded_zm+reflex`):
    // attachments are not weapons of their own here yet, so the base weapon.
    by(name).or_else(|e| match name.split_once('+') {
        Some((base, _)) => by(base),
        None => Err(e),
    })
}

/// A weapon's world model as everyone else sees it (on a wall, in the
/// box): the gun, the parts Black Ops II hangs on it (magazine, scope,
/// rocket: its `attachWorldModel`s) and the tags it hides, lower case.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WeaponLook {
    pub model: String,
    pub parts: Vec<(String, String, xmodel_runtime::TagOffset)>,
    pub hide: Vec<String>,
}

pub(crate) fn weapon_look(world: &mut World, w: u32) -> Option<WeaponLook> {
    let f = frame(world);
    let (model, hide) = f.weapon_world_model(w)?;
    if model.is_empty() {
        return None;
    }
    Some(WeaponLook {
        model: model.to_owned(),
        // Only parts the map loaded: one it lacks would lose the whole gun.
        parts: f
            .weapon_world_parts(w)
            .iter()
            .filter(|p| f.model_capability(&p.model).flatten().is_some())
            .map(|p| {
                let at = xmodel_runtime::TagOffset::new(p.offset, p.rotation);
                (p.model.clone(), p.tag.clone(), at)
            })
            .collect(),
        hide: hide.iter().map(|t| t.to_ascii_lowercase()).collect(),
    })
}

/// A weapon's script name (`_mp` dropped).
pub(crate) fn weapon_text(world: &mut World, w: u32) -> String {
    let f = frame(world);
    let n = crate::script_player::weapon_name(&f, w);
    n.strip_suffix("_mp").map_or(n.clone(), str::to_owned)
}

pub(crate) fn names(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(", ")
}
