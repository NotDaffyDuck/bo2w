//! bo2zm M3: Black Ops II Zombies' own scripts and the engine side they call.
//!
//! A BO2 map hands its zones' compiled scripts to [`install`]; the `gsc_t6`
//! VM runs them, here, beside the old engine's GSC runtime (which then only keeps its
//! stub start-up: dvar `bo2zm_t6` set makes its connect callback step aside).
//! The level start follows the game's order: map entities and structs, the
//! game type's `main`, the map's `main`, `codecallback_startgametype`; then
//! every authority frame connects players and runs the scheduler.
//!
//! Engine state the scripts see lives in [`Zm`]: entities (map and spawned),
//! players, movers, tables. Natives are `fn(&mut Vm<World>, &mut World, self,
//! args)`. Not part of the upstream engine.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Hooks, ObjKind, ObjRef, Program, ScriptObject, Strings, ScriptValue, Vm};

use pairs::KeepByValue;
use crate::step_view::StepWorldView;
use crate::sim_parts::ClientId;

mod actors;
mod asd;
mod audio_triggers;
mod autoplay;
mod autoplay_prison;
mod bots;
mod brushes;
mod buried_egg;
mod cheats; // bo2zm: the solo cheat menu (Num 1)
mod clientfields;
mod closest;
// bo2mp: the console's `bot add` and `bot hold`.
pub(crate) mod console_bots;
// bo2mp: the map's damage triggers (Encore's drums, the billboards).
pub(crate) mod damage_triggers;
mod darts;
mod declassified;
pub(crate) mod destructible;
mod engine_sounds;
mod fields;
pub(crate) mod flashbang;
mod flight;
mod blackhole_fx;
mod freezegun_fx;
mod foreign_weapons;
mod fxanims;
mod glass;
mod glass_test;
mod globallogic;
mod grenades;
// bo2mp: the stuck grenade (a grenade stuck to a player or wall).
pub(crate) mod grenade_stuck;
// bo2mp gun brain window 13: the gun builtins and notifies on the T6 gun state.
mod gun_script;
mod hud;
mod items;
mod killstreaks;
pub(crate) mod knife;
mod loadout;
pub(crate) mod melee;
mod movers;
pub(crate) mod mp_portals;
pub(crate) mod mp_specials;
pub mod menu;
mod mp;
mod mp_fxanims;
mod natives_ai;
mod natives_ent;
mod highrise;
mod natives_fx;
mod natives_game;
mod natives_mp;
mod natives_player;
mod nav;
mod goals;
mod offhand_visible;
mod pairs;
// bo2mp: the claymore / tactical insertion plant probe the gun asks before a plant.
pub(crate) mod plant;
pub(crate) mod riotshield;
mod playeranim;
mod players;
mod presence;
mod semtex_light;
pub(crate) mod sensor_grenade;
mod spawnsystem;
pub mod stats;
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
pub(crate) mod charge_shot;
pub(crate) mod jetgun;
pub(crate) mod mc_specials;
mod limits;
mod throwables;
mod transit;
mod triggers;
mod turrets;
pub(crate) use closest::Closest;
pub(crate) use grenades::note_detonation;
pub(crate) use triggers::{DamageKind, damage_triggers};
mod use_test;
mod vehicles;
mod wallbuys;
mod zbarrier;

pub(crate) use items::collect_item_pickups;
pub(crate) use playeranim::note_body_anims;
pub(crate) use killstreaks::missile_passes;

/// A string table (`mp/zombiemode.csv`, ...).
#[derive(Clone, Debug, Default)]
pub struct T6Table {
    pub col_total: usize, pub row_total: usize,
    /// Row after row, `columns` cells each.
    pub entries: Vec<String>,
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
    /// The map compiler's baked node sight and smooth tables.
    pub path_tables: T6PathTables,
    /// Animations the server times and moves actors by.
    pub anims: Vec<T6Anim>,
    /// Animation state definitions: (name, text).
    pub animstatedefs: Vec<(String, String)>,
    /// Skeletons and hit boxes of the models entities can wear, by name.
    pub models: Vec<(String, Arc<xmodel_runtime::KeptModelAbility>)>,
    /// Clips actors are posed with for hits, by (lowercase) name.
    pub clips: Vec<(String, Arc<xmodel_runtime::SkeletalClip>)>,
    /// The English strings (`ZOMBIE_WEAPON_M14` -> its text).
    pub strings: std::collections::HashMap<String, String>,
    /// Every sound alias the banks hold (lower case).
    pub sound_aliases: BTreeSet<String>,
    /// bo2mp: the game type's settings (`getgametypesetting`), lower-case
    /// name -> value: `mp/gamesettings_default.cfg` then the game type's own.
    pub gamesettings: BTreeMap<String, String>,
    /// bo2mp: the lobby's bots (friends, enemies, difficulty 0-3).
    pub bots: Option<(u32, u32, u32)>,
    /// bo2mp: sound alias lengths in ms (lower-case names).
    pub sound_lengths: std::collections::HashMap<String, u32>,
    /// bo2mp: retrievable weapons (getretrievableweapons).
    pub retrievable_weapons: Vec<String>,
    /// bo2mp: each vehicle's turret and gunner weapons, and how it drives.
    pub vehicles: Vec<T6Vehicle>,
    /// bo2mp: BO2's third-person player animation script, parsed.
    pub playeranim: Option<Arc<crate::body_anim_script::T6PlayerAnims>>,
    /// bo2mp: the destructibles the map's entities name (`destructibledef`).
    pub destructibles: Vec<Arc<xmodel_runtime::BreakableModelDef>>,
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

/// bo2mp: a vehicle def from his zones: its guns, how it drives and how
/// its driver sees it.
#[derive(Clone, Debug, Default)]
pub struct T6Vehicle {
    pub name: String,
    pub turret: String,
    pub gunners: Vec<String>,
    pub drive: T6VehicleDrive,
}

/// bo2mp: a vehicle's driving and camera values (`VehicleDef`); speeds in
/// units a second, angles in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6VehicleDrive {
    /// 0: his view from the vehicle's `tag_player`; else third person.
    pub camera_mode: i32,
    pub top_velocity: f32,
    pub max_speed_vertical: f32,
    pub speed_up_rate: f32,
    pub accel_vertical: f32,
    /// Reversing, a share of `top_velocity`.
    pub reverse_scale: f32,
    /// The third-person camera: how far back, how high it looks from, how
    /// far up and down it may look.
    pub camera_range: f32,
    pub camera_height: f32,
    pub camera_pitch: [f32; 2],
    /// 0: his own field of view.
    pub camera_fov: f32,
    /// How far back the camera falls at top speed, and over how long.
    pub camera_spring: [f32; 2],
    /// How far he may look up and down from its gun (first person).
    pub turret_pitch: [f32; 2],
    /// `thirdPersonDriver`: the driver's seat views from outside.
    pub third_person_driver: i32,
    /// The commands of `moveUpButtonName`, `moveDownButtonName`,
    /// `switchSeatButtonName`, `attackButtonName`, `attackSecondaryButtonName`
    /// in BO2's default controller binds ("" for none).
    pub buttons: [&'static str; 5],
    /// The commands of `boostButtonName`, `handBrakeButtonName`.
    pub boost_button: &'static str,
    pub hand_brake_button: &'static str,
    /// Boost: speed_up_rate scale, seconds of it a vehicle, top speed scale.
    pub boost_accel_mult: f32,
    pub boost_duration: f32,
    pub boost_speed_increase: f32,
    /// Steering (radians, radians a second), tyre grip (forward, side,
    /// braking, hand brake) and the speed its hand brake comes on by itself.
    pub steer_angle_max: f32,
    pub steer_speed: f32,
    pub tire_fric: [f32; 4],
    pub auto_hand_brake_min_speed: f32,
    /// It peels out on full throttle below this speed; a wheel sliding
    /// faster than `skid_speed_min` skids.
    pub peel_out_max_speed: f32,
    pub skid_speed_min: f32,
    /// What hurts it (BO2's `VEH_IsImmune`): `remote_operated` (fire burns a
    /// driven one), then whether bullets, armor-piercing bullets, grenades,
    /// projectiles, projectile splash and heavy explosives do (0: none),
    /// and `no_directional_damage` (no harder blast from underneath).
    pub remote_operated: i32,
    pub bullet_damage: i32,
    pub armor_piercing_damage: i32,
    pub grenade_damage: i32,
    pub projectile_damage: i32,
    pub projectile_splash_damage: i32,
    pub heavy_explosive_damage: i32,
    pub no_directional_damage: i32,
    /// Its kind (`VehicleDef::type`): 2 a tank (the AGR: its driver's gun
    /// fires from his view), 6 a helicopter.
    pub vehicle_type: i32,
    /// How a helicopter (the Dragonfire) flies.
    pub heli: T6HeliDrive,
    /// `collisionDamage`, `collisionSpeed`: a moving vehicle's front hits
    /// whoever it touches for up to this much, full at this many units a
    /// second above 88 (5 mph).
    pub collision_damage: f32,
    pub collision_speed: f32,
}

/// How a helicopter flies (BO2's BG_VehicleHelicopterMove), from its
/// VehicleDef; its top speeds and accelerations are [`T6VehicleDrive`]'s.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6HeliDrive {
    /// `rotRate`, `rotAccel`: the most degrees a second it turns, and how
    /// fast it gets to that.
    pub rot_rate: f32,
    pub rot_accel: f32,
    /// `maxBodyPitch`, `maxBodyRoll`: how far it leans at full tilt.
    pub max_body_pitch: f32,
    pub max_body_roll: f32,
    /// Its tilt from speeding up, slowing down, its speed, and how fast the
    /// tilt follows: forward, then sideways.
    pub tilt_from_accel: [f32; 2],
    pub tilt_from_decel: [f32; 2],
    pub tilt_from_vel: [f32; 2],
    pub tilt_speed: [f32; 2],
    /// `useCollmap`: it moves as its model's collision brush.
    pub use_collmap: i32,
    /// How fast it stops: forward, sideways, up.
    pub decel: [f32; 3],
    /// Tilt from the stick, from turning at speed (full at
    /// `vel_at_max_tilt` of its top speed), and how slowly the tilt answers.
    pub tilt_from_axes: f32,
    pub tilt_from_fwd_yaw: f32,
    pub vel_at_max_tilt: f32,
    pub tilt_momentum: f32,
    /// `vehHelicopterAccelTwardsView`: forward climbs and dives with the
    /// view's pitch (0 for the Dragonfire).
    pub accel_toward_view: i32,
    /// Holding its height over the ground: on, the most it follows a step
    /// a frame, the least height, how far down it looks for the ground,
    /// and how much room it keeps under a ceiling.
    pub maintain_height: i32,
    pub height_limit: f32,
    pub height_min: f32,
    pub height_max: f32,
    pub ceiling_min: f32,
}

/// An AI path node and its links (node, distance, negotiation).
#[derive(Clone, Debug, Default)]
pub struct T6PathNode {
    pub ty: u32,
    pub spawnflags: u32,
    pub target_key: String, pub target: String,
    pub script_note: String, pub script_link: String,
    pub animscript: String,
    pub origin: [f32; 3],
    pub angle: f32, pub radius: f32,
    pub links: Vec<(u16, f32, bool)>,
}

/// The map compiler's baked path tables: which node pairs see each other
/// (`vis`, bit `node_count * i + j` for i < j) and which a straight walk
/// joins (`smooth`, bit `original_count * i + j`).
#[derive(Clone, Debug, Default)]
pub struct T6PathTables {
    pub node_count: u32,
    pub original_count: u32,
    pub vis: Vec<u8>,
    pub smooth: Vec<u8>,
}

