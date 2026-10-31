//! Client fields (`setclientfield` -> `codesetclientfield`,
//! `codesetworldclientfield`): Black Ops II's client scripts turn them into
//! effects and sounds on each client. The engine plays their half here, as
//! those scripts do: a zombie's eye glow, the dirt as it rises, a
//! power-up's glow, a perk machine's trail from the sky, a wall buy's gun.

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ScriptValue, Vm};

use super::{Zm, arg, entnum, origin_of};
use crate::EventRecipients;
use crate::sim_parts::ClientId;

mod ai_tank;
mod heli;
mod swarm;
mod turret;

pub(super) mod vision;

/// A player's script vision and how far in it is.
#[derive(Clone, Debug, Default)]
struct Vision {
    label: String,
    lerp: f32,
}

/// Effects waiting their time (or their entity's first showing).
#[derive(Clone, Debug, Default)]
pub(crate) struct ClientFx {
    pending: Vec<Pending>,
    /// bo2mp: each player's last `missile_drone_active` (a client field's
    /// callback runs only when its value changes; the scripts set it again
    /// every half second).
    drone_lock: std::collections::BTreeMap<u32, i32>,
    /// bo2mp: the client scripts' own minimap icons (their client
    /// objectives), by entity: Express's moving train cars, the Hellstorm
    /// and its bomblets.
    map_icons: std::collections::BTreeMap<u32, MapIcon>,
    /// bo2mp: Uplink's wind vanes, each with the angles it started at, and
    /// how many times the wind has turned (a vane's jitter from an older
    /// turn stops: `windVaneJitter_singleton`).
    vanes: Vec<(u32, [f32; 3])>,
    wind_turns: u32,
    /// bo2mp: Drone's scanner (mp_drone_amb.csc `scanner_alert`).
    scanner: Scanner,
    /// Each entity's client field values as last set (Vulture Aid's bit
    /// fields, `anim_rate`), for `ClientFx::value`; `codegetclientfield`
    /// reads `Zm::client_field_vals` (`recall`), floats included.
    values: std::collections::HashMap<(u32, String), i64>,
    /// Each player's script vision (`visionset_slot`'s name) and its lerp.
    visionsets: std::collections::HashMap<u32, Vision>,
    /// The same for the overlay manager (`overlay_slot`).
    overlays: std::collections::HashMap<u32, Vision>,
    /// `visionsetlaststand`: the vision a player down sees, the seconds it
    /// fades in over and when it was set (the bleed-out's `zombie_death`
    /// fades in from `zombie_last_stand`, the one before it).
    laststand: Option<LastStandVision>,
    /// Each player down: when he went down and the vision he is shown.
    down: std::collections::HashMap<u32, DownView>,
    /// The map's own vision while no script vision is up, as
    /// `bo2zm_vision` text (BO2's client fog-volume vision monitor: Die
    /// Rise's `zm_highrise_off` before the power, `zm_highrise_on` after);
    /// empty draws the map's own grade.
    map_vision: String,
}

/// bo2mp: Drone's `scanner_alert` trigger: the players in it (each with when
/// its enter payload is done and `trigger_thread` starts to look whether he
/// is still touching it), when the script's loop next waits for the trigger,
/// and the alarms to come.
#[derive(Clone, Debug, Default)]
struct Scanner {
    trig: u32,
    inside: std::collections::BTreeMap<u32, i64>,
    next_look: i64,
    alarms: Vec<i64>,
}

/// bo2mp: a client script's minimap icon on an entity (`objective_add`
/// then `objective_onentity`): its picture, width and objectiveMapInfo
/// bits, when it shows, and (once its entity died) when it goes, held at
/// the entity's last place and yaw until then.
#[derive(Clone, Copy, Debug)]
struct MapIcon {
    icon: &'static str,
    size: u32,
    map_info: i32,
    shows: i64,
    /// How long it outlives its entity's death (the Hellstorm's
    /// `wait 0.4` before `objective_delete`).
    linger: i64,
    gone: Option<i64>,
    last: Option<([f32; 3], f32)>,
    /// Who fired it (the client script's `owner`), for a colour set in
    /// code.
    owner: Option<u32>,
}

/// objectiveMapInfo bits: rotateWithEntity, clipToMap, colorMaterialInCode.
const MAP_ROTATE: i32 = 0x2;
const MAP_CLIP: i32 = 0x8;
const MAP_TINT_IN_CODE: i32 = 0x10;

/// bo2mp: a client field's value (Black Ops II's fields are ints or
/// floats).
#[derive(Clone, Copy, Debug)]
pub(crate) enum FieldVal {
    Int(i32),
    Real(f32),
}

/// bo2mp: keep the value a script set, for the `codeget*clientfield` reads.
fn remember(game: &mut World, nn: u32, pstate: bool, ident: &str, v: &ScriptValue) {
    let v = match *v {
        ScriptValue::Float(f) => FieldVal::Real(f),
        ref v => FieldVal::Int(v.as_int().unwrap_or(0)),
    };
    game
        .resource_mut::<Zm>()
        .client_field_vals
        .insert((nn, pstate, ident.to_owned()), v);
}

/// bo2mp: the value last set, or 0 for a field never set.
fn recall(world: &World, nn: u32, pstate: bool, label: &str) -> ScriptValue {
    match world
        .resource::<Zm>()
        .client_field_vals
        .get(&(nn, pstate, label.to_owned()))
    {
        Some(FieldVal::Real(f)) => ScriptValue::Float(*f),
        Some(FieldVal::Int(i)) => ScriptValue::Int(*i),
        None => ScriptValue::Int(0),
    }
}

#[derive(Clone, Debug)]
struct LastStandVision {
    name: String,
    secs: f32,
    set_ms: i64,
}

#[derive(Clone, Debug)]
struct DownView {
    since: i64,
    /// The `visionsetlaststand` he is shown (when it was set), its name,
    /// and the one he saw before it (it fades in from that).
    set_ms: Option<i64>,
    label: String,
    from: Option<String>,
}

impl ClientFx {
    /// An entity's client field as last set (0 before).
    pub(crate) fn value(&self, ent: u32, name: &str) -> i64 {
        self.values.get(&(ent, name.to_owned())).copied().unwrap_or(0)
    }

    /// A player's own vision as `bo2zm_vision` text: his visionset
    /// manager's, else the map's (a vision drawn over it ends to this).
    pub(super) fn restore_text(&self, c: u32) -> Option<String> {
        match self.visionsets.get(&c) {
            Some(Vision { label, lerp }) if !label.is_empty() => {
                Some(format!("{} {lerp}", client_vision_name("visionset", label)))
            }
            _ => Some(self.map_vision.clone()),
        }
    }

    /// The visions of one manager ("visionset", "overlay").
    fn visions(&self, kind: &str) -> &std::collections::HashMap<u32, Vision> {
        if kind == "overlay" { &self.overlays } else { &self.visionsets }
    }

    fn visions_mut(&mut self, kind: &str) -> &mut std::collections::HashMap<u32, Vision> {
        if kind == "overlay" { &mut self.overlays } else { &mut self.visionsets }
    }
}

#[derive(Clone, Debug)]
struct Pending {
    due: i64,
    /// Given up after this (the entity never showed).
    until: i64,
    ent: u32,
    step: Cue,
}

