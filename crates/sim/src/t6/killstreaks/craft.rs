//! Scorestreak aircraft and vehicles the scripts spawn: helicopters
//! (`spawnhelicopter`: the care package chopper, the Stealth Chopper, the
//! Escort Drone, the VTOL Warship), script vehicles (`spawnvehicle`) and
//! planes (`spawnplane`: a model the scripts move themselves).
//!
//! BO2's engine flies a helicopter to the goal its scripts set
//! (`setvehgoalpos(point, stop)`) at the speed they set (`setspeed`, miles
//! per hour, with its acceleration), turned to face where it goes - or a
//! goal yaw, a target yaw, an entity it looks at - and tells the scripts
//! `near_goal` (inside `setneargoalnotifydist`) and `goal` (there). The
//! scripts chain goals into flight paths (`_airsupport::followpath`).

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, Value, Vm};

use super::super::{Ent, Zm, arg, entnum, text};
use super::{notify_all, streaks};

/// Miles per hour in units a second.
const MPH: f32 = 17.6;

#[derive(Clone, Debug)]
pub(crate) struct Craft {
    /// Wanted speed and acceleration (units/s, units/s²), speed now.
    pub speed: f32,
    pub accel: f32,
    pub cur: f32,
    /// Where it flies and whether it stops there.
    pub goal: Option<[f32; 3]>,
    pub stop: bool,
    /// `near_goal` distance (0: none) and whether it was told.
    pub near: f32,
    pub near_sent: bool,
    pub goal_yaw: Option<f32>,
    pub target_yaw: Option<f32>,
    pub look_at: Option<u32>,
    /// Turn rate (degrees a second).
    pub yaw_speed: f32,
    /// The weapon it fires (its vehicle's turret weapon, or `setvehweapon`),
    /// its gunners' weapons, the player it fires for, and what its turret
    /// aims at.
    pub weapon: String,
    pub gunners: Vec<String>,
    pub owner: Option<u32>,
    /// The player riding or driving it (`usevehicle`).
    pub rider: Option<u32>,
    pub aim: Option<Aim>,
    /// It drives on the ground (the AGR, the RC-XD): its goal is reached
    /// over the map's path nodes (`route`), on the ground.
    pub ground: bool,
    pub route: Vec<[f32; 3]>,
    /// Gunners whose weapon may not fire (`disablegunnerfiring`: the AGR's
    /// rockets while it reloads).
    pub gunner_off: [bool; 4],
    /// Its turret's and first gunner's heat (the VTOL Warship's rockets).
    pub heat: [weapon_t6::vehicle_heat::GunHeat; 2],
    /// The barrel each of its guns fired last (its turret's, then each
    /// gunner's: BO2's `fireBarrel`), where the next shot goes round from.
    pub barrel: [usize; 5],
    /// Its own physics' acceleration and spin (degrees a second), and its
    /// most pitch and roll, as its scripts set them: a drone off its
    /// driver's keys flies on them.
    pub phys_accel: [f32; 3],
    pub rot_vel: [f32; 3],
    pub max_pitch_roll: Option<[f32; 2]>,
    /// Its vehicle def's `remoteControl` (a drone driven from afar): another
    /// vehicle's move is never pushed out of it.
    pub remote_control: bool,
    /// Its hover sway (`setjitterparams`: the VTOL Warship's).
    pub jitter: Option<Jitter>,
}

/// `setjitterparams( amplitude, min_time, max_time )`: the craft drifts to
/// a new spot within `amplitude` of where it flies every `min_time` to
/// `max_time` seconds, easing between them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Jitter {
    pub amp: [f32; 3],
    pub min_t: f32,
    pub max_t: f32,
    /// The offset it put on last, where it eases from and to, and how far
    /// through (seconds of `len`).
    pub off: [f32; 3],
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub at: f32,
    pub len: f32,
    pub seed: u32,
}