/// An animation's timing, root motion and notetracks.
#[derive(Clone, Debug, Default)]
pub struct T6Anim {
    pub name: String,
    pub frame_total: u16,
    pub fps: f32, pub looping: bool,
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
    pub class_tag: String,
    pub origin: [f32; 3],
    pub rot: [f32; 3],
    pub mesh: String,
    pub unseen: bool,
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
    pub birth_orientation: Option<[f32; 3]>,
    /// Trigger radius/height (`trigger_radius*`) or brush extents.
    pub reach: f32,
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
    pub repeating_audio: Option<String>,
    pub can_damage: bool,
    /// Players a trigger (or model) is hidden from.
    pub invisible_to: BTreeSet<u32>,
    pub invisible_to_all: bool,
    pub trigger_off: bool,
    /// bo2mp: the engine's dropped item (a gun, a scavenger bag) this is.
    pub item: Option<i32>,
    /// bo2mp: the destructible definition (`destructibledef`) a map entity
    /// breaks by, lower case.
    pub destructible: Option<String>,
    /// bo2mp: `allowtacticalinsertion(0)`: no tactical insertion while a
    /// player's box meets this one's (BO2's entity flag 0x400; a Hardpoint's
    /// live hill).
    pub no_tac_insert: bool,
    /// codmp: a Black Ops II destruct prop's broken model (the clipmap's
    /// `destroyedxModel`), shown once its destructible dies.
    pub dyn_destroyed: Option<String>,
    /// A `trigger_hurt`'s clock: the level time (ms) it may hurt again
    /// (BO2's trigger.timestamp, one for everyone), and whether it went off
    /// after its one hurt (spawnflags 32).
    pub hurt_at: i64,
    pub hurt_off: bool,
    /// bo2mp: `setenemymodel`: the model the other side sees (empty: the
    /// same one for everyone).
    pub enemy_model: String,
    /// bo2mp: its side and owner (BO2's entity faction, from `setteam`,
    /// `setvehicleteam`, `setowner` and its spawn's owner): the team as a
    /// client's team number (`goals::client_team`), the owning player.
    pub faction_team: i32,
    pub faction_owner: Option<u32>,
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
pub(crate) struct Survivor {
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
    /// His weapon and weapon state last tick (weapon events).
    // TODO(rw-zm): Zombies dropped these (its gun brain reads the T6 gun state); kept while
    // players::weapon_events and natives_player still name them.
    pub last_weapon: u32,
    pub last_wstate: i32,
    pub last_clip: i32,
    /// His Jet Gun's spin and heat.
    pub jet: jetgun::JetState,
    /// `forcegrenadethrow`: let go of his grenade for him (t6::offhand).
    pub force_throw: bool,
    /// Whether he was diving / sprinting last tick (`dtp_start` /
    /// `dtp_end`, `sprint_begin` / `sprint_end`).
    pub last_dive: bool,
    pub last_sprint: bool,
    /// The end time of the last climb told to scripts (`mantle_start`).
    pub last_mantle_end: i32,
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
    /// bo2mp: a bot (`addtestclient`), and while its client has not come
    /// yet, when it was made.
    pub test_client: bool,
    pub pending_since: Option<i64>,
    /// bo2mp: the shellshock he is under (its name), when it ends (level
    /// ms) and the speed factor it put on `run_speed_modifier`
    /// (the shock file's `bg_shock_movement`; 1 when none).
    pub shock: Option<(String, i32)>,
    pub shock_factor: f32,
    /// bo2mp: the name each of his guns was given by (`an94_mp+fmj+reflex`)
    /// where the catalog holds it under one attachment's (`an94_mp+reflex`):
    /// what the scripts read back, so their attachment checks see them all.
    pub given_names: BTreeMap<u32, String>,
    /// His body model (`setmodel`; not drawn in third person yet): Mob's
    /// Afterlife copies it onto the body he leaves behind.
    pub mesh: String,
    /// `startpoisoning` / `stoppoisoning`: BO2's player-state poisoned flag
    /// (TranZit's Denizen grab and the fog's gas; the client draws it).
    pub poisoned: bool,
    /// bo2mp: `allowspectateteam`'s bits (BO2's `sess.noSpectate`): a set
    /// bit is a team he may not watch (1 << the team's number), 0x400
    /// freelook, 0x800 the local players.
    pub no_spectate: u32,
}

impl Survivor {
    pub(crate) fn new(obj: ObjRef) -> Self {
        Self {
            obj,
            begun: false,
            sessionstate: "spectator".to_owned(),
            perks: Default::default(),
            linked: None,
            melee_weapon: None,
            laststand: false,
            jet: Default::default(),
            force_throw: false,
            last_weapon: 0,
            last_wstate: 0,
            last_clip: 0,
            last_dive: false,
            last_sprint: false,
            last_mantle_end: 0,
            stats: Default::default(),
            hud_fields: Vec::new(),
            score_fields: Vec::new(),
            camera: None,
            camera_on: false, mesh: String::new(),
            poisoned: false,
            no_spectate: 0,
            test_client: false,
            pending_since: None,
            shock: None,
            shock_factor: 1.0,
            given_names: BTreeMap::new(),
        }
    }
}

/// Engine state for the scripts.
/// A `spawnfx` effect: its name, origin, forward, and up (when given).
#[derive(Clone)]
pub(crate) struct PlacedFx {
    pub ident: String,
    pub spot: [f32; 3],
    pub ahead: [f32; 3],
    pub up: Option<[f32; 3]>,
}

#[derive(Resource, Default)]
pub(crate) struct Zm {
    pub map: String,
    pub ents: BTreeMap<u32, Ent>,
    pub next_entnum: u32,
    pub players: BTreeMap<u32, Survivor>,
    pub tables: Arc<BTreeMap<String, T6Table>>,
    pub fx: Vec<String>,
    /// Client systems by id (`clientsysregister`): "musicCmd", ...
    pub client_sys: Vec<String>,
    /// `spawnfx` effects by entity number.
    pub fx_ents: BTreeMap<u32, PlacedFx>,
    pub unbound: BTreeMap<String, u64>,
    /// bo2mp: `setmatchflag`'s flags, by BO2's index (GScr_SetMatchFlag:
    /// final_killcam 4, round_end_killcam 5, game_ended 0x11, ...): the
    /// HUD's visibility bits read them (CL_UpdateUIVisibilityBits).
    pub match_flags: u64,
    pub underway: bool,
    pub movers: movers::Movers,
    /// Notifies to deliver later: (server ms, object, name).
    pub timers: Vec<(i64, ObjRef, String)>,
    /// bo2mp: `spawntimedfx` entities and when their time is up (server
    /// ms): BO2 frees each then, notifying "death" first.
    pub timed_fx: BTreeMap<u32, i64>,
    /// bo2mp: smoke that blocks sight (`fxblocksight`): entity and radius,
    /// at most 32 (BO2's `SV_FX_SetVisBlocker` slots).
    pub vis_blockers: Vec<(u32, f32)>,
    /// bo2mp: entities bots do not take for targets (`allowbottargetting`).
    pub no_bot_target: BTreeSet<u32>,
    /// bo2mp: client fields as last set, read back by `codegetclientfield`
    /// and the others: (entity, its player state's set, name); the
    /// world's are under `u32::MAX`.
    pub client_field_vals: BTreeMap<(u32, bool, String), clientfields::FieldVal>,
    /// bo2mp: menu answers to deliver later as the player's `menuresponse`
    /// (server ms, player, menu, response).
    pub menu_replies: Vec<(i64, ObjRef, String, String)>,
    /// Script movers showing entities' models.
    pub presences: presence::Presences,
    /// bo2mp: the number each script entity was last sent to clients by (a
    /// shown model's, a projectile's), with the object it was then. The
    /// killcam names its entity by the script number and its clients find it
    /// by this one, also once the entity is gone (a grenade that went off);
    /// a number taken since by another object is not looked up.
    pub wire_numbers: BTreeMap<u32, (Option<ObjRef>, i32)>,
    /// bo2mp: each client's script sound notifies sent so far (how many, the
    /// last four), as its `bo2mp_sndnotify` carries them. Kept over a round
    /// restart; a new map numbers from 1 again.
    pub snd_notify: BTreeMap<u32, (u64, Vec<String>)>,
    /// The map's AI navigation graph.
    pub nav: nav::Nav,
    /// Brush entities' collision as last sent.
    pub brush_rows: BTreeMap<u32, brushes::BrushRow>,
    /// Path node script objects, by node index.
    pub node_objs: Vec<ObjRef>,
    /// Each path node object's node (the bot scripts name nodes by them).
    pub node_of: std::collections::HashMap<ObjRef, u32>,
    /// Animations by lower-case name.
    pub anims: std::collections::HashMap<String, T6Anim>,
    pub clips: std::collections::HashMap<String, Arc<xmodel_runtime::SkeletalClip>>,
    pub strings: std::collections::HashMap<String, String>,
    pub sound_aliases: BTreeSet<String>,
    /// bo2mp: the shellshocks the scripts started (and precached), in order (the index
    /// the player state carries is the position + 1).
    pub shock_names: Vec<String>,
    /// bo2mp: a multiplayer map (game type in `maps/mp/gametypes/`).
    pub mp: bool,
    /// bo2mp: a ranked match (BO2's `level.rankedmatch`: XP counts, the
    /// public class set; Combat Training), else a custom game.
    pub ranked_match: bool,
    /// bo2mp: the game type's settings (`getgametypesetting`).
    pub gamesettings: BTreeMap<String, String>,
    /// bo2mp: the settings a script changed (`setgametypesetting`); these
    /// beat the BO2MP_SETTING_<name> test overrides.
    pub gamesettings_set: BTreeSet<String>,
    /// bo2mp: team scores (`setteamscore`), the match's end time on the
    /// server clock (`setgameendtime`, 0 = no clock), the minimap's centre.
    pub team_scores: BTreeMap<String, i32>,
    pub sound_lengths: std::collections::HashMap<String, u32>,
    /// bo2mp: retrievable weapons (getretrievableweapons).
    pub retrievable_weapons: Vec<String>,
    /// bo2mp: bots (the engine half of BO2's bot scripts), the bot clients
    /// asked for and not yet handed to the bot roster, bots to remove,
    /// their difficulty (0 easy .. 3).
    pub bots: BTreeMap<u32, bots::Bot>,
    pub test_client_requests: Vec<u32>,
    pub bot_leaves: Vec<u32>,
    pub bot_difficulty: i32,
    /// bo2mp: path nodes marked dangerous (`setdangerous`): by node, a
    /// count per team (allies, axis), 0..15 like BO2's.
    pub node_danger: BTreeMap<u32, [u8; 2]>,
    /// bo2mp: items picked up last tick (engine records, see `items`).
    pub pickups_log: Vec<crate::ItemPickupRecord>,
    /// bo2mp: a gun on the floor's full name, by item number, when it was
    /// dropped by one the catalog spells otherwise (`an94_mp+fmj+reflex+
    /// extclip`): its pick-up is given it back (see `items`).
    pub item_names: BTreeMap<i32, String>,
    pub game_end_time: i32,
    pub map_center: [f32; 3],
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
    /// bo2mp: the maps' moving props (their client scripts' fxanims).
    pub mp_fxprops: mp_fxanims::MpFxProps,
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
    /// bo2mp: mp/statstable.csv's items, read once.
    pub items: loadout::ItemCache,
    /// bo2zm M4: each player's d-pad slots (`setactionslot(slot, "weapon",
    /// name)`): (slot, weapon or kind).
    pub action_slots: BTreeMap<u32, Vec<(i32, String)>>,
    /// The test player's fire button, pressed every other tick.
    pub autoplay_fire: bool,
    /// When every expected player of a party was first playing (the test
    /// aids' party clock, `autoplay::party_clock`).
    pub party_in_at: Option<i64>,
    /// The round the scripts last reported (`setroundsplayed`).
    pub games_finished: i32,
    /// Animation state definitions by file stem (`zm_nuked_basic`).
    pub asds: std::collections::HashMap<String, asd::AnimStateDef>,
    /// Actors (zombies) by entity number.
    pub actors: BTreeMap<u32, actors::Actor>,
    /// Server time of the last scheduler run.
    pub clock_millis: i64,
    pub next_thread_dump: i64,
    /// Players holding use (for the press edge).
    pub use_held: BTreeSet<u32>,
    /// The hint of the use trigger each player faces.
    pub hints: BTreeMap<u32, String>,
    /// bo2mp: the destructible definitions by lower-case name.
    pub destructibles: std::collections::HashMap<String, Arc<xmodel_runtime::BreakableModelDef>>,
    /// bo2mp: a round restart the scripts asked for (`map_restart`), run at
    /// the next frame's start: whether the players' `pers[]` carry over.
    pub pending_restart: Option<bool>,
    /// bo2mp: BO2's 32 objectives (`objective_add`), reset with the level.
    pub goal_list: [goals::MissionMarker; goals::MAX],
    /// bo2mp: players coming back from a round restart: their `pers[]` and
    /// whether each is a bot.
    pub rejoin: BTreeMap<u32, (ScriptValue, bool)>,
    /// The map's entities (their text), spawned again at a round restart.
    pub map_entities: Vec<String>,
    /// bo2mp: BO2's spawn influencers and points (`spawnsystem.rs`).
    pub spawns: spawnsystem::SpawnSystem,
    /// bo2mp: the map's `heli_height_lock` patches turned off, by targetname
    /// (`setheliheightpatchenabled( name, 0 )`; each starts on).
    pub heli_patches_off: BTreeSet<String>,
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
    /// Entity `n`'s yaw; 0 when there is no such entity.
    pub(crate) fn yaw_of(&self, n: u32) -> f32 {
        match self.ents.get(&n) {
            Some(en) => en.rot[1],
            None => 0.0,
        }
    }

    pub(crate) fn alloc_entnum(&mut self) -> u32 {
        if self.next_entnum < FIRST_ENTNUM {
            self.next_entnum = FIRST_ENTNUM;
        }
        let n = self.next_entnum;
        self.next_entnum += 1;
        n
    }
}

/// bo2zm: his profile's choices the scripts and the engine read (BO2's
/// Settings menu sets them): mature content (`getlocalprofileint
/// ("cg_mature")`: heads and limbs come off) and how many corpses stay
/// (`ai_corpseCount`: past it the oldest goes).
static MATURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static CORPSES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(16);

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
pub(crate) fn has_perk(game: &crate::step_view::StepWorldView, cid: crate::sim_parts::ClientId, perk: &str) -> bool {
    game
        .ecs_read()
        .get_resource::<Zm>()
        .and_then(|zm| zm.players.get(&cid.0))
        .is_some_and(|pl| pl.perks.contains(perk))
}

pub(crate) fn corpse_limit() -> usize {
    usize::from(CORPSES.load(std::sync::atomic::Ordering::Relaxed))
}

/// The party a Public Match lobby started with (0 = none): wins over
/// BO2MP_T6_PLAYERS.
static PARTY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// bo2zm H28: the players in the lobby that starts the next game (the host
/// and those who joined it), 1-4; 0 = no lobby (his own game).
pub fn set_t6_party_size(n: usize) {
    PARTY.store(n.min(4) as i32, std::sync::atomic::Ordering::Relaxed);
}

/// How many players the host's party is (BO2MP_T6_PLAYERS, 1-4, default 1):
/// BO2 waits for them all before the first round, and a party of one is a
/// solo game.
pub(crate) fn expected_players() -> i32 {
    let party = PARTY.load(std::sync::atomic::Ordering::Relaxed);
    if party > 0 {
        return party;
    }
    static N: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        // 1 to 4 players; unset or unreadable is one.
        let asked = std::env::var("BO2MP_T6_PLAYERS").ok().and_then(|v| v.trim().parse::<i32>().ok());
        asked.map_or(1, |n| n.clamp(1, 4))
    })
}

