//! Client fields (`setclientfield` -> `codesetclientfield`,
//! `codesetworldclientfield`): Black Ops II's client scripts turn them into
//! effects and sounds on each client. The engine plays their half here, as
//! those scripts do: a zombie's eye glow, the dirt as it rises, a
//! power-up's glow, a perk machine's trail from the sky, a wall buy's gun.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, Value, Vm};

use super::{Zm, arg, entnum, origin_of};
use crate::EventAudience;

mod ai_tank;
mod heli;
mod swarm;
mod turret;
mod vision;

pub(super) use vision::naked as vision_naked;

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
    Float(f32),
}

/// bo2mp: keep the value a script set, for the `codeget*clientfield` reads.
fn remember(world: &mut World, n: u32, ps: bool, name: &str, v: &Value) {
    let v = match *v {
        Value::Float(f) => FieldVal::Float(f),
        ref v => FieldVal::Int(v.as_int().unwrap_or(0)),
    };
    world
        .resource_mut::<Zm>()
        .client_field_vals
        .insert((n, ps, name.to_owned()), v);
}

/// bo2mp: the value last set, or 0 for a field never set.
fn recall(world: &World, n: u32, ps: bool, name: &str) -> Value {
    match world
        .resource::<Zm>()
        .client_field_vals
        .get(&(n, ps, name.to_owned()))
    {
        Some(FieldVal::Float(f)) => Value::Float(*f),
        Some(FieldVal::Int(i)) => Value::Int(*i),
        None => Value::Int(0),
    }
}

#[derive(Clone, Debug)]
struct Pending {
    due: i64,
    /// Given up after this (the entity never showed).
    until: i64,
    ent: u32,
    step: Step,
}