impl Jitter {
    fn next(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The offset for one more tick of `dt` seconds.
    fn step(&mut self, dt: f32) -> [f32; 3] {
        self.at += dt;
        if self.at >= self.len {
            self.from = self.to;
            self.to = std::array::from_fn(|i| (self.next() * 2.0 - 1.0) * self.amp[i]);
            let lo = self.min_t.max(0.05);
            self.len = lo + self.next() * (self.max_t - lo).max(0.0);
            self.at = 0.0;
        }
        let t = (self.at / self.len).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        std::array::from_fn(|i| self.from[i] + (self.to[i] - self.from[i]) * t)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Aim {
    Ent(u32),
    Point([f32; 3]),
}

impl Default for Craft {
    fn default() -> Self {
        Self {
            // A helicopter's cruise until its scripts set one.
            speed: 60.0 * MPH,
            accel: 20.0 * MPH,
            cur: 0.0,
            goal: None,
            stop: true,
            near: 0.0,
            near_sent: false,
            goal_yaw: None,
            target_yaw: None,
            look_at: None,
            yaw_speed: 90.0,
            weapon: String::new(),
            gunners: Vec::new(),
            owner: None,
            rider: None,
            aim: None,
            ground: false,
            route: Vec::new(),
            gunner_off: [false; 4],
            heat: Default::default(),
            barrel: [0; 5],
            phys_accel: [0.0; 3],
            rot_vel: [0.0; 3],
            max_pitch_roll: None,
            remote_control: false,
            jitter: None,
        }
    }
}

/// A new script vehicle entity with its model (drawn by the presences).
fn spawn_craft(
    vm: &mut Vm<World>,
    world: &mut World,
    origin: [f32; 3],
    angles: [f32; 3],
    model: &str,
    kind: &str,
    owner: Option<u32>,
    flies: bool,
) -> Value {
    let n = world.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(ObjKind::Entity(n));
    // Its side is its owner's (BO2's `spawnhelicopter`, `spawnplane`); one
    // with no owner has none.
    let faction_team = owner.map_or(entity_t6::team::FREE, |o| player_team(world, o));
    world.resource_mut::<Zm>().ents.insert(
        n,
        Ent {
            obj: Some(obj),
            classname: "script_vehicle".to_owned(),
            origin,
            angles,
            model: model.to_owned(),
            // A vehicle takes damage (BO2's `VEH_InitEntity`); a plane not.
            can_damage: flies,
            faction_team,
            faction_owner: owner,
            ..Default::default()
        },
    );
    let f = vm.intern("vehicletype");
    let k = vm.string(kind);
    vm.set_raw_field(obj, f, k);
    // Its engine loops (BO2's client script `_helicopter_sounds.csc`).
    super::super::engine_sounds::spawned(world, n, kind);
    super::super::clientfields::vehicle_spawned(world, n, kind);
    if flies {
        // Its guns, from the vehicle's own data in his zones.
        let (weapon, gunners) = streaks(world)
            .vehicle_weapons
            .get(&kind.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default();
        // A ground vehicle drives at its vehicle def's own speed.
        let ground = super::ride::is_ground(kind);
        let drive = streaks(world)
            .vehicle_drive
            .get(&kind.to_ascii_lowercase())
            .copied()
            .unwrap_or_default();
        let mut c = Craft {
            owner,
            weapon,
            gunners,
            ground,
            remote_control: drive.remote_control != 0,
            ..Default::default()
        };
        if ground && drive.max_speed > 0.0 {
            c.speed = drive.max_speed;
            c.accel = drive.accel.max(1.0);
        }
        streaks(world).craft.insert(n, c);
    }
    Value::Object(obj)
}

/// A player's team (`sessionteam`) as a client's team number.
fn player_team(world: &mut World, player: u32) -> i32 {
    super::super::frame(world)
        .client_meta(crate::ClientId(player))
        .map_or(entity_t6::team::FREE, |m| m.client_state_team)
}

/// A player or an actor: BO2's `setowner` and `setteam` refuse them.
fn person(world: &World, n: u32) -> bool {
    let zm = world.resource::<Zm>();
    zm.players.contains_key(&n) || zm.actors.contains_key(&n)
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    // `spawnhelicopter(owner, origin, angles, vehicle type, model)`.
    f!("spawnhelicopter", |vm, world, _, a| {
        let owner =
            entnum(vm, arg(a, 0)).filter(|n| world.resource::<Zm>().players.contains_key(n));
        let origin = super::super::vec3(a, 1)?;
        let angles = arg(a, 2).as_vec3().unwrap_or([0.0; 3]);
        let (kind, model) = (text(vm, a, 3), text(vm, a, 4));
        Ok(spawn_craft(
            vm, world, origin, angles, &model, &kind, owner, true,
        ))
    });
    // `spawnvehicle(model, targetname, vehicle type, origin, angles)`.
    f!("spawnvehicle", |vm, world, _, a| {
        let (model, name, kind) = (text(vm, a, 0), text(vm, a, 1), text(vm, a, 2));
        let origin = super::super::vec3(a, 3)?;
        let angles = arg(a, 4).as_vec3().unwrap_or([0.0; 3]);
        let v = spawn_craft(vm, world, origin, angles, &model, &kind, None, true);
        if let Value::Object(o) = &v {
            let f = vm.intern("targetname");
            let s = vm.string(&name);
            vm.set_raw_field(*o, f, s);
        }
        Ok(v)
    });
    // `spawnplane(owner, classname, origin)`: a model the scripts move.
    f!("spawnplane", |vm, world, _, a| {
        let owner =
            entnum(vm, arg(a, 0)).filter(|n| world.resource::<Zm>().players.contains_key(n));
        let origin = super::super::vec3(a, 2)?;
        let v = spawn_craft(vm, world, origin, [0.0; 3], "", "plane", owner, false);
        if let Some(n) = entnum(vm, &v)
            && let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n)
        {
            e.classname = "script_model".to_owned();
        }
        Ok(v)
    });
    fn craft_of(vm: &Vm<World>, world: &mut World, s: &Value) -> Option<u32> {
        let n = entnum(vm, s)?;
        streaks(world).craft.contains_key(&n).then_some(n)
    }
    fn with_craft(vm: &Vm<World>, world: &mut World, s: &Value, f: impl FnOnce(&mut Craft)) {
        if let Some(n) = craft_of(vm, world, s)
            && let Some(c) = streaks(world).craft.get_mut(&n)
        {
            f(c);
        }
    }
    m!("setspeed", |vm, world, s, a| {
        let speed = arg(a, 0).as_float().unwrap_or(0.0) * MPH;
        let accel = arg(a, 1).as_float().map(|x| x * MPH);
        with_craft(vm, world, s, |c| {
            c.speed = speed.max(0.0);
            if let Some(x) = accel.filter(|x| *x > 0.0) {
                c.accel = x;
            }
        });
        Ok(Value::Undefined)
    });
    m!("setspeedimmediate", |vm, world, s, a| {
        let speed = arg(a, 0).as_float().unwrap_or(0.0) * MPH;
        with_craft(vm, world, s, |c| {
            c.speed = speed.max(0.0);
            c.cur = c.speed;
        });
        Ok(Value::Undefined)
    });
    m!("getspeed", |vm, world, s, _| {
        let n = craft_of(vm, world, s);
        Ok(Value::Float(
            n.and_then(|n| streaks(world).craft.get(&n).map(|c| c.cur))
                .unwrap_or(0.0),
        ))
    });
    m!("getspeedmph", |vm, world, s, _| {
        let n = craft_of(vm, world, s);
        Ok(Value::Float(
            n.and_then(|n| streaks(world).craft.get(&n).map(|c| c.cur / MPH))
                .unwrap_or(0.0),
        ))
    });
    // `setvehgoalpos(goal, stop)`: true when it can get there (a ground
    // vehicle over the map's path nodes; anything flying, straight).
    m!("setvehgoalpos", |vm, world, s, a| {
        let goal = super::super::vec3(a, 0)?;
        let stop = super::super::flag(a, 1, false);
        let Some(n) = craft_of(vm, world, s) else {
            return Ok(Value::Int(0));
        };
        let ground = streaks(world).craft.get(&n).is_some_and(|c| c.ground);
        let route = if ground {
            let from = world
                .resource::<Zm>()
                .ents
                .get(&n)
                .map_or(goal, |e| e.origin);
            ground_route(world, from, goal)
        } else {
            Some(Vec::new())
        };
        let Some(route) = route else {
            return Ok(Value::Int(0));
        };
        with_craft(vm, world, s, |c| {
            c.goal = Some(goal);
            c.stop = stop;
            c.near_sent = false;
            c.route = route;
        });
        Ok(Value::Int(1))
    });
    m!("clearvehgoalpos", |vm, world, s, _| {
        with_craft(vm, world, s, |c| {
            c.goal = None;
            c.route.clear();
        });
        Ok(Value::Undefined)
    });
    // Whose it is (`setowner`, BO2's `GScr_SetOwner`): its shots are that
    // player's, and its owner sees it as his side's (`setenemymodel`).
    m!("setowner", |vm, world, s, a| {
        let n = entnum(vm, s);
        if n.is_some_and(|n| person(world, n)) {
            return Err("SetOwner can not be used for players or actors".to_owned());
        }
        let owner =
            entnum(vm, arg(a, 0)).filter(|n| world.resource::<Zm>().players.contains_key(n));
        with_craft(vm, world, s, |c| {
            if owner.is_some() {
                c.owner = owner;
            }
        });
        if let (Some(n), Some(o)) = (n, owner)
            && let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n)
        {
            e.faction_owner = Some(o);
        }
        Ok(Value::Undefined)
    });
    // Its side (`setteam`, `setvehicleteam`: BO2's `GScr_SetTeam`), for the
    // model each player sees it as.
    for name in ["setteam", "setvehicleteam"] {
        vm.bind(name, true, |vm, world, s, a| {
            let n = entnum(vm, s);
            if n.is_some_and(|n| person(world, n)) {
                return Err("SetTeam can not be used for players or actors".to_owned());
            }
            let team = super::super::objectives::team_number(&text(vm, a, 0))?;
            if let Some(n) = n
                && let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n)
            {
                e.faction_team = super::super::objectives::client_team(team);
            }
            Ok(Value::Undefined)
        });
    }
    // Its model for the other side (`setenemymodel`, BO2's
    // `ScrCmd_SetEnemyModel`; "" shows everyone its own): a scorestreak's
    // red-marked look, a dog's black vest.
    m!("setenemymodel", |vm, world, s, a| {
        let model = text(vm, a, 0);
        if let Some(n) = entnum(vm, s)
            && let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n)
        {
            e.enemy_model = model;
        }
        Ok(Value::Undefined)
    });
    m!("setneargoalnotifydist", |vm, world, s, a| {
        let d = arg(a, 0).as_float().unwrap_or(0.0);
        with_craft(vm, world, s, |c| c.near = d);
        Ok(Value::Undefined)
    });
    m!("setgoalyaw", |vm, world, s, a| {
        let y = arg(a, 0).as_float();
        with_craft(vm, world, s, |c| c.goal_yaw = y);
        Ok(Value::Undefined)
    });
    m!("cleargoalyaw", |vm, world, s, _| {
        with_craft(vm, world, s, |c| c.goal_yaw = None);
        Ok(Value::Undefined)
    });
    m!("settargetyaw", |vm, world, s, a| {
        let y = arg(a, 0).as_float();
        with_craft(vm, world, s, |c| c.target_yaw = y);
        Ok(Value::Undefined)
    });
    m!("cleartargetyaw", |vm, world, s, _| {
        with_craft(vm, world, s, |c| c.target_yaw = None);
        Ok(Value::Undefined)
    });
    m!("setyawspeed", |vm, world, s, a| {
        let y = arg(a, 0).as_float().unwrap_or(90.0);
        with_craft(vm, world, s, |c| c.yaw_speed = y.max(1.0));
        Ok(Value::Undefined)
    });
    m!("setlookatent", |vm, world, s, a| {
        let t = entnum(vm, arg(a, 0));
        with_craft(vm, world, s, |c| c.look_at = t);
        Ok(Value::Undefined)
    });
    m!("clearlookatent", |vm, world, s, _| {
        with_craft(vm, world, s, |c| c.look_at = None);
        Ok(Value::Undefined)
    });
    m!("setvehvelocity", |vm, world, s, a| {
        let v = arg(a, 0).as_vec3().unwrap_or([0.0; 3]);
        let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        with_craft(vm, world, s, |c| c.cur = speed);
        // A driven drone's velocity is its ride's.
        if let Some(n) = entnum(vm, s)
            && let Some(r) = streaks(world)
                .rides
                .values_mut()
                .find(|r| r.vehicle == n && r.drive == super::ride::Drive::Air)
        {
            r.vel = v;
        }
        Ok(Value::Undefined)
    });
    // Its own physics (a drone off its driver's keys flies on them).
    m!("setphysacceleration", |vm, world, s, a| {
        let v = arg(a, 0).as_vec3().unwrap_or([0.0; 3]);
        with_craft(vm, world, s, |c| c.phys_accel = v);
        Ok(Value::Undefined)
    });
    m!("setangularvelocity", |vm, world, s, a| {
        let v = arg(a, 0).as_vec3().unwrap_or([0.0; 3]);
        with_craft(vm, world, s, |c| c.rot_vel = v);
        Ok(Value::Undefined)
    });
    m!("getangularvelocity", |vm, world, s, _| {
        let n = craft_of(vm, world, s);
        Ok(Value::Vec3(
            n.and_then(|n| streaks(world).craft.get(&n).map(|c| c.rot_vel))
                .unwrap_or([0.0; 3]),
        ))
    });
    m!("setmaxpitchroll", |vm, world, s, a| {
        let pitch = arg(a, 0).as_float().unwrap_or(180.0).abs();
        let roll = arg(a, 1).as_float().unwrap_or(180.0).abs();
        with_craft(vm, world, s, |c| c.max_pitch_roll = Some([pitch, roll]));
        Ok(Value::Undefined)
    });
    // Its weapon and turret: what it fires and at what.
    m!("setvehweapon", |vm, world, s, a| {
        let w = text(vm, a, 0);
        with_craft(vm, world, s, |c| c.weapon = w);
        Ok(Value::Undefined)
    });
    // Its turret on a target: on it at once (`turret_on_target`), and
    // whether it sees it from there (`turret_on_vistarget`, `turret_no_vis`:
    // the AGR fires only at what its gun sees).
    fn aim_at(vm: &mut Vm<World>, world: &mut World, s: &Value, aim: Option<Aim>) {
        with_craft(vm, world, s, |c| c.aim = aim);
        if let Some(aim) = aim
            && let Some(o) = s.as_obj()
        {
            super::super::natives_game::notify_later(vm, world, o, "turret_on_target", 50);
            let from = super::super::origin_of(vm, world, s).unwrap_or([0.0; 3]);
            let eye = [from[0], from[1], from[2] + 32.0];
            let to = match aim {
                Aim::Point(p) => Some(p),
                Aim::Ent(t) => {
                    let v = world_obj(world, t);
                    let is_player = world.resource::<Zm>().players.contains_key(&t);
                    super::super::origin_of(vm, world, &v)
                        .map(|p| [p[0], p[1], p[2] + if is_player { 40.0 } else { 16.0 }])
                }
            };
            let seen = to.is_some_and(|to| {
                super::super::frame(world)
                    .trace_static_world(
                        eye,
                        to,
                        [0.0; 3],
                        [0.0; 3],
                        crate::bullet_collision::MASK_SHOT,
                    )
                    .fraction
                    >= 1.0
            });
            let note = if seen {
                "turret_on_vistarget"
            } else {
                "turret_no_vis"
            };
            super::super::natives_game::notify_later(vm, world, o, note, 100);
        }
    }
    m!("setturrettargetent", |vm, world, s, a| {
        let t = entnum(vm, arg(a, 0)).map(Aim::Ent);
        aim_at(vm, world, s, t);
        Ok(Value::Undefined)
    });
    m!("setgunnertargetent", |vm, world, s, a| {
        let t = entnum(vm, arg(a, 0)).map(Aim::Ent);
        aim_at(vm, world, s, t);
        Ok(Value::Undefined)
    });
    m!("setgunnertargetvec", |vm, world, s, a| {
        let p = arg(a, 0).as_vec3().map(Aim::Point);
        aim_at(vm, world, s, p);
        Ok(Value::Undefined)
    });
    m!("cleargunnertarget", |vm, world, s, _| {
        with_craft(vm, world, s, |c| c.aim = None);
        Ok(Value::Undefined)
    });
    m!("setturrettargetvec", |vm, world, s, a| {
        let p = arg(a, 0).as_vec3().map(Aim::Point);
        aim_at(vm, world, s, p);
        Ok(Value::Undefined)
    });
    m!("clearturrettarget", |vm, world, s, _| {
        with_craft(vm, world, s, |c| c.aim = None);
        Ok(Value::Undefined)
    });
    // `fireweapon([tag, target])`: its turret weapon at the target (or what
    // its turret aims at); `firegunnerweapon(index)`: that gunner's.
    m!("fireweapon", |vm, world, s, a| {
        let target = entnum(vm, arg(a, 1)).map(Aim::Ent);
        fire(vm, world, s, None, target);
        Ok(Value::Undefined)
    });
    m!("firegunnerweapon", |vm, world, s, a| {
        let gunner = arg(a, 0).as_int().unwrap_or(0).max(0) as usize;
        fire(vm, world, s, Some(gunner), None);
        Ok(Value::Undefined)
    });
    // How it sways, tilts and sounds, its damage looks, its avoidance of
    // others: the look of its flight.
    for name in [
        "sethoverparams",
        "setturningability",
        "setrotorspeed",
        "setvehicleavoidance",
        "setdamagestage",
        "setdefaultdroppitch",
        "setbrake",
        "makevehicleunusable",
        "makevehicleusable",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
    }
    m!("setjitterparams", |vm, world, s, a| {
        let amp = arg(a, 0).as_vec3().unwrap_or([0.0; 3]).map(f32::abs);
        let min_t = arg(a, 1).as_float().unwrap_or(0.0);
        let max_t = arg(a, 2).as_float().unwrap_or(0.0);
        let seed = craft_of(vm, world, s).unwrap_or(0).wrapping_mul(0x9e37_79b9) | 1;
        with_craft(vm, world, s, |c| {
            let j = c.jitter.get_or_insert(Jitter { seed, ..Default::default() });
            j.amp = amp;
            j.min_t = min_t;
            j.max_t = max_t.max(min_t);
            if amp == [0.0; 3] {
                c.jitter = None;
            }
        });
        Ok(Value::Undefined)
    });
    // A chopper's turret sees its target (`heliturretsighttrace(start,
    // target, last)`, `heliturretdogtrace`: the Stealth Chopper's missile
    // and dog checks): 0 when the line from its turret point to the
    // target's middle is clear, else 1.
    fn turret_sight(vm: &mut Vm<World>, world: &mut World, a: &[Value]) -> Result<Value, String> {
        let from = super::super::vec3(a, 0)?;
        let target = arg(a, 1).clone();
        let Some(to) = super::super::origin_of(vm, world, &target) else {
            return Ok(Value::Int(1));
        };
        let mid = if super::super::is_player(vm, world, &target) { 40.0 } else { 16.0 };
        let t = super::super::frame(world).trace_static_world(
            from,
            [to[0], to[1], to[2] + mid],
            [0.0; 3],
            [0.0; 3],
            crate::bullet_collision::MASK_SHOT,
        );
        Ok(Value::Int(i32::from(t.fraction < 1.0)))
    }
    m!("heliturretsighttrace", |vm, world, _, a| turret_sight(vm, world, a));
    m!("heliturretdogtrace", |vm, world, _, a| turret_sight(vm, world, a));
    // `disablegunnerfiring(gunner, off)`: that gunner's weapon holds fire.
    m!("disablegunnerfiring", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let gunner = arg(a, 0).as_int().unwrap_or(0);
        let off = arg(a, 1).as_float().unwrap_or(1.0) != 0.0;
        if let Ok(g) = usize::try_from(gunner)
            && let Some(c) = streaks(world).craft.get_mut(&n)
            && let Some(slot) = c.gunner_off.get_mut(g)
        {
            *slot = off;
        }
        Ok(Value::Undefined)
    });
}