pub(crate) fn install(world: &mut World, inst: T6Install) -> Result<(), String> {
    let mp = mp::is_mp(&inst.map);
    cheats::reset();
    let mut objects = Vec::with_capacity(inst.objects.len() + 1);
    for bytes in &inst.objects {
        objects.push(ScriptObject::parse(bytes)?);
    }
    // bo2mp: the scripts the engine provides (multiplayer's _globallogic).
    if mp {
        objects.extend(mp::engine_scripts()?);
    }
    // BO2MP_T6_FDIS=label,label: write those script functions' decoded code to
    // the log (reading a map's own script when no source is on disk).
    if let Ok(idents) = std::env::var("BO2MP_T6_FDIS") {
        let idents: Vec<&str> = idents.split(',').map(str::trim).collect();
        for o in &objects {
            for f in o.functions.iter().filter(|f| idents.contains(&f.name.as_str())) {
                diag::info!(Sim, "bo2zm t6 fdis: {}::{} params {}", o.name, f.name, f.params);
                for (at, insn) in &f.code {
                    diag::info!(Sim, "bo2zm t6 fdis:   {at:#06x} {insn:?}");
                }
            }
        }
    }
    let mut strings = Strings::default();
    // bo2mp: Black Ops II's linker binds a bare call to the engine's own
    // function when the engine has one, before the script's or its
    // includes'. These are the multiplayer idents a script also defines
    // (_spawning's clearspawnpoints() must not run _spawnlogic's, which
    // empties the spawn lists).
    const ENGINE_FIRST: &[(&str, bool)] = &[
        ("addspawnpoints", false),
        ("clearspawnpoints", false),
        ("vectorcross", false),
        ("suicide", true),
    ];
    let program = Program::link_with(objects, &mut strings, if mp { ENGINE_FIRST } else { &[] })?;
    let functions = program.functions.len();
    let builtins = program.builtins.len();
    let mut vm: Vm<World> = Vm::new(program, strings);
    // A new game rolls new dice (which perk falls next, box guns, drops): the
    // scripts' random numbers start from the clock. BO2MP_T6_SEED=<n> fixes
    // them for a test that must repeat.
    vm.rng = std::env::var("BO2MP_T6_SEED")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or_else(|| std::time::UNIX_EPOCH.elapsed().map_or(1, |d| d.as_nanos() as u64))
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        | 1;
    gsc_t6::natives::bind_core(&mut vm);
    natives_game::bind(&mut vm);
    natives_ent::bind(&mut vm);
    natives_player::bind(&mut vm);
    if !mp {
        globallogic::bind(&mut vm);
    }
    // Sounds and effects replace the quiet stubs bound above.
    natives_fx::bind(&mut vm);
    natives_ai::bind(&mut vm);
    zbarrier::bind(&mut vm);
    darts::bind(&mut vm);
    clientfields::bind(&mut vm);
    brushes::bind(&mut vm);
    vehicles::bind(&mut vm);
    turrets::bind(&mut vm);
    if !mp {
        riotshield::bind(&mut vm);
    }
    jetgun::bind(&mut vm);
    mc_specials::add_to_cheats();
    throwables::bind(&mut vm);
    limits::bind(&mut vm);
    if !mp {
        stats::bind(&mut vm);
    }
    hud::bind(&mut vm);
    if mp {
        natives_mp::bind(&mut vm);
        stats::bind(&mut vm);
        bots::bind(&mut vm);
        killstreaks::bind(&mut vm);
        loadout::bind(&mut vm);
        goals::bind(&mut vm);
        items::bind(&mut vm);
        // bo2mp lane F: the body's animation natives (the death path).
        playeranim::bind(&mut vm);
        // bo2mp gun brain, window 5: BO2's setoffhandvisible (the equipment in his hand).
        offhand_visible::bind(&mut vm);
        // The mod menu's own natives (after the others: its hide/show take players too).
        menu::bind(&mut vm);
        // bo2mp gun brain, window 11: the riot shield plant (canplaceriotshield, deployriotshield).
        riotshield::bind(&mut vm);
        // bo2mp gun brain: the gun builtins on BO2's gun state, last.
        gun_script::bind(&mut vm);
    }
    if !mp {
        // bo2mp gun brain, window 5: BO2's setoffhandvisible (the equipment in his hand).
        offhand_visible::bind(&mut vm);
        // bo2mp gun brain: the gun builtins on BO2's gun state, last.
        gun_script::bind(&mut vm);
    }
    // BO2MP_T6_TRACE=label,label: log those builtins' calls (debugging).
    if let Ok(idents) = std::env::var("BO2MP_T6_TRACE") {
        vm.trace_builtins(&idents, 5000);
    }
    // BO2MP_T6_FTRACE=label,label: log those script functions' calls.
    if let Ok(idents) = std::env::var("BO2MP_T6_FTRACE") {
        vm.trace_functions(&idents);
    }
    vm.hooks = Hooks {
        get_field: Some(fields::get),
        set_field: Some(fields::set),
        unbound: Some(unbound),
    };
    // bo2mp: a multiplayer match: 18 players (a full lobby), no online
    // services, ranked play off (a local match, as Combat Training).
    // BO2MP_BOTS=<friends>,<enemies>[,<difficulty 0-3>]: bots, as BO2's
    // bot setup screen sets them (a test aid until the lobby does).
    // A script thread that runs over 250 ms idents itself (a freeze he
    // would feel); BO2MP_T6_PROF=1: over 20 ms.
    vm.slow_thread_ms = if prof_on() { 20.0 } else { 250.0 };
    // Profiling: which scripts and builtins the ticks spend on.
    if prof_on() {
        vm.profile = Some(gsc_t6::vm::ScriptProfile::default());
    }
    let mut bot_difficulty = 1;
    let lobby = inst.bots.map(|(f, e, d)| format!("{f},{e},{d}"));
    if mp && let Some(spec) = std::env::var("BO2MP_BOTS").ok().or(lobby) {
        let v: Vec<i32> = spec.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let (friends, enemies) = (v.first().copied().unwrap_or(0), v.get(1).copied().unwrap_or(0));
        bot_difficulty = v.get(2).copied().unwrap_or(1);
        for (k, n) in [("bot_friends", friends), ("bot_enemies", enemies), ("bot_difficulty", bot_difficulty)] {
            vm.dvars.insert(k.to_owned(), n.to_string());
        }
    }
    if mp {
        for (k, v) in [
            ("sv_maxclients", "18"),
            ("party_maxplayers", "18"),
            ("xblive_wagermatch", "0"),
            ("xblive_basictraining", "0"),
            ("xblive_privatematch", "1"),
            ("scr_xpscale", "1"),
        ] {
            vm.dvars.insert(k.to_owned(), v.to_owned());
        }
        // Combat Training (ranked, lane C): BO2's public basic-training
        // match. BO2's ranked bot branch (_bot.gsc: one bot per human
        // player, "comp stomp") stays off (sv_botsoak, its own switch): the
        // stand-in globallogic fills the teams with the playlist's bots
        // (`combattrainingbots`).
        if stats::ranked() {
            for (k, v) in [("xblive_basictraining", "1"), ("xblive_privatematch", "0"), ("sv_botsoak", "1")] {
                vm.dvars.insert(k.to_owned(), v.to_owned());
            }
        }
        // The engine's perk dvars at their registered defaults. The scripts
        // read them (Hardline: a kill's momentum x perk_killstreakMomentumMultiplier
        // in giveplayermomentumnotification); unset they read 0, so Hardline
        // gave 0 momentum a kill and no scorestreak was ever earned.
        for (k, v) in [
            ("perk_weapswitchmultiplier", "0.5"),
            ("perk_weapspreadmultiplier", "0.65"),
            ("perk_weapreloadmultiplier", "0.5"),
            ("perk_weapratemultiplier", "0.75"),
            ("perk_weapadsmultiplier", "0.5"),
            ("perk_weapmeleemultiplier", "0.5"),
            ("perk_weaptossmultiplier", "0.5"),
            ("perk_weapequipmentusemultiplier", "0.6"),
            ("perk_extrabreath", "5"),
            ("perk_bulletpenetrationmultiplier", "2"),
            ("perk_sprintmultiplier", "2"),
            ("perk_grenadetossbacktimer", "2500"),
            ("perk_killstreakreduction", "1"),
            ("perk_killstreakmomentummultiplier", "1.2"),
            ("perk_killstreakdeathpenaltymultiplier", "0.5"),
            ("perk_delayexplosivetime", "0.9"),
            ("perk_disarmexplosivetime", "2"),
            ("perk_speedmultiplier", "1.07"),
            ("perk_deathstreakcountrequired", "3"),
            ("perk_sprintrecoverymultiplier", "0.3"),
            ("perk_healthregenmultiplier", "1.5"),
            ("perk_damagekickreduction", "0.25"),
            ("perk_shellshockreduction", "0.5"),
            ("perk_mantlereduction", "0.4"),
            ("perk_interactspeedmultiplier", "1.15"),
            ("perk_throwbackinnerradius", "120"),
            ("perk_flakjacket", "35"),
            ("perk_fireproof", "95"),
            ("perk_armorpiercing", "95"),
            ("perk_dogsattackghost", "1"),
            ("perk_blackbirdshowsgpsjammer", "1"),
            ("perk_fastladderclimbmultiplier", "1.75"),
            ("perk_marksmanenemynamefadein", "5"),
            ("perk_marksmanenemynamefadeout", "5"),
            ("perk_nottargetedbyai_min_speed", "0"),
            ("perk_nottargetedbyai_graceperiod", "2000"),
        ] {
            vm.dvars.entry(k.to_owned()).or_insert_with(|| v.to_owned());
        }
        // The engine's riot shield dvars at their registered defaults. The
        // planted shield's health and damage scales are read by
        // _riotshield.gsc; unset they read 0, so one bullet broke a planted
        // shield and its wreck vanished at once (BO2: 300 health, bullets
        // x0.025, the wreck stays 10 s).
        for (k, v) in [
            ("riotshield_deploy_limit_radius", "100"),
            ("riotshield_bullet_damage_scale", "0.025"),
            ("riotshield_explosive_damage_scale", "1"),
            ("riotshield_melee_damage_scale", "1"),
            ("riotshield_projectile_damage_scale", "0.01"),
            ("riotshield_deployed_health", "300"),
            ("riotshield_destroyed_cleanup_time", "10"),
            ("riotshield_deploy_pitch_max", "20"),
            ("riotshield_deploy_roll_max", "25"),
            ("riotshield_deploy_zdiff_max", "30"),
            ("riotshield_assist_time", "4"),
            ("riotshield_damage_score_threshold", "400"),
            ("riotshield_damage_score_max", "2000"),
        ] {
            vm.dvars.entry(k.to_owned()).or_insert_with(|| v.to_owned());
        }
        // The K9 Unit's turn: its animscript reads them (dog_turn.gsc), and
        // so does the dog think that starts it (`actors::should_turn`).
        for (k, v) in [
            ("dog_turn90_angle", "70"),
            ("dog_turn180_angle", "135"),
            ("dog_turn_min_goal_dist", "40"),
        ] {
            vm.dvars.entry(k.to_owned()).or_insert_with(|| v.to_owned());
        }
    }
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
        ("sv_maxclients", if mp { "18" } else { "4" }),
        ("party_maxplayers", if mp { "18" } else { "4" }),
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
    // BO2MP_TEST_SD_BOTS=1, test runs only: Search and Destroy's bots run
    // BO2's _bot_sd (mp_globallogic.gsc reads the dvar). Unset in normal games.
    if mp && std::env::var_os("BO2MP_TEST_SD_BOTS").is_some() {
        vm.dvars.insert("bo2mp_test_sd_bots".to_owned(), "1".to_owned());
    }
    diag::info!(
        Sim,
        "bo2zm t6 scripts: {} objects, {functions} functions, {builtins} builtins, {} unbound",
        inst.objects.len(),
        vm.unbound_builtins().len()
    );
    // BO2MP_T6_UNBOUND=1: every builtin the scripts label that has no native.
    if std::env::var("BO2MP_T6_UNBOUND").is_ok() {
        let mut unbound: Vec<String> = vm
            .unbound_builtins()
            .into_iter()
            .map(|(n, m)| if m { format!(".{n}") } else { n })
            .collect();
        unbound.sort();
        diag::info!(Sim, "bo2zm t6 unbound: {}", unbound.join(" "));
    }
    // Path nodes are stored where the designer placed them (Nuketown's
    // town grid floats at z 0, the ground is near -60); BO2 drops them to
    // the floor. Ours did not: every "can it walk straight there" sweep ran
    // 60 units up, over low walls, fences and cars, and zombies took those
    // routes into them and stood there (M4 retest 1).
    let mut nodes = inst.path_nodes.clone();
    if mp {
        drop_nodes_to_floor_mp(world, &mut nodes);
    } else {
        actors::drop_nodes_to_floor(world, &mut nodes);
    }
    let node_objs = node_objects(&mut vm, &nodes);
    let baked = nav::Baked::new(&inst.path_tables, nodes.len());
    let linked: Vec<(u32, u32)> = nodes
        .iter()
        .enumerate()
        .flat_map(|(i, n)| n.links.iter().map(move |l| (i as u32, u32::from(l.0))))
        .collect();
    let seen = linked.iter().filter(|&&(i, j)| baked.visible(i, j) == Some(true)).count();
    let smooth = linked.iter().filter(|&&(i, j)| baked.smooth(i, j)).count();
    diag::info!(
        Sim,
        "bo2mp path tables: {} nodes ({} compiled), sight table {} bytes (side {}), smooth table {} bytes (side {}); of {} links, {} see each other, {} walk straight",
        inst.path_tables.node_count,
        inst.path_tables.original_count,
        inst.path_tables.vis.len(),
        baked.sides().0,
        inst.path_tables.smooth.len(),
        baked.sides().1,
        linked.len(),
        seen,
        smooth,
    );
    let anims: std::collections::HashMap<String, T6Anim> = inst
        .anims
        .into_iter()
        .map(|a| (a.name.to_ascii_lowercase(), a))
        .collect();
    let asds = inst
        .animstatedefs
        .iter()
        .map(|(label, body_text)| {
            let file = label.rsplit('/').next().unwrap_or(label);
            (file.trim_end_matches(".asd").to_ascii_lowercase(), asd::AnimStateDef::parse(body_text))
        })
        .collect::<std::collections::HashMap<String, asd::AnimStateDef>>();
    // BO2MP_T6_ASDLOG=<part of a def label>: each state of those animstatedefs,
    // its notify and animations (test aid).
    if let Ok(want) = std::env::var("BO2MP_T6_ASDLOG") {
        for (stem, def) in asds.iter().filter(|(k, _)| k.contains(want.as_str())) {
            let mut idents: Vec<&String> = def.states.keys().collect();
            idents.sort_unstable();
            for k in idents {
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
    // BO2MP_T6_ANIM=<part of a label>: those animations' timing and root motion.
    if let Ok(want) = std::env::var("BO2MP_T6_ANIM") {
        let mut matching: Vec<&String> = anims.keys().filter(|k| k.contains(want.as_str())).collect();
        matching.sort();
        for k in matching.into_iter().take(40) {
            let a = &anims[k];
            diag::info!(
                Sim,
                "bo2zm t6 anim {k}: {} frames at {} fps, looping {}, {} root keys {:?} .. {:?}, speed {:.1}, notes {:?}",
                a.frame_total,
                a.fps,
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
            .where_value(|ch| ch.joint_hit_volume.iter().any(Option::is_some))
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
        mp,
        ranked_match: mp && stats::ranked(),
        bot_difficulty,
        sound_lengths: inst.sound_lengths,
        retrievable_weapons: inst.retrievable_weapons,
        gamesettings: inst.gamesettings,
        gamesettings_set: BTreeSet::new(),
        zbarrier_defs: inst
            .zbarrier_defs
            .into_iter()
            .map(|d| (d.name.to_ascii_lowercase(), Arc::new(d)))
            .collect(),
        donor_effects: inst.donor_effects,
        tables: Arc::new(inst.tables),
        next_entnum: FIRST_ENTNUM,
        nav: nav::Nav::new(nodes).with_baked(baked),
        node_of: node_objs.iter().enumerate().map(|(i, o)| (*o, i as u32)).collect(),
        node_objs,
        anims,
        asds,
        destructibles: inst
            .destructibles
            .into_iter()
            .map(|d| (d.name.to_ascii_lowercase(), d))
            .collect(),
        ..Default::default()
    });
    killstreaks::install(world, inst.vehicles);
    world.insert_resource(T6Runtime { vm, omni: crate::frame_step::omni_movement_setting() });
    world.insert_resource(playeranim::Bodies::new(inst.playeranim));
    world.insert_resource(PendingEntities(inst.entities));
    Ok(())
}

/// bo2mp: BO2's own node drop (G_DropPathnodesToFloor, node_droptofloor).
/// A node without spawnflag 0x100 goes down onto what a capsule 30 wide and
/// 30 tall, swept from 1 unit above it to 256 below (the AI mask), lands
/// on. A node that starts in solid or meets nothing stays where it is (BO2
/// prints an error and marks it bad). Multiplayer used the zombies' drop
/// before (a thin box from 16 above to 512 below); on Grind both put the
/// nodes in the same places, and one node in solid now stays put.
fn drop_nodes_to_floor_mp(world: &mut World, nodes: &mut [T6PathNode]) {
    const MASK_AI: u32 = 0x0082_0011;
    let f = frame(world);
    let (mut moved, mut total, mut most) = (0usize, 0.0f32, 0.0f32);
    let (mut kept, mut solid, mut floating) = (0usize, 0usize, 0usize);
    for n in nodes.iter_mut() {
        if n.spawnflags & 0x100 != 0 {
            kept += 1;
            continue;
        }
        let from = [n.origin[0], n.origin[1], n.origin[2] + 1.0];
        let to = [n.origin[0], n.origin[1], n.origin[2] - 256.0];
        let t = f.trace_static_world(from, to, [-15.0, -15.0, 0.0], [15.0, 15.0, 30.0], MASK_AI);
        if t.starts_inside || t.fully_embedded {
            solid += 1;
            continue;
        }
        if t.fraction >= 1.0 {
            floating += 1;
            continue;
        }
        let z = from[2] + (to[2] - from[2]) * t.fraction;
        let d = (n.origin[2] - z).abs();
        if d > 1.0 {
            moved += 1;
            total += d;
            most = most.max(d);
        }
        n.origin[2] = z;
    }
    diag::info!(
        Sim,
        "bo2mp t6 nav: {moved} of {} path nodes dropped to the floor (avg {:.0}, most {most:.0}); {kept} kept (spawnflag 0x100), {solid} in solid, {floating} floating",
        nodes.len(),
        if moved > 0 { total / moved as f32 } else { 0.0 }
    );
}

/// The path nodes' script objects, by node index.
fn node_objects(vm: &mut Vm<World>, nodes: &[T6PathNode]) -> Vec<ObjRef> {
    let mut node_objs = Vec::with_capacity(nodes.len());
    for n in nodes {
        let o = vm.alloc_object(ObjKind::Struct);
        let kind = nav::NODE_TYPE_NAMES.get(n.ty as usize).copied().unwrap_or("Path");
        let fields: [(&str, ScriptValue); 4] = [
            ("origin", ScriptValue::Vec3(n.origin)),
            ("angles", ScriptValue::Vec3([0.0, n.angle, 0.0])),
            ("spawnflags", ScriptValue::Int(n.spawnflags as i32)),
            ("radius", ScriptValue::Float(n.radius)),
        ];
        for (k, v) in fields {
            let f = vm.intern(k);
            vm.set_raw_field(o, f, v);
        }
        for (k, v) in [
            ("type", kind),
            ("targetname", n.target_key.as_str()),
            ("target", n.target.as_str()),
            ("script_noteworthy", n.script_note.as_str()),
            ("script_linkname", n.script_link.as_str()),
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
    node_objs
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

/// bo2mp: Black Ops II's killcam (`_killcam.gsc`) puts a dead player in
/// spectator with the fields the engine replays from: whom to watch
/// (`spectatorclient`), the entity that killed (`killcamentity`), how far
/// back (`archivetime`), the time offset and the length. The same seats
/// MW2's scripts fill, so the engine's own killcam replay plays it.
pub(crate) fn killcam_seats(world: &World) -> Vec<(ClientId, crate::ScriptSeat)> {
    let (Some(zm), Some(rt)) = (world.get_resource::<Zm>(), world.get_resource::<T6Runtime>()) else {
        return Vec::new();
    };
    if !zm.mp {
        return Vec::new();
    }
    let vm = &rt.vm;
    let field = |o: ObjRef, name: &str| -> Option<ScriptValue> {
        let f = vm.strings.find(name)?;
        Some(vm.raw_field(o, f))
    };
    let num = |o: ObjRef, name: &str| field(o, name).and_then(|v| v.as_float());
    // The scripts name the killcam's entity by its script number
    // (getentitynumber); its clients know it by the number it was sent by.
    // A player's is the same; one never sent is no entity to them.
    let wire = |n: i32| -> i32 {
        let Ok(n) = u32::try_from(n) else {
            return -1;
        };
        if zm.players.contains_key(&n) {
            return n as i32;
        }
        let now = zm.ents.get(&n).and_then(|e| e.obj);
        zm.presences
            .by_ent
            .get(&n)
            .map(|s| s.num)
            .or_else(|| {
                let (then, number) = *zm.wire_numbers.get(&n)?;
                (now.is_none() || now == then).then_some(number)
            })
            .unwrap_or(-1)
    };
    // BO2's final replay (_killcam.gsc dofinalkillcam) seats every player,
    // the living too: the match end freezes them, it does not kill them.
    let final_kill = field(vm.level, "infinalkillcam").and_then(|v| v.as_int()).is_some_and(|v| v != 0);
    let mut out = Vec::new();
    for (peer, pt) in &zm.players {
        if pt.sessionstate != "spectator" {
            continue;
        }
        let archive = num(pt.obj, "archivetime").unwrap_or(0.0);
        let spectator = num(pt.obj, "spectatorclient").map_or(-1, |v| v as i32);
        if archive <= 0.0 || spectator < 0 {
            continue;
        }
        let script_ent = num(pt.obj, "killcamentity").map_or(-1, |v| v as i32);
        let death_replay_target = wire(script_ent);
        if script_ent >= 0 {
            KILLCAM_TOLD.with(|told| {
                if told.borrow_mut().insert(*peer, script_ent) != Some(script_ent) {
                    diag::info!(
                        Sim,
                        "bo2mp killcam: client {peer} follows script entity {script_ent}, sent as {death_replay_target}"
                    );
                }
            });
        }
        let seat = crate::ScriptSeat {
            spectator_client: spectator,
            death_replay_target,
            look_at_entity: num(pt.obj, "killcamentitylookat").map_or(-1, |v| wire(v as i32)),
            target_client: num(pt.obj, "killcamtargetentity").map_or(-1, |v| v as i32),
            archive_ms: (archive * 1000.0).round() as i32,
            ps_offset_ms: num(pt.obj, "psoffsettime").map_or(0, |v| v as i32),
            length_ms: (num(pt.obj, "killcamlength").unwrap_or(0.0) * 1000.0).round() as i32,
            last_kill: final_kill,
        };
        out.push((ClientId(*peer), seat));
    }
    out
}

/// bo2mp: the engine's killcam moved a seat's `archivetime` nearer the
/// present. Real SpectatorClientEndFrame steps `sess.archiveTime` 50 ms at a
/// time while the watched player has no saved state, and keeps it; the
/// script reads the field back (`checkforabruptkillcamend` ends at 0).
pub(crate) fn store_killcam_archive(game: &mut World, peer: ClientId, ms: i32) {
    let Some(obj) = game.get_resource::<Zm>().and_then(|zm| zm.players.get(&peer.0).map(|p| p.obj)) else {
        return;
    };
    let Some(mut rt) = game.get_resource_mut::<T6Runtime>() else {
        return;
    };
    let f = rt.vm.intern("archivetime");
    rt.vm.set_raw_field(obj, f, ScriptValue::Float(ms as f32 / 1000.0));
}

/// bo2mp: bot clients asked for (`addtestclient`), drained.
pub(crate) fn take_test_clients(world: &mut World) -> Vec<u32> {
    world
        .get_resource_mut::<Zm>()
        .map(|mut z| std::mem::take(&mut z.test_client_requests))
        .unwrap_or_default()
}

/// bo2mp: bots to remove (`botleavegame`), drained.
pub(crate) fn take_bot_leaves(world: &mut World) -> Vec<u32> {
    world
        .get_resource_mut::<Zm>()
        .map(|mut z| std::mem::take(&mut z.bot_leaves))
        .unwrap_or_default()
}

/// Run `f` with the VM out of the world.
pub(crate) fn with_vm<R>(
    game: &mut World,
    f: impl FnOnce(&mut Vm<World>, &mut World) -> R,
) -> Option<R> {
    if !game.contains_resource::<T6Runtime>() {
        return None;
    }
    Some(
        game.resource_scope(|game, mut rt: bevy_ecs::prelude::Mut<T6Runtime>| {
            f(&mut rt.vm, game)
        }),
    )
}

/// bo2zm M4: a player's answer from a BO2 menu (`Engine.SendMenuResponse`:
/// "popup_leavegame" "endround", ...) reaches the scripts as his
/// `menuresponse` notify. False when no BO2 map runs.
pub(crate) fn menu_response(game: &mut World, peer: u32, menu: &str, response: &str) -> bool {
    with_vm(game, |vm, game| {
        let Some(p) = game.resource::<Zm>().players.get(&peer).map(|p| p.obj) else {
            return;
        };
        let (m, r) = (vm.string(menu), vm.string(response));
        diag::info!(
            Sim,
            "bo2zm t6 menuresponse from {peer}: {menu} {response}"
        );
        vm.notify_str(game, p, "menuresponse", &[m, r]);
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
    let mp_map = world.resource::<Zm>().mp;
    let gametype = with_vm(world, |vm, _| {
        vm.dvars.get("g_gametype").cloned().unwrap_or_default()
    })
    .unwrap_or_default();
    with_vm(world, |vm, world| {
        let level = ScriptValue::Object(vm.level);
        vm.spawn_named(
            world,
            "codescripts/struct",
            "initstructs",
            level.clone(),
            vec![],
        );
        // BO2MP_T6_ENTDUMP=1 (tests): the map's entities as the map gives
        // them, to map_entities.txt in the run's folder.
        if std::env::var_os("BO2MP_T6_ENTDUMP").is_some() {
            let _ = std::fs::write("map_entities.txt", texts.join("\n"));
        }
        natives_ent::spawn_map_entities(vm, world, &texts);
        // The engine's calls, in order. Functions no script calls are the
        // engine's (measured: zm_nuked::gamemode_callback_setup registers
        // the map's game modes, which the game type's start needs;
        // _zm::post_main; codecallback_finalizeinitialization). bo2mp: a
        // multiplayer map starts its game type, the map, then the match.
        // A map main that calls _zm::post_main itself (Buried) gets it once: twice ran
        // init_custom_ai_type twice (two Leroys, doubled spawn functions).
        let post_main_scripted = vm
            .program
            .find(&vm.strings, "maps/mp/zombies/_zm", "post_main")
            .is_some_and(|fi| vm.program.script_refers_to(&format!("maps/mp/{map}"), fi));
        let calls: Vec<(String, &str, bool)> = if mp_map {
            // The map's two factions first (bodies, arms, voices per team:
            // game["set_player_model"]): no script starts their teamset;
            // the engine does, for each faction zone it loaded.
            let mut v: Vec<(String, &str, bool)> = vm
                .program
                .scripts
                .iter()
                .map(|s| s.name.clone())
                .filter(|n| {
                    n.starts_with("maps/mp/teams/_teamset_") && n != "maps/mp/teams/_teamset_multiteam"
                })
                .map(|n| (n, "main", false))
                .collect();
            v.extend([
                (format!("maps/mp/gametypes/{gametype}"), "main", true),
                (format!("maps/mp/{map}"), "main", true),
                (
                    "maps/mp/gametypes/_callbacksetup".to_owned(),
                    "codecallback_startgametype",
                    true,
                ),
            ]);
            v
        } else {
            vec![
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
            ]
        };
        // bo2mp: the test aid's script last (BO2MP_DEBUG_GSC).
        let mut calls = calls;
        if mp_map {
            calls.push(("bo2mp/debug".to_owned(), "main", false));
            // The mods folder's own scripts: `main`, then `init` (mp::mods).
            let mods: Vec<String> = vm
                .program
                .scripts
                .iter()
                .map(|s| s.name.clone())
                .filter(|n| n.starts_with(mp::MOD_SCRIPTS))
                .collect();
            for m in mods {
                calls.push((m.clone(), "main", false));
                calls.push((m, "init", false));
            }
        }
        for (script, func, required) in calls {
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
        if mp_map {
            clientfields::map_start(vm, world, &map);
        }
        foreign_weapons::set_effects(vm, world);
        diag::info!(
            Sim,
            "bo2zm t6: level started, {} threads",
            vm.thread_count()
        );
    });
    let mut zm = world.resource_mut::<Zm>();
    zm.underway = true;
    zm.map_entities = texts;
    declassified::start(world, &map);
    report(world);
}

/// bo2mp: Black Ops II's `map_restart` (a round of Search and Destroy over,
/// Demolition's half time): the level starts again in the same match. Every
/// script thread and object goes, and all the level put in the world (its
/// entities, grenades in the air, dropped guns, bodies); `game[]`, the
/// dvars and the loaded assets stay, and each player comes back in with his
/// `pers[]` when `persist` (`map_restart( 1 )`).
fn restart(game: &mut World, tick_no: crate::Tick, persist: bool) {
    killstreaks::restart(game);
    presence::retire_all(game);
    brushes::retire_all(game);
    let rejoin: BTreeMap<u32, (ScriptValue, bool)> = with_vm(game, |vm, game| {
        let pers = vm.intern("pers");
        let rejoin = game
            .resource::<Zm>()
            .players
            .iter()
            .map(|(c, p)| {
                let kept = if persist { vm.raw_field(p.obj, pers) } else { ScriptValue::Undefined };
                (*c, (kept, p.test_client))
            })
            .collect();
        vm.restart();
        rejoin
    })
    .unwrap_or_default();
    {
        let mut fl = frame(game);
        for p in crate::step_view::gather_shells(fl.ecs()) {
            fl.drop_projectile_slot(p.ent_no);
        }
        for number in fl.ordered_loot_slots() {
            if fl.take_loot_at_slot(number).is_some() {
                fl.free_dynamic_entity_number(number);
            }
        }
        *fl.corpses_mut() = crate::PlayerCorpsePool::default();
        for cid in fl.ordered_player_ids() {
            let (spot, rot) = fl
                .avatar(cid)
                .map_or(([0.0; 3], [0.0; 3]), |ps| (ps.origin, ps.look_angles));
            crate::client_script::spawn(&mut fl, tick_no, cid, spot, rot, "spectator");
        }
    }
    let old = std::mem::take(&mut *game.resource_mut::<Zm>());
    let baked = old.nav.baked;
    let nodes = old.nav.graph;
    let node_objs = with_vm(game, |vm, _| node_objects(vm, &nodes)).unwrap_or_default();
    let texts = old.map_entities.clone();
    let players = rejoin.len();
    // What the level loaded and the client was last sent stay (the HUD
    // goes by what it was sent; the models' serials keep counting).
    game.insert_resource(Zm {
        map: old.map,
        tables: old.tables,
        fx: old.fx,
        client_sys: old.client_sys,
        unbound: old.unbound,
        presences: old.presences,
        nav: nav::Nav::new(nodes).with_baked(baked),
        brush_rows: old.brush_rows,
        node_of: node_objs.iter().enumerate().map(|(i, o)| (*o, i as u32)).collect(),
        node_objs,
        anims: old.anims,
        clips: old.clips,
        strings: old.strings,
        sound_aliases: old.sound_aliases,
        shock_names: old.shock_names,
        mp: old.mp,
        ranked_match: old.ranked_match,
        gamesettings: old.gamesettings,
        gamesettings_set: old.gamesettings_set,
        team_scores: old.team_scores,
        sound_lengths: old.sound_lengths,
        retrievable_weapons: old.retrievable_weapons,
        bots: old.bots.into_iter().map(|(n, b)| (n, b.restarted())).collect(),
        test_client_requests: old.test_client_requests,
        bot_leaves: old.bot_leaves,
        bot_difficulty: old.bot_difficulty,
        game_end_time: old.game_end_time,
        map_center: old.map_center,
        hud_sent: old.hud_sent,
        autoplay_fire: old.autoplay_fire,
        games_finished: old.games_finished,
        asds: old.asds,
        clock_millis: old.clock_millis,
        next_thread_dump: old.next_thread_dump,
        destructibles: old.destructibles,
        zbarrier_defs: old.zbarrier_defs,
        donor_effects: old.donor_effects,
        next_entnum: FIRST_ENTNUM,
        rejoin,
        map_entities: old.map_entities,
        snd_notify: old.snd_notify,
        ..Default::default()
    });
    game.insert_resource(engine_sounds::EngineSounds::default());
    let script = game.get_resource::<playeranim::Bodies>().and_then(|b| b.script.clone());
    game.insert_resource(playeranim::Bodies::new(script));
    game.insert_resource(PendingEntities(texts));
    diag::info!(Sim, "bo2mp map_restart({persist}): the level starts again, {players} players come back");
}

/// Each authority frame: connect players, move movers, run the scheduler.
/// BO2MP_T6_PROF=1: where a tick's time goes (milliseconds per tick, every
/// 5 s of game), and each slow tick's parts. Without it, only a stall
/// (over STALL_TICK_MS) logs its parts.
#[derive(Default)]
struct Prof {
    ticks: u32,
    parts: Vec<(&'static str, f64)>,
    /// This tick's parts so far.
    tick: Vec<(&'static str, f64)>,
}

/// A tick slower than this logs its parts (ms).
const SLOW_TICK_MS: f64 = 30.0;
/// Profiling or not, a tick slower than this logs its parts (ms): a freeze
/// he would feel names its cause.
const STALL_TICK_MS: f64 = 250.0;

fn prof_on() -> bool {
    static PROFILING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PROFILING.get_or_init(|| std::env::var_os("BO2MP_T6_PROF").is_some())
}

thread_local! {
    /// The thread's CPU time when this tick began (a slow tick says how
    /// much of it was work).
    static TICK_CPU: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
    /// The profile line just went out: the scripts' own follows it.
    static SCRIPT_REPORT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Each client's killcam entity as last logged (one line per change).
    static KILLCAM_TOLD: std::cell::RefCell<BTreeMap<u32, i32>> = const { std::cell::RefCell::new(BTreeMap::new()) };
}

fn prof_add(name: &'static str, since: std::time::Instant) -> std::time::Instant {
    static PROF: std::sync::Mutex<Option<Prof>> = std::sync::Mutex::new(None);
    let stamp = std::time::Instant::now();
    if let Ok(mut p) = PROF.lock() {
        let p = p.get_or_insert_with(Prof::default);
        if name == "tick" {
            let ms = (stamp - since).as_secs_f64() * 1000.0;
            let slow = if prof_on() { SLOW_TICK_MS } else { STALL_TICK_MS };
            if ms > slow {
                let pieces: Vec<String> = p
                    .tick
                    .iter()
                    .filter(|(n, _)| *n != "total")
                    .map(|(n, ms)| format!("{n} {ms:.1}"))
                    .collect();
                let busy = diag::busy_note(TICK_CPU.get());
                diag::warn!(Sim, "bo2mp slow tick: {ms:.1} ms ({}){busy}", pieces.join(", "));
            }
            p.tick.clear();
            p.ticks += 1;
            if !prof_on() {
                p.parts.clear();
                p.ticks = 0;
            } else if p.ticks >= 100 {
                let line: Vec<String> = p
                    .parts
                    .iter()
                    .map(|(n, ms)| format!("{n} {:.2}", ms / f64::from(p.ticks)))
                    .collect();
                diag::info!(Sim, "bo2zm t6 prof (ms per tick): {}", line.join(", "));
                *p = Prof::default();
                SCRIPT_REPORT.set(true);
            }
        } else {
            let ms = (stamp - since).as_secs_f64() * 1000.0;
            match p.parts.iter_mut().find(|(n, _)| *n == name) {
                Some(row) => row.1 += ms,
                None => p.parts.push((name, ms)),
            }
            p.tick.push((name, ms));
        }
    }
    stamp
}

pub(crate) fn advance(world: &mut World) {
    if !world.contains_resource::<T6Runtime>() {
        return;
    }
    // Always timed (a few clock reads a tick): a stall names its part.
    let t0 = std::time::Instant::now();
    TICK_CPU.set(diag::thread_cpu_ms());
    advance_inner(world);
    if prof_on() {
        prof_add("total", t0);
    }
    prof_add("tick", t0);
    if SCRIPT_REPORT.replace(false)
        && let Some(Some(line)) = with_vm(world, |vm, _| vm.profile_report(10))
    {
        diag::info!(Sim, "bo2mp script prof (ms over 100 ticks): {line}");
    }
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

fn timed(label: &'static str, game: &mut World, fl: impl FnOnce(&mut World)) {
    phase(label);
    let t0 = std::time::Instant::now();
    fl(game);
    prof_add(label, t0);
}

fn advance_inner(world: &mut World) {
    let (advances, tick) = {
        let request = world.resource::<crate::frame_step::FrameTicket>();
        (request.reason.moves_server_world_forward(), request.tick)
    };
    if !advances {
        return;
    }
    // bo2mp: a round restart (`map_restart`) asked for last frame: the
    // level starts again before anything else runs.
    let restart_asked = world.resource_mut::<Zm>().pending_restart.take();
    if let Some(persist) = restart_asked {
        restart(world, tick, persist);
    }
    if world.contains_resource::<PendingEntities>() {
        start(world);
    }
    let now = i64::from(tick.0) * i64::from(crate::SIM_STEP_MILLIS);
    declassified::advance(world, now);
    world.resource_mut::<Zm>().clock_millis = now;
    start_hang_tracer();
    TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    timed("players", world, |world| {
        // (Its parts timed too: "p." in the profile line.)
        timed("p.sync", world, players::sync);
        // bo2mp gun brain: BO2's client events from the usercmds (reload_start, weapon_fired,
        // weapon_melee, detonate, ...), on the T6 gun state.
        timed("p.weapons", world, gun_script::frame_notifies);
        timed("p.charge", world, charge_shot::sync_dvars);
        timed("p.shock", world, players::shock_tick);
        // bo2mp: each player's third-person animations, as BO2's script picks.
        if world.resource::<Zm>().mp {
            timed("p.anim", world, playeranim::advance);
            timed("p.inhand", world, offhand_visible::advance);
        }
        timed("p.moves", world, players::movement_events);
        if !world.resource::<Zm>().debris_paths_cut {
            brushes::cut_debris_paths(world);
        }
    });
    timed("bots", world, |world| {
        // The test player's buttons go in before the triggers read them.
        autoplay::drive(world, now);
        if !world.resource::<Zm>().mp {
            cheats::apply(world);
            cheats::hold(world);
        }
        // bo2mp: console-asked bots go to BO2's spawn_bot before the bots
        // steer, then the bots' commands go in too.
        console_bots::spawn_waiting(world);
        if world.resource::<Zm>().mp {
            bots::drive(world, now);
        }
    });
    // Scorestreaks once every command is in (a bot capturing a crate
    // holds use): thrown and fired projectiles reach the scripts,
    // crafts fly, turrets aim.
    if world.resource::<Zm>().mp {
        timed("streaks", world, killstreaks::advance);
        // The hack menu's guest weapons: bow, stick, Gravity Gun hits.
        timed("guests", world, mp_specials::tick);
        // What a missile stuck to or bounced off hears it (the cars).
        timed("grenade stuck", world, grenade_stuck::advance);
        // The Sensor Grenade's scan, and whom it tracks.
        timed("sensor grenade", world, sensor_grenade::advance);
    }
    timed("tests", world, |world| autoplay_tests(world, now));
    if !world.resource::<Zm>().mp {
        timed("zm systems", world, |world| {
            darts::advance(world, now);
            darts::log(world, now);
            grenades::advance(world, now);
            // limits first: its BO2MP_T6_PRESS holds are test aids too.
            limits::tick(world);
            // After the test aids: their held fire spins it too.
            jetgun::tick(world, crate::SIM_STEP_MILLIS as f32);
            mc_specials::tick(world);
            throwables::tick(world);
        });
    }
    // Each timed apart: a stall names the one.
    timed("vehicles", world, |world| vehicles::advance(world, crate::SIM_STEP_MILLIS as f32 / 1000.0));
    timed("movers", world, |world| movers::advance(world, now));
    if !world.resource::<Zm>().mp {
        timed("riders", world, |world| riders::advance(world));
        timed("turrets", world, |world| turrets::advance(world));
        timed("nav movers", world, follow_nav_movers);
    }
    timed("zbarriers", world, |world| zbarrier::advance(world, now));
    timed("wallbuys", world, |world| wallbuys::advance(world, now));
    if world.resource::<Zm>().mp {
        timed("items", world, items::settle);
    }
    timed("triggers", world, triggers::dispatch);
    timed("trigger_hurt", world, triggers::hurt);
    timed("hud", world, |world| {
        publish_hud(world);
        hud::publish(world);
    });
    if world.resource::<Zm>().mp {
        timed("spawns", world, spawnsystem::advance);
    }
    timed("scripts", world, |world| {
        with_vm(world, |vm, world| {
            if vm.clock_stamp_ms < now {
                vm.clock_stamp_ms = now;
            }
            natives_game::deliver_timers(vm, world, now);
            natives_fx::free_timed_fx(vm, world, now);
            vm.run_due(world);
        });
        // bo2mp gun brain: BO2's per-frame weapon notifies come after the threads.
        gun_script::frame_watch(world);
    });
    // bo2mp lane C: his multiplayer screen's values, after the scripts as
    // real BO2's ClientEndFrame, so a killcam the scripts start this tick
    // shows its bands with the replay's first frame.
    if world.resource::<Zm>().mp {
        timed("screen", world, |world| {
            natives_mp::publish_screen(world);
            stats::save(world, false);
        });
    }
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
        // bo2mp: breaks first, so a prop that broke this tick leaves the
        // snapshot its debris flies in (BO2 swaps a dyn ent's model the
        // frame it breaks); after the sync it stood one snapshot more, a
        // second copy beside its own flying piece.
        destructible::flush(world);
        presence::sync(world, now);
        // The numbers clients know shown models and thrown projectiles by
        // (the killcam's, `killcam_seats`); a grenade's is gone by the time
        // its blast is dealt.
        let thrown: Vec<(u32, i32)> = crate::step_view::gather_shells(world)
            .into_iter()
            .filter_map(|p| Some((killstreaks::thrown_ent(world, p.id.0)?, p.ent_no)))
            .collect();
        let mut zm = world.resource_mut::<Zm>();
        let zm = &mut *zm;
        let shown = zm.presences.by_ent.iter().map(|(n, s)| (*n, s.num));
        for (n, number) in shown.chain(thrown) {
            let obj = zm.ents.get(&n).and_then(|e| e.obj);
            zm.wire_numbers.insert(n, (obj, number));
        }
        glass::flush(world);
        if !world.resource::<Zm>().mp {
            fxanims::advance(world, now);
            prison_wolves::advance(world, now);
            audio_triggers::advance(world, now);
        }
        brushes::sync(world, now);
        if world.resource::<Zm>().mp {
            triggers::publish_tac_insert_blockers(world);
        } else {
            players::follow_sync(world);
            publish_melee_targets(world);
        }
    });
    timed("loops+fx", world, |world| {
        presence::publish_loops(world);
        clientfields::advance(world, now);
        mp_fxanims::advance(world, now);
        semtex_light::advance(world, now);
        if !world.resource::<Zm>().mp {
            tomb_beacon::advance(world, now);
            tomb_staff_fx::advance(world, now);
            tomb_staff_charge::advance(world, now);
        }
    });
    phase("report");
    report(world);
    phase("idle");
}

/// A living zombie's feet by its entity number: a knife lunge's target (BO2 lunges at AI
/// the same as at players).
pub(crate) fn zombie_feet(world: &World, ent: u32) -> Option<[f32; 3]> {
    let zm = world.get_resource::<Zm>()?;
    zm.actors.get(&ent).filter(|a| a.living)?;
    zm.ents.get(&ent).map(|e| e.origin)
}

/// bo2zm: path nodes that sit on movers go where their mover goes (and their script
/// `origin` with them).
fn follow_nav_movers(world: &mut World) {
    let moved = {
        let mut zm = world.resource_mut::<Zm>();
        let zm = &mut *zm;
        let ents = &zm.ents;
        zm.nav
            .follow_movers(|n| ents.get(&n).map(|e| (e.origin, e.rot)))
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
                vm.set_raw_field(o, f, ScriptValue::Vec3(p));
            }
        });
    }
}

/// The test runs' hooks (each does nothing unless its test is on).
fn autoplay_tests(world: &mut World, now: i64) {
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
    destructible::prop_blast_test(world, now);
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
    glass_test::glass_test(world, now);
    autoplay::ent_log(world, now);
    autoplay::floor_map(world, now);
    autoplay::hole_scan(world, now);
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
}

/// bo2mp: he sits in a vehicle driven from afar (BO2's
/// `EF2_REMOTE_CONTROL`: the RC-XD's driver).
pub(crate) fn in_remote_vehicle(world: &World, client: u32) -> bool {
    world.get_resource::<Zm>().is_some_and(|z| z.mp) && killstreaks::in_remote_vehicle(world, client)
}

/// bo2mp: he steers a scorestreak from where he stands (a vehicle he
/// drives or rides, the Lodestar's view).
pub(crate) fn remote_operated(world: &World, client: u32) -> bool {
    world.get_resource::<Zm>().is_some_and(|z| z.mp) && killstreaks::remote_operated(world, client)
}

/// bo2mp: he pressed a d-pad slot's key (`+actionslot`, 0-based slot).
pub(crate) fn action_slot_pressed(game: &mut World, peer: u32, slot_no: u8) {
    if game.get_resource::<Zm>().is_some_and(|z| z.mp) {
        killstreaks::action_slot_pressed(game, peer, slot_no);
    }
}

/// bo2mp: a player picked a spot on the map for a scorestreak (or backed
/// out); BO2's scripts hear it. False when no BO2 match runs.
pub(crate) fn location_pick(game: &mut World, peer: u32, picked: [u8; 3], confirm: bool) -> bool {
    if !game.get_resource::<Zm>().is_some_and(|z| z.mp) {
        return false;
    }
    killstreaks::location_pick(game, peer, picked, confirm);
    true
}

/// bo2mp: the console's `give killstreak`, to BO2's own scripts. False when
/// no BO2 match runs.
pub(crate) fn give_killstreak(game: &mut World, peer: u32, menu_name: &str) -> bool {
    if !game.get_resource::<Zm>().is_some_and(|z| z.mp) {
        return false;
    }
    killstreaks::give(game, peer, menu_name);
    true
}

/// A client dvar for one player's client (its HUD, its music).
/// bo2zm: the living zombies near each player (`n x y z;...`: entity
/// number and feet, nearest first, at most 24 within 2000 units), for his
/// client's knife lunge (BO2 lunges at a zombie in reach and stabs; the old
/// lunge only looked for players) and the pad's aim assist (BO2 assists onto
/// zombies; the old one only onto players).
fn publish_melee_targets(world: &mut World) {
    const REACH: f32 = 2000.0;
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for n in players {
        let Some(me) = frame(world).avatar(ClientId(n)).map(|ps| ps.origin) else {
            continue;
        };
        let list = {
            let zm = world.resource::<Zm>();
            let mut close_by: Vec<(f32, u32, [f32; 3])> = zm
                .actors
                .iter()
                .where_value(|lhs| lhs.living)
                .filter_map(|(k, _)| zm.ents.get(k).map(|e| (*k, e.origin)))
                .map(|(k, o)| {
                    let d: [f32; 3] = std::array::from_fn(|i| o[i] - me[i]);
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2], k, o)
                })
                .filter(|(d2, _, _)| *d2 <= REACH * REACH)
                .collect();
            close_by.sort_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));
            close_by.iter()
                .take(24)
                .map(|(_, k, o)| format!("{k} {:.0} {:.0} {:.0}", o[0], o[1], o[2]))
                .collect::<Vec<_>>()
                .join(";")
        };
        set_client_dvar(world, n, "bo2zm_zombies", &list);
    }
}

pub(crate) fn set_client_dvar(game: &mut World, peer: u32, lookup_key: &str, setting: &str) {
    let mut f = frame(game);
    if f.client_meta(crate::sim_parts::ClientId(peer)).is_none() {
        return;
    }
    let dvars = &mut f
        .client_meta_mut(crate::sim_parts::ClientId(peer))
        .client_dvars;
    match dvars.iter_mut().find(|(k, _)| k == lookup_key) {
        Some(row) => setting.clone_into(&mut row.1),
        None => dvars.push((lookup_key.to_owned(), setting.to_owned())),
    }
}

fn report(world: &mut World) {
    let msgs = with_vm(world, |vm, _| std::mem::take(&mut vm.messages)).unwrap_or_default();
    for m in msgs {
        diag::warn!(Sim, "bo2zm t6: {m}");
    }
    // BO2MP_T6_THREADS=<seconds>: list every waiting thread that often.
    let every = std::env::var("BO2MP_T6_THREADS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok());
    if let Some(every) = every {
        let now = world.resource::<Zm>().clock_millis;
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
            for (l, nn) in tally(lines) {
                diag::info!(Sim, "bo2zm t6 thread x{nn}: {l}");
            }
        }
    }
    // BO2MP_T6_ROUND_JUMP=<nn>[@<secs>]: 10 s in (or then), set
    // level.round_number to nn (lhs test aid for things that only start in lhs
    // round-1 game, like the Nuketown transmission and blue eyes; or that
    // wait rounds, like TranZit's Avogadro in his cloud: `5@300`).
    static ROUND_JUMPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if let Some((nn, at)) = std::env::var("BO2MP_T6_ROUND_JUMP").ok().and_then(|v| {
        let (nn, at) = v.split_once('@').unwrap_or((&v, "10"));
        Some((nn.trim().parse::<i32>().ok()?, at.trim().parse::<f32>().ok()?))
    }) && world.resource::<Zm>().clock_millis >= (at * 1000.0) as i64
        && !ROUND_JUMPED.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        with_vm(world, |vm, world| {
            let f = vm.intern("round_number");
            let level = vm.level;
            vm.set_field(world, level, f, ScriptValue::Int(nn));
        });
        diag::info!(Sim, "bo2zm t6: round jumped to {nn}");
    }
    // BO2MP_T6_KILLS_ADD=<nn>: 10 s in, nn more kills on the books
    // (level.total_zombies_killed): the population sign counts them down.
    // BO2MP_T6_DOOR_GRAB_AT=<nn>: when the sign reads nn, the player steps onto
    // the power-up behind the door (Nuketown's first song egg).
    static KILLS_ADDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if let Some(nn) = std::env::var("BO2MP_T6_KILLS_ADD").ok().and_then(|v| v.parse::<i32>().ok())
        && world.resource::<Zm>().clock_millis >= 10_000
        && !KILLS_ADDED.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        with_vm(world, |vm, world| {
            let f = vm.intern("total_zombies_killed");
            let level = vm.level;
            let had = match vm.get_field(world, level, f) {
                ScriptValue::Int(i) => i,
                _ => 0,
            };
            vm.set_field(world, level, f, ScriptValue::Int(had + nn));
        });
        diag::info!(Sim, "bo2zm t6: {nn} kills added");
    }
    if let Some(want) = std::env::var("BO2MP_T6_DOOR_GRAB_AT").ok().and_then(|v| v.parse::<i32>().ok()) {
        let at = with_vm(world, |vm, world| {
            let level = vm.level;
            let f = vm.intern("population_count");
            let pop = vm.get_field(world, level, f);
            let f = vm.intern("door_powerup");
            let pu = match vm.get_field(world, level, f) {
                ScriptValue::Object(o) if vm.alive(o) => Some(o),
                _ => None,
            };
            let f = vm.intern("origin");
            let origin = pu.map(|o| vm.get_field(world, o, f));
            (pop, origin)
        });
        if let Some((ScriptValue::Int(pop), Some(ScriptValue::Vec3(o)))) = at
            && pop == want
        {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                diag::info!(Sim, "bo2zm t6 test: sign at {pop}, grabbing the door power-up at {o:?}");
            }
            teleport_player(world, crate::sim_parts::ClientId(0), [o[0], o[1], o[2] - 40.0]);
        }
    }
    // BO2MP_T6_SETLEVEL=field=int,...: set on level at 15 s (lhs test aid:
    // Die Rise's next_leaper_round=2 brings lhs Jumping Jack round early).
    if let Ok(sets) = std::env::var("BO2MP_T6_SETLEVEL") {
        let now = world.resource::<Zm>().clock_millis;
        if (15000..15000 + i64::from(crate::SIM_STEP_MILLIS)).contains(&now) {
            with_vm(world, |vm, _| {
                for (k, v) in sets.split(',').filter_map(|kv| kv.split_once('=')) {
                    let Ok(v) = v.parse::<i32>() else {
                        continue;
                    };
                    let f = vm.intern(k);
                    vm.set_raw_field(vm.level, f, ScriptValue::Int(v));
                    diag::info!(Sim, "bo2zm t6 setlevel {k} = {v}");
                }
            });
        }
    }
    // BO2MP_T6_EVAL=level.a.b,...: those values every 5 s (debugging; roots level, anim, player).
    if let Ok(paths) = std::env::var("BO2MP_T6_EVAL") {
        let now = world.resource::<Zm>().clock_millis;
        if now % 5000 < i64::from(crate::SIM_STEP_MILLIS) {
            let lines = with_vm(world, |vm, world| {
                let mut out = Vec::new();
                for path in paths.split(',') {
                    let mut parts = path.split('.');
                    let mut v = match parts.next() {
                        Some("level") => ScriptValue::Object(vm.level),
                        Some("anim") => ScriptValue::Object(vm.anim),
                        Some("player") => {
                            let Some(first) = world.resource::<Zm>().players.get(&0) else { continue };
                            ScriptValue::Object(first.obj)
                        }
                        _ => continue,
                    };
                    for p in parts {
                        v = match &v {
                            ScriptValue::Object(o) => {
                                let f = vm.intern(p);
                                vm.get_field(world, *o, f)
                            }
                            ScriptValue::Array(lhs) => {
                                let k = match p.parse::<i32>() {
                                    Ok(i) => gsc_t6::Key::Int(i),
                                    Err(_) => gsc_t6::Key::Str(vm.intern(p)),
                                };
                                lhs.get(&k).unwrap_or_default()
                            }
                            _ => ScriptValue::Undefined,
                        };
                    }
                    let text = match &v {
                        ScriptValue::Array(lhs) => {
                            let snap = lhs.snapshot();
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
    // BO2MP_T6_ACTORS=<ms>: every actor's place, animscript and animation.
    let every = std::env::var("BO2MP_T6_ACTORS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok());
    if let Some(every) = every {
        let zm = world.resource::<Zm>();
        let now = zm.clock_millis;
        if now % every.max(50) < i64::from(crate::SIM_STEP_MILLIS) {
            for (nn, lhs) in &zm.actors {
                let e = zm.ents.get(nn);
                let o = e.map_or([0.0; 3], |e| e.origin);
                let anim = lhs.playing.as_ref().map_or("-".to_owned(), |p| {
                    format!("{}/{}:{}", p.state, p.substate, p.anim)
                });
                let next = lhs.path.get(lhs.path_i).copied().unwrap_or([0.0; 3]);
                let goal = lhs.goal.unwrap_or([0.0; 3]);
                diag::info!(
                    Sim,
                    "bo2zm t6 actor {nn} at {}ms: ({:.0} {:.0} {:.0}) yaw {:.0} script {} anim {} hp {} alive {} path {}/{} next ({:.0} {:.0} {:.0}) goal ({:.0} {:.0} {:.0}) scripted {} traverse {} model {}",
                    now,
                    o[0],
                    o[1],
                    o[2],
                    e.map_or(0.0, |e| e.rot[1]),
                    lhs.script,
                    anim,
                    lhs.hp,
                    lhs.living,
                    lhs.path_i,
                    lhs.path.len(),
                    next[0],
                    next[1],
                    next[2],
                    goal[0],
                    goal[1],
                    goal[2],
                    lhs.scripted.is_some(),
                    lhs.traverse.is_some(),
                    e.map_or("", |e| e.mesh.as_str())
                );
            }
            // BO2MP_T6_ACTOR_FIELDS=lhs,b,...: those script fields of each actor too.
            if let Ok(fields) = std::env::var("BO2MP_T6_ACTOR_FIELDS") {
                let objs: Vec<(u32, ObjRef)> = zm
                    .actors
                    .iter()
                    .filter_map(|(nn, lhs)| lhs.obj.map(|o| (*nn, o)))
                    .collect();
                let lines = with_vm(world, |vm, world| {
                    objs.iter()
                        .map(|(nn, o)| {
                            let vals: Vec<String> = fields
                                .split(',')
                                .map(|f| {
                                    let id = vm.intern(f);
                                    let v = vm.get_field(world, *o, id);
                                    format!("{f}={}", vm.to_text(&v))
                                })
                                .collect();
                            format!("{nn}: {}", vals.join(" "))
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

fn unbound(vm: &mut Vm<World>, game: &mut World, cid: u32, _self: &ScriptValue, args: &[ScriptValue]) -> ScriptValue {
    let (name, method) = vm.program.builtins[cid as usize];
    let n = format!("{}{}", if method { "." } else { "" }, vm.str(name));
    let mut zm = game.resource_mut::<Zm>();
    let count = zm.unbound.entry(n.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        let a: Vec<String> = args.iter().map(|a| vm.to_text(a)).collect();
        diag::warn!(Sim, "bo2zm t6: unbound builtin {n}({})", a.join(", "));
    }
    ScriptValue::Undefined
}

/// Black Ops II's hit locations, in the order its models' bones name them
/// (part classification) and its weapons' location multipliers run: one
/// more than the legacy list (torso_mid), so the legacy names would be off from
/// torso_mid on. (t6mp's own table.)
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

/// How many times each item comes up, in the items' order.
pub(crate) fn tally<K: Ord>(items: impl IntoIterator<Item = K>) -> BTreeMap<K, usize> {
    items.into_iter().fold(BTreeMap::new(), |mut seen, item| {
        *seen.entry(item).or_default() += 1;
        seen
    })
}

/// The name the scripts know hit location `part` by; "none" past the table.
pub(crate) fn hitloc_name(part: u8) -> &'static str {
    match T6_HITLOC_NAMES.get(usize::from(part)) {
        Some(name) => name,
        None => T6_HITLOC_NAMES[0],
    }
}

/// bo2mp: the weapon a multiplayer knife hit names to the scripts, and
/// the least it does. BO2's melee button slashes with the knife every
/// class carries (knife_mp, given as the melee weapon; its file's
/// iMeleeDamage 150 - a kill), whatever gun is up (the guns' own
/// iMeleeDamage is 25); the riot shield bashes and a held knife stabs
/// with their own. The gun's row already melees with the knife's damage
/// (the combat table's melee weapon); the scripts are told it was the
/// knife - unless the gun carries a Tactical Knife (`+tacknife`): BO2
/// names the gun, and its scripts count the kill as the Tactical
/// Knife's (`weaponhasattachment(weapon, "tacknife")` on MOD_MELEE).
/// A player and a K9 Unit dog are told the same.
fn mp_melee(game: &mut World, hitter: Option<u32>, gun: u32) -> (String, i32) {
    let mut wname = weapon_text_of(game, hitter, gun);
    let mut least = 0;
    let tacknife = wname.split('+').skip(1).any(|att| att == "tacknife");
    let f = frame(game);
    if let Some(facts) = f.damage_profile_of(gun)
        && facts.blade_mesh != 0
        && facts.blade_mesh != gun
    {
        least = facts.bash_hurt;
        if !tacknife {
            wname = crate::client_script::weapon_name(&f, facts.blade_mesh);
        }
    }
    (wname, least)
}

/// bo2zm M3: engine damage on a player (fall, explosion, ...) goes to
/// the BO2 scripts' CodeCallback_PlayerDamage; false when they don't run.
pub(crate) fn player_hit(world: &mut World, hit: &crate::client_script::Hit) -> bool {
    if !world.contains_resource::<T6Runtime>() {
        return false;
    }
    // The shield's own hit location is the last name in the table, so a hit
    // the riot shield stopped reads as the shield too.
    let hitloc = hitloc_name(hit.hit_part);
    let mut wname = weapon_text_of(world, hit.striker.map(|a| a.0), hit.gun);
    let mut amount = hit.amount;
    // BO2's G_Damage names the projectile that went off as the inflictor
    // (its scripts follow it, or its `killcament`, in the killcam); the
    // attacker is the inflictor only for his own bullets and blade.
    let inflictor = hit.inflictor.and_then(|id| {
        let ent = killstreaks::thrown_ent(world, id.0)?;
        let zm = world.resource::<Zm>();
        let obj = zm.ents.get(&ent).and_then(|e| e.obj);
        diag::info!(
            Sim,
            "bo2mp inflictor: projectile {} is script entity {ent} ({}), sent as {:?}",
            id.0,
            if obj.is_some() { "live" } else { "gone" },
            zm.wire_numbers.get(&ent).map(|w| w.1)
        );
        obj.map(ScriptValue::Object)
    });
    if hit.means == "MOD_MELEE" && world.resource::<Zm>().mp {
        let (name, least) = mp_melee(world, hit.striker.map(|a| a.0), hit.gun);
        wname = name;
        amount = amount.max(least);
    }
    if std::env::var_os("BO2MP_HITLOG").is_some() {
        diag::info!(
            Sim,
            "bo2mp hit: player {} by {:?} with {wname} ({}): {hitloc} for {amount}",
            hit.victim.0,
            hit.striker.map(|a| a.0),
            hit.means
        );
    }
    with_vm(world, |vm, world| {
        // bo2mp: with no attacker (a fall), BO2's `G_Damage` names the world
        // entity, and the death scripts read its classname, "worldspawn".
        let zm = world.resource::<Zm>();
        let attacker = match hit.striker {
            Some(a) => zm.players.get(&a.0).map(|p| ScriptValue::Object(p.obj)),
            None => zm
                .ents
                .values()
                .find(|e| e.class_tag == "worldspawn")
                .and_then(|e| e.obj)
                .map(ScriptValue::Object),
        }
        .unwrap_or(ScriptValue::Undefined);
        natives_ai::player_damage(
            vm,
            world,
            hit.victim.0,
            inflictor.unwrap_or_else(|| attacker.clone()),
            attacker,
            amount,
            hit.bits,
            hit.means,
            &wname,
            hit.point,
            hit.way,
            hitloc,
        );
    });
    // bo2mp: G_DamageClient tells the spawn system right after the damage
    // callback, not for a player in last stand.
    if !world.resource::<Zm>().players.get(&hit.victim.0).is_some_and(|p| p.laststand) {
        spawnsystem::player_damaged(world, hit.victim.0);
    }
    true
}

/// A broken prop's piece (a mannequin's head, BO2's stage spawn model)
/// flies off its bone, tumbling, lands on the floor below and lies there
/// 20 s, as BO2's physics debris does.
pub(crate) fn throw_debris(game: &mut World, mdl: &str, from: [f32; 3], aim: [f32; 3], floor: f32) {
    if !game.contains_resource::<T6Runtime>() {
        return;
    }
    let now = game.resource::<Zm>().clock_millis;
    let g = game.resource_mut::<Zm>().alloc_entnum();
    // Out that way, a little to one side (by its number), and up: a short
    // hop that lands within about 40 units.
    let (fx, fy) = (aim[0], aim[1]);
    let len = (fx * fx + fy * fy).sqrt().max(0.001);
    let side = (g % 7) as f32 / 3.0 - 1.0;
    let (hx, hy) = (fx / len - fy / len * side * 0.3, fy / len + fx / len * side * 0.3);
    let motion_vec = [hx * 55.0, hy * 55.0, 160.0];
    // Flight time to the floor: z(t) = z0 + vz*t - 400*t^2.
    let drop = (from[2] - floor).max(0.0);
    let secs = ((motion_vec[2] + (motion_vec[2] * motion_vec[2] + 1600.0 * drop).sqrt()) / 800.0).clamp(0.2, 3.0);
    let yw = fy.atan2(fx).to_degrees();
    let obj = with_vm(game, |vm, _| vm.alloc_object(ObjKind::Entity(g)));
    let Some(obj) = obj else {
        return;
    };
    let spin = if g % 2 == 0 { 1.0 } else { -1.0 };
    let mut zm = game.resource_mut::<Zm>();
    zm.ents.insert(
        g,
        Ent {
            obj: Some(obj),
            class_tag: "script_model".into(),
            origin: from,
            rot: [0.0, yw, 0.0],
            mesh: mdl.into(),
            ..Default::default()
        },
    );
    zm.movers.gravity(g, from, motion_vec, now, secs);
    zm.movers.rotate_to(g, [0.0, yw, 0.0], [90.0 * spin, yw + 200.0 * spin, 0.0], now, secs, 0.0, 0.0);
    zm.gibs.push((g, now + (secs * 1000.0) as i64 + 20000));
    let at = [from[0] + motion_vec[0] * secs, from[1] + motion_vec[1] * secs, floor];
    diag::info!(Sim, "bo2zm t6 debris {mdl} thrown from {from:?}, lands in {secs:.2}s at {at:?}");
}

/// bo2zm M3: a bullet (or a knife, a blast) on a script model that shows a
/// BO2 entity: an actor takes it through CodeCallback_ActorDamage with the
/// hit location of the bone it struck (the weapon's location multiplier
/// applied, as the engine does); false when the model isn't one of ours.
/// A map prop's breakable piece broke (`destructibledef` stage with a
/// break notify): the engine's `codecallback_destructibleevent("broken",
/// notify, attacker, weapon)` on the prop (the Nuketown mannequin heads
/// are "headless").
pub(crate) fn destructible_broken(game: &mut World, tgt: crate::ScriptModelId, notify: &str, hitter: Option<ClientId>, gun: u32) {
    if !game.contains_resource::<T6Runtime>() {
        return;
    }
    let Some(n) = game
        .resource::<Zm>()
        .presences
        .by_ent
        .iter()
        .find(|(_, s)| s.id == tgt)
        .map(|(n, _)| *n)
    else {
        return;
    };
    let Some(obj) = game.resource::<Zm>().ents.get(&n).and_then(|e| e.obj) else {
        return;
    };
    let w = weapon_text(game, gun);
    diag::info!(Sim, "bo2zm t6 destructible ent{n} broken {notify}");
    with_vm(game, |vm, game| {
        let att = hitter
            .and_then(|a| {
                game
                    .resource::<Zm>()
                    .players
                    .get(&a.0)
                    .map(|p| ScriptValue::Object(p.obj))
            })
            .unwrap_or(ScriptValue::Undefined);
        let args = vec![vm.string("broken"), vm.string(notify), att, vm.string(&w)];
        vm.spawn_named(
            game,
            "maps/mp/zombies/_zm",
            "codecallback_destructibleevent",
            ScriptValue::Object(obj),
            args,
        );
    });
}

pub(crate) fn entity_hit(world: &mut World, hit: &crate::gsc_host::EntityHit) -> bool {
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
        .or_else(|| brushes::ent_of(world, hit.target))
    else {
        return false;
    };
    let living = world
        .resource::<Zm>()
        .actors
        .get(&n)
        .is_some_and(|a| a.living);
    // BO2's G_DamageActor takes no damage below 1 (a Paralyzer shot: its
    // script does the harm), so no callback and no "damage".
    if living && hit.amount <= 0 {
        return true;
    }
    if !living {
        // bo2mp: a map's breakable (a Nuketown mannequin, a car) takes a
        // blast or a knife by the piece's own scale.
        let breakable = world
            .resource::<Zm>()
            .ents
            .get(&n)
            .is_some_and(|e| e.destructible.is_some());
        // bo2mp: a scorestreak's plane, chopper, drone, vehicle or turret:
        // its scripts hear the hit.
        if !breakable && killstreaks::damage_target(world, n) {
            let weapon = weapon_text_of(world, hit.attacker.map(|a| a.0), hit.gun);
            killstreaks::damage_entity(
                world,
                n,
                hit.amount,
                hit.attacker,
                hit.means,
                &weapon,
                hit.point,
                hit.way,
                hit.bits,
            );
            return true;
        }
        // A blast's share reached the pieces already (damage.rs, BO2's
        // DestructibleRadiusDamage); BO2's G_Damage skips DestructibleDamage
        // for radius damage (dflags & 1).
        if breakable && hit.bits & 1 == 0 {
            let t = tick(world);
            crate::staged_props::apply_damage(
                &mut frame(world),
                t,
                crate::ServerModelSource::ScriptModel(hit.target),
                hit.bone.and_then(|b| u16::try_from(b).ok()),
                hit.amount,
                destructible::kind(hit.means),
                crate::glass_and_props::t6_means_from_name(hit.means),
                hit.point,
                Some(hit.way),
                hit.bits,
                hit.attacker,
                hit.gun,
            );
        }
        return true;
    }
    // The struck bone's hit location.
    let part = hit.bone.and_then(|bone| {
        let f = frame(world);
        f.entity_hit_support()
            .iter()
            .find(|row| row.owner.scripted_model() == Some(hit.target))
            .and_then(|row| row.dobj.as_ref())
            .and_then(|d| d.live_hitbox.as_ref())
            .and_then(|c| c.bones.iter().find(|b| usize::from(b.bone) == bone))
            .map(|b| b.body_part_category)
    });
    // A blast strikes no bone: BO2 calls its location "none" and its point
    // is the blast's centre, so the scripts tear off the limb nearest the
    // blast (a grenade at the feet takes the legs: a crawler).
    let blast = part.is_none() && hit.bits & 1 != 0;
    let part = part.unwrap_or(4);
    let hitloc = if blast {
        "none"
    } else {
        hitloc_name(part)
    };
    let point = if blast {
        std::array::from_fn(|i| hit.point[i] - hit.way[i])
    } else {
        hit.point
    };
    // The weapon's location multiplier (not for a knife).
    let melee = hit.means == "MOD_MELEE";
    let scale = if melee {
        1.0
    } else {
        let f = frame(world);
        f.damage_profile_of(hit.gun)
            .map_or(1.0, |facts| facts.location_scale(part))
    };
    let amount = if melee {
        hit.amount.max(1)
    } else {
        weapon_t6::hitloc::scaled_damage(hit.amount, scale)
    };
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
    let mut amount = match held_knife.as_deref().map(|k| weapon(world, k)) {
        Some(Ok(knife)) if knife != 0 => {
            let f = frame(world);
            let own = f.damage_profile_of(knife).map_or(0, |k| k.bash_hurt);
            match f.damage_profile_of(hit.gun) {
                Some(gun) if gun.blade_mesh != 0 && own > 0 => {
                    (amount - gun.bash_hurt + own).max(1)
                }
                _ => amount,
            }
        }
        _ => amount,
    };
    let own_melee = melee
        && frame(world)
            .damage_profile_of(hit.gun)
            .is_some_and(|f| f.blade_mesh == 0);
    // In multiplayer, the K9 Unit dog is told what a player would be.
    let weapon = if melee && world.resource::<Zm>().mp {
        let (name, least) = mp_melee(world, hit.attacker.map(|a| a.0), hit.gun);
        amount = amount.max(least);
        name
    } else if melee && !own_melee {
        held_knife.unwrap_or_else(|| "knife_zm".to_owned())
    } else {
        weapon_text(world, hit.gun)
    };
    if melee || std::env::var("BO2MP_T6_HITLOG").is_ok() {
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
                    .map(|p| ScriptValue::Object(p.obj))
            })
            .unwrap_or(ScriptValue::Undefined);
        let inflictor = attacker.clone();
        actors::damage(
            vm,
            world,
            n,
            inflictor,
            attacker,
            amount,
            hit.bits,
            hit.means,
            &weapon,
            point,
            hit.way,
            hitloc,
        );
    });
    true
}

/// A script string as the player reads it: a localized key (`&"KEY"` or a
/// bare `KEY`) becomes its English text with `&&1`.. filled from `args`
/// (themselves localized when they are keys); other values print as is.
pub(crate) fn localize(vm: &Vm<World>, world: &World, v: &ScriptValue, args: &[ScriptValue]) -> String {
    let zm = world.resource::<Zm>();
    let raw = vm.to_text(v);
    let key = raw.trim_start_matches('&');
    let mut out = match zm.strings.get(&key.to_ascii_uppercase()) {
        Some(t) => t.clone(),
        None if matches!(v, ScriptValue::IStr(_)) => key.to_owned(),
        None => raw.clone(),
    };
    for (i, a) in args.iter().enumerate() {
        let text = match a {
            ScriptValue::IStr(_) => {
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
pub(crate) fn name_players(vm: &mut Vm<World>, game: &mut World, argv: &[ScriptValue]) -> Vec<ScriptValue> {
    argv.iter()
        .map(|a| match client(vm, game, a) {
            Ok(cid) => {
                let name = frame(game)
                    .client_meta(cid)
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
    const OFFHANDS: [&str; 16] = [
        "frag_grenade_zm",
        "sticky_grenade_zm",
        "cymbal_monkey_zm",
        "claymore_zm",
        // bo2mp: a multiplayer class's lethal then tactical equipment.
        "frag_grenade_mp",
        "sticky_grenade_mp",
        "hatchet_mp",
        "claymore_mp",
        "satchel_charge_mp",
        "bouncingbetty_mp",
        "flash_grenade_mp",
        "concussion_grenade_mp",
        "willy_pete_mp",
        "emp_grenade_mp",
        "trophy_system_mp",
        "tactical_insertion_mp",
    ];
    let f = frame(world);
    let id = crate::sim_parts::ClientId(c);
    let held = crate::client_script::weapons(&f, id, crate::client_script::WeaponList::All);
    let mut out: Vec<String> = Vec::new();
    for want in OFFHANDS {
        for &w in &held {
            let name = crate::client_script::weapon_name(&f, w);
            if name == want || name.trim_end_matches("_mp") == want {
                let (clip, stock) = (
                    crate::client_script::ammo_clip(&f, id, w),
                    crate::client_script::rounds_held(&f, id, w),
                );
                if std::env::var_os("BO2MP_T6_HINTLOG").is_some() {
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
    let id = crate::sim_parts::ClientId(c);
    let mut f = frame(world);
    if f.client_meta(id).is_none() {
        return;
    }
    let heat = crate::gun_brain::script_read(&f, id, |ps, facts| {
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
    let mut table_rows = Vec::new();
    for c in clients {
        let (name, score, kills) = {
            let f = frame(world);
            let Some(m) = f.client_meta(crate::sim_parts::ClientId(c)) else {
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
                m.frags,
            )
        };
        let zm = world.resource::<Zm>();
        let stat = |k: &str| {
            zm.players
                .get(&c)
                .and_then(|p| p.stats.get(k).copied())
                .unwrap_or(0)
        };
        let (downs, revives, headshots) = (stat("downs"), stat("revives"), stat("headshots"));
        table_rows.push(format!("{c}:{name}:{score}:{kills}:{downs}:{revives}:{headshots}"));
    }
    table_rows.join(";")
}

/// A bot (`addtestclient`) has no screen: the server builds it no HUD
/// (BO2 sends a test client none).
pub(crate) fn is_bot(world: &World, c: u32) -> bool {
    world.resource::<Zm>().players.get(&c).is_some_and(|p| p.test_client)
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
    .map_or(String::from("0"), |n| n.to_string());
    // The game type (the HUD's Turned and Grief checks read it).
    let gametype = with_vm(world, |vm, _| vm.dvars.get("g_gametype").cloned())
        .flatten()
        .unwrap_or_default();
    let board = scoreboard_rows(world);
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    // bo2mp: the scorestreak column's default picks.
    let picks = loadout::streak_picks(world);
    for c in clients {
        if is_bot(world, c) {
            continue;
        }
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
        // bo2mp: the scorestreak column: his picks (once) and his momentum.
        let momentum = world
            .resource::<Zm>()
            .players
            .get(&c)
            .and_then(|p| p.stats.get("momentum").copied())
            .map_or(String::from("0"), |m| m.to_string());
        // His own picks, each lit while he holds it; the defaults until his
        // class script has given him his.
        let picks = with_vm(world, |vm, world| loadout::his_streaks(vm, world, c))
            .flatten()
            .unwrap_or_else(|| picks.clone());
        if !picks.is_empty() {
            let mut f = frame(world);
            if f.client_meta(crate::sim_parts::ClientId(c)).is_some() {
                let dvars = &mut f.client_meta_mut(crate::sim_parts::ClientId(c)).client_dvars;
                for (name, value) in [("bo2mp_streaks", picks.clone()), ("bo2mp_momentum", momentum)] {
                    match dvars.iter_mut().find(|(k, _)| k == name) {
                        Some(row) => row.1 = value,
                        None => dvars.push((name.to_owned(), value)),
                    }
                }
            }
        }
        let sent = world.resource::<Zm>().hud_sent.get(&c).cloned();
        if sent.as_ref().is_some_and(|(r, h, o, a, b)| {
            *r == round && *h == hint && *o == offhand && *a == slots && *b == board
        }) {
            continue;
        }
        let mut f = frame(world);
        if f.client_meta(crate::sim_parts::ClientId(c)).is_none() {
            continue;
        }
        if std::env::var_os("BO2MP_T6_HINTLOG").is_some() {
            diag::info!(
                Sim,
                "bo2zm t6 hud out for {c}: round {round:?} hint {hint:?} offhand {offhand:?} scoreboard {board:?}"
            );
        }
        let dvars = &mut f.client_meta_mut(crate::sim_parts::ClientId(c)).client_dvars;
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
pub(crate) fn teleport_player(world: &mut World, peer: crate::sim_parts::ClientId, spot: [f32; 3]) {
    let mut f = frame(world);
    f.place_at(peer, spot);
    if let Some(ps) = f.occupant_mut(peer) {
        ps.e_flags ^= playerstate_t6::game::entity_flag_bits::TELEPORT;
    }
}

/// Turn a player's view from the server (`setplayerangles`): the delta
/// under his last command's angles, so his client's view turns too.
pub(crate) fn set_player_view(game: &mut World, peer: crate::sim_parts::ClientId, view: [f32; 3]) {
    let mut f = frame(game);
    let cmd = f
        .prev_input_angles_mut()
        .iter()
        .find(|(c, _)| *c == peer)
        .map(|(_, a)| *a);
    if let Some(ps) = f.occupant_mut(peer) {
        if let Some(cmd) = cmd {
            ps.view_offset_angles =
                std::array::from_fn(|i| view[i] - (cmd[i] as u16) as f32 * (360.0 / 65536.0));
        }
        ps.look_angles = view;
    }
}

/// bo2mp: a blast on a scorestreak vehicle `id` (BO2's `G_RadiusDamage`
/// hands it to the scripts' vehicle radius callback); false when no BO2 map
/// runs or `id` is not a vehicle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blast_vehicle(
    game: &mut World,
    id: crate::ScriptModelId,
    spot: [f32; 3],
    reach: f32,
    max: f32,
    min: f32,
    cone: Option<([f32; 3], f32)>,
    means: &str,
    hitter: Option<crate::sim_parts::ClientId>,
    gun: u32,
) -> bool {
    if !game.contains_resource::<T6Runtime>() {
        return false;
    }
    let Some(n) = game
        .resource::<Zm>()
        .presences
        .by_ent
        .iter()
        .find(|(_, p)| p.id == id)
        .map(|(n, _)| *n)
    else {
        return false;
    };
    if !killstreaks::is_vehicle(game, n) {
        return false;
    }
    let gun = weapon_text_of(game, hitter.map(|lhs| lhs.0), gun);
    killstreaks::vehicle_blast(game, n, spot, reach, max, min, cone, means, hitter, &gun);
    true
}

/// bo2zm M3: the living zombies a blast at `origin` reaches: (their shown
/// model, their middle, distance), nearest first. As BO2's engine does, the
/// distance is to the nearest point of each one's box, not its middle: the
/// Sliquifier's bolt (radius 25) goes off on the floor at a zombie's feet.
pub(crate) fn radius_targets(
    game: &mut World,
    spot: [f32; 3],
    reach: f32,
) -> Vec<(crate::ScriptModelId, [f32; 3], f32)> {
    if !game.contains_resource::<T6Runtime>() {
        return Vec::new();
    }
    let zm = game.resource::<Zm>();
    let mut dest: Vec<(crate::ScriptModelId, [f32; 3], f32)> = zm
        .actors
        .iter()
        .where_value(|lhs| lhs.living)
        .filter_map(|(nn, lhs)| {
            let e = zm.ents.get(nn)?;
            let id = zm.presences.by_ent.get(nn)?.id;
            let o = e.origin;
            let mid = [o[0], o[1], o[2] + lhs.height * 0.5];
            let lo = [o[0] - lhs.reach, o[1] - lhs.reach, o[2]];
            let hi = [o[0] + lhs.reach, o[1] + lhs.reach, o[2] + lhs.height];
            let near: [f32; 3] = std::array::from_fn(|i| spot[i].clamp(lo[i], hi[i]));
            let d = gsc_t6::math::length(gsc_t6::math::sub(near, spot));
            (d < reach).then_some((id, mid, d))
        })
        .collect();
    // bo2mp: and the map's breakables, measured to their nearest part.
    dest.extend(destructible::radius_targets(game, spot, reach));
    // bo2mp: and the scorestreaks' planes, drones, vehicles and turrets.
    let streak_ents: Vec<(crate::ScriptModelId, [f32; 3], f32)> = {
        let zm = game.resource::<Zm>();
        zm.presences
            .by_ent
            .iter()
            .filter(|(nn, _)| !zm.actors.contains_key(nn) && killstreaks::damage_target(game, **nn))
            .filter_map(|(nn, p)| {
                let at = zm.ents.get(nn)?.origin;
                let d = gsc_t6::math::length(gsc_t6::math::sub(at, spot));
                (d < reach).then_some((p.id, at, d))
            })
            .collect()
    };
    dest.extend(streak_ents);
    dest.sort_by(|lhs, rhs| lhs.2.total_cmp(&rhs.2));
    dest
}

// ---- helpers for natives -------------------------------------------------

pub(crate) fn arg(a: &[ScriptValue], i: usize) -> &ScriptValue {
    a.get(i).unwrap_or(&ScriptValue::Undefined)
}

pub(crate) fn num(a: &[ScriptValue], i: usize) -> Result<f32, String> {
    arg(a, i).as_float().ok_or_else(|| {
        format!(
            "argument {} is {}, not a number",
            i + 1,
            arg(a, i).type_name()
        )
    })
}

pub(crate) fn int(a: &[ScriptValue], i: usize) -> Result<i32, String> {
    arg(a, i).as_int().ok_or_else(|| {
        format!(
            "argument {} is {}, not a number",
            i + 1,
            arg(a, i).type_name()
        )
    })
}

pub(crate) fn vec3(lhs: &[ScriptValue], ix: usize) -> Result<[f32; 3], String> {
    arg(lhs, ix).as_vec3().ok_or_else(|| {
        format!(
            "argument {} is {}, not a vector",
            ix + 1,
            arg(lhs, ix).type_name()
        )
    })
}

pub(crate) fn text(vm: &Vm<World>, a: &[ScriptValue], i: usize) -> String {
    match arg(a, i) {
        ScriptValue::Undefined => String::new(),
        v => vm.to_text(v),
    }
}

pub(crate) fn flag(a: &[ScriptValue], i: usize, default: bool) -> bool {
    match arg(a, i) {
        ScriptValue::Undefined => default,
        v => gsc_t6::truthy(v),
    }
}

pub(crate) fn list(vals: Vec<ScriptValue>) -> ScriptValue {
    let mut a = gsc_t6::Array::new();
    for v in vals {
        a.push(v);
    }
    ScriptValue::array(a)
}

/// The entity number of an entity object.
pub(crate) fn entnum(vm: &Vm<World>, v: &ScriptValue) -> Option<u32> {
    match v {
        ScriptValue::Object(o) => match vm.kind(*o)? {
            ObjKind::Entity(n) => Some(n),
            _ => None,
        },
        _ => None,
    }
}

/// The client of a player object.
pub(crate) fn client(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Result<ClientId, String> {
    let n = entnum(vm, v).ok_or("not an entity")?;
    if world.resource::<Zm>().players.contains_key(&n) {
        Ok(ClientId(n))
    } else {
        Err("not a player".into())
    }
}

pub(crate) fn is_player(vm: &Vm<World>, world: &World, v: &ScriptValue) -> bool {
    entnum(vm, v).is_some_and(|n| world.resource::<Zm>().players.contains_key(&n))
}

/// BO2's grenade suicide in a multiplayer match (ClientEvents: a grenade held past its fuse):
/// player_die with 100000 damage, MOD_SUICIDE by themselves, or MOD_GRENADE by the thrower when
/// it was thrown back; no direction, no hit location. It goes the way finishplayerdamage's kill
/// goes: the death animation, the engine's death (which lets the grenade go), the "death"
/// notify, then BO2's CodeCallback_PlayerKilled (kill feed, score, respawn). False when no T6
/// match runs this player, so the caller can use the old path.
pub(crate) fn grenade_suicide(
    game: &mut World,
    tick_no: crate::Tick,
    client: u32,
    thrower: Option<u32>,
    gun: u32,
) -> bool {
    let Some(zm) = game.get_resource::<Zm>() else { return false };
    if !zm.mp {
        return false;
    }
    let Some(obj) = zm.players.get(&client).map(|p| p.obj) else { return false };
    let by = thrower.and_then(|t| zm.players.get(&t).map(|p| (t, p.obj)));
    let cid = ClientId(client);
    let alive = frame(game)
        .client_meta(cid)
        .is_some_and(|m| m.lifecycle == crate::PlayerSessionStage::Alive);
    diag::info!(Sim, "t6 grenade suicide: client {client} alive {alive} thrower {thrower:?} weapon {gun}");
    if !alive {
        return true;
    }
    let means = if by.is_some() { "MOD_GRENADE" } else { "MOD_SUICIDE" };
    let wname = crate::client_script::weapon_name(&frame(game), gun);
    with_vm(game, |vm, game| {
        self_death(vm, game, tick_no, client, obj, by, means, &wname)
    });
    true
}

/// bo2mp: BO2's player_die for a death with no direction and no hit location, 100000 damage
/// (a grenade held past its fuse, a team switch's `suicide`): the death animation, the
/// engine's death, the "death" notify (the attacker, not a headshot, the weapon's name), then
/// CodeCallback_PlayerKilled. `by` is the thrower of a grenade thrown back, else he did it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn self_death(
    vm: &mut Vm<World>,
    game: &mut World,
    tick_no: crate::Tick,
    client: u32,
    obj: ObjRef,
    by: Option<(u32, ObjRef)>,
    means: &str,
    wname: &str,
) {
    let id = ClientId(client);
    playeranim::death(game, id, means, None, wname);
    let killer = by.map_or(id, |(t, _)| ClientId(t));
    crate::client_script::kill(&mut frame(game), tick_no, id, Some(killer), None);
    let me = ScriptValue::Object(obj);
    let attacker = by.map_or(me.clone(), |(_, o)| ScriptValue::Object(o));
    let w = vm.string(wname);
    vm.notify_str(game, obj, "death", &[attacker.clone(), ScriptValue::Int(0), w.clone()]);
    let inflictor = attacker.clone();
    let args = vec![
        inflictor,
        attacker,
        ScriptValue::Int(100_000),
        vm.string(means),
        w,
        ScriptValue::Undefined,
        vm.string("none"),
        ScriptValue::Int(0),
        ScriptValue::Int(0),
    ];
    let cb = mp::callbacks(game);
    vm.spawn_named(game, &cb, "codecallback_playerkilled", me, args);
}

pub(crate) fn tick(world: &World) -> crate::Tick {
    world.resource::<crate::frame_step::FrameTicket>().tick
}

pub(crate) fn frame(world: &mut World) -> StepWorldView<'_> {
    StepWorldView::from_world(world)
}

/// A shot's line against the map as a bullet sees it (walls, fences, hedges,
/// props), players left out: `trace_world` stops only at brushes, so an
/// arrow or a portal shot went through a hedge.
pub(crate) fn shot_trace(world: &mut World, a: [f32; 3], b: [f32; 3], mask: u32) -> trace_t6::Trace {
    use crate::shot_trace::{ShotRayRequest, RayResult};
    let query = ShotRayRequest { from: a, to: b, bits: mask, ignore: None, skip_client_hit: None, ignore_model: None };
    match frame(world).trace_shot(query, Some(&[])) {
        RayResult::Hit { fraction: part_frac, normal: norm, .. } => trace_t6::Trace { fraction: part_frac, normal: norm, ..Default::default() },
        RayResult::BeganInside { .. } => trace_t6::Trace { fraction: 0.0, starts_inside: true, ..Default::default() },
        RayResult::Miss { .. } | RayResult::Invalid { .. } => trace_t6::Trace::new(),
    }
}

/// An entity's origin (players from the sim).
/// Clients split into one side and the rest, each keeping its order; every
/// client comes marked with whether it is on that side.
pub(crate) fn split_sides(marked: impl IntoIterator<Item = (u32, bool)>) -> (crate::EventRecipients, crate::EventRecipients) {
    let (ours, theirs): (Vec<_>, Vec<_>) = marked.into_iter().partition(|m| m.1);
    let clients = |side: Vec<(u32, bool)>| crate::EventRecipients::Clients(side.into_iter().map(|m| ClientId(m.0)).collect());
    (clients(ours), clients(theirs))
}

pub(crate) fn origin_of(vm: &Vm<World>, game: &mut World, v: &ScriptValue) -> Option<[f32; 3]> {
    let n = entnum(vm, v)?;
    if game.resource::<Zm>().players.contains_key(&n) {
        return frame(game).avatar(ClientId(n)).map(|ps| ps.origin);
    }
    game.resource::<Zm>().ents.get(&n).map(|e| e.origin)
}

/// bo2mp: Black Ops II's sight attachments (`asset_game::T6_SIGHT_ATTACHMENTS`,
/// the ones the catalog gives a sight model, zoom and overlay).
pub(crate) const T6_SIGHTS: [&str; 10] =
    ["reflex", "holo", "acog", "dualoptic", "ir", "rangefinder", "mms", "vzoom", "swayreduc", "is"];

/// The script names that mean no weapon.
const NO_WEAPON: [&str; 2] = ["", "none"];

/// Weapon index by script name (`m1911_zm`; the old engine registers `_mp` names).
pub(crate) fn weapon(world: &mut World, ident: &str) -> Result<u32, String> {
    if NO_WEAPON.contains(&ident) { Ok(0) } else { named_weapon(world, ident) }
}

/// A weapon that has a name, by that name.
fn named_weapon(world: &mut World, ident: &str) -> Result<u32, String> {
    // bo2mp: a gun with attachments (`mk48_mp+fmj+reflex`) is the catalog's
    // weapon of that ident when there is one, else the same attachments in
    // ident order (the catalog's pairs: `asset_game::t6_attachment_combo_name`),
    // else (three attachments) a pair of them, the sight's first, else the
    // gun with its sight (`mk48_mp+reflex`: sight model, ADS, zoom,
    // overlay), else with its first other attachment the catalog has
    // (`mk48_mp+fmj`). The catalog holds every gun's pairs, so a third
    // attachment's stats and models are the one thing still left out (code
    // fix, needs local proof).
    if ident.contains('+')
        && let Ok(w) = crate::client_script::weapon_named(&frame(world), ident)
    {
        return Ok(w);
    }
    let mut parts = ident.split('+');
    let ident = parts.next().unwrap_or(ident);
    let mut atts: Vec<&str> = parts.collect();
    atts.sort_by_key(|att| !T6_SIGHTS.contains(att));
    let base = if ident.ends_with("_mp") { ident.to_owned() } else { format!("{ident}_mp") };
    let f = frame(world);
    for tried in attachment_tries(&atts) {
        if let Ok(w) = crate::client_script::weapon_named(&f, &format!("{base}+{tried}")) {
            return Ok(w);
        }
    }
    crate::client_script::weapon_named(&f, &format!("{ident}_mp"))
        .or_else(|_| crate::client_script::weapon_named(&f, ident))
}

/// A weapon's world model as everyone else sees it (on a wall, in the
/// box): the gun, the parts Black Ops II hangs on it (magazine, scope,
/// rocket: its `attachWorldModel`s) and the tags it hides, lower case.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WeaponLook {
    pub mesh: String,
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
        mesh: model.into(),
        // Only parts the map loaded: one it lacks would lose the whole gun.
        parts: f
            .weapon_world_parts(w)
            .iter()
            .filter(|p| f.model_capability(&p.mesh).flatten().is_some())
            .map(|p| {
                let at = xmodel_runtime::TagOffset::new(p.offset, p.rotation);
                (p.mesh.clone(), p.joint.clone(), at)
            })
            .collect(),
        hide: hide.iter().map(|t| t.to_ascii_lowercase()).collect(),
    })
}

/// bo2mp: the attachment sets a class gun's name is tried as, best first
/// (`atts` with the sight first): all of them in name order, then each
/// pair in name order (pairs with the sight first), then each one alone.
fn attachment_tries(atts: &[&str]) -> Vec<String> {
    let joined = |set: &[&str]| {
        let mut set = set.to_vec();
        set.sort_unstable();
        set.dedup();
        set.join("+")
    };
    let mut tries = Vec::new();
    if atts.len() > 1 {
        tries.push(joined(atts));
    }
    if atts.len() > 2 {
        for (i, a) in atts.iter().enumerate() {
            for b in &atts[i + 1..] {
                tries.push(joined(&[a, b]));
            }
        }
    }
    tries.extend(atts.iter().map(|a| (*a).to_owned()));
    tries.dedup();
    tries
}

/// A weapon's script name: Zombies' `m1911_zm` (the engine's `_mp`
/// dropped); multiplayer's own names end in `_mp` (`mp7_mp`, `radar_mp`),
/// as BO2's scripts compare them.
pub(crate) fn weapon_text(world: &mut World, w: u32) -> String {
    let mp = world.get_resource::<Zm>().is_some_and(|z| z.mp);
    let f = frame(world);
    let n = crate::client_script::weapon_name(&f, w);
    if mp {
        return n;
    }
    n.strip_suffix("_mp").map_or(n.clone(), str::to_owned)
}

/// bo2mp: a weapon's script name as this player was given it (the class's
/// full `an94_mp+fmj+reflex`, not the catalog's one-attachment name), else
/// its own (`weapon_text`).
pub(crate) fn weapon_text_of(world: &mut World, who: Option<u32>, w: u32) -> String {
    let given = who
        .and_then(|n| world.resource::<Zm>().players.get(&n))
        .and_then(|p| p.given_names.get(&w))
        .cloned();
    given.unwrap_or_else(|| weapon_text(world, w))
}

#[cfg(test)]
mod attachment_name_tests {
    use super::attachment_tries;

    #[test]
    fn a_class_gun_tries_all_then_pairs_with_the_sight_then_singles() {
        // (`weapon` puts the sight first.)
        assert_eq!(attachment_tries(&["reflex", "fmj"]), ["fmj+reflex", "reflex", "fmj"]);
        assert_eq!(
            attachment_tries(&["reflex", "fmj", "extclip"]),
            ["extclip+fmj+reflex", "fmj+reflex", "extclip+reflex", "extclip+fmj", "reflex", "fmj", "extclip"]
        );
        assert_eq!(attachment_tries(&["fmj"]), ["fmj"]);
    }
}

/// The melee weapon each client holds as his knife (0: none given), for
/// his first-person swing: BO2 stabs with the Bowie Knife, the
/// Galvaknuckles or the Spork he was given, drawn as itself, not the knife.
/// (bo2zm d0be9eed; bo2mp: the mod menu gives them.)
static MELEE_WEAPONS: [std::sync::atomic::AtomicU32; 18] =
    [const { std::sync::atomic::AtomicU32::new(0) }; 18];

pub fn melee_weapon(client: u32) -> u32 {
    MELEE_WEAPONS
        .get(client as usize)
        .map_or(0, |w| w.load(std::sync::atomic::Ordering::Relaxed))
}

pub(super) fn set_melee_weapon(peer: u32, gun: u32) {
    if let Some(w) = MELEE_WEAPONS.get(peer as usize) {
        w.store(gun, std::sync::atomic::Ordering::Relaxed);
    }
}