#[derive(Clone, Debug)]
enum Step {
    /// An effect where the entity stands, offset.
    At { fx: &'static str, offset: [f32; 3] },
    /// An effect on one of the entity's tags (or taken off it).
    Bolt {
        fx: &'static str,
        tag: &'static str,
        stop: bool,
    },
    /// bo2mp: a light on one of its tags, for some players only (an
    /// RC-XD's on `tag_origin`, the Dragonfire's on `tag_body`: his team
    /// sees green, the others red).
    Light {
        fx: &'static str,
        tag: &'static str,
        stop: bool,
        to: EventAudience,
    },
    /// bo2mp: `_qrdrone.csc`'s `blink_fx_and_sound`: a beep and the
    /// Dragonfire's lights on for half of `interval`, off for the other
    /// half, then again 1.17 times quicker (never under 0.1 s). The first
    /// beep after a state change keeps the interval it had (`quicken`
    /// false).
    QrBlink {
        interval: f32,
        quicken: bool,
        sides: [(&'static str, EventAudience); 3],
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
    /// bo2mp: _rotating_object.csc's `rotating_object_think`: 1000 turns on
    /// one axis (`degrees` in all, over `secs`), again each time it is done.
    Turn { axis: usize, degrees: f32, secs: f32 },
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
}

// The client scripts' effects (clientscripts/mp/zombies/_zm.csc,
// _zm_powerups.csc, zm_nuked_fx.csc).
const RISE_BURST: &str = "maps/zombie/fx_mp_zombie_hand_dirt_burst";
const RISE_BILLOW: &str = "maps/zombie/fx_mp_zombie_body_dirt_billowing";
const RISE_DUST: &str = "maps/zombie/fx_mp_zombie_body_dust_falling";
const EYE_GLOW: &str = "misc/fx_zombie_eye_single";
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
fn rc_sides(vm: &mut Vm<World>, world: &World, car: &Value) -> (EventAudience, EventAudience) {
    let Value::Object(o) = car else {
        return (EventAudience::Clients(Vec::new()), EventAudience::All);
    };
    let (team_s, owner_s, tb) = (vm.intern("team"), vm.intern("owner"), vm.intern("teambased"));
    let team = vm.to_text(&vm.raw_field(*o, team_s));
    let owner = vm.raw_field(*o, owner_s);
    let owner = entnum(vm, &owner);
    let teambased = gsc_t6::truthy(&vm.raw_field(vm.level, tb));
    let players: Vec<(u32, gsc_t6::ObjRef)> =
        world.resource::<Zm>().players.iter().map(|(n, p)| (*n, p.obj)).collect();
    let (mut friends, mut foes) = (Vec::new(), Vec::new());
    for (n, p) in players {
        let friend = Some(n) == owner || (teambased && !team.is_empty() && vm.to_text(&vm.raw_field(p, team_s)) == team);
        if friend { &mut friends } else { &mut foes }.push(crate::world::ClientId(n));
    }
    (EventAudience::Clients(friends), EventAudience::Clients(foes))
}

/// bo2mp: an RC-XD comes into play (`spawned` -> `spawn_solid_fx`): its
/// solid light, green to his side, blinking red to the other.
pub(super) fn rcbomb_spawned(vm: &mut Vm<World>, world: &mut World, car: &Value) {
    let Some(n) = entnum(vm, car) else { return };
    rc_cancel(world, n);
    let (friends, foes) = rc_sides(vm, world, car);
    for (fx, to) in [(RC_FRIEND, friends), (RC_ENEMY, foes)] {
        queue(world, n, 0, Step::Light { fx, tag: "tag_origin", stop: false, to });
    }
}

/// Its light scripts end (`light_disable`): what is still to come is
/// dropped, its solid lights go out.
fn rc_cancel(world: &mut World, n: u32) {
    world
        .resource_mut::<Zm>()
        .client_fx
        .pending
        .retain(|p| p.ent != n || !matches!(p.step, Step::Light { .. } | Step::Sound { .. }));
}

/// Sent at once while the car still shows: it blows up and is deleted in the
/// same tick, and a stop queued to the tick's end would find it gone and its
/// lights would hang in the air.
fn rc_lights_off(vm: &mut Vm<World>, world: &mut World, car: &Value, n: u32) {
    rc_cancel(world, n);
    let (friends, foes) = rc_sides(vm, world, car);
    for (fx, to) in [(RC_FRIEND, friends), (RC_ENEMY, foes)] {
        if !super::natives_fx::bolt_effect_to(world, to.clone(), n, fx, "tag_origin", true) {
            queue(world, n, 0, Step::Light { fx, tag: "tag_origin", stop: true, to });
        }
    }
}

/// It blows up (`death`): `stop_light_fx` takes off whichever light it
/// showed last, the solid one or a countdown flash, and `blowup` hides it
/// that tick, so nothing of them is left in the air.
fn rc_lights_gone(vm: &mut Vm<World>, world: &mut World, car: &Value, n: u32) {
    if std::env::var_os("BO2MP_T6_BOLTLOG").is_some() {
        diag::info!(Sim, "bo2mp rcxd death: ent{n} lights off");
    }
    rc_cancel(world, n);
    let (friends, foes) = rc_sides(vm, world, car);
    for (fx, to) in [
        (RC_FRIEND, friends.clone()),
        (RC_ENEMY, foes.clone()),
        (RC_FRIEND_BLINK, friends),
        (RC_ENEMY_BLINK, foes),
    ] {
        if !super::natives_fx::bolt_effect_kill(world, to.clone(), n, fx, "tag_origin") {
            queue(world, n, 0, Step::Light { fx, tag: "tag_origin", stop: true, to });
        }
    }
}

/// The Dragonfire's three lights and who sees each: its driver
/// (`islocalclientdriver`, its owner), the rest of his side
/// (`friendnotfoe`), everyone else.
fn qr_sides(vm: &mut Vm<World>, world: &World, drone: &Value) -> [(&'static str, EventAudience); 3] {
    let (friends, foes) = rc_sides(vm, world, drone);
    let owner = match drone {
        Value::Object(o) => {
            let s = vm.intern("owner");
            let v = vm.raw_field(*o, s);
            entnum(vm, &v)
        }
        _ => None,
    };
    let mut friends = match friends {
        EventAudience::Clients(c) => c,
        _ => Vec::new(),
    };
    friends.retain(|c| Some(c.0) != owner);
    let driver = EventAudience::Clients(owner.map(crate::world::ClientId).into_iter().collect());
    [(QR_DRIVER, driver), (QR_FRIEND, EventAudience::Clients(friends)), (QR_ENEMY, foes)]
}

/// bo2mp: the Dragonfire comes into play (its client script's `spawned`:
/// `qrdronefx( 0 )`): its steady light.
pub(super) fn qrdrone_spawned(vm: &mut Vm<World>, world: &mut World, drone: &Value) {
    let Some(n) = entnum(vm, drone) else { return };
    let sides = qr_sides(vm, world, drone);
    qrdrone_state(world, n, 0, sides);
}

/// bo2mp: `restartfx`: whatever it showed goes (`restart_fx`), then 0
/// lights it steady, 1 or 2 start `blink_fx_and_sound`, 3 leaves it dark
/// (`stopfx`, `fx_death`). The blink goes on at the interval it had reached:
/// BO2 sets `fx_interval` (1.0, 0.133) there, but the blink reads
/// `interval`, so a 2 six seconds after the 1 keeps quickening from where it
/// was (about 0.13 s by then); a first blink starts at 1 s.
fn qrdrone_state(world: &mut World, n: u32, value: i64, sides: [(&'static str, EventAudience); 3]) {
    let interval = {
        let mut zm = world.resource_mut::<Zm>();
        let pending = &mut zm.client_fx.pending;
        let was = pending.iter().find_map(|p| match &p.step {
            Step::QrBlink { interval, .. } if p.ent == n => Some(*interval),
            _ => None,
        });
        pending.retain(|p| p.ent != n || !matches!(p.step, Step::QrBlink { .. } | Step::Light { .. }));
        was.unwrap_or(1.0)
    };
    for (fx, to) in sides.iter().cloned() {
        super::natives_fx::bolt_effect_to(world, to, n, fx, QR_TAG, true);
    }
    match value {
        0 => {
            for (fx, to) in sides {
                queue(world, n, 0, Step::Light { fx, tag: QR_TAG, stop: false, to });
            }
        }
        1 | 2 => queue(world, n, 0, Step::QrBlink { interval, quicken: false, sides }),
        _ => {}
    }
}

fn queue(world: &mut World, ent: u32, after_ms: i64, step: Step) {
    let mut zm = world.resource_mut::<Zm>();
    let due = zm.now_ms + after_ms;
    zm.client_fx.pending.push(Pending {
        due,
        until: due + 2000,
        ent,
        step,
    });
}

fn rand_range(vm: &mut Vm<World>, lo: i32, hi: i32) -> f32 {
    (lo + (vm.rand_u32() % (hi - lo) as u32) as i32) as f32
}

/// An entity's field changed: its client script's effects.
fn entity_field(vm: &mut Vm<World>, world: &mut World, ent: &Value, name: &str, value: i64) {
    let Some(n) = entnum(vm, ent) else { return };
    match name {
        // handle_zombie_risers: a burst of dirt, a billow, then dust
        // falling off its back while it climbs out.
        "zombie_riser_fx" if value != 0 => {
            let origin = origin_of(vm, world, ent).unwrap_or([0.0; 3]);
            super::natives_fx::sound(world, EventAudience::All, "zmb_zombie_spawn", origin);
            let burst = [0.0, 0.0, rand_range(vm, 5, 10)];
            queue(
                world,
                n,
                0,
                Step::At {
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
                world,
                n,
                250,
                Step::At {
                    fx: RISE_BILLOW,
                    offset: billow,
                },
            );
            let mut t = 0;
            while t < 5500 {
                queue(
                    world,
                    n,
                    2250 + t,
                    Step::Bolt {
                        fx: RISE_DUST,
                        tag: "j_spineupper",
                        stop: false,
                    },
                );
                t += 300;
            }
        }
        // createzombieeyes / deletezombieeyes.
        "zombie_has_eyes" => queue(
            world,
            n,
            0,
            Step::Bolt {
                fx: EYE_GLOW,
                tag: "j_eyeball_le",
                stop: value == 0,
            },
        ),
        // powerup_fx_callback: the glow by kind (no glow for 0).
        "powerup_fx" => {
            if let Some(fx) = usize::try_from(value - 1).ok().and_then(|i| POWERUP.get(i)) {
                queue(
                    world,
                    n,
                    0,
                    Step::Bolt {
                        fx,
                        tag: "tag_origin",
                        stop: false,
                    },
                );
            }
        }
        // perk_meteor_fx: the trail while the machine falls.
        "clientfield_perk_intro_fx" => queue(
            world,
            n,
            0,
            Step::Bolt {
                fx: PERK_METEOR,
                tag: "tag_origin",
                stop: value == 0,
            },
        ),
        // bo2mp: the RC-XD's time runs out (`countdown`): a beep and a
        // flash, a second apart at 1 and quickening by 1.17 each time (from
        // 0.133 s at 2), never under 0.1 s, until the next value or its end.
        // Already blown up (`blowup` hides it, and a hidden entity is not
        // sent, so his client never hears the timer's last step): no flash
        // over the scorch mark.
        "rcbomb_countdown" if value != 0 && world.resource::<Zm>().ents.get(&n).is_some_and(|e| e.hidden) => {}
        "rcbomb_countdown" if value != 0 => {
            rc_lights_off(vm, world, ent, n);
            let (friends, foes) = rc_sides(vm, world, ent);
            let (mut interval, until) = if value == 2 { (0.133f32, 5000) } else { (1.0f32, 6500) };
            let mut t = 0i64;
            while t < until {
                queue(world, n, t, Step::Sound { alias: RC_BEEP });
                for (fx, to) in [(RC_FRIEND_BLINK, friends.clone()), (RC_ENEMY_BLINK, foes.clone())] {
                    queue(world, n, t, Step::Light { fx, tag: "tag_origin", stop: false, to });
                }
                t += (interval * 1000.0).round() as i64;
                interval = (interval / 1.17).clamp(0.1, 1.0);
            }
        }
        // bo2mp: it blows up (`death`): its lights go out.
        "rcbomb_death" if value != 0 => rc_lights_gone(vm, world, ent, n),
        // bo2mp: the Dragonfire's light (_qrdrone.csc's `statechange`):
        // its last 10 s blink with a beep, 3 when it is done.
        "qrdrone_state" => {
            let sides = qr_sides(vm, world, ent);
            qrdrone_state(world, n, value, sides);
        }
        // bo2mp: the Hellstorm on the minimap (_remotemissile.csc): fired
        // (1), `remotemissile_target` 50 wide; its bomblets out (2), taken
        // off. Each bomblet shows its own, 32 wide (the default). They turn
        // with their missile and tint in code, not cut off at the edge
        // (`objective_onentity( lcn, id, self, 1, 0, 1 )`); one that dies
        // stays 0.4 s where it went off.
        "remote_missile_fired" | "remote_missile_bomblet_fired" => {
            let size = if name == "remote_missile_fired" { 50 } else { 32 };
            missile_icon(world, n, value == 1, size);
        }
        // bo2mp: destructible_car_lights (Aftermath, Detour, Rush, Magma): on,
        // the light bar on tag_origin after randomfloatrange(0.1, 0.5) s; off
        // (the car is wrecked), taken off. Magma's cars take its own bar.
        "police_car_lights" | "ambulance_lights" => {
            let fx = if name != "police_car_lights" {
                AMBULANCE_LIGHTS
            } else if world.resource::<Zm>().map == "mp_magma" {
                POLICE_LIGHTS_JAPAN
            } else {
                POLICE_LIGHTS
            };
            let after = if value != 0 { 100 + i64::from(vm.rand_u32() % 401) } else { 0 };
            queue(
                world,
                n,
                after,
                Step::Bolt {
                    fx,
                    tag: "tag_origin",
                    stop: value == 0,
                },
            );
        }
        // missile_drone_projectile_active_cb: blinky_light on tag_target,
        // or stop_blinky_light (its last flash taken off).
        "train_moving" => train_move(world, n, value != 0),
        "missile_drone_projectile_active" => {
            world
                .resource_mut::<Zm>()
                .client_fx
                .pending
                .retain(|p| p.ent != n || !matches!(p.step, Step::Blink { .. }));
            if value != 0 {
                queue(world, n, 0, Step::Blink { fx: DRONE_LIGHT, tag: "tag_target" });
            } else {
                queue(world, n, 0, Step::Bolt { fx: DRONE_LIGHT, tag: "tag_target", stop: true });
            }
        }
        // _planemortar.csc's planemortar_contrail: the Lightning Strike
        // jet's afterburner on tag_origin once it is set (nothing takes it
        // off: it goes with the jet).
        "planemortar_contrail" if value != 0 => queue(
            world,
            n,
            0,
            Step::Bolt {
                fx: PLANE_EXHAUST,
                tag: "tag_origin",
                stop: false,
            },
        ),
        // _ai_tank.csc's `missile_fire` and `death`.
        "ai_tank_missile_fire" => ai_tank::missile_fire(world, n, value),
        "ai_tank_death" if value != 0 => ai_tank::death(world, n),
        // _helicopter.csc's `heli_comlink_bootup_anim`: the Stealth Chopper
        // lowers its gear and opens its bay as it arrives.
        "heli_comlink_bootup_anim" if value != 0 => {
            super::mp_fxanims::play(world, n, "veh_anim_future_heli_gearup_bay_open", false)
        }
        // _helicopter_sounds.csc's `vehicle_is_firing`: the Escort Drone's
        // gun fires, and its brass falls.
        "vehicle_is_firing" => super::engine_sounds::set_firing(world, n, value != 0),
        // _turret.csc's `turret_microwave_sounds`: the Guardian's hum.
        "turret_microwave_sounds" => turret::sounds(world, n, value),
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
/// Not ported: `train_move_think`'s shake and rumble near the train.
fn missile_icon(world: &mut World, n: u32, on: bool, size: u32) {
    let mut zm = world.resource_mut::<Zm>();
    let shows = zm.now_ms;
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
            },
        );
    }
}

/// bo2mp: an entity died (a missile went off): its minimap icon goes once
/// it has lingered (`death` / `entityshutdown` then the client script's
/// wait).
pub(super) fn map_icon_died(world: &mut World, n: u32) {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
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
        .map(|e| (e.classname == "script_vehicle", e.model == "p6_bullet_train_engine_rev"))
    else {
        return;
    };
    let shows = zm.now_ms + 100;
    let fx = &mut zm.client_fx;
    fx.pending.retain(|p| p.ent != n || !matches!(p.step, Step::TrainDust { .. }));
    fx.map_icons.remove(&n);
    if !on {
        return;
    }
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
            },
        );
    }
    if let Some((next, last)) = dust {
        fx.pending.push(Pending {
            due: shows,
            until: i64::MAX,
            ent: n,
            step: Step::TrainDust { next, last },
        });
    }
}

/// `train_fx_think`'s looks since the last tick: the dust at each next
/// exploder the car has passed (`vectordot( anglestoforward( self.angles ),
/// vectornormalize( origin - self.origin ) ) <= 0`), in order (the script
/// looks again each client frame, 0.01 s; a tick here is 50 ms); the next
/// one to wait for, none once past `last`.
fn train_dust(world: &mut World, n: u32, mut next: i32, last: i32) -> Option<i32> {
    use gsc_t6::math::{angle_vectors, dot, normalize, sub};
    let (origin, angles) = world.resource::<Zm>().ents.get(&n).map(|e| (e.origin, e.angles))?;
    let forward = angle_vectors(angles).0;
    super::with_vm(world, |vm, world| {
        while next <= last {
            let at = super::natives_fx::exploder_origin(vm, next)?;
            if dot(forward, normalize(sub(at, origin))) > 0.0 {
                return Some(next);
            }
            super::natives_fx::fire_exploder(vm, world, &gsc_t6::Key::Int(next));
            diag::info!(Sim, "bo2mp mp_express: train dust {next} (ent{n})");
            next += 1;
        }
        None
    })
    .flatten()
}

/// bo2mp: one client script minimap icon to send: its entity, picture,
/// where it is and its yaw, its width and objectiveMapInfo bits.
pub(super) struct ClientObjective {
    pub n: u32,
    pub icon: &'static str,
    pub at: [f32; 3],
    pub yaw: f32,
    pub size: u32,
    pub map_info: i32,
}

/// bo2mp: the minimap icons the client scripts give, once shown: Express's
/// train cars (`scr_express_size` 45 wide, turning with the car and cut off
/// at the map's edge: `objective_onentity( lcn, id, self, 1, 1, 0 )`) and
/// the Hellstorm's. Each follows its entity; one whose entity died or went
/// stays at its last place until its linger is over, then is dropped.
pub(super) fn client_objectives(world: &mut World) -> Vec<ClientObjective> {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    let zm = &mut *zm;
    let (ents, icons) = (&zm.ents, &mut zm.client_fx.map_icons);
    icons.retain(|n, i| {
        if i.gone.is_none()
            && let Some(e) = ents.get(n)
        {
            i.last = Some((e.origin, e.angles[1]));
        } else if i.gone.is_none() {
            i.gone = Some(now + i.linger);
        }
        i.gone.is_none_or(|g| now < g) && i.last.is_some()
    });
    icons
        .iter()
        .filter(|(_, i)| now >= i.shows)
        .filter_map(|(n, i)| {
            let (at, yaw) = i.last?;
            Some(ClientObjective { n: *n, icon: i.icon, at, yaw, size: i.size, map_info: i.map_info })
        })
        .collect()
}

/// A player's own field (`setclientfieldtoplayer`): the perks and power-ups
/// his HUD shows go to his client as `bo2zm_icons` ("field:value,...").
fn player_field(world: &mut World, n: u32, name: &str, value: i32) {
    if name == "missile_drone_active" {
        drone_lock(world, n, value);
        return;
    }
    if !(name.starts_with("perk_") || name.starts_with("powerup_")) {
        return;
    }
    let list = {
        let mut zm = world.resource_mut::<Zm>();
        let Some(p) = zm.players.get_mut(&n) else {
            return;
        };
        let at = p.hud_fields.iter().position(|(k, _)| k == name);
        match (at, value) {
            (Some(i), 0) => {
                p.hud_fields.remove(i);
            }
            (Some(i), v) if p.hud_fields[i].1 != v => p.hud_fields[i].1 = v,
            (Some(_), _) => return,
            (None, 0) => return,
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

pub(super) fn bind(vm: &mut Vm<World>) {
    // A script camera: camerasetposition(entity), cameraactivate(on);
    // camerasetlookat() keeps the camera entity's own angles.
    vm.bind("camerasetposition", true, |vm, world, s, a| {
        if let (Some(n), Some(cam)) = (entnum(vm, s), entnum(vm, arg(a, 0)))
            && let Some(p) = world.resource_mut::<Zm>().players.get_mut(&n)
        {
            p.camera = Some(cam);
        }
        Ok(Value::Undefined)
    });
    vm.bind("cameraactivate", true, |vm, world, s, a| {
        let on = !matches!(arg(a, 0), Value::Int(0)) && !arg(a, 0).is_undefined();
        if let Some(n) = entnum(vm, s)
            && let Some(p) = world.resource_mut::<Zm>().players.get_mut(&n)
        {
            p.camera_on = on;
        }
        Ok(Value::Undefined)
    });
    vm.bind("camerasetlookat", true, |_, _, _, _| Ok(Value::Undefined));
    // setclientuivisibilityflag("hud_visible", 0): his HUD hides (the
    // game-over shot); script HUD text still shows.
    // bo2mp: setclientuivisibilityflag("g_compassShowEnemies", 1): a moment
    // of radar (the Lightning Strike's pick shows where enemies are).
    vm.bind("setclientuivisibilityflag", true, |vm, world, s, a| {
        let flag = vm.to_text(arg(a, 0));
        if flag == "hud_visible"
            && let Some(n) = entnum(vm, s)
        {
            let hidden = matches!(arg(a, 1), Value::Int(0));
            super::set_client_dvar(world, n, "bo2zm_hud_hidden", if hidden { "1" } else { "" });
        }
        if flag == "g_compassShowEnemies"
            && arg(a, 1).as_float().unwrap_or(0.0) != 0.0
            && let Some(n) = entnum(vm, s)
            && world.resource::<Zm>().mp
        {
            super::killstreaks::radar_pulse(world, n);
        }
        Ok(Value::Undefined)
    });
    vm.bind("codesetplayerstateclientfield", false, |vm, world, _, a| {
        if let Some(n) = entnum(vm, arg(a, 0)) {
            let name = vm.to_text(arg(a, 1));
            remember(world, n, true, &name, arg(a, 2));
            let value = arg(a, 2).as_int().unwrap_or(0);
            player_field(world, n, &name, value);
        }
        Ok(Value::Undefined)
    });
    // maps/mp/_utility::setclientfield calls these.
    vm.bind("codesetworldclientfield", false, |vm, world, _, a| {
        let name = vm.to_text(arg(a, 0));
        remember(world, u32::MAX, false, &name, arg(a, 1));
        let value = arg(a, 1).as_int().unwrap_or(0);
        super::wallbuys::world_field(vm, world, &name, value);
        swarm::world_field(world, &name, value);
        Ok(Value::Undefined)
    });
    vm.bind("codesetclientfield", false, |vm, world, _, a| {
        let ent = arg(a, 0).clone();
        let name = vm.to_text(arg(a, 1));
        if let Some(n) = entnum(vm, &ent) {
            remember(world, n, false, &name, arg(a, 2));
        }
        let value = arg(a, 2).as_int().map_or(0, i64::from);
        entity_field(vm, world, &ent, &name, value);
        Ok(Value::Undefined)
    });
    // Scripts that call the method directly.
    vm.bind("setclientfield", true, |vm, world, s, a| {
        let name = vm.to_text(arg(a, 0));
        let value = arg(a, 1).as_int().unwrap_or(0);
        if matches!(s, Value::Object(o) if matches!(vm.kind(*o), Some(ObjKind::Level))) {
            remember(world, u32::MAX, false, &name, arg(a, 1));
            super::wallbuys::world_field(vm, world, &name, value);
            swarm::world_field(world, &name, value);
        } else {
            if let Some(n) = entnum(vm, s) {
                remember(world, n, false, &name, arg(a, 1));
            }
            entity_field(vm, world, s, &name, i64::from(value));
        }
        Ok(Value::Undefined)
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
fn drone_lock(world: &mut World, n: u32, value: i32) {
    {
        let mut zm = world.resource_mut::<Zm>();
        let fx = &mut zm.client_fx;
        if fx.drone_lock.insert(n, value).unwrap_or(0) == value {
            return;
        }
        fx.pending
            .retain(|p| p.ent != n || !matches!(p.step, Step::HeldLight { .. }));
    }
    match value {
        2 => queue(world, n, 50, Step::HeldLight { sounded: false }),
        1 => {}
        _ => super::natives_fx::viewmodel_effect(world, n, DRONE_LIGHT, true),
    }
}

/// One turn of `targetacquired`'s loop: the light and when to look again,
/// or None when he is gone.
fn held_light(world: &mut World, n: u32, sounded: bool) -> Option<(i64, bool)> {
    use weapon_t6::state as S;
    let id = crate::world::ClientId(n);
    let (ready, origin) = {
        let f = super::frame(world);
        let ps = f.player(id)?;
        let name = crate::script_player::weapon_name(&f, ps.weapon);
        let held = name == "missile_drone_mp" || name == "inventory_missile_drone_mp";
        let busy = ps.f_weapon_pos_frac > 0.0
            || (S::is_offhand(ps.weapon_state_t6) && ps.weapon_state_t6 != S::OFFHAND_DETONATE)
            || (S::MELEE_INIT..=S::MELEE_END).contains(&ps.weapon_state_t6);
        let ready = held && !busy && crate::script_player::ammo_clip(&f, id, ps.weapon) > 0;
        (ready, ps.origin)
    };
    if !ready {
        return Some((1000, sounded));
    }
    super::natives_fx::viewmodel_effect(world, n, DRONE_LIGHT, false);
    if !sounded {
        super::natives_fx::sound(world, EventAudience::Client(id), "fly_hunter_raise_plr", origin);
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
pub(super) fn map_start(vm: &mut Vm<World>, world: &mut World, map: &str) {
    core_start(vm, world, map);
    match map {
        "mp_carrier" => carrier_start(vm, world),
        "mp_nuketown_2020" => nuked_start(vm, world),
        "mp_hydro" | "mp_vertigo" => cameras_start(vm, world, map),
        "mp_uplink" => uplink_start(vm, world),
        "mp_turbine" => turbine_start(world),
        "mp_drone" => drone_start(vm, world),
        _ => {}
    }
}

/// bo2mp: what a map's client script does on a `clientnotify` from the
/// server scripts.
pub(super) fn client_notify(vm: &mut Vm<World>, world: &mut World, name: &str) {
    if world.resource::<Zm>().map == "mp_nuketown_2020" {
        nuked_notify(vm, world, name);
    }
    // A client `level notify` the props may wait on (`fxanim_waittill_N`).
    super::mp_fxanims::level_notify(world, name);
}

/// The entities a map's client script finds by targetname.
fn named(vm: &mut Vm<World>, world: &mut World, name: &str) -> Vec<u32> {
    let name = Value::Str(vm.intern(name));
    let key = Value::Str(vm.intern("targetname"));
    super::natives_ent::matching(vm, world, &[name, key])
        .iter()
        .filter_map(|e| entnum(vm, e))
        .collect()
}

fn carrier_start(vm: &mut Vm<World>, world: &mut World) {
    // vista_ships_init: every `carrier_vista_ships` floats
    // (floatmyboat(clientnum, startleft 0, randomize 1)).
    let name = Value::Str(vm.intern("carrier_vista_ships"));
    let key = Value::Str(vm.intern("targetname"));
    let ships = super::natives_ent::matching(vm, world, &[name, key]);
    for ship in &ships {
        let Some(n) = entnum(vm, ship) else { continue };
        let Some(base) = world.resource::<Zm>().ents.get(&n).map(|e| e.angles) else {
            continue;
        };
        world.resource_mut::<Zm>().client_fx.pending.push(Pending {
            due: 0,
            until: i64::MAX,
            ent: n,
            step: Step::Sway {
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
    let Some(from) = zm.ents.get(&n).map(|e| e.angles) else { return };
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
        step: Step::Sway {
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
            step: Step::Spin,
        });
    }
    let cars = nuked_cars(vm, world);
    let glass = named(vm, world, NUKED_GLASS);
    let mut zm = world.resource_mut::<Zm>();
    for n in &cars {
        if let Some(e) = zm.ents.get_mut(n) {
            e.origin[2] -= NUKED_CAR_DROP;
            e.hidden = true;
        }
    }
    for n in &glass {
        if let Some(e) = zm.ents.get_mut(n) {
            e.hidden = true;
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

fn nuked_notify(vm: &mut Vm<World>, world: &mut World, name: &str) {
    match name {
        // nuked_car_flip: the cars come back up and show, with the glass
        // (its `wind_global_vector` is the effects' wind: not modelled).
        "bomb_drop_pre" => {
            let cars = nuked_cars(vm, world);
            let glass = named(vm, world, NUKED_GLASS);
            let mut zm = world.resource_mut::<Zm>();
            for n in &cars {
                if let Some(e) = zm.ents.get_mut(n) {
                    e.origin[2] += NUKED_CAR_DROP;
                    e.hidden = false;
                }
            }
            for n in &glass {
                if let Some(e) = zm.ents.get_mut(n) {
                    e.hidden = false;
                }
            }
            diag::info!(Sim, "bo2mp mp_nuketown_2020: {} end-game cars up", cars.len());
        }
        // rotate_flags: each `ending_flag` turns to ( 0, randomintrange( 75,
        // 105 ), 0 ) over randomfloatrange( 0.5, 0.65 ) s.
        "bomb_drop" => {
            let flags = named(vm, world, "ending_flag");
            let now = world.resource::<Zm>().now_ms;
            for &n in &flags {
                let yaw = (75.0 + (with_rand(world) * 30.0).floor()).min(104.0);
                let secs = 0.5 + with_rand(world) * 0.15;
                let mut zm = world.resource_mut::<Zm>();
                let Some(from) = zm.ents.get(&n).map(|e| e.angles) else { continue };
                zm.movers.rotate_to(n, from, [0.0, yaw, 0.0], now, secs, 0.0, 0.0);
            }
            diag::info!(Sim, "bo2mp mp_nuketown_2020: {} flags turn", flags.len());
            // nuked_dome_explosion_think: the dome's fxanim waits on this.
            super::mp_fxanims::level_notify(world, "fxanim_dome_explode_start");
        }
        // nuked_car_flip: the cars play cardestroy1/2 (their notes break the
        // glass, exploder 1005, and black the screen out) and the glass
        // displayGlassDestroy.
        "nuke_car_flip" => {
            for (name, clip) in NUKED_CARS.iter().zip(super::mp_fxanims::NUKED_CAR_ANIMS) {
                for n in named(vm, world, name) {
                    super::mp_fxanims::play(world, n, clip, true);
                }
            }
            for n in named(vm, world, NUKED_GLASS) {
                super::mp_fxanims::play(world, n, super::mp_fxanims::NUKED_GLASS_ANIM, false);
            }
        }
        _ => {}
    }
}

/// One `rotatevelocity( ( 0, 90, 0 ), 3600 )`, then `rotatedone` and again.
fn spin(world: &mut World, n: u32, now: i64) {
    let mut zm = world.resource_mut::<Zm>();
    let Some(from) = zm.ents.get(&n).map(|e| e.angles) else { return };
    let to = [from[0], from[1] + NUKED_SPIN * NUKED_SPIN_TIME, from[2]];
    zm.movers.rotate_to(n, from, to, now, NUKED_SPIN_TIME, 0.0, 0.0);
    zm.client_fx.pending.push(Pending {
        due: now + (NUKED_SPIN_TIME * 1000.0) as i64,
        until: i64::MAX,
        ent: n,
        step: Step::Spin,
    });
}

/// A client script's own loop: its next turn at `due`, for as long as the
/// map runs.
fn again(world: &mut World, ent: u32, due: i64, step: Step) {
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
fn rotate_to(world: &mut World, n: u32, to: [f32; 3], now: i64, secs: f32, accel: f32, decel: f32) {
    let mut zm = world.resource_mut::<Zm>();
    let Some(from) = zm.ents.get(&n).map(|e| e.angles) else { return };
    let mut target = to;
    for i in 0..3 {
        target[i] = from[i] + gsc_t6::math::angle_clamp180(to[i] - from[i]);
    }
    zm.movers.rotate_to(n, from, target, now, secs, accel, decel);
}

/// `rotateyaw` / `rotatepitch` / `rotateroll`: `degrees` more on one axis.
fn rotate_by(world: &mut World, n: u32, axis: usize, degrees: f32, now: i64, secs: f32) {
    let mut zm = world.resource_mut::<Zm>();
    let Some(from) = zm.ents.get(&n).map(|e| e.angles) else { return };
    let mut to = from;
    to[axis] += degrees;
    zm.movers.rotate_to(n, from, to, now, secs, 0.0, 0.0);
}

/// The entities a map's client script finds by targetname, with their
/// script objects (for their keys).
fn named_objs(vm: &mut Vm<World>, world: &mut World, name: &str) -> Vec<(u32, gsc_t6::ObjRef)> {
    let name = Value::Str(vm.intern(name));
    let key = Value::Str(vm.intern("targetname"));
    super::natives_ent::matching(vm, world, &[name, key])
        .iter()
        .filter_map(|e| match e {
            Value::Object(o) => Some((entnum(vm, e)?, *o)),
            _ => None,
        })
        .collect()
}

// bo2mp: _rotating_object.csc's defaults: 1000 turns of 360, each 12 s.
const TURN_REVOLUTIONS: f32 = 1000.0;
const TURN_DEGREES: f32 = 360.0;
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
fn core_start(vm: &mut Vm<World>, world: &mut World, map: &str) {
    let noteworthy = vm.intern("script_noteworthy");
    let float = vm.intern("script_float");
    let objects = named_objs(vm, world, "rotating_object");
    for &(n, o) in &objects {
        let axis = vm.raw_field(o, noteworthy);
        let axis = if axis.is_undefined() { "yaw".to_string() } else { vm.to_text(&axis) };
        let time = vm.raw_field(o, float);
        let mut secs = if time.is_undefined() { TURN_TIME } else { time.as_float().unwrap_or(0.0) };
        if secs == 0.0 {
            secs = TURN_TIME;
        }
        let mut direction = TURN_DEGREES;
        if secs < 0.0 {
            direction = -direction;
            secs = -secs;
        }
        let axis = match axis.as_str() {
            "roll" => 2,
            "pitch" => 0,
            _ => 1,
        };
        let step = Step::Turn {
            axis,
            degrees: direction * TURN_REVOLUTIONS,
            secs: secs * TURN_REVOLUTIONS,
        };
        again(world, n, 0, step);
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
        for n in named(vm, world, name) {
            let step = Step::Clock {
                stage: 0,
                set,
                first,
                bit,
                last: now,
            };
            again(world, n, 0, step);
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
        let s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        [((s / 3600) % 24) as u32, ((s / 60) % 60) as u32, (s % 60) as u32]
    }
}

/// One turn of rotating_object_think's loop: `rotate*( direction *
/// revolutions, rotate_time * revolutions )`, then `rotatedone` and again.
fn turn(world: &mut World, n: u32, axis: usize, degrees: f32, secs: f32, now: i64) {
    if !world.resource::<Zm>().ents.contains_key(&n) {
        return;
    }
    rotate_by(world, n, axis, degrees, now, secs);
    again(world, n, now + ms(secs), Step::Turn { axis, degrees, secs });
}

/// One step of clock_run: `rotatepitch( hand_time * rotate, 0.05 )`, then
/// `first_rotate` the same way, each waiting `rotatedone`; then once a
/// second it reads the time again and, when that differs from the last,
/// `rotatepitch( rotate_bit, 0.05 )`.
fn clock(world: &mut World, n: u32, stage: u8, set: f32, first: Option<f32>, bit: f32, last: [u32; 3], now: i64) {
    if !world.resource::<Zm>().ents.contains_key(&n) {
        return;
    }
    let mut step = Step::Clock {
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
            step = Step::Clock {
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
            step = Step::Clock {
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

fn cameras_start(vm: &mut Vm<World>, world: &mut World, map: &str) {
    // After the first snapshot: every `security_camera_ball` keeps its
    // pitch and roll and turns about its yaw.
    let cameras = named(vm, world, "security_camera_ball");
    for &n in &cameras {
        let Some(angles) = world.resource::<Zm>().ents.get(&n).map(|e| e.angles) else {
            continue;
        };
        let step = Step::Camera {
            pitch: angles[0],
            roll: angles[2],
        };
        again(world, n, 0, step);
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
            .find_map(|p| f.player(crate::world::ClientId(p)).map(|ps| ps.origin))
    };
    if let Some(at) = player {
        let angles = gsc_t6::math::vector_to_angles(gsc_t6::math::sub(at, origin));
        rotate_to(world, n, [pitch, angles[1] + 90.0, roll], now, CAMERA_TIME, 0.0, 0.0);
    }
    again(world, n, now + ms(CAMERA_TIME), Step::Camera { pitch, roll });
}

// bo2mp: Uplink (mp_uplink.csc). The rain and window exploders the wind
// switch starts and stops are left out here: this engine's world effects
// cannot be stopped once placed.
const UPLINK_WIND_TIME: f32 = 8.0;
const UPLINK_VANE_WAIT: f32 = 0.5;
const UPLINK_LIGHTNING: [i32; 2] = [7001, 7005];

fn uplink_start(vm: &mut Vm<World>, world: &mut World) {
    let now = world.resource::<Zm>().now_ms;
    // uplinkonplayerconnect: the one `sway_lightpost` sways; every
    // `wind_direction` vane waits on the wind.
    let post = named(vm, world, "sway_lightpost");
    if let Some(&n) = post.first() {
        again(world, n, 0, Step::Post { angle: 0.0, secs: 0.0, leg: 0 });
    }
    let vanes: Vec<(u32, [f32; 3])> = named(vm, world, "wind_direction")
        .into_iter()
        .filter_map(|n| Some((n, world.resource::<Zm>().ents.get(&n)?.angles)))
        .collect();
    let count = vanes.len();
    world.resource_mut::<Zm>().client_fx.vanes = vanes;
    // rainexploderswitch turns the wind at once (after waittillframeend);
    // initlightningloop waits randomfloatrange( 10, 15 ) first.
    again(world, LEVEL, now, Step::Wind);
    let wait = rand_float(world, 10.0, 15.0);
    again(world, LEVEL, now + ms(wait), Step::Lightning);
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
    again(world, n, now + ms(secs), Step::Post { angle, secs, leg: 1 - leg });
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
        let step = Step::Vane {
            orig,
            yaw,
            stage: 0,
            wind: turn,
        };
        again(world, n, now + ms(UPLINK_VANE_WAIT), step);
    }
    again(world, LEVEL, now + ms(UPLINK_WIND_TIME), Step::Wind);
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
    let step = Step::Vane {
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
    let origin = super::with_vm(world, |vm, world| {
        let at = super::natives_fx::exploder_origin(vm, id)?;
        super::natives_fx::fire_exploder(vm, world, &gsc_t6::Key::Int(id));
        Some(at)
    })
    .flatten();
    match origin {
        Some(origin) => {
            diag::info!(Sim, "bo2mp mp_uplink: lightning {id}");
            let wait = rand_float(world, 0.05, 0.15);
            again(world, LEVEL, now + ms(wait), Step::Thunder { origin, left: None });
        }
        None => {
            let wait = rand_float(world, 10.0, 15.0);
            again(world, LEVEL, now + ms(wait), Step::Lightning);
        }
    }
}

/// One of playlightning's `randomintrange( 2, 4 )` strikes: the thunder at
/// the lightning, then `serverwait( randomfloatrange( 0, 0.25 ) )`; after
/// the last, initlightningloop's `serverwait( randomfloatrange( 10, 15 ) )`.
fn thunder(world: &mut World, origin: [f32; 3], left: Option<u8>, now: i64) {
    let left = left.unwrap_or_else(|| 2 + rand_int(world, 2) as u8);
    let blend = rand_float(world, 0.0, 0.25);
    super::natives_fx::sound(world, EventAudience::All, "amb_thunder_flash", origin);
    let due = now + ms(blend);
    if left > 1 {
        again(world, LEVEL, due, Step::Thunder { origin, left: Some(left - 1) });
    } else {
        let wait = rand_float(world, 10.0, 15.0);
        again(world, LEVEL, due + ms(wait), Step::Lightning);
    }
}

// bo2mp: Turbine (mp_turbine_amb.csc wmill_sfx_setup): the whoosh at three
// windmills every 1.35 s, from half a second in.
const TURBINE_WHOOSH: [[f32; 3]; 3] = [[-2579.0, 3014.0, 385.0], [-1644.0, -636.0, 563.0], [-1073.0, 1351.0, 1941.0]];
const TURBINE_WHOOSH_WAIT: f32 = 1.35;

fn turbine_start(world: &mut World) {
    let now = world.resource::<Zm>().now_ms;
    again(world, LEVEL, now + ms(0.5), Step::Whoosh);
    diag::info!(Sim, "bo2mp mp_turbine: windmill whoosh at {} spots", TURBINE_WHOOSH.len());
}

fn whoosh(world: &mut World, now: i64) {
    for at in TURBINE_WHOOSH {
        super::natives_fx::sound(world, EventAudience::All, "amb_wmill_whoosh", at);
    }
    again(world, LEVEL, now + ms(TURBINE_WHOOSH_WAIT), Step::Whoosh);
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
    let now = world.resource::<Zm>().now_ms;
    again(world, LEVEL, now + ms(0.0), Step::Scanner);
    diag::info!(Sim, "bo2mp mp_drone: scanner alarm on trigger ent{}", trig);
}

fn scanner(world: &mut World, now: i64) {
    let trig = world.resource::<Zm>().client_fx.scanner.trig;
    let Some(e) = world.resource::<Zm>().ents.get(&trig).cloned() else {
        return;
    };
    let players: Vec<u32> = world
        .resource::<Zm>()
        .players
        .iter()
        .filter(|(_, p)| p.sessionstate == "playing")
        .map(|(c, _)| *c)
        .collect();
    let mut touching = Vec::new();
    for c in players {
        if let Some((mins, maxs)) = super::triggers::player_box(world, c)
            && super::triggers::touches(world, &e, mins, maxs)
        {
            touching.push(c);
        }
    }
    let (alarms, entered) = {
        let mut zm = world.resource_mut::<Zm>();
        let s = &mut zm.client_fx.scanner;
        let alarms = s.alarms.iter().filter(|&&at| at <= now).count();
        s.alarms.retain(|&at| at > now);
        // trigger_thread: once the enter payload is done, the player stays
        // in it while he touches it (trig_leave_alarm does nothing).
        s.inside.retain(|c, done| now < *done || touching.contains(c));
        let mut entered = None;
        if now >= s.next_look
            && let Some(&c) = touching.first()
        {
            s.next_look = now + ms(SCANNER_WAIT);
            if !s.inside.contains_key(&c) {
                s.inside.insert(c, now + ms(SCANNER_WAIT));
                s.alarms.push(now + ms(SCANNER_WAIT));
                entered = Some(c);
            }
        }
        (alarms, entered)
    };
    for _ in 0..alarms {
        super::natives_fx::sound(world, EventAudience::All, "amb_scanner_alarm", SCANNER_ALARM);
    }
    if let Some(c) = entered {
        super::natives_fx::sound(world, EventAudience::All, "amb_scanner_detect", e.origin);
        diag::info!(Sim, "bo2mp mp_drone: client {} sets off the scanner", c);
    }
    again(world, LEVEL, now + ms(0.0), Step::Scanner);
}

/// The scripts' random number (0..1), as `randomfloat` draws it.
fn with_rand(world: &mut World) -> f32 {
    super::with_vm(world, |vm, _| vm.rand_unit()).unwrap_or(0.0)
}

/// Each tick (after the entities show): the effects that are due.
pub(crate) fn advance(world: &mut World, now: i64) {
    let due: Vec<Pending> = {
        let mut zm = world.resource_mut::<Zm>();
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
        if let Step::Sway { base, leg, time } = p.step {
            sway(world, p.ent, base, leg, time, now);
            continue;
        }
        if let Step::Spin = p.step {
            spin(world, p.ent, now);
            continue;
        }
        match p.step {
            Step::Turn { axis, degrees, secs } => {
                turn(world, p.ent, axis, degrees, secs, now);
                continue;
            }
            Step::Clock {
                stage,
                set,
                first,
                bit,
                last,
            } => {
                clock(world, p.ent, stage, set, first, bit, last, now);
                continue;
            }
            Step::Camera { pitch, roll } => {
                camera(world, p.ent, pitch, roll, now);
                continue;
            }
            Step::Post { angle, secs, leg } => {
                post(world, p.ent, angle, secs, leg, now);
                continue;
            }
            Step::Wind => {
                wind(world, now);
                continue;
            }
            Step::Vane {
                orig,
                yaw,
                stage,
                wind,
            } => {
                vane(world, p.ent, orig, yaw, stage, wind, now);
                continue;
            }
            Step::Lightning => {
                lightning(world, now);
                continue;
            }
            Step::Thunder { origin, left } => {
                thunder(world, origin, left, now);
                continue;
            }
            Step::Whoosh => {
                whoosh(world, now);
                continue;
            }
            Step::Scanner => {
                scanner(world, now);
                continue;
            }
            Step::Swarm => {
                swarm::tick(world, now);
                continue;
            }
            Step::VisionFade => {
                vision::tick(world, now);
                continue;
            }
            _ => {}
        }
        if let Step::Tank(t) = p.step {
            if let Some((due, t)) = ai_tank::tick(world, p.ent, t, now) {
                world.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due,
                    until: i64::MAX,
                    ent: p.ent,
                    step: Step::Tank(t),
                });
            }
            continue;
        }
        if let Step::Heli(h) = p.step {
            if let Some((due, h)) = heli::tick(world, p.ent, h, now) {
                world.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due,
                    until: i64::MAX,
                    ent: p.ent,
                    step: Step::Heli(h),
                });
            }
            continue;
        }
        if let Step::TrainDust { next, last } = p.step {
            if let Some(next) = train_dust(world, p.ent, next, last) {
                world.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due: now + 10,
                    step: Step::TrainDust { next, last },
                    ..p
                });
            }
            continue;
        }
        if let Step::HeldLight { sounded } = p.step {
            if let Some((after, sounded)) = held_light(world, p.ent, sounded) {
                world.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due: now + after,
                    until: i64::MAX,
                    ent: p.ent,
                    step: Step::HeldLight { sounded },
                });
            }
            continue;
        }
        let shown = super::natives_fx::shown(world, p.ent).map(|s| s.3);
        let Some(origin) = shown else {
            // Not drawn yet (spawned this tick): try again until it is.
            if now < p.until {
                world
                    .resource_mut::<Zm>()
                    .client_fx
                    .pending
                    .push(Pending { due: now + 50, ..p });
            }
            continue;
        };
        match p.step {
            Step::At { fx, offset } => {
                let at = [
                    origin[0] + offset[0],
                    origin[1] + offset[1],
                    origin[2] + offset[2],
                ];
                super::natives_fx::effect(world, fx, at, [0.0, 0.0, 1.0]);
            }
            Step::Bolt { fx, tag, stop } => {
                super::natives_fx::bolt_effect(world, p.ent, fx, tag, stop);
            }
            Step::Light { fx, tag, stop, to } => {
                super::natives_fx::bolt_effect_to(world, to, p.ent, fx, tag, stop);
            }
            Step::QrBlink { interval, quicken, sides } => {
                let i = if quicken { (interval / 1.17).max(0.1) } else { interval };
                let half = (i * 500.0).round() as i64;
                super::natives_fx::sound(world, EventAudience::All, RC_BEEP, origin);
                let mut zm = world.resource_mut::<Zm>();
                for (fx, to) in sides.iter().cloned() {
                    zm.client_fx.pending.push(Pending {
                        due: now,
                        until: now + 2500,
                        ent: p.ent,
                        step: Step::Light { fx, tag: QR_TAG, stop: false, to: to.clone() },
                    });
                    zm.client_fx.pending.push(Pending {
                        due: now + half,
                        until: now + 2500,
                        ent: p.ent,
                        step: Step::Light { fx, tag: QR_TAG, stop: true, to },
                    });
                }
                zm.client_fx.pending.push(Pending {
                    due: now + 2 * half,
                    until: now + 2500,
                    ent: p.ent,
                    step: Step::QrBlink { interval: i, quicken: true, sides },
                });
            }
            Step::Sound { alias } => {
                super::natives_fx::sound(world, EventAudience::All, alias, origin);
            }
            Step::HeldLight { .. }
            | Step::Sway { .. }
            | Step::Spin
            | Step::TrainDust { .. }
            | Step::Turn { .. }
            | Step::Clock { .. }
            | Step::Camera { .. }
            | Step::Post { .. }
            | Step::Wind
            | Step::Vane { .. }
            | Step::Lightning
            | Step::Thunder { .. }
            | Step::Whoosh
            | Step::Scanner
            | Step::Swarm
            | Step::VisionFade => {}
            Step::Blink { fx, tag } => {
                super::natives_fx::bolt_effect(world, p.ent, fx, tag, false);
                world.resource_mut::<Zm>().client_fx.pending.push(Pending {
                    due: now + 500,
                    until: now + 2500,
                    ..p
                });
            }
            Step::Tank(_) => {}
            Step::Heli(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_missile() -> World {
        let mut zm = Zm::default();
        zm.ents.insert(90, crate::t6::Ent { origin: [1.0, 2.0, 3.0], angles: [0.0, 30.0, 0.0], ..Default::default() });
        let mut world = World::new();
        world.insert_resource(zm);
        world
    }

    fn now(world: &mut World, ms: i64) {
        world.resource_mut::<Zm>().now_ms = ms;
    }

    #[test]
    fn the_hellstorm_shows_on_the_minimap_and_lingers_after_it_goes_off() {
        let mut world = world_with_missile();
        missile_icon(&mut world, 90, true, 50);
        let rows = client_objectives(&mut world);
        assert_eq!(rows.len(), 1);
        let o = &rows[0];
        assert_eq!((o.n, o.icon, o.size, o.yaw), (90, "remotemissile_target", 50, 30.0));
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
                Step::QrBlink { interval, quicken, .. } if p.ent == n => Some((*interval, *quicken)),
                _ => None,
            })
            .collect()
    }

    fn no_sides() -> [(&'static str, EventAudience); 3] {
        [
            (QR_DRIVER, EventAudience::Clients(Vec::new())),
            (QR_FRIEND, EventAudience::Clients(Vec::new())),
            (QR_ENEMY, EventAudience::Clients(Vec::new())),
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
            if let Step::QrBlink { interval, quicken, .. } = &mut p.step {
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