/// One shot of a craft's weapon (its turret's, or a gunner's) at its aim:
/// a gun's bullet hits when the line is clear (its damage less with
/// range), anything else leaves as the engine's own projectile. The shot
/// is its player's.
fn fire(vm: &mut Vm<World>, world: &mut World, s: &Value, gunner: Option<usize>, aim: Option<Aim>) {
    if let Some(n) = entnum(vm, s) {
        let aim = aim.or_else(|| streaks(world).craft.get(&n).and_then(|c| c.aim));
        if let Some(aim) = aim {
            fire_n(vm, world, n, gunner, aim);
        }
    }
}

/// One server frame of heat on a player-driven craft's gun (its turret, or
/// its first gunner's), with its trigger `held`: whether it may fire, and
/// what a gunner's gun tells its scripts (`gunner_turret_overheat`, then
/// `gunner_turret_stop_overheat`: the VTOL Warship's rocket reload sound).
pub(super) fn heat_step(
    world: &mut World,
    n: u32,
    gunner: Option<usize>,
    held: bool,
) -> (bool, Option<&'static str>) {
    use weapon_t6::vehicle_heat::HeatChange;
    let Some(name) = streaks(world).craft.get(&n).map(|c| match gunner {
        Some(i) => c.gunners.get(i).cloned().unwrap_or_default(),
        None => c.weapon.clone(),
    }) else {
        return (true, None);
    };
    let w = super::super::weapon(world, &name).unwrap_or(0);
    if w == 0 {
        return (true, None);
    }
    let facts = super::super::frame(world).gun_facts().get(w).fire;
    let mut st = streaks(world);
    let Some(heat) = st.craft.get_mut(&n).and_then(|c| c.heat.get_mut(usize::from(gunner.is_some())))
    else {
        return (true, None);
    };
    let change = heat.step(&facts, held, crate::MATCH_TICK_MS as i32);
    let note = match (gunner, change) {
        (Some(_), HeatChange::Overheated) => Some("gunner_turret_overheat"),
        (Some(_), HeatChange::Cooled) => Some("gunner_turret_stop_overheat"),
        _ => None,
    };
    (heat.can_fire(), note)
}