#[derive(Clone, Debug)]
enum Cue {
    /// An effect where the entity stands, offset.
    At { fx: &'static str, offset: [f32; 3] },
    /// An effect on one of the entity's tags (or taken off it).
    Bolt {
        fx: &'static str,
        tag: &'static str,
        off: bool,
    },
    /// bo2mp: a light on one of its tags, for some players only (an
    /// RC-XD's on `tag_origin`, the Dragonfire's on `tag_body`: his team
    /// sees green, the others red).
    Light {
        fx: &'static str,
        tag: &'static str,
        off: bool,
        to: EventRecipients,
    },
    /// bo2mp: `_qrdrone.csc`'s `blink_fx_and_sound`: a beep and the
    /// Dragonfire's lights on for half of `interval`, off for the other
    /// half, then again 1.17 times quicker (never under 0.1 s). The first
    /// beep after a state change keeps the interval it had (`quicken`
    /// false).
    QrBlink {
        interval: f32,
        quicken: bool,
        sides: [(&'static str, EventRecipients); 3],
    },
    /// bo2mp: a sound where the entity is now (the RC-XD's countdown beep).
    Sound { alias: &'static str },
    /// bo2mp: `_fx.csc`'s `blinky_light`: the effect on a tag again every
    /// half second while its entity lives.
    Blink { fx: &'static str, tag: &'static str },
    /// bo2mp: `targetacquired` (the Hunter Killer in player `ent`'s hands
    /// has a target): its light on his gun, the raise sound the first time.
    HeldLight { sounded: bool },
    /// bo2mp: mp_carrier.csc's `floatmyboat`: a far ship rocks to its
    /// `angles` pitched by the roll (leg 0), then back (leg 1), each leg
    /// over `time` seconds.
    Sway { base: [f32; 3], leg: u8, time: f32 },
    /// bo2mp: mp_nuketown_2020.csc's `spinner_rotate_match`: the sign
    /// topper turns at `rotatevelocity( ( 0, 90, 0 ), 3600 )`, again each
    /// time it is done.
    Spin,
    /// bo2mp: mp_express.csc's `train_fx_think`: the dust at exploders
    /// `next` to `last` as the car passes each.
    TrainDust { next: i32, last: i32 },
    /// bo2mp: mp_express.csc's `train_move_think`: each 0.05 s, every player
    /// within 512 of the moving car (not in the killcam) feels its rumble
    /// and a small earthquake.
    TrainShake,
    /// bo2mp: _rotating_object.csc's `rotating_object_think`: 1000 turns on
    /// one axis (`degrees` in all, over `secs`), again each time it is done.
    Turn { around: usize, degrees: f32, secs: f32 },
    /// bo2mp: _ambient.csc's `clock_run`: the hand turned to the local time
    /// (`set`, then `first`, each over 0.05 s; stages 0 and 1), then `bit`
    /// more each time the clock's time has changed since it last looked
    /// (`last`), looking once a second.
    Clock {
        stage: u8,
        set: f32,
        first: Option<f32>,
        bit: f32,
        last: [u32; 3],
    },
    /// bo2mp: `cameratrackplayer` (mp_hydro.csc, mp_vertigo.csc): a camera
    /// ball turns to face the player over half a second, every half second.
    Camera { pitch: f32, roll: f32 },
    /// bo2mp: mp_uplink.csc's `lightpostsway`: out one way (leg 0), then
    /// back the other way (leg 1).
    Post { angle: f32, secs: f32, leg: u8 },
    /// bo2mp: mp_uplink.csc's `rainexploderswitch`: the wind turns each 8 s.
    Wind,
    /// bo2mp: mp_uplink.csc's `windvanejitter`: half a second after the
    /// wind turns a vane turns to it (stage 0), then jitters about it.
    Vane {
        orig: [f32; 3],
        yaw: f32,
        stage: u8,
        wind: u32,
    },
    /// bo2mp: mp_uplink.csc's `initlightningloop`: a lightning exploder.
    Lightning,
    /// bo2mp: `playlightning`'s thunder at the lightning, `left` strikes to
    /// come (None: not counted yet).
    Thunder { origin: [f32; 3], left: Option<u8> },
    /// bo2mp: mp_turbine_amb.csc's `wmill_sfx_setup`: the windmills' whoosh.
    Whoosh,
    /// bo2mp: mp_drone_amb.csc's `scanner_alert`: the scanner looks for
    /// players in its trigger each frame.
    Scanner,
    /// bo2mp: _ai_tank.csc: an AGR's lights, stun, pods and treads.
    Tank(Box<ai_tank::Tank>),
    /// bo2mp: _helicopter.csc's `startfx`: a helicopter's lights and the
    /// VTOL's exhausts.
    Heli(Box<heli::Heli>),
    /// bo2mp: _missile_swarm.csc: the Swarm's sky drones and hum.
    Swarm,
    /// bo2mp: `visionsetnaked`'s fade.
    VisionFade,
    /// A script's `playfxontag` on an entity not drawn yet: its effect and
    /// joint by name.
    BoltNamed { fx: String, joint: String },
}

// The client scripts' effects (clientscripts/mp/zombies/_zm.csc,
// _zm_powerups.csc, zm_nuked_fx.csc).
const RISE_BURST: &str = "maps/zombie/fx_mp_zombie_hand_dirt_burst";
const RISE_BILLOW: &str = "maps/zombie/fx_mp_zombie_body_dirt_billowing";
const RISE_DUST: &str = "maps/zombie/fx_mp_zombie_body_dust_falling";
const EYE_GLOW: &str = "misc/fx_zombie_eye_single";
/// zm_nuked.csc zombie_eye_clientfield: after the moon transmission
/// (round 25) every zombie's eyes glow blue.
const EYE_GLOW_BLUE: &str = "maps/zombie/fx_zombie_eye_single_blue";
const GUTS_EXPLOSION: &str = "maps/zombie/fx_zmb_tranzit_torso_explo";
/// A standing zombie's J_SpineLower above its feet.
const SPINE_LOWER: f32 = 40.0;
const PERK_METEOR: &str = "maps/zombie/fx_zmb_trail_perk_meteor";
const POWERUP: [&str; 4] = [
    "misc/fx_zombie_powerup_on",
    "misc/fx_zombie_powerup_solo_on",
    "misc/fx_zombie_powerup_on_red",
    "misc/fx_zombie_powerup_caution_on",
];

// bo2mp: the RC-XD's lights (clientscripts/mp/_rcbomb.csc): solid while it
// drives, a one-shot flash with each countdown beep.
const RC_FRIEND: &str = "vehicle/light/fx_rcbomb_solid_light";
const RC_ENEMY: &str = "vehicle/light/fx_rcbomb_blinky_light";
const RC_FRIEND_BLINK: &str = "vehicle/light/fx_rcbomb_light_green_os";
const RC_ENEMY_BLINK: &str = "vehicle/light/fx_rcbomb_light_red_os";
const RC_BEEP: &str = "wpn_crossbow_alert";

// bo2mp: the Dragonfire's light on its `tag_body`
// (clientscripts/mp/_qrdrone.csc): green in its driver's own view, green to
// his side, red to the others; it beeps the RC-XD's beep as it blinks.
const QR_TAG: &str = "tag_body";
const QR_DRIVER: &str = "weapon/qr_drone/fx_qr_light_green_1p";
const QR_FRIEND: &str = "weapon/qr_drone/fx_qr_light_green_3p";
const QR_ENEMY: &str = "weapon/qr_drone/fx_qr_light_red_3p";

// bo2mp: the Hunter Killer's light (clientscripts/mp/_missile_drone.csc),
// the same red for friend and foe, on while it has a target.
const DRONE_LIGHT: &str = "weapon/missile/fx_missile_drone_light_red";
/// The Lightning Strike jet's afterburner (`level.planemortarexhaustfx`).
const PLANE_EXHAUST: &str = "vehicle/exhaust/fx_exhaust_f35_afterburner";

// bo2mp: Aftermath's police car and ambulance light bars (mp_la.csc's
// destructible_car_lights, effects from mp_la_fx.csc); Magma's police cars
// (mp_magma.csc, mp_magma_fx.csc).
const POLICE_LIGHTS: &str = "maps/mp_maps/fx_mp_light_police_car";
const AMBULANCE_LIGHTS: &str = "maps/mp_maps/fx_mp_light_ambulance";
const POLICE_LIGHTS_JAPAN: &str = "maps/mp_maps/fx_mp_light_police_car_japan";

/// Who counts an RC-XD as a friend (its `friendnotfoe`): its team in a team
/// game, else only its owner; and everyone else.
/// A light one client sees on a vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Lit {
    seer: u32,
    fx: &'static str,
}

/// A vehicle's light for each side.
#[derive(Clone, Copy, Debug)]
struct Lamps {
    friend: &'static str,
    foe: &'static str,
}

impl Lamps {
    /// Who sees which: its side the friend light, the rest the foe light
    /// (no one when the sides are not lists of clients).
    fn shown_to(self, sides: Option<(EventRecipients, EventRecipients)>) -> Vec<Lit> {
        let Some((EventRecipients::Clients(friends), EventRecipients::Clients(foes))) = sides else {
            return Vec::new();
        };
        let lit = |cs: Vec<ClientId>, fx| cs.into_iter().map(move |c| Lit { seer: c.0, fx });
        lit(friends, self.friend).chain(lit(foes, self.foe)).collect()
    }
}

/// A vehicle's lights brought from `shown` to `want`: each one no longer
/// wanted comes off (for players still here), then each new one goes on.
fn relight(world: &mut World, n: u32, tag: &'static str, shown: &[Lit], want: &[Lit]) {
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let off = shown.iter().filter(|l| !want.contains(l) && players.contains(&l.seer)).map(|l| (l, true));
    let on = want.iter().filter(|l| !shown.contains(l)).map(|l| (l, false));
    for (l, stop) in off.chain(on) {
        super::natives_fx::bolt_effect_to(world, EventRecipients::Client(ClientId(l.seer)), n, l.fx, tag, stop);
    }
}

fn rc_sides(vm: &mut Vm<World>, world: &World, car: &ScriptValue) -> (EventRecipients, EventRecipients) {
    let ScriptValue::Object(o) = car else {
        return (EventRecipients::Clients(Vec::new()), EventRecipients::All);
    };
    let (team_s, owner_s, tb) = (vm.intern("team"), vm.intern("owner"), vm.intern("teambased"));
    let team = vm.to_text(&vm.raw_field(*o, team_s));
    let owner = vm.raw_field(*o, owner_s);
    let owner = entnum(vm, &owner);
    let teambased = gsc_t6::truthy(&vm.raw_field(vm.level, tb));
    let players: Vec<(u32, gsc_t6::ObjRef)> =
        world.resource::<Zm>().players.iter().map(|(n, p)| (*n, p.obj)).collect();
    super::split_sides(players.into_iter().map(|(n, p)| {
        let friend = Some(n) == owner || (teambased && !team.is_empty() && vm.to_text(&vm.raw_field(p, team_s)) == team);
        (n, friend)
    }))
}

/// bo2mp: an RC-XD comes into play (`spawned` -> `spawn_solid_fx`): its
/// solid light, green to his side, blinking red to the other.
pub(super) fn rcbomb_spawned(vm: &mut Vm<World>, game: &mut World, car: &ScriptValue) {
    let Some(nn) = entnum(vm, car) else { return };
    rc_cancel(game, nn);
    let (friends, foes) = rc_sides(vm, game, car);
    for (fx, goal) in [(RC_FRIEND, friends), (RC_ENEMY, foes)] {
        queue(game, nn, 0, Cue::Light { fx, tag: "tag_origin", off: false, to: goal });
    }
}

/// Its light scripts end (`light_disable`): what is still to come is
/// dropped, its solid lights go out.
fn rc_cancel(world: &mut World, n: u32) {
    world
        .resource_mut::<Zm>()
        .client_fx
        .pending
        .retain(|p| p.ent != n || !matches!(p.step, Cue::Light { .. } | Cue::Sound { .. }));
}

/// Sent at once while the car still shows: it blows up and is deleted in the
/// same tick, and a stop queued to the tick's end would find it gone and its
/// lights would hang in the air.
fn rc_lights_off(vm: &mut Vm<World>, game: &mut World, car: &ScriptValue, nn: u32) {
    rc_cancel(game, nn);
    let (friends, foes) = rc_sides(vm, game, car);
    for (fx, to) in [(RC_FRIEND, friends), (RC_ENEMY, foes)] {
        if !super::natives_fx::bolt_effect_to(game, to.clone(), nn, fx, "tag_origin", true) {
            queue(game, nn, 0, Cue::Light { fx, tag: "tag_origin", off: true, to });
        }
    }
}

/// It blows up (`death`): `stop_light_fx` takes off whichever light it
/// showed last, the solid one or a countdown flash, and `blowup` hides it
/// that tick, so nothing of them is left in the air.
fn rc_lights_gone(vm: &mut Vm<World>, game: &mut World, car: &ScriptValue, nn: u32) {
    if std::env::var_os("BO2MP_T6_BOLTLOG").is_some() {
        diag::info!(Sim, "bo2mp rcxd death: ent{nn} lights off");
    }
    rc_cancel(game, nn);
    let (friends, foes) = rc_sides(vm, game, car);
    for (fx, to) in [
        (RC_FRIEND, friends.clone()),
        (RC_ENEMY, foes.clone()),
        (RC_FRIEND_BLINK, friends),
        (RC_ENEMY_BLINK, foes),
    ] {
        if !super::natives_fx::bolt_effect_kill(game, to.clone(), nn, fx, "tag_origin") {
            queue(game, nn, 0, Cue::Light { fx, tag: "tag_origin", off: true, to });
        }
    }
}

/// The Dragonfire's three lights and who sees each: its driver
/// (`islocalclientdriver`, its owner), the rest of his side
/// (`friendnotfoe`), everyone else.
fn qr_sides(vm: &mut Vm<World>, world: &World, drone: &ScriptValue) -> [(&'static str, EventRecipients); 3] {
    let (friends, foes) = rc_sides(vm, world, drone);
    let owner = match drone {
        ScriptValue::Object(o) => {
            let s = vm.intern("owner");
            let v = vm.raw_field(*o, s);
            entnum(vm, &v)
        }
        _ => None,
    };
    let mut friends = match friends {
        EventRecipients::Clients(c) => c,
        _ => Vec::new(),
    };
    friends.retain(|c| Some(c.0) != owner);
    let driver = EventRecipients::Clients(owner.map(crate::sim_parts::ClientId).into_iter().collect());
    [(QR_DRIVER, driver), (QR_FRIEND, EventRecipients::Clients(friends)), (QR_ENEMY, foes)]
}

/// bo2mp: the Dragonfire comes into play (its client script's `spawned`:
/// `qrdronefx( 0 )`): its steady light.
pub(super) fn qrdrone_spawned(vm: &mut Vm<World>, game: &mut World, drone: &ScriptValue) {
    let Some(n) = entnum(vm, drone) else { return };
    let sides = qr_sides(vm, game, drone);
    qrdrone_state(game, n, 0, sides);
}

/// bo2mp: `restartfx`: whatever it showed goes (`restart_fx`), then 0
/// lights it steady, 1 or 2 start `blink_fx_and_sound`, 3 leaves it dark
/// (`stopfx`, `fx_death`). The blink goes on at the interval it had reached:
/// BO2 sets `fx_interval` (1.0, 0.133) there, but the blink reads
/// `interval`, so a 2 six seconds after the 1 keeps quickening from where it
/// was (about 0.13 s by then); a first blink starts at 1 s.
fn qrdrone_state(game: &mut World, nn: u32, val: i64, sides: [(&'static str, EventRecipients); 3]) {
    let interval = {
        let mut zm = game.resource_mut::<Zm>();
        let pending = &mut zm.client_fx.pending;
        let was = pending.iter().find_map(|p| match &p.step {
            Cue::QrBlink { interval, .. } if p.ent == nn => Some(*interval),
            _ => None,
        });
        pending.retain(|p| p.ent != nn || !matches!(p.step, Cue::QrBlink { .. } | Cue::Light { .. }));
        was.unwrap_or(1.0)
    };
    for (fx, goal) in sides.iter().cloned() {
        super::natives_fx::bolt_effect_to(game, goal, nn, fx, QR_TAG, true);
    }
    match val {
        0 => {
            for (fx, goal) in sides {
                queue(game, nn, 0, Cue::Light { fx, tag: QR_TAG, off: false, to: goal });
            }
        }
        1 | 2 => queue(game, nn, 0, Cue::QrBlink { interval, quicken: false, sides }),
        _ => {}
    }
}

fn queue(world: &mut World, ent: u32, after_ms: i64, step: Cue) {
    let mut zm = world.resource_mut::<Zm>();
    let due = zm.clock_millis + after_ms;
    zm.client_fx.pending.push(Pending {
        due,
        until: due + 2000,
        ent,
        step,
    });
}

/// An effect bolted to entity `ent`'s tag (or, with `off`, taken off it)
/// in turn with the others queued, once the entity shows.
pub(super) fn bolt_queued(world: &mut World, ent: u32, fx: &'static str, tag: &'static str, off: bool) {
    queue(world, ent, 0, Cue::Bolt { fx, tag, off });
}

/// A script's `playfxontag` on entity `ent` once the entity shows (it was
/// spawned this tick: the script spawns a `tag_origin` model and bolts an
/// effect to it in the same frame).
pub(super) fn bolt_named_queued(world: &mut World, ent: u32, fx: &str, tag: &str) {
    queue(
        world,
        ent,
        0,
        Cue::BoltNamed {
            fx: fx.to_owned(),
            joint: tag.to_owned(),
        },
    );
}

/// An effect bolted to entity `ent`'s tag after a moment (or once the
/// entity first shows): the gibs' neck spurt and blood trails.
pub(super) fn bolt_later(
    world: &mut World,
    ent: u32,
    after_ms: i64,
    fx: &'static str,
    tag: &'static str,
) {
    queue(
        world,
        ent,
        after_ms,
        Cue::Bolt {
            fx,
            tag,
            off: false,
        },
    );
}

/// Both eye glows off (the csc's deletezombieeyes).
pub(super) fn eyes_out(world: &mut World, n: u32) {
    for fx in [EYE_GLOW, EYE_GLOW_BLUE] {
        bolt_queued(world, n, fx, "j_eyeball_le", true);
    }
}

fn rand_range(vm: &mut Vm<World>, lo: i32, hi: i32) -> f32 {
    (lo + (vm.rand_u32() % (hi - lo) as u32) as i32) as f32
}

/// One Inch Punch: the client's launchragdoll on a punched (already limp) body.
fn punch_launch(vm: &mut Vm<World>, game: &mut World, en: &ScriptValue, nn: u32) {
    let Some(at) = origin_of(vm, game, en) else { return };
    let clients: Vec<u32> = game.resource::<Zm>().players.keys().copied().collect();
    let puncher = clients
        .into_iter()
        .filter_map(|c| super::frame(game).avatar(crate::sim_parts::ClientId(c)).map(|p| p.origin))
        .min_by(|lhs, rhs| separation_sq(*lhs, at).total_cmp(&separation_sq(*rhs, at)));
    let Some(from) = puncher else { return };
    let d = [at[0] - from[0], at[1] - from[1], at[2] - from[2]];
    let len = separation_sq(at, from).sqrt().max(1e-3);
    let out = rand_range(vm, 125, 150);
    let up = rand_range(vm, 75, 150);
    let throw = [d[0] / len * out, d[1] / len * out, d[2] / len * out + up];
    if let Some(ragdoll) = game
        .resource_mut::<Zm>()
        .into_inner()
        .ents
        .get_mut(&nn)
        .and_then(|e| e.ragdoll.as_mut())
    {
        ragdoll.launch = throw;
        ragdoll.launches = ragdoll.launches.wrapping_add(1);
    }
}

/// The Wind Staff's client state: `level.launch_source` (the firing
/// player's origin) and `level.whirlwind_pos` (the whirlwind standing).
#[derive(bevy_ecs::prelude::Resource, Default)]
struct AirStaff {
    source: [f32; 3],
    whirlwind: Option<[f32; 3]>,
    /// Bodies to throw once the frame's fields are all in: the server sets
    /// `air_staff_launch` in its cone before the shot's `air_staff_source`,
    /// and the client sees both in one snapshot.
    pending: Vec<u32>,
}

const WHIRLWIND_FX: &str = "weapon/zmb_staff/fx_zmb_staff_air_ug_impact_miss";

/// whirlwind_fx: the funnel on the whirlwind's tag_origin and its tornado
/// loop while it stands; its spot is where the bodies it kills fly from.
fn whirlwind_fx(vm: &mut Vm<World>, game: &mut World, en: &ScriptValue, nn: u32, on: bool) {
    let at = origin_of(vm, game, en);
    game.get_resource_or_insert_with(AirStaff::default).whirlwind = if on { at } else { None };
    if on {
        bolt_named_queued(game, nn, WHIRLWIND_FX, "tag_origin");
    } else {
        super::natives_fx::bolt_effect(game, nn, WHIRLWIND_FX, "tag_origin", true);
    }
    if let Some(e) = game.resource_mut::<Zm>().into_inner().ents.get_mut(&nn) {
        e.repeating_audio = on.then(|| "wpn_airstaff_tornado".to_owned());
    }
}

fn rand_float_vm(vm: &mut Vm<World>, lo: f32, hi: f32) -> f32 {
    lo + (vm.rand_u32() % 1_000_000) as f32 / 1_000_000.0 * (hi - lo)
}

/// air_staff_launch: the body flies away from the blast's source (or, inside
/// 150 of the whirlwind, from a random spot up to 1000 around it), 300 hard,
/// 5-35% of that upward.
fn air_staff_launch(vm: &mut Vm<World>, game: &mut World, nn: u32) {
    let Some(at) = game.resource::<Zm>().ents.get(&nn).map(|e| e.origin) else { return };
    let (mut source, whirlwind) = {
        let a = game.get_resource_or_insert_with(AirStaff::default);
        (a.source, a.whirlwind)
    };
    let up = rand_float_vm(vm, 0.05, 0.35);
    if let Some(w) = whirlwind {
        if separation_sq(w, at) < 22500.0 {
            source = [w[0] + rand_float_vm(vm, -1000.0, 1000.0), w[1] + rand_float_vm(vm, -1000.0, 1000.0), w[2]];
        }
    }
    let dd = [at[0] - source[0], at[1] - source[1], at[2] - source[2]];
    let len = separation_sq(at, source).sqrt();
    let aim = if len > 0.0 { [dd[0] / len, dd[1] / len, dd[2] / len] } else { [0.0; 3] };
    // v_force = length(aim) * 300: 300 for a real direction, 0 for none.
    let force = if len > 0.0 { 300.0 } else { 0.0 };
    let throw = [aim[0] * force, aim[1] * force, up * force];
    if let Some(ragdoll) = game
        .resource_mut::<Zm>()
        .into_inner()
        .ents
        .get_mut(&nn)
        .and_then(|e| e.ragdoll.as_mut())
    {
        ragdoll.launch = throw;
        ragdoll.launches = ragdoll.launches.wrapping_add(1);
    }
    diag::info!(Sim, "bo2zm t6 air staff: ent{nn} thrown {throw:?} from {source:?}");
}

/// Buried's Subwoofer (_zm_equip_subwoofer.csc): the subwoofers that
/// pulsed (`equipment_activated` on a `t6_wpn_zmb_subwoofer`), each with
/// the time it leaves the list (0.5 s after its pulse ends; none while on).
#[derive(bevy_ecs::prelude::Resource, Default)]
struct Subwoofers {
    subs: std::collections::BTreeMap<u32, Option<i64>>,
}

fn subwoofer_active(world: &mut World, n: u32, on: bool) {
    let zm = world.resource::<Zm>();
    if zm.ents.get(&n).is_none_or(|e| e.mesh != "t6_wpn_zmb_subwoofer") {
        return;
    }
    let gone = (!on).then_some(zm.clock_millis + 500);
    world.get_resource_or_insert_with(Subwoofers::default).subs.insert(n, gone);
}

/// subwoofer_fling_zombie: the killed body flies along the closest
/// subwoofer's facing, pitched 30 up, 512 / 4 hard.
fn subwoofer_fling(world: &mut World, n: u32) {
    let now = world.resource::<Zm>().clock_millis;
    let Some(at) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else { return };
    let subs: Vec<u32> = {
        let mut s = world.get_resource_or_insert_with(Subwoofers::default);
        s.subs.retain(|_, gone| gone.is_none_or(|t| t > now));
        s.subs.keys().copied().collect()
    };
    let zm = world.resource::<Zm>();
    let closest = subs
        .into_iter()
        .filter_map(|s| zm.ents.get(&s).map(|e| (e.origin, e.rot[1])))
        .min_by(|lhs, rhs| separation_sq(lhs.0, at).total_cmp(&separation_sq(rhs.0, at)));
    let Some(yw) = closest.map(|c| c.1) else { return };
    let (yw, pitch) = (yw.to_radians(), (-30.0f32).to_radians());
    let force = 512.0 / 4.0;
    let throw = [
        pitch.cos() * yw.cos() * force,
        pitch.cos() * yw.sin() * force,
        -pitch.sin() * force,
    ];
    if let Some(ragdoll) = world
        .resource_mut::<Zm>()
        .into_inner()
        .ents
        .get_mut(&n)
        .and_then(|e| e.ragdoll.as_mut())
    {
        ragdoll.launch = throw;
        ragdoll.launches = ragdoll.launches.wrapping_add(1);
    }
    diag::info!(Sim, "bo2zm t6 subwoofer: ent{n} flung {throw:?}");
}

fn separation_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// An entity's field changed: its client script's effects.
fn entity_field(vm: &mut Vm<World>, game: &mut World, en: &ScriptValue, ident: &str, val: i64) {
    let Some(nn) = entnum(vm, en) else { return };
    game
        .resource_mut::<Zm>()
        .client_fx
        .values
        .insert((nn, ident.to_owned()), val);
    if ident.starts_with("score_cf_") {
        score_field(game, nn, ident, val as i32);
        return;
    }
    // Origins' fire, lightning and ice staffs (their client scripts).
    if super::tomb_staff_fx::entity_field(vm, game, nn, ident, val) {
        return;
    }
    // BO1's Winter's Howl (the Declassified maps).
    if super::freezegun_fx::entity_field(game, nn, ident, val) {
        return;
    }
    // BO1's Gersh Device (the Declassified maps).
    if super::blackhole_fx::entity_field(game, nn, ident, val) {
        return;
    }
    match ident {
        // handle_zombie_risers: a burst of dirt, a billow, then dust
        // falling off its back while it climbs out.
        "zombie_riser_fx" if val != 0 => {
            let origin = origin_of(vm, game, en).unwrap_or([0.0; 3]);
            super::natives_fx::sound(game, EventRecipients::All, "zmb_zombie_spawn", origin);
            let burst = [0.0, 0.0, rand_range(vm, 5, 10)];
            queue(
                game,
                nn,
                0,
                Cue::At {
                    fx: RISE_BURST,
                    offset: burst,
                },
            );
            let billow = [
                rand_range(vm, -10, 10),
                rand_range(vm, -10, 10),
                rand_range(vm, 5, 10),
            ];
            queue(
                game,
                nn,
                250,
                Cue::At {
                    fx: RISE_BILLOW,
                    offset: billow,
                },
            );
            let mut tt = 0;
            while tt < 5500 {
                queue(
                    game,
                    nn,
                    2250 + tt,
                    Cue::Bolt {
                        fx: RISE_DUST,
                        tag: "j_spineupper",
                        off: false,
                    },
                );
                tt += 300;
            }
        }
        // Origins: oneinchpunch_physics_launchragdoll (_zm_weap_one_inch_punch.csc)
        // throws the punched body away from the puncher, 125-150 out and
        // 75-150 up (the csc picks the latest puncher; solo, the one player).
        "oneinchpunch_physics_launchragdoll" if val == 1 => punch_launch(vm, game, en, nn),
        // Origins' Wind Staff (_zm_weap_staff_air.csc): where the blast came
        // from, the whirlwind, and the throw of each body it kills.
        "air_staff_source" if val == 1 => {
            if let Some(at) = origin_of(vm, game, en) {
                game.get_resource_or_insert_with(AirStaff::default).source = at;
            }
        }
        "whirlwind_play_fx" => whirlwind_fx(vm, game, en, nn, val != 0),
        "air_staff_launch" if val == 1 => game.get_resource_or_insert_with(AirStaff::default).pending.push(nn),
        // Buried's Subwoofer: which subwoofers pulsed, and the throw of each
        // body its blast kills (_zm_equip_subwoofer.csc).
        "equipment_activated" => subwoofer_active(game, nn, val != 0),
        "subwoofer_flings_zombie" if val == 1 => subwoofer_fling(game, nn),
        // Origins: the G-Strike's glow and shells (_zm_weap_beacon.csc).
        "play_beacon_fx" | "play_artillery_barrage" => super::tomb_beacon::entity_field(vm, game, nn, ident, val),
        // Mob: pose_dead_body (zm_prison_weap_quest.csc) keeps the body
        // for the wolf head that eats it.
        "make_client_clone" if val != 0 => super::prison_wolves::clone_body(game, nn),
        // zombie_gut_explosion_cb: a grenade's kill bursts the body at
        // J_SpineLower as the server ghosts it.
        "zombie_gut_explosion" if val != 0 => queue(
            game,
            nn,
            0,
            Cue::At {
                fx: GUTS_EXPLOSION,
                offset: [0.0, 0.0, SPINE_LOWER],
            },
        ),
        // zombie_wait_explode: the same burst two seconds on, where the
        // thrown body lies (mature only).
        "zombie_ragdoll_explode" if val != 0 && super::profile_mature() => queue(
            game,
            nn,
            2000,
            Cue::At {
                fx: GUTS_EXPLOSION,
                offset: [0.0, 0.0, SPINE_LOWER],
            },
        ),
        // createzombieeyes / deletezombieeyes with level._override_eye_fx:
        // blue_eyes from the csc main of TranZit, Die Rise and Buried
        // (orange_eyes once Buried's buried_sq_maxis_eye_glow_override is
        // set), and on Nuketown once the game field zombie_eye_change is.
        // The stop takes both glows off, whichever one this zombie got.
        "zombie_has_eyes" => {
            // zombie_eyes_clientfield_cb also sets the eye shader's brightness
            // (mapshaderconstant scriptVector2.y): get_eyeball_luminance()
            // alive (2; Origins' csc main overrides it goal 3), 0.25 once the
            // eyes go out.
            let luminance = if val == 0 {
                0.25
            } else if game.resource::<Zm>().map == "zm_tomb" {
                3.0
            } else {
                2.0
            };
            if let Some(e) = game.resource_mut::<Zm>().ents.get_mut(&nn) {
                e.eye_luminance = Some(luminance);
            }
            let zm = game.resource::<Zm>();
            let field = |ident: &str| zm.wallbuys.world_fields.get(ident) == Some(&1);
            let blue = match zm.map.as_str() {
                "zm_transit" | "zm_highrise" => true,
                "zm_buried" => !field("buried_sq_maxis_eye_glow_override"),
                _ => field("zombie_eye_change"),
            };
            let fxs: &[&'static str] = if val == 0 {
                &[EYE_GLOW, EYE_GLOW_BLUE]
            } else if blue {
                &[EYE_GLOW_BLUE]
            } else {
                &[EYE_GLOW]
            };
            for &fx in fxs {
                queue(
                    game,
                    nn,
                    0,
                    Cue::Bolt {
                        fx,
                        tag: "j_eyeball_le",
                        off: val == 0,
                    },
                );
            }
        }
        // powerup_fx_callback: the glow by kind (no glow for 0).
        "powerup_fx" => {
            if let Some(fx) = usize::try_from(val - 1).ok().and_then(|i| POWERUP.get(i)) {
                queue(
                    game,
                    nn,
                    0,
                    Cue::Bolt {
                        fx,
                        tag: "tag_origin",
                        off: false,
                    },
                );
            }
        }
        // Mob's Mystery Box glow (_zm_magicbox_prison.csc).
        "magicbox_initial_fx" | "magicbox_amb_fx" | "magicbox_open_fx" | "magicbox_leaving_fx" => {
            super::prison_box::field(vm, game, nn, ident, val);
        }
        // perk_meteor_fx: the trail while the machine falls.
        "clientfield_perk_intro_fx" => queue(
            game,
            nn,
            0,
            Cue::Bolt {
                fx: PERK_METEOR,
                tag: "tag_origin",
                off: val == 0,
            },
        ),
        // bo2mp: the RC-XD's time runs out (`countdown`): a beep and a
        // flash, a second apart at 1 and quickening by 1.17 each time (from
        // 0.133 s at 2), never under 0.1 s, until the next val or its end.
        // Already blown up (`blowup` hides it, and a hidden entity is not
        // sent, so his client never hears the timer's last step): no flash
        // over the scorch mark.
        "rcbomb_countdown" if val != 0 && game.resource::<Zm>().ents.get(&nn).is_some_and(|e| e.unseen) => {}
        "rcbomb_countdown" if val != 0 => {
            rc_lights_off(vm, game, en, nn);
            let (friends, foes) = rc_sides(vm, game, en);
            let (mut interval, until) = if val == 2 { (0.133f32, 5000) } else { (1.0f32, 6500) };
            let mut tt = 0i64;
            while tt < until {
                queue(game, nn, tt, Cue::Sound { alias: RC_BEEP });
                for (fx, goal) in [(RC_FRIEND_BLINK, friends.clone()), (RC_ENEMY_BLINK, foes.clone())] {
                    queue(game, nn, tt, Cue::Light { fx, tag: "tag_origin", off: false, to: goal });
                }
                tt += (interval * 1000.0).round() as i64;
                interval = (interval / 1.17).clamp(0.1, 1.0);
            }
        }
        // bo2mp: it blows up (`death`): its lights go out.
        "rcbomb_death" if val != 0 => rc_lights_gone(vm, game, en, nn),
        // bo2mp: the Dragonfire's light (_qrdrone.csc's `statechange`):
        // its last 10 s blink with a beep, 3 when it is done.
        "qrdrone_state" => {
            let sides = qr_sides(vm, game, en);
            qrdrone_state(game, nn, val, sides);
        }
        // bo2mp: the Hellstorm on the minimap (_remotemissile.csc): fired
        // (1), `remotemissile_target` 50 wide; its bomblets out (2), taken
        // off. Each bomblet shows its own, 32 wide (the default). They turn
        // with their missile and tint in code, not cut off at the edge
        // (`objective_onentity( lcn, id, self, 1, 0, 1 )`); one that dies
        // stays 0.4 s where it went off.
        "remote_missile_fired" | "remote_missile_bomblet_fired" => {
            let size = if ident == "remote_missile_fired" { 50 } else { 32 };
            missile_icon(game, nn, val == 1, size);
        }
        // bo2mp: destructible_car_lights (Aftermath, Detour, Rush, Magma): on,
        // the light bar on tag_origin after randomfloatrange(0.1, 0.5) s; off
        // (the car is wrecked), taken off. Magma's cars take its own bar.
        "police_car_lights" | "ambulance_lights" => {
            let fx = if ident != "police_car_lights" {
                AMBULANCE_LIGHTS
            } else if game.resource::<Zm>().map == "mp_magma" {
                POLICE_LIGHTS_JAPAN
            } else {
                POLICE_LIGHTS
            };
            let after = if val != 0 { 100 + i64::from(vm.rand_u32() % 401) } else { 0 };
            queue(
                game,
                nn,
                after,
                Cue::Bolt {
                    fx,
                    tag: "tag_origin",
                    off: val == 0,
                },
            );
        }
        // missile_drone_projectile_active_cb: blinky_light on tag_target,
        // or stop_blinky_light (its last flash taken off).
        "train_moving" => train_move(game, nn, val != 0),
        "missile_drone_projectile_active" => {
            game
                .resource_mut::<Zm>()
                .client_fx
                .pending
                .retain(|p| p.ent != nn || !matches!(p.step, Cue::Blink { .. }));
            if val != 0 {
                queue(game, nn, 0, Cue::Blink { fx: DRONE_LIGHT, tag: "tag_target" });
            } else {
                queue(game, nn, 0, Cue::Bolt { fx: DRONE_LIGHT, tag: "tag_target", off: true });
            }
        }
        // _planemortar.csc's planemortar_contrail: the Lightning Strike
        // jet's afterburner on tag_origin once it is set (nothing takes it
        // off: it goes with the jet).
        "planemortar_contrail" if val != 0 => queue(
            game,
            nn,
            0,
            Cue::Bolt {
                fx: PLANE_EXHAUST,
                tag: "tag_origin",
                off: false,
            },
        ),
        // _ai_tank.csc's `missile_fire` and `death`.
        "ai_tank_missile_fire" => ai_tank::missile_fire(game, nn, val),
        "ai_tank_death" if val != 0 => ai_tank::death(game, nn),
        // _helicopter.csc's `heli_comlink_bootup_anim`: the Stealth Chopper
        // lowers its gear and opens its bay as it arrives.
        "heli_comlink_bootup_anim" if val != 0 => {
            super::mp_fxanims::play(game, nn, "veh_anim_future_heli_gearup_bay_open", false)
        }
        // _helicopter_sounds.csc's `vehicle_is_firing`: the Escort Drone's
        // gun fires, and its brass falls.
        "vehicle_is_firing" => super::engine_sounds::set_firing(game, nn, val != 0),
        // _turret.csc's `turret_microwave_sounds`: the Guardian's hum.
        "turret_microwave_sounds" => turret::sounds(game, nn, val),
        _ => {}
    }
}

/// bo2mp: _callbacks.csc's vehicle spawn: a vehicle's client script starts
/// with it (the AGR's, for now).
pub(super) fn vehicle_spawned(world: &mut World, n: u32, kind: &str) {
    if kind == ai_tank::VEHICLE {
        ai_tank::spawned(world, n);
    }
    heli::spawned(world, n, kind);
}

/// bo2mp: a client flag set or cleared on entity `n` (its client script's
/// flag callback; only the AGR's stun for now).
pub(super) fn client_flag(world: &mut World, n: u32, bit: i32, on: bool) {
    if super::killstreaks::is_turret(world, n) {
        turret::flag(world, n, bit, on);
        return;
    }
    if bit == ai_tank::STUN_FLAG {
        ai_tank::stun(world, n, on);
    }
}

/// bo2mp: mp_express.csc's `train_move`: a car of the bullet train starts
/// (or ends) its run. 0.1 s after it starts, the engine (`self.type ==
/// "vehicle"`) shows `compass_train_engine` on the minimap and raises the
/// dust along the track (`train_fx_think( 1001, 1011 )`), the rear engine
/// (`p6_bullet_train_engine_rev`) raises its own (2001 to 2011) and shows
/// no icon (its objective stays "invisible"), and every other car shows
/// `compass_train_carriage`. Ending (`train_stop`) takes both off.
///
/// Every car also shakes and rumbles near it while it moves
/// (`train_move_think`, started before the 0.1 s wait: `train_shake`).
fn missile_icon(world: &mut World, n: u32, on: bool, size: u32) {
    let owner = super::killstreaks::thrown_owner(world, n);
    let mut zm = world.resource_mut::<Zm>();
    let shows = zm.clock_millis;
    let icons = &mut zm.client_fx.map_icons;
    icons.remove(&n);
    if on {
        icons.insert(
            n,
            MapIcon {
                icon: "remotemissile_target",
                size,
                map_info: MAP_ROTATE | MAP_TINT_IN_CODE,
                shows,
                linger: 400,
                gone: None,
                last: None,
                owner,
            },
        );
    }
}

/// bo2mp: an entity died (a missile went off): its minimap icon goes once
/// it has lingered (`death` / `entityshutdown` then the client script's
/// wait).
pub(super) fn map_icon_died(world: &mut World, n: u32) {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.clock_millis;
    if let Some(i) = zm.client_fx.map_icons.get_mut(&n)
        && i.gone.is_none()
    {
        i.gone = Some(now + i.linger);
    }
}

fn train_move(world: &mut World, n: u32, on: bool) {
    let mut zm = world.resource_mut::<Zm>();
    let Some((vehicle, rev)) = zm
        .ents
        .get(&n)
        .map(|e| (e.class_tag == "script_vehicle", e.mesh == "p6_bullet_train_engine_rev"))
    else {
        return;
    };
    let shows = zm.clock_millis + 100;
    let fx = &mut zm.client_fx;
    fx.pending.retain(|p| p.ent != n || !matches!(p.step, Cue::TrainDust { .. } | Cue::TrainShake));
    fx.map_icons.remove(&n);
    if !on {
        return;
    }
    fx.pending.push(Pending { due: shows - 100, until: i64::MAX, ent: n, step: Cue::TrainShake });
    let (icon, dust) = if vehicle {
        (Some("compass_train_engine"), Some((1001, 1011)))
    } else if rev {
        (None, Some((2001, 2011)))
    } else {
        (Some("compass_train_carriage"), None)
    };
    if let Some(icon) = icon {
        fx.map_icons.insert(
            n,
            MapIcon {
                icon,
                size: 45,
                map_info: MAP_ROTATE | MAP_CLIP,
                shows,
                linger: 0,
                gone: None,
                last: None,
                owner: None,
            },
        );
    }
    if let Some((next, last)) = dust {
        fx.pending.push(Pending {
            due: shows,
            until: i64::MAX,
            ent: n,
            step: Cue::TrainDust { next, last },
        });
    }
}

/// `train_fx_think`'s looks since the last tick: the dust at each next
/// exploder the car has passed (`vectordot( anglestoforward( self.angles ),
/// vectornormalize( origin - self.origin ) ) <= 0`), in order (the script
/// looks again each client frame, 0.01 s; a tick here is 50 ms); the next
/// one to wait for, none once past `last`.
fn train_dust(game: &mut World, nn: u32, mut next: i32, last: i32) -> Option<i32> {
    use gsc_t6::math::{facing_axes, dot, normalize, sub};
    let (spot, rot) = game.resource::<Zm>().ents.get(&nn).map(|en| (en.origin, en.rot))?;
    let forward = facing_axes(rot).0;
    super::with_vm(game, |vm, game| {
        while next <= last {
            let at = super::natives_fx::exploder_origin(vm, next)?;
            if dot(forward, normalize(sub(at, spot))) > 0.0 {
                return Some(next);
            }
            super::natives_fx::fire_exploder(vm, game, &gsc_t6::Key::Int(next));
            diag::info!(Sim, "bo2mp mp_express: train dust {next} (ent{nn})");
            next += 1;
        }
        None
    })
    .flatten()
}

/// mp_express.csc's `train_move_think`: `distancesquared( self.origin,
/// player.origin ) < 262144` (512 units), then `playrumbleonposition(
/// "grenade_rumble", self.origin )` and `earthquake( 0.2, 0.25, self.origin,
/// 512 )` on that player's own machine. False once the car is gone.
fn train_shake(game: &mut World, nn: u32) -> bool {
    const REACH: f32 = 512.0;
    const SCALE: f32 = 0.2;
    const SECS: f32 = 0.25;
    let Some(spot) = game.resource::<Zm>().ents.get(&nn).map(|en| en.origin) else {
        return false;
    };
    let watching: Vec<u32> = super::killcam_seats(game).into_iter().map(|(c, _)| c.0).collect();
    let players: Vec<u32> = game.resource::<Zm>().players.keys().copied().collect();
    for c in players.into_iter().filter(|c| !watching.contains(c)) {
        let Some(at) = super::frame(game).avatar(ClientId(c)).map(|p| p.origin) else {
            continue;
        };
        let d2: f32 = (0..3).map(|i| (at[i] - spot[i]).powi(2)).sum();
        if d2 >= REACH * REACH {
            continue;
        }
        super::natives_fx::rumble_to(game, c, "grenade_rumble", spot);
        let _ = super::killstreaks::shake_to(game, c, SCALE, SECS, spot, REACH);
        if std::env::var_os("BO2MP_RUMBLELOG").is_some() {
            diag::info!(Sim, "bo2mp mp_express: train shake ent{nn} client {c} at {:.0} (scale {SCALE}, {SECS} s, radius {REACH})", d2.sqrt());
        }
    }
    true
}

/// bo2mp: one client script minimap icon to send: its entity, picture,
/// where it is and its yaw, its width and objectiveMapInfo bits, and who
/// fired it.
pub(super) struct ClientObjective {
    pub n: u32,
    pub icon: &'static str,
    pub at: [f32; 3],
    pub heading: f32,
    pub size: u32,
    pub map_info: i32,
    pub owner: Option<u32>,
}

/// bo2mp: the minimap icons the client scripts give, once shown: Express's
/// train cars (`scr_express_size` 45 wide, turning with the car and cut off
/// at the map's edge: `objective_onentity( lcn, id, self, 1, 1, 0 )`) and
/// the Hellstorm's. Each follows its entity; one whose entity died or went
/// stays at its last place until its linger is over, then is dropped.
pub(super) fn client_objectives(world: &mut World) -> Vec<ClientObjective> {
    let mut zm = world.resource_mut::<Zm>();
    let now_ms = zm.clock_millis;
    let zm = &mut *zm;
    let (ents, icons) = (&zm.ents, &mut zm.client_fx.map_icons);
    icons.retain(|nn, ix| {
        if ix.gone.is_none()
            && let Some(e) = ents.get(nn)
        {
            ix.last = Some((e.origin, e.rot[1]));
        } else if ix.gone.is_none() {
            ix.gone = Some(now_ms + ix.linger);
        }
        ix.gone.is_none_or(|g| now_ms < g) && ix.last.is_some()
    });
    icons
        .iter()
        .where_value(|ix| now_ms >= ix.shows)
        .filter_map(|(nn, ix)| {
            let (at, yaw) = ix.last?;
            Some(ClientObjective { n: *nn, icon: ix.icon, at, heading: yaw, size: ix.size, map_info: ix.map_info, owner: ix.owner })
        })
        .collect()
}

/// The other toplayer fields BO2's client scripts hand to the HUD
/// (`setupclientfieldcodecallbacks("toplayer", ...)` in the maps' .csc
/// files), besides every `perk_` and `powerup_` one: Mob's Afterlife
/// (`player_lives`, `player_in_afterlife`, the float `player_afterlife_mana`:
/// its time bar, `afterlife_mana_watch` maps 200 mana to 1) and tomahawk,
/// the carried part (`buildable`: Tranzit, Die Rise, Buried; `craftable`:
/// Mob, Origins), Buried's Vulture Aid meter (float) and tablet, Origins'
/// challenges.
const HUD_FIELDS: [&str; 13] = [
    "player_lives",
    "player_in_afterlife",
    "player_afterlife_mana",
    "tomahawk_in_use",
    "upgraded_tomahawk_in_use",
    "buildable",
    "craftable",
    "vulture_perk_disease_meter",
    "player_tablet_state",
    "challenge_complete_1",
    "challenge_complete_2",
    "challenge_complete_3",
    "challenge_complete_4",
];

/// The world fields BO2's client scripts hand to the HUD
/// (`setupclientfieldcodecallbacks("world", ...)`): Mob's collected parts
/// (the plane's `quest_state1..5` and `quest_plane_craft_complete`, who
/// holds a part `piece_player1..4`, the shield's, the acid kit's, the
/// warden's key) and Buried's Time Bomb and special round.
const WORLD_HUD_FIELDS: [&str; 21] = [
    "quest_state1",
    "quest_state2",
    "quest_state3",
    "quest_state4",
    "quest_state5",
    "quest_plane_craft_complete",
    "piece_player1",
    "piece_player2",
    "piece_player3",
    "piece_player4",
    "piece_riotshield_clamp",
    "piece_riotshield_dolly",
    "piece_riotshield_door",
    "piece_packasplat_blood",
    "piece_packasplat_case",
    "piece_packasplat_fuse",
    "piece_key_warden",
    "time_bomb_hud_toggle",
    "time_bomb_lua_override",
    "time_bomb_saved_round_number",
    "sq_tpo_special_round_active",
];

/// The HUD's world fields as set (0 before) go to every player as
/// `bo2zm_worldcf` ("field:value,...", in `WORLD_HUD_FIELDS`' order); his
/// HUD fires each change as the field's own event, as with his own fields.
fn world_hud_fields(world: &mut World) {
    let (text, players) = {
        let zm = world.resource::<Zm>();
        let text = WORLD_HUD_FIELDS
            .iter()
            .filter_map(|f| {
                let v = *zm.wallbuys.world_fields.get(*f)?;
                (v != 0).then(|| format!("{f}:{v}"))
            })
            .collect::<Vec<_>>()
            .join(",");
        (text, zm.players.keys().copied().collect::<Vec<_>>())
    };
    for n in players {
        super::set_client_dvar(world, n, "bo2zm_worldcf", &text);
    }
}

/// A player's own field (`setclientfieldtoplayer`): the ones his HUD reads
/// (perks, power-ups, `HUD_FIELDS`) go to his client as `bo2zm_icons`
/// ("field:value,...", a float field's value as a float).
fn player_field(world: &mut World, n: u32, name: &str, fvalue: f32) {
    let value = fvalue as i32;
    // bo2mp: the Swarm's lock-on light (a client field callback runs only
    // when its value changes).
    if name == "missile_drone_active" {
        drone_lock(world, n, value);
        return;
    }
    // Buried's town lighting (its client script, solo or online): the sun
    // light and the sky's horizon brightness, as `bo2zm_sunlight`.
    if name == "clientfield_underground_lighting" {
        let text = if value != 0 { "18 10" } else { "13 1" };
        super::set_client_dvar(world, n, "bo2zm_sunlight", text);
        return;
    }
    // Origins' weather (its client script): fog bank 1 dry, 2 rain, 4 snow;
    // snow pales the sun colour (`bo2zm_suncolor`), the others keep the map's.
    // Origins' fire staff (fire_muzzle_fx, toplayer): his own shot's roar.
    if name == "fire_muzzle_fx" {
        if value == 1 {
            super::natives_fx::local_sound(world, EventRecipients::Client(crate::sim_parts::ClientId(n)), "wpn_firestaff_fire_plr");
        }
        return;
    }
    // Origins' staff charging (player_staff_charge_level, zm_tomb.csc): his
    // pad rumbles the level's rumble every 0.1 s until the level changes;
    // his client loops it from `bo2zm_rumble`.
    if name == "player_staff_charge" {
        let rumble = match value {
            1 => "reload_small",
            2 => "damage_light",
            3 => "damage_heavy",
            _ => "",
        };
        if std::env::var_os("BO2MP_T6_WEAPLOG").is_some() {
            diag::info!(Sim, "bo2zm t6 staff charge: client {n} level {value} rumble `{rumble}`");
        }
        super::set_client_dvar(world, n, "bo2zm_rumble", rumble);
        return;
    }
    if name == "player_weather_visionset" {
        let (bank, colour) = match value {
            1 => ("2", ""),
            2 => ("4", "0.769943 0.894102 1"),
            _ => ("", ""),
        };
        super::set_client_dvar(world, n, "bo2zm_fogbank", bank);
        super::set_client_dvar(world, n, "bo2zm_suncolor", colour);
        return;
    }
    if !(name.starts_with("perk_") || name.starts_with("powerup_") || HUD_FIELDS.contains(&name)) {
        return;
    }
    let list = {
        let mut zm = world.resource_mut::<Zm>();
        let Some(p) = zm.players.get_mut(&n) else {
            return;
        };
        let at = p.hud_fields.iter().position(|(k, _)| k == name);
        match (at, fvalue) {
            (Some(i), 0.0) => {
                p.hud_fields.remove(i);
            }
            (Some(i), v) if p.hud_fields[i].1 != v => p.hud_fields[i].1 = v,
            (Some(_), _) => return,
            (None, 0.0) => return,
            (None, v) => p.hud_fields.push((name.to_owned(), v)),
        }
        p.hud_fields
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    super::set_client_dvar(world, n, "bo2zm_icons", &list);
}

/// A player's points counter (`_zm_score::score_cf_increment_info` steps
/// `score_cf_damage`, `score_cf_death_head`, ... on each hit and kill, and
/// wraps): his client gets them as `bo2zm_scorecf` ("field:value,..."), and
/// his HUD flies one "+10" per step (hudcompetitivescoreboardzombie.lua).
fn score_field(game: &mut World, nn: u32, ident: &str, val: i32) {
    let list = {
        let mut zm = game.resource_mut::<Zm>();
        let Some(p) = zm.players.get_mut(&nn) else {
            return;
        };
        match p.score_fields.iter_mut().find(|(k, _)| k == ident) {
            Some((_, v)) if *v == val => return,
            Some((_, v)) => *v = val,
            None => p.score_fields.push((ident.to_owned(), val)),
        }
        p.score_fields
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    super::set_client_dvar(game, nn, "bo2zm_scorecf", &list);
}

/// The visionset manager's two client fields (maps/mp/_visionset_mgr.gsc
/// `update_clientfields`): `visionset_slot`, the player's top vision as its
/// index in `level.vsmgr["visionset"].sorted_name_keys` (the default
/// "none" draws the map's own), and `visionset_lerp`, how far in it is.
/// His client gets them as `bo2zm_vision` ("name lerp"); its vision file is
/// the info's name (Mob's `zm_afterlife` is `vision/zm_afterlife.vision`).
/// The overlay type's two (`overlay_slot`, `overlay_lerp`: Mob's Afterlife
/// screen filter `zm_afterlife_filter`) go as `bo2zm_overlay`.
fn vision_field(vm: &mut Vm<World>, game: &mut World, nn: u32, fld: &str, val: &ScriptValue) {
    let (kind, dvar) = if fld.starts_with("overlay_") {
        ("overlay", "bo2zm_overlay")
    } else {
        ("visionset", "bo2zm_vision")
    };
    let Vision { label: name, lerp } = game
        .resource::<Zm>()
        .client_fx
        .visions(kind)
        .get(&nn)
        .cloned()
        .unwrap_or_else(|| Vision { label: String::new(), lerp: 1.0 });
    let (name, lerp) = if fld.ends_with("_slot") {
        let slot = val.as_int().unwrap_or(0);
        (vision_slot_name(vm, kind, slot).unwrap_or_default(), lerp)
    } else {
        (name, val.as_float().unwrap_or(1.0))
    };
    let name = if name.eq_ignore_ascii_case("none") { String::new() } else { name };
    let text = if !name.is_empty() {
        format!("{} {lerp}", client_vision_name(kind, &name))
    } else if kind == "visionset" {
        game.resource::<Zm>().client_fx.map_vision.clone()
    } else {
        String::new()
    };
    let mut zm = game.resource_mut::<Zm>();
    let before = zm.client_fx.visions_mut(kind).insert(nn, Vision { label: name.clone(), lerp });
    // Down, he sees the last stand's vision; this one comes back when he
    // is up (`laststand_visions`).
    let downed = zm.client_fx.down.get(&nn).is_some_and(|v| v.set_ms.is_some());
    drop(zm);
    if kind == "overlay" {
        let before = before.map(|v| v.label).unwrap_or_default();
        if let Some((to, secs)) = overlay_blur_change(&before, &name) {
            overlay_blur(game, nn, to, secs);
        }
    }
    if kind == "visionset" && downed {
        return;
    }
    super::set_client_dvar(game, nn, dvar, &text);
}

/// The visionset manager's blur overlays (style 2,
/// `vsmgr_register_overlay_info_style_blur`): name, blur, seconds in,
/// seconds out. TranZit's Denizen blurs the screen of the player it holds
/// (`_zm_ai_screecher.csc`); no other map has one.
const BLUR_OVERLAYS: &[(&str, f32, f32, f32)] = &[("zm_ai_screecher_blur", 4.0, 0.1, 0.25)];

/// A player's overlay went from `before` to `now`: a blur overlay blurs his
/// screen to its strength over its seconds in, and back to none over its
/// seconds out when it goes (`overlay_update_cb`). The blur and the seconds.
fn overlay_blur_change(before: &str, now: &str) -> Option<(f32, f32)> {
    if before.eq_ignore_ascii_case(now) {
        return None;
    }
    let find = |name: &str| BLUR_OVERLAYS.iter().find(|b| b.0.eq_ignore_ascii_case(name));
    match (find(before), find(now)) {
        (_, Some(&(_, blur, secs_in, _))) => Some((blur, secs_in)),
        (Some(&(_, _, _, secs_out)), None) => Some((0.0, secs_out)),
        (None, None) => None,
    }
}

/// `setblurbylocalclientnum(blur, seconds)`: his screen blurs from where it
/// is to `to` (as `setblur`).
fn overlay_blur(world: &mut World, n: u32, to: f32, secs: f32) {
    let now = crate::level_time_ms(super::tick(world));
    let mut f = super::frame(world);
    let id = crate::sim_parts::ClientId(n);
    if f.client_meta(id).is_none() {
        return;
    }
    let view = &mut f.client_meta_mut(id).view_effects;
    view.blur = Some(crate::ScriptBlur {
        from: view.blur.map_or(0.0, |b| b.sample(now)),
        to,
        set_ms: now,
        lasts_ms: (secs * 1000.0).round() as i32,
    });
}

/// A player went down (a lethal hit): a `visionsetlaststand` set from now
/// on is his to see.
pub(super) fn went_down(world: &mut World, n: u32) {
    let now_ms = world.resource::<Zm>().clock_millis;
    world
        .resource_mut::<Zm>()
        .client_fx
        .down
        .entry(n)
        .or_insert_with(|| DownView { since: now_ms, set_ms: None, label: String::new(), from: None });
}

/// Each tick: a player down sees `visionsetlaststand`'s vision (BO2's grey
/// `zombie_last_stand`, then `zombie_death` half way to bleeding out),
/// faded in over its seconds - from the one he saw before it, if any. One
/// set before he went down is not his (Who's Who sets none). Up again, he
/// gets the visionset manager's vision back.
pub(super) fn laststand_visions(world: &mut World) {
    let now = world.resource::<Zm>().clock_millis;
    let players: Vec<(u32, bool)> = world
        .resource::<Zm>()
        .players
        .iter()
        .map(|(c, p)| (*c, p.laststand))
        .collect();
    for (c, down) in players {
        let text = {
            let mut zm = world.resource_mut::<Zm>();
            let fx = &mut zm.client_fx;
            if !down {
                let Some(view) = fx.down.remove(&c) else { continue };
                if view.set_ms.is_none() {
                    continue;
                }
                let Vision { label: name, lerp } = fx.visionsets.get(&c).cloned().unwrap_or_default();
                if name.is_empty() { fx.map_vision.clone() } else { format!("{name} {lerp}") }
            } else {
                let Some(v) = fx.laststand.clone() else { continue };
                let view = fx.down.entry(c).or_insert_with(|| DownView {
                    since: now,
                    set_ms: None,
                    label: String::new(),
                    from: None,
                });
                if v.set_ms < view.since {
                    continue;
                }
                if view.set_ms != Some(v.set_ms) {
                    view.from = (!view.label.is_empty() && view.label != v.name).then(|| view.label.clone());
                    view.set_ms = Some(v.set_ms);
                    view.label.clone_from(&v.name);
                }
                let t = if v.secs > 0.0 {
                    ((now - v.set_ms) as f32 / (v.secs * 1000.0)).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                // In steps of 5%: his client re-grades on each new value.
                let t = (t * 20.0).round() / 20.0;
                match &view.from {
                    Some(from) if t < 1.0 => format!("{} {t} {from}", v.name),
                    _ => format!("{} {t}", v.name),
                }
            }
        };
        super::set_client_dvar(world, c, "bo2zm_vision", &text);
    }
}

/// The map's own vision (`map_vision`), sent to each player who is up with
/// no script vision of his own (a player down, or in Who's Who's, gets it
/// when that ends).
pub(crate) fn set_map_vision(game: &mut World, body_text: &str) {
    let players: Vec<u32> = {
        let mut zm = game.resource_mut::<Zm>();
        body_text.clone_into(&mut zm.client_fx.map_vision);
        let fx = &zm.client_fx;
        zm.players
            .keys()
            .copied()
            .filter(|c| {
                !fx.down.get(c).is_some_and(|v| v.set_ms.is_some())
                    && !fx.visionsets.get(c).is_some_and(|v| !v.label.is_empty())
            })
            .collect()
    };
    for c in players {
        super::set_client_dvar(game, c, "bo2zm_vision", body_text);
    }
}

/// The client scripts' registrations (`vsmgr_register_visionset_info`,
/// `vsmgr_register_overlay_info_style_filter`) where the vision file or
/// filter material is not the info's own name: Origins' Zombie Blood info
/// `zm_powerup_zombie_blood_visionset` draws `zm_powerup_zombie_blood`, its
/// overlay the filter `generic_filter_zombie_blood_b`.
const CLIENT_VISIONS: &[(&str, &str, &str)] = &[
    ("visionset", "zm_powerup_zombie_blood_visionset", "zm_powerup_zombie_blood"),
    ("visionset", "zm_perk_divetonuke", "zombie_cosmodrome_divetonuke"),
    ("visionset", "zm_audio_log", "cheat_bw"),
    ("visionset", "zm_power_high_low", "zm_transit_power_int_fluctuate"),
    ("overlay", "zm_powerup_zombie_blood_overlay", "zombie_blood_b"),
    ("overlay", "vulture_stink_overlay", "zombie_perk_vulture"),
];

/// The name his client is sent for an info: its vision file, or its
/// filter's material word; the info's own name for the rest.
fn client_vision_name<'a>(kind: &str, name: &'a str) -> &'a str {
    CLIENT_VISIONS
        .iter()
        .find(|(k, info, _)| *k == kind && info.eq_ignore_ascii_case(name))
        .map_or(name, |&(_, _, to)| to)
}

fn vision_slot_name(vm: &mut Vm<World>, kind: &str, slot: i32) -> Option<String> {
    let vsmgr = vm.intern("vsmgr");
    let ScriptValue::Array(types) = vm.raw_field(vm.level, vsmgr) else {
        return None;
    };
    let visionset = gsc_t6::Key::Str(vm.intern(kind));
    let Some(ScriptValue::Object(info)) = types.get(&visionset) else {
        return None;
    };
    let keys = vm.intern("sorted_name_keys");
    let ScriptValue::Array(names) = vm.raw_field(info, keys) else {
        return None;
    };
    names.get(&gsc_t6::Key::Int(slot)).map(|v| vm.to_text(&v))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // A script camera: camerasetposition(entity), cameraactivate(on);
    // camerasetlookat() keeps the camera entity's own angles.
    vm.bind("camerasetposition", true, |vm, world, s, a| {
        if let (Some(n), Some(cam)) = (entnum(vm, s), entnum(vm, arg(a, 0)))
            && let Some(p) = world.resource_mut::<Zm>().players.get_mut(&n)
        {
            p.camera = Some(cam);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("cameraactivate", true, |vm, world, s, a| {
        let on = !matches!(arg(a, 0), ScriptValue::Int(0)) && !arg(a, 0).is_undefined();
        if let Some(n) = entnum(vm, s)
            && let Some(p) = world.resource_mut::<Zm>().players.get_mut(&n)
        {
            p.camera_on = on;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("camerasetlookat", true, |_, _, _, _| Ok(ScriptValue::Undefined));
    // setviewmodel(name): his first-person hands (Mob's Afterlife turns on
    // c_zom_ghost_viewhands, then puts his own back), sent as the client
    // dvar `bo2zm_viewhands`.
    vm.bind("setviewmodel", true, |vm, world, s, a| {
        if let Some(n) = entnum(vm, s) {
            let name = vm.to_text(arg(a, 0)).to_ascii_lowercase();
            super::set_client_dvar(world, n, "bo2zm_viewhands", &name);
        }
        Ok(ScriptValue::Undefined)
    });
    // setclientuivisibilityflag("hud_visible", 0): his HUD hides (the
    // game-over shot); script HUD text still shows.
    // bo2mp: setclientuivisibilityflag("g_compassShowEnemies", 1): a moment
    // of radar (the Lightning Strike's pick shows where enemies are).
    vm.bind("setclientuivisibilityflag", true, |vm, world, s, a| {
        let flag = vm.to_text(arg(a, 0));
        if flag == "hud_visible"
            && let Some(n) = entnum(vm, s)
        {
            let hidden = matches!(arg(a, 1), ScriptValue::Int(0));
            super::set_client_dvar(world, n, "bo2zm_hud_hidden", if hidden { "1" } else { "" });
        }
        if flag == "g_compassShowEnemies"
            && arg(a, 1).as_float().unwrap_or(0.0) != 0.0
            && let Some(n) = entnum(vm, s)
            && world.resource::<Zm>().mp
        {
            super::killstreaks::radar_pulse(world, n);
        }
        Ok(ScriptValue::Undefined)
    });
    // setclientammocounterhide(1) / setclientminiscoreboardhide(1) (the
    // game over): his ammo counter and points hide (BO2's HUD bit 12).
    fn hide(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, lhs: &[ScriptValue], lookup_key: &str) {
        if let Some(n) = entnum(vm, st) {
            let on = !matches!(arg(lhs, 0), ScriptValue::Int(0)) && !arg(lhs, 0).is_undefined();
            super::set_client_dvar(game, n, lookup_key, if on { "1" } else { "" });
        }
    }
    vm.bind("setclientammocounterhide", true, |vm, world, s, a| {
        hide(vm, world, s, a, "bo2zm_ammo_hide");
        Ok(ScriptValue::Undefined)
    });
    vm.bind("setclientminiscoreboardhide", true, |vm, world, s, a| {
        hide(vm, world, s, a, "bo2zm_score_hide");
        Ok(ScriptValue::Undefined)
    });
    vm.bind("codesetplayerstateclientfield", false, |vm, world, _, a| {
        if let Some(n) = entnum(vm, arg(a, 0)) {
            let name = vm.to_text(arg(a, 1));
            remember(world, n, true, &name, arg(a, 2));
            if matches!(
                name.as_str(),
                "visionset_slot" | "visionset_lerp" | "overlay_slot" | "overlay_lerp"
            ) {
                vision_field(vm, world, n, &name, arg(a, 2));
                return Ok(ScriptValue::Undefined);
            }
            let fvalue = arg(a, 2).as_float().unwrap_or(0.0);
            let value = fvalue as i32;
            if name == "clientfield_sq_vo" {
                super::highrise::maxis_vo(vm, world, n, value);
            }
            player_field(world, n, &name, fvalue);
        }
        Ok(ScriptValue::Undefined)
    });
    // maps/mp/_utility::setclientfield calls these.
    vm.bind("codesetworldclientfield", false, |vm, world, _, a| {
        let name = vm.to_text(arg(a, 0));
        remember(world, u32::MAX, false, &name, arg(a, 1));
        let value = arg(a, 1).as_int().unwrap_or(0);
        super::wallbuys::world_field(vm, world, &name, value);
        swarm::world_field(world, &name, value);
        super::prison_wolves::world_field(vm, world, &name, i64::from(value));
        super::tomb_beacon::world_field(vm, world, &name, i64::from(value));
        if WORLD_HUD_FIELDS.contains(&name.as_str()) {
            world_hud_fields(world);
        }
        Ok(ScriptValue::Undefined)
    });
    // visionsetlaststand(name, seconds): the vision a player down sees
    // (maps/mp/zombies/_zm::player_laststand "zombie_last_stand" over 1 s;
    // _zm_laststand "zombie_death" over the bleed-out's second half), shown
    // by `laststand_visions`.
    vm.bind("visionsetlaststand", false, |vm, world, _, a| {
        let name = vm.to_text(arg(a, 0)).to_ascii_lowercase();
        let secs = arg(a, 1).as_float().unwrap_or(0.0).max(0.0);
        let mut zm = world.resource_mut::<Zm>();
        let set_ms = zm.clock_millis;
        zm.client_fx.laststand = (!name.is_empty()).then_some(LastStandVision { name, secs, set_ms });
        Ok(ScriptValue::Undefined)
    });
    vm.bind("codesetclientfield", false, |vm, world, _, a| {
        let ent = arg(a, 0).clone();
        let name = vm.to_text(arg(a, 1));
        if let Some(n) = entnum(vm, &ent) {
            remember(world, n, false, &name, arg(a, 2));
        }
        let value = arg(a, 2).as_int().map_or(0, i64::from);
        entity_field(vm, world, &ent, &name, value);
        Ok(ScriptValue::Undefined)
    });
    // Scripts that call the method directly.
    vm.bind("setclientfield", true, |vm, world, s, a| {
        let name = vm.to_text(arg(a, 0));
        let value = arg(a, 1).as_int().unwrap_or(0);
        if matches!(s, ScriptValue::Object(o) if matches!(vm.kind(*o), Some(ObjKind::Level))) {
            remember(world, u32::MAX, false, &name, arg(a, 1));
            super::wallbuys::world_field(vm, world, &name, value);
            swarm::world_field(world, &name, value);
            super::prison_wolves::world_field(vm, world, &name, i64::from(value));
            super::tomb_beacon::world_field(vm, world, &name, i64::from(value));
        } else {
            if let Some(n) = entnum(vm, s) {
                remember(world, n, false, &name, arg(a, 1));
            }
            entity_field(vm, world, s, &name, i64::from(value));
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: maps/mp/_utility::getclientfield and getclientfieldtoplayer:
    // Black Ops II reads back the value last set (0 for one never set).
    vm.bind("codegetclientfield", false, |vm, world, _, a| {
        let n = entnum(vm, arg(a, 0)).ok_or("not an entity")?;
        let name = vm.to_text(arg(a, 1));
        Ok(recall(world, n, false, &name))
    });
    vm.bind("codegetplayerstateclientfield", false, |vm, world, _, a| {
        let n = entnum(vm, arg(a, 0))
            .filter(|n| world.resource::<Zm>().players.contains_key(n))
            .ok_or("GScr_CodeGetPlayerStateClientField: must be called on a player.")?;
        let name = vm.to_text(arg(a, 1));
        Ok(recall(world, n, true, &name))
    });
    vm.bind("codegetworldclientfield", false, |vm, world, _, a| {
        let name = vm.to_text(arg(a, 0));
        Ok(recall(world, u32::MAX, false, &name))
    });
}

/// bo2mp: `missile_drone_active_cb` (`_missile_drone.csc`): 2 is
/// `targetacquired` (after a twentieth of a second, the light on his gun's
/// tag_target every half second while he holds it ready); 1 is `targetscan`
/// (nothing shows); 0 is `targetlost` (the light off).
fn drone_lock(game: &mut World, nn: u32, val: i32) {
    {
        let mut zm = game.resource_mut::<Zm>();
        let fx = &mut zm.client_fx;
        if fx.drone_lock.insert(nn, val).unwrap_or(0) == val {
            return;
        }
        fx.pending
            .retain(|p| p.ent != nn || !matches!(p.step, Cue::HeldLight { .. }));
    }
    match val {
        2 => queue(game, nn, 50, Cue::HeldLight { sounded: false }),
        1 => {}
        _ => super::natives_fx::viewmodel_effect(game, nn, DRONE_LIGHT, true),
    }
}

/// One turn of `targetacquired`'s loop: the light and when to look again,
/// or None when he is gone.
fn held_light(game: &mut World, n: u32, sounded: bool) -> Option<(i64, bool)> {
    use weapon_t6::state;
    let cid = crate::sim_parts::ClientId(n);
    let (ready, spot) = {
        let fl = super::frame(game);
        let pstate = fl.avatar(cid)?;
        let label = crate::client_script::weapon_name(&fl, pstate.weapon);
        let held = label == "missile_drone_mp" || label == "inventory_missile_drone_mp";
        let busy = pstate.weapon_raise_fraction > 0.0
            || (state::is_secondary_gear(pstate.weapon_state_t6) && pstate.weapon_state_t6 != state::OFFHAND_DETONATE)
            || (state::MELEE_INIT..=state::MELEE_END).contains(&pstate.weapon_state_t6);
        let ready = held && !busy && crate::client_script::ammo_clip(&fl, cid, pstate.weapon) > 0;
        (ready, pstate.origin)
    };
    if !ready {
        return Some((1000, sounded));
    }
    super::natives_fx::viewmodel_effect(game, n, DRONE_LIGHT, false);
    if !sounded {
        super::natives_fx::sound(game, EventRecipients::Client(cid), "fly_hunter_raise_plr", spot);
    }
    Some((500, true))
}

// bo2mp: Carrier's far ships (mp_carrier.csc vista_ships_init and
// floatmyboat; the cscr_boat* dvars are unset, so their defaults).
const BOAT_SWAY_TIME: f32 = 4.0;
const BOAT_SWAY_SLOWDOWN_TIME: f32 = 2.0;
const BOAT_ROLL: f32 = 1.0;

/// bo2mp: what a map's client script starts on its own once the level is up
/// (client scripts do not run here; their part is played by the engine).
pub(super) fn map_start(vm: &mut Vm<World>, game: &mut World, map: &str) {
    core_start(vm, game, map);
    match map {
        "mp_carrier" => carrier_start(vm, game),
        "mp_nuketown_2020" => nuked_start(vm, game),
        "mp_hydro" | "mp_vertigo" => cameras_start(vm, game, map),
        "mp_uplink" => uplink_start(vm, game),
        "mp_turbine" => turbine_start(game),
        "mp_drone" => drone_start(vm, game),
        _ => {}
    }
}

/// bo2mp: what a map's client script does on a `clientnotify` from the
/// server scripts.
pub(super) fn client_notify(vm: &mut Vm<World>, game: &mut World, label: &str) {
    if game.resource::<Zm>().map == "mp_nuketown_2020" {
        nuked_notify(vm, game, label);
    }
    // A client `level notify` the props may wait on (`fxanim_waittill_N`).
    super::mp_fxanims::level_notify(game, label);
}

/// The entities a map's client script finds by targetname.
fn named(vm: &mut Vm<World>, game: &mut World, label: &str) -> Vec<u32> {
    let label = ScriptValue::Str(vm.intern(label));
    let key = ScriptValue::Str(vm.intern("targetname"));
    super::natives_ent::matching(vm, game, &[label, key])
        .iter()
        .filter_map(|e| entnum(vm, e))
        .collect()
}

fn carrier_start(vm: &mut Vm<World>, world: &mut World) {
    // vista_ships_init: every `carrier_vista_ships` floats
    // (floatmyboat(clientnum, startleft 0, randomize 1)).
    let name = ScriptValue::Str(vm.intern("carrier_vista_ships"));
    let key = ScriptValue::Str(vm.intern("targetname"));
    let ships = super::natives_ent::matching(vm, world, &[name, key]);
    for ship in &ships {
        let Some(n) = entnum(vm, ship) else { continue };
        let Some(base) = world.resource::<Zm>().ents.get(&n).map(|e| e.rot) else {
            continue;
        };
        world.resource_mut::<Zm>().client_fx.pending.push(Pending {
            due: 0,
            until: i64::MAX,
            ent: n,
            step: Cue::Sway {
                base,
                leg: 0,
                time: BOAT_SWAY_TIME,
            },
        });
    }
    diag::info!(Sim, "bo2mp mp_carrier: {} far ships rock", ships.len());
}

/// One leg of floatmyboat. Its loop adds randomfloat(3) to the sway time
/// before each rock out and back and never resets it (the reset is in a
/// developer block), so the ships rock slower and slower. The script
/// writes the original pitch as the roll too: `( pitch - roll, yaw, pitch )`.
fn sway(world: &mut World, n: u32, base: [f32; 3], leg: u8, time: f32, now: i64) {
    let time = if leg == 0 {
        let r = with_rand(world);
        time + r * 3.0
    } else {
        time
    };
    let to = if leg == 0 {
        [base[0] - BOAT_ROLL, base[1], base[0]]
    } else {
        [base[0], base[1], base[0]]
    };
    let mut zm = world.resource_mut::<Zm>();
    let Some(from) = zm.ents.get(&n).map(|e| e.rot) else { return };
    zm.movers.rotate_to(
        n,
        from,
        to,
        now,
        time,
        BOAT_SWAY_SLOWDOWN_TIME,
        BOAT_SWAY_SLOWDOWN_TIME,
    );
    // `waittill( "rotatedone" )`, then the other leg.
    zm.client_fx.pending.push(Pending {
        due: now + (time * 1000.0).round().max(1.0) as i64,
        until: i64::MAX,
        ent: n,
        step: Cue::Sway {
            base,
            leg: 1 - leg,
            time,
        },
    });
}

// bo2mp: Nuketown 2025 (mp_nuketown_2020.csc setupclientsideobjects).
// nuked_car_flip lowers the end-game cars 120 units and hides them and the
// display glass until the server's `bomb_drop_pre`.
const NUKED_CARS: [&str; 2] = ["nuke_animated_car01", "nuke_animated_car02"];
const NUKED_GLASS: &str = "nuke_display_glass_client";
const NUKED_CAR_DROP: f32 = 120.0;
// spinner_rotate( 1 ): step 180, `rotatevelocity( ( 0, step / 2, 0 ), 3600 )`.
const NUKED_SPIN: f32 = 90.0;
const NUKED_SPIN_TIME: f32 = 3600.0;

fn nuked_start(vm: &mut Vm<World>, world: &mut World) {
    // nuked_energy_sign_think: the topper spins.
    let topper = named(vm, world, "nuketown_sign_topper");
    for &n in &topper {
        world.resource_mut::<Zm>().client_fx.pending.push(Pending {
            due: 0,
            until: i64::MAX,
            ent: n,
            step: Cue::Spin,
        });
    }
    let cars = nuked_cars(vm, world);
    let glass = named(vm, world, NUKED_GLASS);
    let mut zm = world.resource_mut::<Zm>();
    for n in &cars {
        if let Some(en) = zm.ents.get_mut(n) {
            en.origin[2] -= NUKED_CAR_DROP;
            en.unseen = true;
        }
    }
    for n in &glass {
        if let Some(en) = zm.ents.get_mut(n) {
            en.unseen = true;
        }
    }
    diag::info!(
        Sim,
        "bo2mp mp_nuketown_2020: {} sign topper spins, {} end-game cars and {} display glass hidden",
        topper.len(),
        cars.len(),
        glass.len()
    );
}

fn nuked_cars(vm: &mut Vm<World>, world: &mut World) -> Vec<u32> {
    NUKED_CARS
        .iter()
        .flat_map(|name| named(vm, world, name))
        .collect()
}

fn nuked_notify(vm: &mut Vm<World>, sim: &mut World, ident: &str) {
    match ident {
        // nuked_car_flip: the cars come back up and show, with the glass
        // (its `wind_global_vector` is the effects' wind: not modelled).
        "bomb_drop_pre" => {
            let cars = nuked_cars(vm, sim);
            let glass = named(vm, sim, NUKED_GLASS);
            let mut zm = sim.resource_mut::<Zm>();
            for n in &cars {
                if let Some(en) = zm.ents.get_mut(n) {
                    en.origin[2] += NUKED_CAR_DROP;
                    en.unseen = false;
                }
            }
            for n in &glass {
                if let Some(en) = zm.ents.get_mut(n) {
                    en.unseen = false;
                }
            }
            diag::info!(Sim, "bo2mp mp_nuketown_2020: {} end-game cars up", cars.len());
        }
        // rotate_flags: each `ending_flag` turns to ( 0, randomintrange( 75,
        // 105 ), 0 ) over randomfloatrange( 0.5, 0.65 ) s.
        "bomb_drop" => {
            let flags = named(vm, sim, "ending_flag");
            let now = sim.resource::<Zm>().clock_millis;
            for &n in &flags {
                let yaw = (75.0 + (with_rand(sim) * 30.0).floor()).min(104.0);
                let secs = 0.5 + with_rand(sim) * 0.15;
                let mut zm = sim.resource_mut::<Zm>();
                let Some(from) = zm.ents.get(&n).map(|en| en.rot) else { continue };
                zm.movers.rotate_to(n, from, [0.0, yaw, 0.0], now, secs, 0.0, 0.0);
            }
            diag::info!(Sim, "bo2mp mp_nuketown_2020: {} flags turn", flags.len());
            // nuked_dome_explosion_think: the dome's fxanim waits on this.
            super::mp_fxanims::level_notify(sim, "fxanim_dome_explode_start");
        }
        // nuked_car_flip: the cars play cardestroy1/2 (their notes break the
        // glass, exploder 1005, and black the screen out) and the glass
        // displayGlassDestroy.
        "nuke_car_flip" => {
            for (ident, clip) in NUKED_CARS.iter().zip(super::mp_fxanims::NUKED_CAR_ANIMS) {
                for n in named(vm, sim, ident) {
                    super::mp_fxanims::play(sim, n, clip, true);
                }
            }
            for n in named(vm, sim, NUKED_GLASS) {
                super::mp_fxanims::play(sim, n, super::mp_fxanims::NUKED_GLASS_ANIM, false);
            }
        }
        _ => {}
    }
}

/// One `rotatevelocity( ( 0, 90, 0 ), 3600 )`, then `rotatedone` and again.
fn spin(world: &mut World, n: u32, now: i64) {
    let mut zm = world.resource_mut::<Zm>();
    let Some(from) = zm.ents.get(&n).map(|e| e.rot) else { return };
    let to = [from[0], from[1] + NUKED_SPIN * NUKED_SPIN_TIME, from[2]];
    zm.movers.rotate_to(n, from, to, now, NUKED_SPIN_TIME, 0.0, 0.0);
    zm.client_fx.pending.push(Pending {
        due: now + (NUKED_SPIN_TIME * 1000.0) as i64,
        until: i64::MAX,
        ent: n,
        step: Cue::Spin,
    });
}

/// A client script's own loop: its next turn at `due`, for as long as the
/// map runs.
fn again(world: &mut World, ent: u32, due: i64, step: Cue) {
    world.resource_mut::<Zm>().client_fx.pending.push(Pending {
        due,
        until: i64::MAX,
        ent,
        step,
    });
}

/// The loops that belong to no entity (the wind, the lightning, a sound).
const LEVEL: u32 = trace_t6::ENTITYNUM_WORLD as u32;

/// A wait of `secs`, in milliseconds (a zero wait is the next frame).
fn ms(secs: f32) -> i64 {
    (secs * 1000.0).round().max(1.0) as i64
}

/// `randomfloatrange( lo, hi )`.
fn rand_float(world: &mut World, lo: f32, hi: f32) -> f32 {
    lo + with_rand(world) * (hi - lo)
}

/// `randomint( n )`.
fn rand_int(world: &mut World, n: u32) -> u32 {
    super::with_vm(world, |vm, _| vm.rand_u32() % n).unwrap_or(0)
}

/// `rotateto`: the shortest way round to `to`, as the engine turns.
fn rotate_to(world: &mut World, n: u32, to: [f32; 3], now: i64, secs: f32, speed_up_rate: f32, decel: f32) {
    let mut zm = world.resource_mut::<Zm>();
    let Some(from) = zm.ents.get(&n).map(|e| e.rot) else { return };
    let mut target = to;
    for i in 0..3 {
        target[i] = from[i] + gsc_t6::math::angle_clamp180(to[i] - from[i]);
    }
    zm.movers.rotate_to(n, from, target, now, secs, speed_up_rate, decel);
}

/// `rotateyaw` / `rotatepitch` / `rotateroll`: `degrees` more on one axis.
fn rotate_by(game: &mut World, nn: u32, around: usize, degrees: f32, now_ms: i64, secs: f32) {
    let mut zm = game.resource_mut::<Zm>();
    let Some(from) = zm.ents.get(&nn).map(|e| e.rot) else { return };
    let mut to = from;
    to[around] += degrees;
    zm.movers.rotate_to(nn, from, to, now_ms, secs, 0.0, 0.0);
}

/// The entities a map's client script finds by targetname, with their
/// script objects (for their keys).
fn named_objs(vm: &mut Vm<World>, game: &mut World, label: &str) -> Vec<(u32, gsc_t6::ObjRef)> {
    let label = ScriptValue::Str(vm.intern(label));
    let key = ScriptValue::Str(vm.intern("targetname"));
    super::natives_ent::matching(vm, game, &[label, key])
        .iter()
        .filter_map(|e| match e {
            ScriptValue::Object(o) => Some((entnum(vm, e)?, *o)),
            _ => None,
        })
        .collect()
}

// bo2mp: _rotating_object.csc's defaults: 1000 turns of 360, each 12 s.
const TURN_REVOLUTIONS: f32 = 1000.0;
const FULL_CIRCLE_DEGREES: f32 = 360.0;
const TURN_TIME: f32 = 12.0;
// _ambient.csc clocks_init: each hand's turn per hour, minute or second
// shown, and its tick (`rotate_bit`).
const HOUR_TURN: f32 = 30.0;
const HOUR_BIT: f32 = 0.00833333;
const MINUTE_TURN: f32 = 6.0;
const MINUTE_BIT: f32 = 0.1;
const SECOND_TURN: f32 = 6.0;
const SECOND_BIT: f32 = 6.0;
const CLOCK_TURN_TIME: f32 = 0.05;

/// bo2mp: what _players.csc's on_player_connect starts on every map:
/// _rotating_object.csc (every `rotating_object` spins on the axis its
/// `script_noteworthy` names, one turn per `script_float` seconds, the
/// other way when that is below 0) and _ambient.csc's clocks_init (its
/// ceiling fans and anemometers, `ceiling_fan`, `spinner1` and `spinner2`,
/// are on no map).
fn core_start(vm: &mut Vm<World>, game: &mut World, map: &str) {
    let noteworthy = vm.intern("script_noteworthy");
    let float = vm.intern("script_float");
    let objects = named_objs(vm, game, "rotating_object");
    for &(n, o) in &objects {
        let around = vm.raw_field(o, noteworthy);
        let around = if around.is_undefined() { "yaw".to_string() } else { vm.to_text(&around) };
        let time = vm.raw_field(o, float);
        let mut secs = if time.is_undefined() { TURN_TIME } else { time.as_float().unwrap_or(0.0) };
        if secs == 0.0 {
            secs = TURN_TIME;
        }
        let mut direction = FULL_CIRCLE_DEGREES;
        if secs < 0.0 {
            direction = -direction;
            secs = -secs;
        }
        let around = match around.as_str() {
            "roll" => 2,
            "pitch" => 0,
            _ => 1,
        };
        let step = Cue::Turn {
            around,
            degrees: direction * TURN_REVOLUTIONS,
            secs: secs * TURN_REVOLUTIONS,
        };
        again(game, n, 0, step);
    }
    // clocks_init: the hands are set to the PC's time (`getsystemtime()`,
    // local), the hour to 1..12.
    let now = local_time();
    let hours = match now[0] {
        h if h > 12 => h - 12,
        0 => 12,
        h => h,
    };
    let (minutes, seconds) = (now[1], now[2]);
    let hands = [
        ("hour_hand", hours as f32 * HOUR_TURN, Some((minutes * 60 + seconds) as f32 * HOUR_BIT), HOUR_BIT),
        ("minute_hand", minutes as f32 * MINUTE_TURN, Some(seconds as f32 * MINUTE_BIT), MINUTE_BIT),
        ("second_hand", seconds as f32 * SECOND_TURN, None, SECOND_BIT),
    ];
    let mut clock_hands = 0;
    for (name, set, first, bit) in hands {
        for n in named(vm, game, name) {
            let step = Cue::Clock {
                stage: 0,
                set,
                first,
                bit,
                last: now,
            };
            again(game, n, 0, step);
            clock_hands += 1;
        }
    }
    if !objects.is_empty() || clock_hands > 0 {
        diag::info!(
            Sim,
            "bo2mp {map}: {} props spin, {clock_hands} clock hands set to {:02}:{:02}:{:02}",
            objects.len(),
            now[0],
            now[1],
            now[2]
        );
    }
}

/// The PC's local time (hour, minute, second), as the client's
/// `getsystemtime()` reads it (Com_RealTime, local).
fn local_time() -> [u32; 3] {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            year: u16,
            month: u16,
            day_of_week: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            milliseconds: u16,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLocalTime(time: *mut SystemTime);
        }
        let mut t = SystemTime::default();
        // SAFETY: GetLocalTime only fills in the struct it is given.
        unsafe { GetLocalTime(&mut t) };
        [u32::from(t.hour), u32::from(t.minute), u32::from(t.second)]
    }
    #[cfg(not(windows))]
    {
        let s = std::time::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs();
        [((s / 3600) % 24) as u32, ((s / 60) % 60) as u32, (s % 60) as u32]
    }
}

/// One turn of rotating_object_think's loop: `rotate*( direction *
/// revolutions, rotate_time * revolutions )`, then `rotatedone` and again.
fn turn(game: &mut World, nn: u32, around: usize, degrees: f32, secs: f32, now_ms: i64) {
    if !game.resource::<Zm>().ents.contains_key(&nn) {
        return;
    }
    rotate_by(game, nn, around, degrees, now_ms, secs);
    again(game, nn, now_ms + ms(secs), Cue::Turn { around, degrees, secs });
}

/// One step of clock_run: `rotatepitch( hand_time * rotate, 0.05 )`, then
/// `first_rotate` the same way, each waiting `rotatedone`; then once a
/// second it reads the time again and, when that differs from the last,
/// `rotatepitch( rotate_bit, 0.05 )`.
fn clock(world: &mut World, n: u32, stage: u8, set: f32, first: Option<f32>, bit: f32, last: [u32; 3], now: i64) {
    if !world.resource::<Zm>().ents.contains_key(&n) {
        return;
    }
    let mut step = Cue::Clock {
        stage: stage + 1,
        set,
        first,
        bit,
        last,
    };
    let due = match (stage, first) {
        (0, _) => {
            rotate_by(world, n, 0, set, now, CLOCK_TURN_TIME);
            now + ms(CLOCK_TURN_TIME)
        }
        (1, Some(first)) => {
            rotate_by(world, n, 0, first, now, CLOCK_TURN_TIME);
            now + ms(CLOCK_TURN_TIME)
        }
        // `prev_time = getsystemtime()`; the loop's first look is the same
        // moment, so the first tick waits for the next look.
        (1, None) | (2, _) => {
            step = Cue::Clock {
                stage: 3,
                set,
                first,
                bit,
                last: local_time(),
            };
            now + 1000
        }
        _ => {
            let time = local_time();
            if time != last {
                rotate_by(world, n, 0, bit, now, CLOCK_TURN_TIME);
            }
            step = Cue::Clock {
                stage: 3,
                set,
                first,
                bit,
                last: time,
            };
            now + 1000
        }
    };
    again(world, n, due, step);
}

// bo2mp: cameratrackplayer: the turn to the player and the wait.
const CAMERA_TIME: f32 = 0.5;

fn cameras_start(vm: &mut Vm<World>, game: &mut World, map: &str) {
    // After the first snapshot: every `security_camera_ball` keeps its
    // pitch and roll and turns about its yaw.
    let cameras = named(vm, game, "security_camera_ball");
    for &n in &cameras {
        let Some(angles) = game.resource::<Zm>().ents.get(&n).map(|e| e.rot) else {
            continue;
        };
        let step = Cue::Camera {
            pitch: angles[0],
            roll: angles[2],
        };
        again(game, n, 0, step);
    }
    diag::info!(Sim, "bo2mp {map}: {} security cameras follow the player", cameras.len());
}

/// One turn of cameratrackplayer's loop: `rotateto( ( pitch, angles[1] +
/// 90, roll ), 0.5 )` where `angles = vectortoangles( localplayer.origin -
/// self.origin )`, then `wait 0.5`. Its local player is the first player
/// in the game here.
fn camera(world: &mut World, n: u32, pitch: f32, roll: f32, now: i64) {
    let Some(origin) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else {
        return;
    };
    let ids: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let player = {
        let f = super::frame(world);
        ids.into_iter()
            .find_map(|p| f.avatar(crate::sim_parts::ClientId(p)).map(|ps| ps.origin))
    };
    if let Some(at) = player {
        let angles = gsc_t6::math::vector_to_angles(gsc_t6::math::sub(at, origin));
        rotate_to(world, n, [pitch, angles[1] + 90.0, roll], now, CAMERA_TIME, 0.0, 0.0);
    }
    again(world, n, now + ms(CAMERA_TIME), Cue::Camera { pitch, roll });
}

// bo2mp: Uplink (mp_uplink.csc). The rain and window exploders the wind
// switch starts and stops are left out here: this engine's world effects
// cannot be stopped once placed.
const UPLINK_WIND_TIME: f32 = 8.0;
const UPLINK_VANE_WAIT: f32 = 0.5;
const UPLINK_LIGHTNING: [i32; 2] = [7001, 7005];

fn uplink_start(vm: &mut Vm<World>, world: &mut World) {
    let now = world.resource::<Zm>().clock_millis;
    // uplinkonplayerconnect: the one `sway_lightpost` sways; every
    // `wind_direction` vane waits on the wind.
    let post = named(vm, world, "sway_lightpost");
    if let Some(&n) = post.first() {
        again(world, n, 0, Cue::Post { angle: 0.0, secs: 0.0, leg: 0 });
    }
    let vanes: Vec<(u32, [f32; 3])> = named(vm, world, "wind_direction")
        .into_iter()
        .filter_map(|n| Some((n, world.resource::<Zm>().ents.get(&n)?.rot)))
        .collect();
    let count = vanes.len();
    world.resource_mut::<Zm>().client_fx.vanes = vanes;
    // rainexploderswitch turns the wind at once (after waittillframeend);
    // initlightningloop waits randomfloatrange( 10, 15 ) first.
    again(world, LEVEL, now, Cue::Wind);
    let wait = rand_float(world, 10.0, 15.0);
    again(world, LEVEL, now + ms(wait), Cue::Lightning);
    diag::info!(
        Sim,
        "bo2mp mp_uplink: {} light post sways, {count} wind vanes, the wind turns every 8 s, lightning in {wait:.1} s",
        post.len().min(1)
    );
}

/// One leg of lightpostsway: `rotateto( ( a * 0.5, a * 0.6, a * 0.8 ), t,
/// t * 0.3, t * 0.3 )` with a = randomfloatrange( 0.25, 0.5 ) and t =
/// randomfloatrange( 2, 4 ), then the same negated, each waiting
/// `rotatedone`.
fn post(world: &mut World, n: u32, angle: f32, secs: f32, leg: u8, now: i64) {
    let (angle, secs, sign) = if leg == 0 {
        let angle = rand_float(world, 0.25, 0.5);
        (angle, rand_float(world, 2.0, 4.0), 1.0)
    } else {
        (angle, secs, -1.0)
    };
    let to = [angle * 0.5 * sign, angle * 0.6 * sign, angle * 0.8 * sign];
    rotate_to(world, n, to, now, secs, secs * 0.3, secs * 0.3);
    again(world, n, now + ms(secs), Cue::Post { angle, secs, leg: 1 - leg });
}

/// One turn of rainexploderswitch: `randomint( 2 )` blows the wind to 270
/// (rain exploder 1002) or to 90 (1001), `wind_changed` sets every vane
/// going, then `serverwait( 8 )`.
fn wind(world: &mut World, now: i64) {
    let yaw = if rand_int(world, 2) != 0 { 270.0 } else { 90.0 };
    let (vanes, turn) = {
        let mut zm = world.resource_mut::<Zm>();
        zm.client_fx.wind_turns += 1;
        (zm.client_fx.vanes.clone(), zm.client_fx.wind_turns)
    };
    for (n, orig) in vanes {
        let step = Cue::Vane {
            orig,
            yaw,
            stage: 0,
            wind: turn,
        };
        again(world, n, now + ms(UPLINK_VANE_WAIT), step);
    }
    again(world, LEVEL, now + ms(UPLINK_WIND_TIME), Cue::Wind);
}

/// One step of windvanejitter: `rotateto( ( o[0], yaw, o[2] ), 1 )`, then
/// over and over `rotateto( ( o[0], randomfloatrange( yaw - 30, yaw + 30 ),
/// o[2] ), randomfloatrange( 0.1, 0.5 ) )`, each waiting `rotatedone`, until
/// the wind turns again.
fn vane(world: &mut World, n: u32, orig: [f32; 3], yaw: f32, stage: u8, wind: u32, now: i64) {
    if world.resource::<Zm>().client_fx.wind_turns != wind {
        return;
    }
    let (to, secs) = if stage == 0 {
        (yaw, 1.0)
    } else {
        let secs = rand_float(world, 0.1, 0.5);
        (rand_float(world, yaw - 30.0, yaw + 30.0), secs)
    };
    rotate_to(world, n, [orig[0], to, orig[2]], now, secs, 0.0, 0.0);
    let step = Cue::Vane {
        orig,
        yaw,
        stage: 1,
        wind,
    };
    again(world, n, now + ms(secs), step);
}

/// playlightning: `randomintrange( 7001, 7005 )`'s exploder, then
/// `serverwait( randomfloatrange( 0.05, 0.15 ) )` before the thunder (its
/// flash of exposure and sunlight is the renderer's: not here). A missing
/// exploder is skipped to the next wait.
fn lightning(world: &mut World, now: i64) {
    let [lo, hi] = UPLINK_LIGHTNING;
    let id = lo + rand_int(world, (hi - lo) as u32) as i32;
    let spot = super::with_vm(world, |vm, world| {
        let at = super::natives_fx::exploder_origin(vm, id)?;
        super::natives_fx::fire_exploder(vm, world, &gsc_t6::Key::Int(id));
        Some(at)
    })
    .flatten();
    match spot {
        Some(spot) => {
            diag::info!(Sim, "bo2mp mp_uplink: lightning {id}");
            let wait = rand_float(world, 0.05, 0.15);
            again(world, LEVEL, now + ms(wait), Cue::Thunder { origin: spot, left: None });
        }
        None => {
            let wait = rand_float(world, 10.0, 15.0);
            again(world, LEVEL, now + ms(wait), Cue::Lightning);
        }
    }
}

/// One of playlightning's `randomintrange( 2, 4 )` strikes: the thunder at
/// the lightning, then `serverwait( randomfloatrange( 0, 0.25 ) )`; after
/// the last, initlightningloop's `serverwait( randomfloatrange( 10, 15 ) )`.
fn thunder(game: &mut World, spot: [f32; 3], left: Option<u8>, now_ms: i64) {
    let left = left.unwrap_or_else(|| 2 + rand_int(game, 2) as u8);
    let blend = rand_float(game, 0.0, 0.25);
    super::natives_fx::sound(game, EventRecipients::All, "amb_thunder_flash", spot);
    let due = now_ms + ms(blend);
    if left > 1 {
        again(game, LEVEL, due, Cue::Thunder { origin: spot, left: Some(left - 1) });
    } else {
        let wait = rand_float(game, 10.0, 15.0);
        again(game, LEVEL, due + ms(wait), Cue::Lightning);
    }
}

// bo2mp: Turbine (mp_turbine_amb.csc wmill_sfx_setup): the whoosh at three
// windmills every 1.35 s, from half a second in.
const TURBINE_WHOOSH: [[f32; 3]; 3] = [[-2579.0, 3014.0, 385.0], [-1644.0, -636.0, 563.0], [-1073.0, 1351.0, 1941.0]];
const TURBINE_WHOOSH_WAIT: f32 = 1.35;

fn turbine_start(world: &mut World) {
    let now = world.resource::<Zm>().clock_millis;
    again(world, LEVEL, now + ms(0.5), Cue::Whoosh);
    diag::info!(Sim, "bo2mp mp_turbine: windmill whoosh at {} spots", TURBINE_WHOOSH.len());
}

fn whoosh(game: &mut World, now: i64) {
    for at in TURBINE_WHOOSH {
        super::natives_fx::sound(game, EventRecipients::All, "amb_wmill_whoosh", at);
    }
    again(game, LEVEL, now + ms(TURBINE_WHOOSH_WAIT), Cue::Whoosh);
}

// bo2mp: Drone (mp_drone_amb.csc scanner_alert): a player walking into the
// scanner's trigger sets it off (trig_enter_alarm): the detect sound at the
// trigger, and a quarter second later the alarm at the scanner. The loop
// takes one player each time it wakes (`waittill( "trigger" )`, then `wait
// 0.25`); one already in it (`ent_already_in_trigger`) sets nothing off.
const SCANNER_ALARM: [f32; 3] = [-460.0, -809.0, -438.0];
const SCANNER_WAIT: f32 = 0.25;

fn drone_start(vm: &mut Vm<World>, world: &mut World) {
    let Some(&trig) = named(vm, world, "scanner_alert").first() else {
        diag::info!(Sim, "bo2mp mp_drone: no scanner_alert trigger");
        return;
    };
    world.resource_mut::<Zm>().client_fx.scanner.trig = trig;
    let now = world.resource::<Zm>().clock_millis;
    again(world, LEVEL, now + ms(0.0), Cue::Scanner);
    diag::info!(Sim, "bo2mp mp_drone: scanner alarm on trigger ent{}", trig);
}

fn scanner(game: &mut World, now: i64) {
    let trig = game.resource::<Zm>().client_fx.scanner.trig;
    let Some(e) = game.resource::<Zm>().ents.get(&trig).cloned() else {
        return;
    };
    let players: Vec<u32> = game
        .resource::<Zm>()
        .players
        .iter()
        .where_value(|pt| pt.sessionstate == "playing")
        .map(|(ch, _)| *ch)
        .collect();
    let mut touching = Vec::new();
    for ch in players {
        if let Some((low, high)) = super::triggers::player_box(game, ch)
            && super::triggers::touches(game, &e, low, high)
        {
            touching.push(ch);
        }
    }
    let (alarms, entered) = {
        let mut zm = game.resource_mut::<Zm>();
        let s = &mut zm.client_fx.scanner;
        let alarms = s.alarms.iter().filter(|&&at| at <= now).count();
        s.alarms.retain(|&at| at > now);
        // trigger_thread: once the enter payload is done, the player stays
        // in it while he touches it (trig_leave_alarm does nothing).
        s.inside.retain(|ch, done| now < *done || touching.contains(ch));
        let mut entered = None;
        if now >= s.next_look
            && let Some(&ch) = touching.first()
        {
            s.next_look = now + ms(SCANNER_WAIT);
            if !s.inside.contains_key(&ch) {
                s.inside.insert(ch, now + ms(SCANNER_WAIT));
                s.alarms.push(now + ms(SCANNER_WAIT));
                entered = Some(ch);
            }
        }
        (alarms, entered)
    };
    for _ in 0..alarms {
        super::natives_fx::sound(game, EventRecipients::All, "amb_scanner_alarm", SCANNER_ALARM);
    }
    if let Some(ch) = entered {
        super::natives_fx::sound(game, EventRecipients::All, "amb_scanner_detect", e.origin);
        diag::info!(Sim, "bo2mp mp_drone: client {} sets off the scanner", ch);
    }
    again(game, LEVEL, now + ms(0.0), Cue::Scanner);
}

/// The scripts' random number (0..1), as `randomfloat` draws it.
fn with_rand(world: &mut World) -> f32 {
    super::with_vm(world, |vm, _| vm.rand_unit()).unwrap_or(0.0)
}

/// Each tick (after the entities show): the effects that are due.
pub(crate) fn advance(game: &mut World, now: i64) {
    let thrown = game.get_resource_mut::<AirStaff>().map(|mut a| std::mem::take(&mut a.pending)).unwrap_or_default();
    if !thrown.is_empty() {
        super::with_vm(game, |vm, game| {
            for n in thrown {
                air_staff_launch(vm, game, n);
            }
        });
    }
    let due: Vec<Pending> = {
        let mut zm = game.resource_mut::<Zm>();
        if zm.client_fx.pending.is_empty() {
            return;
        }
        let (due, keep) = std::mem::take(&mut zm.client_fx.pending)
            .into_iter()
            .partition(|p| p.due <= now);
        zm.client_fx.pending = keep;
        due
    };
    for p in due {
        if let Cue::Sway { base, leg, time } = p.step {
            sway(game, p.ent, base, leg, time, now);
            continue;
        }
        if let Cue::Spin = p.step {
            spin(game, p.ent, now);
            continue;
        }
        match p.step {
            Cue::Turn { around, degrees, secs } => {
                turn(game, p.ent, around, degrees, secs, now);
                continue;
            }
            Cue::Clock {
                stage,
                set,
                first,
                bit,
                last,
            } => {
                clock(game, p.ent, stage, set, first, bit, last, now);
                continue;
            }
            Cue::Camera { pitch, roll } => {
                camera(game, p.ent, pitch, roll, now);
                continue;
            }
            Cue::Post { angle, secs, leg } => {
                post(game, p.ent, angle, secs, leg, now);
                continue;
            }
            Cue::Wind => {
                wind(game, now);
                continue;
            }
            Cue::Vane {
                orig,
                yaw,
                stage,
                wind,
            } => {
                vane(game, p.ent, orig, yaw, stage, wind, now);
                continue;
            }
            Cue::Lightning => {
                lightning(game, now);
                continue;
            }
            Cue::Thunder { origin: spot, left } => {
                thunder(game, spot, left, now);
                continue;
            }
            Cue::Whoosh => {
                whoosh(game, now);
                continue;
            }
            Cue::Scanner => {
                scanner(game, now);
                continue;
            }
            Cue::Swarm => {
                swarm::tick(game, now);
                continue;
            }
            Cue::VisionFade => {
                vision::tick(game, now);
                continue;
            }
            _ => {}
        }
        if let Cue::Tank(t) = p.step {
            if let Some((due, t)) = ai_tank::tick(game, p.ent, t, now) {
                game.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due,
                    until: i64::MAX,
                    ent: p.ent,
                    step: Cue::Tank(t),
                });
            }
            continue;
        }
        if let Cue::Heli(h) = p.step {
            if let Some((due, h)) = heli::tick(game, p.ent, h, now) {
                game.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due,
                    until: i64::MAX,
                    ent: p.ent,
                    step: Cue::Heli(h),
                });
            }
            continue;
        }
        if let Cue::TrainDust { next, last } = p.step {
            if let Some(next) = train_dust(game, p.ent, next, last) {
                game.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due: now + 10,
                    step: Cue::TrainDust { next, last },
                    ..p
                });
            }
            continue;
        }
        if let Cue::TrainShake = p.step {
            if train_shake(game, p.ent) {
                game.resource_mut::<Zm>().client_fx.pending.push(Pending { due: now + 50, ..p });
            }
            continue;
        }
        if let Cue::HeldLight { sounded } = p.step {
            if let Some((after, sounded)) = held_light(game, p.ent, sounded) {
                game.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due: now + after,
                    until: i64::MAX,
                    ent: p.ent,
                    step: Cue::HeldLight { sounded },
                });
            }
            continue;
        }
        let shown = super::natives_fx::shown(game, p.ent).map(|s| s.3);
        let Some(spot) = shown else {
            // Not drawn yet (spawned this tick): try again until it is.
            if now < p.until {
                game
                    .resource_mut::<Zm>()
                    .client_fx
                    .pending
                    .push(Pending { due: now + 50, ..p });
            }
            continue;
        };
        match p.step {
            Cue::At { fx, offset } => {
                let at = [
                    spot[0] + offset[0],
                    spot[1] + offset[1],
                    spot[2] + offset[2],
                ];
                super::natives_fx::effect(game, fx, at, [0.0, 0.0, 1.0]);
            }
            Cue::Bolt { fx, tag, off } => {
                super::natives_fx::bolt_effect(game, p.ent, fx, tag, off);
            }
            Cue::Light { fx, tag, off, to: goal } => {
                super::natives_fx::bolt_effect_to(game, goal, p.ent, fx, tag, off);
            }
            Cue::QrBlink { interval, quicken, sides } => {
                let i = if quicken { (interval / 1.17).max(0.1) } else { interval };
                let half = (i * 500.0).round() as i64;
                super::natives_fx::sound(game, EventRecipients::All, RC_BEEP, spot);
                let mut zm = game.resource_mut::<Zm>();
                for (fx, goal) in sides.iter().cloned() {
                    zm.client_fx.pending.push(Pending {
                        due: now,
                        until: now + 2500,
                        ent: p.ent,
                        step: Cue::Light { fx, tag: QR_TAG, off: false, to: goal.clone() },
                    });
                    zm.client_fx.pending.push(Pending {
                        due: now + half,
                        until: now + 2500,
                        ent: p.ent,
                        step: Cue::Light { fx, tag: QR_TAG, off: true, to: goal },
                    });
                }
                zm.client_fx.pending.push(Pending {
                    due: now + 2 * half,
                    until: now + 2500,
                    ent: p.ent,
                    step: Cue::QrBlink { interval: i, quicken: true, sides },
                });
            }
            Cue::Sound { alias: snd_alias } => {
                super::natives_fx::sound(game, EventRecipients::All, snd_alias, spot);
            }
            Cue::HeldLight { .. }
            | Cue::Sway { .. }
            | Cue::Spin
            | Cue::TrainDust { .. }
            | Cue::TrainShake
            | Cue::Turn { .. }
            | Cue::Clock { .. }
            | Cue::Camera { .. }
            | Cue::Post { .. }
            | Cue::Wind
            | Cue::Vane { .. }
            | Cue::Lightning
            | Cue::Thunder { .. }
            | Cue::Whoosh
            | Cue::Scanner
            | Cue::Swarm
            | Cue::VisionFade => {}
            Cue::Blink { fx, tag } => {
                super::natives_fx::bolt_effect(game, p.ent, fx, tag, false);
                game.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due: now + 500,
                    until: now + 2500,
                    ..p
                });
            }
            Cue::Tank(_) => {}
            Cue::Heli(_) => {}
            Cue::BoltNamed { fx, joint } => {
                super::natives_fx::bolt_effect(game, p.ent, &fx, &joint, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_missile() -> World {
        let mut zm = Zm::default();
        zm.ents.insert(90, crate::zombie_rt::Ent { origin: [1.0, 2.0, 3.0], rot: [0.0, 30.0, 0.0], ..Default::default() });
        let mut world = World::new();
        world.insert_resource(zm);
        world
    }

    fn now(world: &mut World, ms: i64) {
        world.resource_mut::<Zm>().clock_millis = ms;
    }

    #[test]
    fn the_hellstorm_shows_on_the_minimap_and_lingers_after_it_goes_off() {
        let mut world = world_with_missile();
        missile_icon(&mut world, 90, true, 50);
        let rows = client_objectives(&mut world);
        assert_eq!(rows.len(), 1);
        let o = &rows[0];
        assert_eq!((o.n, o.icon, o.size, o.heading), (90, "remotemissile_target", 50, 30.0));
        assert_eq!(o.map_info, MAP_ROTATE | MAP_TINT_IN_CODE);
        // It goes off at 1000: still there 0.4 s, where it went off, then gone.
        now(&mut world, 1000);
        map_icon_died(&mut world, 90);
        world.resource_mut::<Zm>().ents.get_mut(&90).unwrap().origin = [9.0; 3];
        now(&mut world, 1399);
        assert_eq!(client_objectives(&mut world)[0].at, [1.0, 2.0, 3.0]);
        now(&mut world, 1400);
        assert!(client_objectives(&mut world).is_empty());
    }

    #[test]
    fn bomblets_out_takes_the_hellstorms_mark_off_at_once() {
        let mut world = world_with_missile();
        missile_icon(&mut world, 90, true, 50);
        missile_icon(&mut world, 90, false, 50);
        assert!(client_objectives(&mut world).is_empty());
    }

    fn qr_blinks(world: &World, n: u32) -> Vec<(f32, bool)> {
        world
            .resource::<Zm>()
            .client_fx
            .pending
            .iter()
            .filter_map(|p| match &p.step {
                Cue::QrBlink { interval, quicken, .. } if p.ent == n => Some((*interval, *quicken)),
                _ => None,
            })
            .collect()
    }

    fn no_sides() -> [(&'static str, EventRecipients); 3] {
        [
            (QR_DRIVER, EventRecipients::Clients(Vec::new())),
            (QR_FRIEND, EventRecipients::Clients(Vec::new())),
            (QR_ENEMY, EventRecipients::Clients(Vec::new())),
        ]
    }

    #[test]
    fn the_dragonfire_blinks_its_last_ten_seconds_and_keeps_quickening_into_its_last_four() {
        let mut world = world_with_missile();
        qrdrone_state(&mut world, 90, 0, no_sides());
        assert!(qr_blinks(&world, 90).is_empty());
        // 1: a first blink, a second apart.
        qrdrone_state(&mut world, 90, 1, no_sides());
        assert_eq!(qr_blinks(&world, 90), vec![(1.0, false)]);
        // Six seconds of quickening, as the blink runs it.
        let (mut i, mut t) = (1.0f32, 0.0f32);
        while t + i <= 6.0 {
            t += i;
            i = (i / 1.17).max(0.1);
        }
        assert!((0.12..0.15).contains(&i), "{i}");
        for p in world.resource_mut::<Zm>().client_fx.pending.iter_mut() {
            if let Cue::QrBlink { interval, quicken, .. } = &mut p.step {
                (*interval, *quicken) = (i, true);
            }
        }
        // 2: it goes on from there, beeping at once.
        qrdrone_state(&mut world, 90, 2, no_sides());
        assert_eq!(qr_blinks(&world, 90), vec![(i, false)]);
        // 3: dark, nothing more to come.
        qrdrone_state(&mut world, 90, 3, no_sides());
        assert!(qr_blinks(&world, 90).is_empty());
        let lights = world.resource::<Zm>().client_fx.pending.iter().filter(|p| p.ent == 90).count();
        assert_eq!(lights, 0);
    }
}

#[cfg(test)]
mod overlay_blur_tests {
    use super::*;

    #[test]
    fn the_denizen_blurs_in_and_out() {
        assert_eq!(overlay_blur_change("", "zm_ai_screecher_blur"), Some((4.0, 0.1)));
        assert_eq!(overlay_blur_change("zm_ai_screecher_blur", ""), Some((0.0, 0.25)));
        assert_eq!(overlay_blur_change("zm_ai_screecher_blur", "zm_ai_screecher_blur"), None);
        assert_eq!(overlay_blur_change("", "zm_afterlife_filter"), None);
    }
}