/// A shot from a player-driven craft (the Dragonfire, the VTOL's guns):
/// the time until that gun fires again (ms), None when it has no such gun.
pub(super) fn fire_from(world: &mut World, n: u32, gunner: Option<usize>, aim: Aim) -> Option<i32> {
    super::super::with_vm(world, |vm, world| fire_n(vm, world, n, gunner, aim)).flatten()
}

fn fire_n(
    vm: &mut Vm<World>,
    world: &mut World,
    n: u32,
    gunner: Option<usize>,
    aim: Aim,
) -> Option<i32> {
    let c = streaks(world).craft.get(&n).cloned()?;
    let name = match gunner {
        Some(i) => c.gunners.get(i).cloned().unwrap_or_default(),
        None => c.weapon.clone(),
    };
    let owner = c.owner.or(c.rider)?;
    let w = super::super::weapon(world, &name)
        .ok()
        .filter(|w| *w != 0)?;
    let to = match aim {
        Aim::Point(p) => p,
        Aim::Ent(t) => {
            let target = world_obj(world, t);
            let o = super::super::origin_of(vm, world, &target)?;
            if world.resource::<Zm>().players.contains_key(&t) {
                [o[0], o[1], o[2] + 40.0]
            } else {
                o
            }
        }
    };
    let facts = super::super::frame(world).combat_facts_for(w);
    let interval = facts.map_or(100, |f| f.fire_time_ms.max(1));
    let bullet = facts.is_some_and(|f| f.weap_type == i32::from(weapon_t6::facts::weap_type::BULLET));
    // BG_GetWeaponFireBarrelInfo: the barrels that fire this time, round
    // from the one that fired last (the VTOL Warship's minigun fires both,
    // its rockets go side to side, the AGR's gun round its four).
    let slot = gunner.map_or(0, |i| i + 1);
    let last = c.barrel.get(slot).copied().unwrap_or(0);
    let (first, count) = super::super::frame(world).gun_facts().get(w).turret.barrels(last);
    for b in first..first + count {
        if let Some(c) = streaks(world).craft.get_mut(&n)
            && let Some(fired) = c.barrel.get_mut(slot)
        {
            *fired = b;
        }
        let start = muzzle_at(world, n, gunner, b);
        if bullet {
            let dir = gsc_t6::math::normalize(gsc_t6::math::sub(to, start));
            // VEH_FireGunnerWeapon: a gunner's gun with its own vehicle in
            // the way does not fire, nor do its barrels after.
            if gunner.is_some() && super::bullet::own_hull_blocks(world, n, start, dir) {
                return Some(interval);
            }
            // Its gun's flash and sound at the barrel, for everyone; its
            // bullet traced as a player's is (G_TraceBulletPathForVehTurret),
            // the player at it its attacker, from his view when he aims it.
            gun_event(world, n, gunner, b, w, start, to);
            let from = shot_view(world, n, gunner).unwrap_or(start);
            super::bullet::fire(
                vm,
                world,
                &super::bullet::Gun {
                    ent: n,
                    shooter: c.rider,
                    owner: Some(owner),
                    weapon: &name,
                    w,
                },
                from,
                gsc_t6::math::normalize(gsc_t6::math::sub(to, from)),
            );
            continue;
        }
        // A rocket leaves its barrel, or his view where his view aims it, as
        // a bullet does (`muzzleTrace`); it flies out through the vehicle he
        // rides (`missile_passes`).
        gun_event(world, n, gunner, b, w, start, to);
        let from = shot_view(world, n, gunner).unwrap_or(start);
        let t = super::super::tick(world);
        let _ = crate::missile::magic_bullet(
            &mut super::super::frame(world),
            t,
            crate::world::ClientId(owner),
            w,
            from,
            to,
        );
    }
    Some(interval)
}

/// BO2's `VEH_TANK` vehicle type (the AGR).
const VEH_TANK: i32 = 2;

/// Whether a missile passes the vehicle its owner rides
/// (`G_RunMissileInternal`): any but a tank.
pub(super) fn passes_own_vehicle(vehicle_type: i32) -> bool {
    vehicle_type != VEH_TANK
}

/// Where a craft's shot starts when a player's view aims it (BO2's
/// `muzzleTrace`): a gunner gun's rider's view (gunner `i` sits in seat
/// `i + 1`), or a tank's driver's at its own gun; none when its muzzle.
fn shot_view(world: &World, n: u32, gunner: Option<usize>) -> Option<[f32; 3]> {
    let seat = gunner.map_or(0, |i| i as i32 + 1);
    let (view, vehicle_type) = super::ride::seat_view(world, n, seat)?;
    fires_from_view(gunner, vehicle_type).then_some(view)
}

/// Whether the rider at a craft's gun fires it from his view: any gunner
/// seat's (`VEH_FireGunnerWeapon`), a driver's only on a tank
/// (`CMD_VEH_FireWeapon`: the AGR's); the others' leave its muzzle.
fn fires_from_view(gunner: Option<usize>, vehicle_type: i32) -> bool {
    gunner.is_some() || vehicle_type == VEH_TANK
}

/// Where a craft's gun fires from: its turret's first barrel, or a
/// gunner's.
pub(super) fn muzzle(world: &mut World, n: u32, gunner: Option<usize>) -> [f32; 3] {
    muzzle_at(world, n, gunner, 0)
}

/// The flash tag of a craft gun's barrel: its turret's `tag_flash`, then
/// `tag_flash2` to `tag_flash4` (CMD_VEH_FireWeapon); a gunner's
/// `tag_flash_gunner<N>`, then `tag_flash_gunner<N>a` for any barrel after
/// (VEH_FireGunnerWeapon).
fn flash_tag(gunner: Option<usize>, barrel: usize) -> String {
    match gunner {
        Some(i) if barrel < 1 => format!("tag_flash_gunner{}", i + 1),
        Some(i) => format!("tag_flash_gunner{}a", i + 1),
        None => ["tag_flash", "tag_flash2", "tag_flash3", "tag_flash4"]
            .get(barrel)
            .copied()
            .unwrap_or("tag_flash")
            .to_owned(),
    }
}

/// Where a craft gun's barrel fires from: its model's flash tag for it
/// (else its first barrel's, else `tag_flash`) turned with it, else under
/// its middle.
fn muzzle_at(world: &mut World, n: u32, gunner: Option<usize>, barrel: usize) -> [f32; 3] {
    let Some((origin, angles, model)) = world
        .resource::<Zm>()
        .ents
        .get(&n)
        .map(|e| (e.origin, e.angles, e.model.clone()))
    else {
        return [0.0; 3];
    };
    let local = super::ride::tag_in_model(world, &model, &flash_tag(gunner, barrel))
        .or_else(|| super::ride::tag_in_model(world, &model, &flash_tag(gunner, 0)))
        .or_else(|| super::ride::tag_in_model(world, &model, "tag_flash"));
    let Some(l) = local else {
        return [origin[0], origin[1], origin[2] - 48.0];
    };
    let (f, r, u) = gsc_t6::math::angle_vectors(angles);
    std::array::from_fn(|i| origin[i] + f[i] * l[0] - r[i] * l[1] + u[i] * l[2])
}

/// The gun firing (`EV_FIRE_WEAPON` on the craft's own entity): its world
/// flash and fire sound play at the muzzle, as a player's do at his gun.
pub(super) fn fire_event(world: &mut World, n: u32, w: u32, start: [f32; 3], to: [f32; 3]) {
    gun_event(world, n, None, 0, w, start, to);
}

/// The event a craft gun's barrel sends firing: its turret's
/// `EV_FIRE_WEAPON` from any barrel (BO2 numbers them 58 to 61, each the
/// weapon firing at that barrel's flash; ours carries the barrel's place),
/// a gunner's own (VEH_FireGunnerWeapon: 202 plus the gunner from his first
/// flash tag, 206 plus the gunner from his second).
fn fire_kind(gunner: Option<usize>, barrel: usize) -> entity_t6::EntityEventKind {
    gunner.map_or(entity_t6::predicted_weapon_fire_event(0, false), |i| {
        entity_t6::gunner_fire_event(i, barrel >= 1)
    })
}

/// A craft's gun firing, on the craft's own entity: its flash and fire
/// sound at the muzzle. A gunner's names him (BO2's event parm, the player
/// who fired it) and carries where the craft stands (origin2), where every
/// client starts BO2's gunner shake.
fn gun_event(
    world: &mut World,
    n: u32,
    gunner: Option<usize>,
    barrel: usize,
    w: u32,
    start: [f32; 3],
    to: [f32; 3],
) {
    let zm = world.resource::<Zm>();
    let Some(number) = zm.presences.by_ent.get(&n).map(|p| p.number) else {
        return;
    };
    let at = zm.ents.get(&n).map_or(start, |e| e.origin);
    // The player in his seat (gunner `i` sits in seat `i + 1`), else the
    // craft's owner; a player's entity number is his client's.
    let shooter = match gunner {
        Some(i) => {
            let st = streaks(world);
            let seat = i as i32 + 1;
            st.rides
                .iter()
                .find(|(_, r)| r.vehicle == n && r.seat == seat)
                .map(|(c, _)| *c)
                .or_else(|| st.craft.get(&n).and_then(|c| c.owner))
        }
        None => None,
    };
    let event_parm = shooter.map_or(playerstate_t6::game::ENTITYNUM_NONE, |c| c as i32);
    let t = super::super::tick(world);
    super::super::frame(world).push_entity_event(
        t,
        crate::EventAudience::All,
        fire_kind(gunner, barrel),
        crate::EntityEventPayload {
            number,
            event_parm,
            weapon: w,
            origin: start,
            origin2: if gunner.is_some() { at } else { [0.0; 3] },
            direction: super::aim_angles(gsc_t6::math::sub(to, start)),
            ..Default::default()
        },
    );
}

pub(super) fn world_obj(world: &World, n: u32) -> Value {
    let zm = world.resource::<Zm>();
    zm.players
        .get(&n)
        .map(|p| Value::Object(p.obj))
        .or_else(|| zm.ents.get(&n).and_then(|e| e.obj).map(Value::Object))
        .unwrap_or(Value::Undefined)
}

fn angle_norm(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a > 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

/// Each tick: every craft toward its goal, turned toward what it faces.
pub(super) fn advance(world: &mut World) {
    let dt = crate::MATCH_TICK_MS as f32 / 1000.0;
    let ids: Vec<u32> = streaks(world).craft.keys().copied().collect();
    let mut events = Vec::new();
    for n in ids {
        let Some((pos, ang)) = world
            .resource::<Zm>()
            .ents
            .get(&n)
            .map(|e| (e.origin, e.angles))
        else {
            streaks(world).craft.remove(&n);
            continue;
        };
        let Some(mut c) = streaks(world).craft.get(&n).cloned() else {
            continue;
        };
        let mut pos = pos;
        // Where it flies under its sway (the sway put on last tick off).
        let sway = c.jitter.map_or([0.0; 3], |j| j.off);
        pos = std::array::from_fn(|i| pos[i] - sway[i]);
        let mut heading = None;
        if c.ground && c.goal.is_some() && c.rider.is_none() {
            // On the ground along its route, at its own speed.
            let next = c.route.first().copied().or(c.goal).unwrap_or(pos);
            let d = [next[0] - pos[0], next[1] - pos[1]];
            let dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
            c.cur = (c.cur + c.accel * dt).min(c.speed);
            let step = c.cur * dt;
            if dist <= step.max(4.0) {
                pos = [next[0], next[1], pos[2]];
                if c.route.is_empty() {
                    c.goal = None;
                    c.cur = 0.0;
                    events.push((n, "reached_end_node", Vec::new()));
                    events.push((n, "goal", Vec::new()));
                } else {
                    c.route.remove(0);
                }
            } else {
                pos = [
                    pos[0] + d[0] / dist * step,
                    pos[1] + d[1] / dist * step,
                    pos[2],
                ];
                heading = Some(d[1].atan2(d[0]).to_degrees());
            }
            // Its wheels on the ground under it.
            let up = [pos[0], pos[1], pos[2] + 24.0];
            let down = [pos[0], pos[1], pos[2] - 96.0];
            let t = super::super::frame(world).trace_static_world(
                up,
                down,
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            );
            if t.fraction < 1.0 {
                pos[2] = up[2] + (down[2] - up[2]) * t.fraction;
            }
        } else if let Some(goal) = c.goal {
            let d = [goal[0] - pos[0], goal[1] - pos[1], goal[2] - pos[2]];
            let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            // Speed up toward the wanted speed; brake to stop at the goal.
            let mut want = c.speed;
            if c.stop {
                want = want.min((2.0 * c.accel * dist).sqrt());
            }
            if c.cur < want {
                c.cur = (c.cur + c.accel * dt).min(want);
            } else {
                c.cur = (c.cur - c.accel * dt).max(want);
            }
            let step = (c.cur * dt).max(if c.stop { 2.0 } else { 0.0 });
            if c.near > 0.0 && !c.near_sent && dist <= c.near {
                c.near_sent = true;
                events.push((n, "near_goal", Vec::new()));
            }
            if dist <= step.max(1.0) {
                pos = goal;
                c.goal = None;
                if c.stop {
                    c.cur = 0.0;
                }
                events.push((n, "goal", Vec::new()));
            } else {
                pos = std::array::from_fn(|i| pos[i] + d[i] / dist * step);
                if d[0].abs() + d[1].abs() > 1.0 {
                    heading = Some(d[1].atan2(d[0]).to_degrees());
                }
            }
        }
        // What it faces: an entity it looks at, a target yaw, its goal yaw
        // once there, else where it flies.
        let look = c.look_at.and_then(|t| {
            let o = if world.resource::<Zm>().players.contains_key(&t) {
                super::super::frame(world)
                    .player(crate::world::ClientId(t))
                    .map(|ps| ps.origin)
            } else {
                world.resource::<Zm>().ents.get(&t).map(|e| e.origin)
            }?;
            Some((o[1] - pos[1]).atan2(o[0] - pos[0]).to_degrees())
        });
        let want_yaw = look
            .or(c.target_yaw)
            .or(if c.goal.is_none() { c.goal_yaw } else { None })
            .or(heading);
        let mut yaw = ang[1];
        if let Some(w) = want_yaw {
            let delta = angle_norm(w - yaw);
            let turn = c.yaw_speed * dt;
            yaw += delta.clamp(-turn, turn);
        }
        if let Some(j) = c.jitter.as_mut() {
            j.off = j.step(dt);
            pos = std::array::from_fn(|i| pos[i] + j.off[i]);
        }
        if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
            e.origin = pos;
            e.angles = [ang[0], angle_norm(yaw), ang[2]];
        }
        streaks(world).craft.insert(n, c);
    }
    notify_all(world, events);
}

/// A ground vehicle's way to `goal`: the map's path nodes from the one
/// nearest it to the one nearest the goal, then the goal (straight when
/// close and clear). None when no route joins.
fn ground_route(world: &mut World, from: [f32; 3], goal: [f32; 3]) -> Option<Vec<[f32; 3]>> {
    let d = [goal[0] - from[0], goal[1] - from[1]];
    if (d[0] * d[0] + d[1] * d[1]).sqrt() < 256.0
        && super::super::frame(world)
            .trace_static_world(
                [from[0], from[1], from[2] + 24.0],
                [goal[0], goal[1], goal[2] + 24.0],
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            )
            .fraction
            >= 1.0
    {
        return Some(vec![goal]);
    }
    let zm = world.resource::<Zm>();
    let nav = &zm.nav;
    let (a, b) = (nav.nearest(from)?, nav.nearest(goal)?);
    let nodes = nav.path(a, b)?;
    let mut route: Vec<[f32; 3]> = nodes
        .iter()
        .map(|&i| nav.nodes[i as usize].origin)
        .collect();
    route.push(goal);
    Some(route)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The VTOL's sway (amplitude 3, a new spot every 0.5 to 1.5 s) stays
    /// within its amplitude, eases (no jump of more than a little a tick)
    /// and keeps moving.
    #[test]
    fn hover_sway_stays_within_its_amplitude_and_eases() {
        let mut j = Jitter { amp: [3.0; 3], min_t: 0.5, max_t: 1.5, seed: 7, ..Default::default() };
        let dt = 0.05;
        let mut last = j.step(dt);
        let mut far = 0.0f32;
        for _ in 0..400 {
            let o = j.step(dt);
            for i in 0..3 {
                assert!(o[i].abs() <= 3.0 + 1e-4, "{o:?}");
                assert!((o[i] - last[i]).abs() < 1.0, "{last:?} -> {o:?}");
                far = far.max(o[i].abs());
            }
            assert!(j.len >= 0.5 && j.len <= 1.5, "{}", j.len);
            last = o;
        }
        assert!(far > 1.0, "it never swayed: {far}");
    }

    /// A gunner seat fires from its rider's view on any vehicle; a driver
    /// only on a tank (the AGR), not on a helicopter (the VTOL, the drone)
    /// or a car.
    #[test]
    fn gunners_and_a_tanks_driver_fire_from_their_view() {
        assert!(fires_from_view(Some(0), 6));
        assert!(fires_from_view(Some(1), 2));
        assert!(fires_from_view(None, VEH_TANK));
        assert!(!fires_from_view(None, 6));
        assert!(!fires_from_view(None, 0));
    }

    /// A craft's turret fires as a player's gun does from any barrel; each
    /// gunner sends his own (BO2's 202 plus the gunner), 206 plus the
    /// gunner from his second barrel.
    #[test]
    fn a_gunners_shot_sends_his_own_fire_event() {
        use entity_t6::EntityEventKind as K;
        assert_eq!(fire_kind(None, 0), K::FIRE_WEAPON);
        assert_eq!(fire_kind(None, 3), K::FIRE_WEAPON);
        assert_eq!(fire_kind(Some(0), 0), K::FIRE_GUNNER_1);
        assert_eq!(fire_kind(Some(1), 0), K::FIRE_GUNNER_2);
        assert_eq!(fire_kind(Some(3), 0), K::FIRE_GUNNER_4);
        assert_eq!(fire_kind(Some(0), 1), K::FIRE_GUNNER_1A);
        assert_eq!(fire_kind(Some(3), 1), K::FIRE_GUNNER_4A);
    }

    /// Each barrel fires from its own flash tag: the VTOL Warship's minigun
    /// from `tag_flash` and `tag_flash2`, its rockets from
    /// `tag_flash_gunner1` and `tag_flash_gunner1a`, the AGR's gun round
    /// `tag_flash` to `tag_flash4`.
    #[test]
    fn each_barrel_fires_from_its_own_flash_tag() {
        assert_eq!(flash_tag(None, 0), "tag_flash");
        assert_eq!(flash_tag(None, 1), "tag_flash2");
        assert_eq!(flash_tag(None, 3), "tag_flash4");
        assert_eq!(flash_tag(None, 4), "tag_flash");
        assert_eq!(flash_tag(Some(0), 0), "tag_flash_gunner1");
        assert_eq!(flash_tag(Some(0), 1), "tag_flash_gunner1a");
        assert_eq!(flash_tag(Some(2), 3), "tag_flash_gunner3a");
    }

    /// A missile flies out through the vehicle its owner rides, but not
    /// through a tank.
    #[test]
    fn a_missile_passes_its_riders_vehicle_but_a_tank() {
        assert!(passes_own_vehicle(6));
        assert!(passes_own_vehicle(0));
        assert!(!passes_own_vehicle(VEH_TANK));
    }
}
