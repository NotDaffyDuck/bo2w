//! Scorestreak turrets (`spawnturret`: the Sentry Gun, the Guardian, the
//! remote turrets).
//!
//! BO2's scripts run a turret's life: carried in front of its owner
//! (`carryturret`) until he finds a good spot (`canplayerplaceturret`) and
//! fires to set it down (`stopcarryturret`), then it fires in bursts
//! (`_mgturret::burst_fire_unmanned`: `shootturret` each 0.112 s while
//! `isfiringturret`). The engine half kept here: where a carried turret
//! sits, the enemy an automatic turret has in its sights (nearest living
//! enemy it can see; `turret_target_aquired` / `turret_target_lost`), and a
//! shot's hit on it with the turret's own weapon. A turret turns only so far
//! from the facing it was set down with (its weapon's arcs: the Sentry Gun's
//! 65 left and right, 45 up, 20 down), hunting or driven.
//!
//! With no one to shoot, a placed Sentry Gun sweeps side to side across its
//! arcs (BO2's turret scan: `turret_scan_start` and `turret_scan_stop` start
//! and stop the scripts' scanning sound).
//!
//! Its gun turns and tilts on its own bones while its legs stay put, as
//! BO2's does: the entity turns to its gun's yaw, and its picture is sent
//! its gun's pitch and the facing it was set down with (`turret_aim`), so
//! the client tilts its gun on `tag_aim_pitch` and turns its legs back to
//! that facing. Its bullets leave its tilted muzzle.
//!
//! The Sentry Gun can be driven (`remotecontrolturret`): its owner's view
//! sits on its `tag_player` (the scripts link it, kept to its arcs from
//! `getturretarclimits`), it turns to where he looks, and his trigger fires
//! it (BO2's turret_clientaim and turret_track).

use super::bare_notify;
use bevy_ecs::prelude::World;
use gsc_t6::{Array, Key, ObjKind, ScriptValue, Vm};

use super::super::{Closest, Ent, Zm, arg, entnum, frame, text};
use super::{notify_all, streaks};
use crate::sim_parts::ClientId;

/// Where its gun sits above its origin.
const EYE: f32 = 40.0;

#[derive(Clone, Debug, Default)]
pub(crate) struct Turret {
    pub gun: String,
    pub owner: Option<u32>,
    pub team: String,
    /// The player carrying it (`carryturret`).
    pub carrier: Option<u32>,
    /// `setmode`: an "auto_nonai" turret picks its own (player) targets;
    /// "auto_ai" ones (carried, stunned) none here; "manual" ones are driven
    /// by a player.
    pub setting: String,
    pub target: Option<u32>,
    /// The facing it was set down with (`stopcarryturret`): its arcs are
    /// measured from it.
    pub base_yaw: Option<f32>,
    /// The player at its controls (`remotecontrolturret`).
    pub remote: Option<u32>,
    /// When its gun may fire next (level ms) under his trigger, whether his
    /// trigger was down last tick (a single-shot gun wants a fresh press),
    /// and its heat (a gun that overheats).
    pub next_fire: i64,
    pub trigger_down: bool,
    pub heat: weapon_t6::vehicle_heat::GunHeat,
    /// `setscanningpitch`: the pitch its gun sweeps at (down as plus).
    pub scanning_pitch: f32,
    pub scan: Scan,
}

/// Its gun's idle sweep (BO2's turret scan). It waits a moment, speeds up to
/// its weapon's scan speed, slows to a stop just inside its arc, and turns
/// back the other way; a target stops it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Scan {
    /// Its gun's yaw from its facing (left as plus) and tilt (down as plus).
    pub heading: f32,
    pub tilt: f32,
    /// Degrees a second.
    pub deg_per_sec: f32,
    /// It slows once the far side is this near.
    pub slow_at: f32,
    pub pause_until: i64,
    pub on: bool,
    /// Sweeping to its right.
    pub back: bool,
}

impl Scan {
    /// It stops sweeping (a target, or the far side): it sweeps the other
    /// way next. BO2 turns it round on every stop, so a turret tracking
    /// someone turns it each think. `turret_scan_stop` when it was sweeping.
    fn stop(&mut self) -> Option<&'static str> {
        let was = self.on;
        self.on = false;
        self.back = !self.back;
        self.deg_per_sec = 0.0;
        self.slow_at = 0.0;
        self.pause_until = 0;
        was.then_some("turret_scan_stop")
    }

    /// One think of its sweep at `now` (level ms): first its gun settles to
    /// `pitch`, then it sweeps. The notifies it sends.
    fn step(
        &mut self,
        a: weapon_t6::facts::TurretFacts,
        pitch: f32,
        now: i64,
    ) -> Vec<&'static str> {
        let mut told = Vec::new();
        if a.scan_speed <= 0.0 || a.scan_accel <= 0.0 {
            return told;
        }
        let tick = THINK_MS as f32 * 0.001;
        self.tilt = track(self.tilt, pitch, a.scan_speed * tick);
        if self.tilt != pitch {
            return told;
        }
        let right = (4.0 - a.right_arc).max(-90.0);
        let left = (a.left_arc - 4.0).min(90.0);
        let (ahead, way) = if self.back {
            (self.heading - right, -1.0)
        } else {
            (left - self.heading, 1.0)
        };
        if !self.on {
            self.on = true;
            self.deg_per_sec = 0.0;
            self.pause_until = now + i64::from(a.scan_pause_ms);
            self.slow_at = (a.scan_speed * a.scan_speed / (2.0 * a.scan_accel)).min(ahead * 0.5);
            told.push("turret_scan_start");
        }
        if now < self.pause_until {
            return told;
        }
        if self.slow_at <= ahead {
            self.deg_per_sec = (self.deg_per_sec + a.scan_accel * tick).min(a.scan_speed);
        } else {
            self.deg_per_sec -= a.scan_accel * tick;
            if self.deg_per_sec <= 0.0 {
                told.extend(self.stop());
                return told;
            }
        }
        self.heading += self.deg_per_sec * tick * way;
        told
    }
}

/// `from` turned toward `to` by at most `step` degrees, the short way.
fn track(from: f32, to: f32, step: f32) -> f32 {
    let d = wrap(to - from);
    if d.abs() <= step {
        to
    } else {
        from + step.copysign(d)
    }
}

/// BO2's turret thinks every 50 ms: its fire time runs down by whole thinks.
const THINK_MS: i32 = 50;

/// The turret a turret method was called on, else BO2's error.
fn turret_ent(vm: &Vm<World>, game: &mut World, st: &ScriptValue) -> Result<u32, String> {
    let n = entnum(vm, st).ok_or("not an entity")?;
    if streaks(game).turrets.contains_key(&n) {
        return Ok(n);
    }
    let cls = game
        .resource::<Zm>()
        .ents
        .get(&n)
        .map(|e| e.class_tag.clone())
        .unwrap_or_default();
    Err(format!("entity type '{cls}' is not a turret"))
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
    // `spawnturret(classname, spot, weapon)`. It is solid as BO2's
    // `G_SpawnTurret` makes it (contents 0x200004), so a closing door's
    // `gettouchingvolume` finds it and crushes it (Vertigo's, Drone's).
    f!("spawnturret", |vm, world, _, a| {
        let cls = text(vm, a, 0);
        let spot = super::super::vec3(a, 1)?;
        let weapon = text(vm, a, 2);
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        world.resource_mut::<Zm>().ents.insert(
            n,
            Ent {
                obj: Some(obj),
                class_tag: cls,
                origin: spot,
                can_damage: true,
                solid: true,
                ..Default::default()
            },
        );
        streaks(world).turrets.insert(
            n,
            Turret {
                gun: weapon,
                setting: "auto_ai".to_owned(),
                ..Default::default()
            },
        );
        Ok(ScriptValue::Object(obj))
    });
    fn with_turret(vm: &Vm<World>, game: &mut World, st: &ScriptValue, fl: impl FnOnce(&mut Turret)) {
        if let Some(n) = entnum(vm, st)
            && let Some(t) = streaks(game).turrets.get_mut(&n)
        {
            fl(t);
        }
    }
    m!("setturretowner", |vm, world, s, a| {
        let o = entnum(vm, arg(a, 0));
        with_turret(vm, world, s, |t| t.owner = o);
        Ok(ScriptValue::Undefined)
    });
    m!("setturretteam", |vm, world, s, a| {
        let team = text(vm, a, 0);
        with_turret(vm, world, s, |t| t.team = team);
        Ok(ScriptValue::Undefined)
    });
    m!("setmode", |vm, world, s, a| {
        let mode = text(vm, a, 0);
        with_turret(vm, world, s, |t| t.setting = mode);
        Ok(ScriptValue::Undefined)
    });
    m!("settargetentity", |vm, world, s, a| {
        let target = entnum(vm, arg(a, 0));
        with_turret(vm, world, s, |t| t.target = target);
        Ok(ScriptValue::Undefined)
    });
    m!("cleartargetentity", |vm, world, s, _| {
        with_turret(vm, world, s, |t| t.target = None);
        Ok(ScriptValue::Undefined)
    });
    m!("getturrettarget", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let t = streaks(world).turrets.get(&n).and_then(|t| t.target);
        Ok(t.map_or(ScriptValue::Undefined, |t| super::craft::world_obj(world, t)))
    });
    // A player carries it in front of him.
    m!("carryturret", |vm, world, s, a| {
        let carrier = entnum(vm, s);
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        if let Some(t) = streaks(world).turrets.get_mut(&n) {
            t.carrier = carrier;
            t.target = None;
            t.base_yaw = None;
            t.scan = Scan::default();
        }
        Ok(ScriptValue::Undefined)
    });
    m!("stopcarryturret", |vm, world, _, a| {
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        let spot = arg(a, 1).as_vec3();
        let angles = arg(a, 2).as_vec3();
        let mut facing = None;
        if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
            if let Some(o) = spot {
                e.origin = o;
            }
            if let Some(ang) = angles {
                e.rot = ang;
            }
            facing = Some(e.rot[1]);
        }
        if let Some(t) = streaks(world).turrets.get_mut(&n) {
            t.carrier = None;
            t.base_yaw = facing;
        }
        Ok(ScriptValue::Undefined)
    });
    // Its turn limits, as the scripts link a driver's view to them: up and
    // right as minus ("arc_min_pitch", "arc_min_yaw"), down and left as plus
    // ("arc_max_pitch", "arc_max_yaw").
    m!("getturretarclimits", |vm, world, s, _| {
        let n = turret_ent(vm, world, s)?;
        let weapon = streaks(world).turrets.get(&n).map(|t| t.gun.clone()).unwrap_or_default();
        let a = arcs(world, &weapon).unwrap_or_default();
        let mut arr = Array::new();
        for (k, v) in [
            ("arc_min_pitch", (-a.top_arc).min(0.0)),
            ("arc_max_pitch", a.bottom_arc.max(0.0)),
            ("arc_min_yaw", (-a.right_arc).min(0.0)),
            ("arc_max_yaw", a.left_arc.max(0.0)),
        ] {
            let key = Key::Str(vm.intern(k));
            arr.set(key, ScriptValue::Float(v));
        }
        Ok(ScriptValue::array(arr))
    });
    // `player remotecontrolturret(turret)`: he drives it (his view is the
    // scripts' to link).
    m!("remotecontrolturret", |vm, world, s, a| {
        let id = super::super::client(vm, world, s)?;
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        if let Some(t) = streaks(world).turrets.get_mut(&n) {
            t.remote = Some(id.0);
        }
        Ok(ScriptValue::Undefined)
    });
    // He lets go of it: only the player at it (or its owner) may.
    m!("remotecontrolturretoff", |vm, world, s, a| {
        let id = super::super::client(vm, world, s)?;
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        let mut st = streaks(world);
        let Some(t) = st.turrets.get_mut(&n) else {
            return Ok(ScriptValue::Undefined);
        };
        if t.remote != Some(id.0) && t.owner != Some(id.0) {
            return Err(
                "Calling StopRemoteUse on a turret that's not owned by that player.".to_owned(),
            );
        }
        t.remote = None;
        Ok(ScriptValue::Undefined)
    });
    // Where a carried turret (or a drone, `canplayerplacevehicle(width,
    // length, distance, ...)`) would stand: on the ground that far in front
    // of him, facing his way; good when the ground is there and level.
    m!("canplayerplaceturret", |vm, world, s, _| place_ahead(
        vm, world, s, 40.0
    ));
    m!("canplayerplacevehicle", |vm, world, s, a| {
        let dist = arg(a, 2).as_float().unwrap_or(50.0);
        place_ahead(vm, world, s, dist)
    });
    // It has an enemy in its sights.
    m!("isfiringturret", |vm, world, s, _| {
        let n = turret_ent(vm, world, s)?;
        Ok(ScriptValue::bool(streaks(world).turrets.get(&n).is_some_and(
            |t| t.carrier.is_none() && t.target.is_some(),
        )))
    });
    // One shot at its target: the turret weapon's damage (less with range)
    // when the shot can reach him.
    m!("shootturret", |vm, world, s, _| {
        let n = turret_ent(vm, world, s)?;
        let Some(t) = streaks(world).turrets.get(&n).cloned() else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(target) = t.target else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(spot) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else {
            return Ok(ScriptValue::Undefined);
        };
        let eye_at = [spot[0], spot[1], spot[2] + EYE];
        let Some(pstate) = frame(world).avatar(ClientId(target)).copied() else {
            return Ok(ScriptValue::Undefined);
        };
        let centre = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + 40.0];
        if pstate.health <= 0 || !sees(world, eye_at, centre) {
            return Ok(ScriptValue::Undefined);
        }
        let w = super::super::weapon(world, &t.gun).unwrap_or(0);
        // Fire_Lead: its flash and sound, then its bullet from its gun's
        // muzzle toward him, with the weapon's aiSpread, traced as a
        // player's is through whatever stands in the way. The turret is the
        // attacker: the scripts credit its owner.
        let muzzle = gun_muzzle(world, n, t.scan.tilt);
        super::craft::fire_event(world, n, w, muzzle, centre);
        let dir = gsc_t6::math::normalize(gsc_t6::math::sub(centre, muzzle));
        super::bullet::fire(
            vm,
            world,
            &super::bullet::Gun {
                ent: n,
                shooter: None,
                owner: t.owner,
                weapon: &t.gun,
                w,
            },
            muzzle,
            dir,
        );
        Ok(ScriptValue::Undefined)
    });
    // The pitch its gun sweeps at (down as plus).
    m!("setscanningpitch", |vm, world, s, a| {
        let pitch = arg(a, 0).as_float().unwrap_or(0.0);
        with_turret(vm, world, s, |t| t.scanning_pitch = pitch);
        Ok(ScriptValue::Undefined)
    });
    // Its look: type, minimap mark, laser, use prompt, being carried.
    for native in [
        "setturrettype",
        "setturretminimapvisible",
        "maketurretusable",
        "maketurretunusable",
        "setturretcarried",
        "laseron",
        "laseroff",
        "setturretspinning",
        "setturretfiretime",
    ] {
        vm.bind(native, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // Burst fire's own switch (`startfiring`, `stopfiring`) and its firing
    // look (`stopshootturret`): only `_mgturret::burst_fire` uses them, which
    // no MP script calls, and the flag they set has no reader in MP.
    for name in ["startfiring", "stopfiring", "stopshootturret"] {
        vm.bind(name, true, |vm, world, s, _| {
            turret_ent(vm, world, s)?;
            Ok(ScriptValue::Undefined)
        });
    }
    // `player relinktoturret(turret)`: his view goes back onto a TOW turret
    // he is using (`_weapons`' turretownerchange, turret_tow_fire,
    // turret_tow_unlink). Only a map's usable TOW turret sends those, and no
    // map turret is usable here.
    m!("relinktoturret", |vm, world, s, a| {
        super::super::client(vm, world, s)?;
        entnum(vm, arg(a, 0)).ok_or("not an entity")?;
        Ok(ScriptValue::Undefined)
    });
}

fn place_ahead(
    vm: &mut Vm<World>,
    sim_world: &mut World,
    s: &ScriptValue,
    dist: f32,
) -> Result<ScriptValue, String> {
    let id = super::super::client(vm, sim_world, s)?;
    let Some(ps) = frame(sim_world).avatar(id).copied() else {
        return Ok(ScriptValue::Undefined);
    };
    let yaw = ps.look_angles[1];
    let (sy, cy) = yaw.to_radians().sin_cos();
    let ahead = [
        ps.origin[0] + cy * dist,
        ps.origin[1] + sy * dist,
        ps.origin[2] + 32.0,
    ];
    let down = [ahead[0], ahead[1], ahead[2] - 96.0];
    let t = frame(sim_world).trace_static_world(
        ahead,
        down,
        [0.0; 3],
        [0.0; 3],
        crate::shot_trace::MASK_SHOT,
    );
    let f = t.fraction.clamp(0.0, 1.0);
    let at: [f32; 3] = std::array::from_fn(|i| ahead[i] + (down[i] - ahead[i]) * f);
    let good = f < 1.0 && t.normal[2] > 0.7;
    let mut arr = Array::new();
    let k = |vm: &mut Vm<World>, s: &str| Key::Str(vm.intern(s));
    arr.set(k(vm, "origin"), ScriptValue::Vec3(at));
    arr.set(k(vm, "angles"), ScriptValue::Vec3([0.0, yaw, 0.0]));
    arr.set(k(vm, "result"), ScriptValue::Int(i32::from(good)));
    Ok(ScriptValue::array(arr))
}

/// Its weapon's turn limits; none on content without Black Ops II weapon
/// facts (it turns all the way round).
fn arcs(world: &mut World, weapon: &str) -> Option<weapon_t6::facts::TurretFacts> {
    let w = super::super::weapon(world, weapon).ok().filter(|w| *w != 0)?;
    let f = frame(world);
    (!f.gun_facts().is_empty()).then(|| f.gun_facts().get(w).turret)
}

/// `a` in degrees, -180..180.
fn wrap(a: f32) -> f32 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// Where its gun points for a view (pitch down as plus, yaw), kept to its
/// arcs around its facing (turret_clientaim).
fn kept_to_arcs(
    view: [f32; 2],
    base: f32,
    a: Option<weapon_t6::facts::TurretFacts>,
) -> [f32; 2] {
    let mut pitch = wrap(view[0]);
    let mut yaw = wrap(view[1] - base);
    if let Some(a) = a {
        pitch = pitch.clamp(-a.top_arc.max(0.0), a.bottom_arc.max(0.0));
        yaw = yaw.clamp(-a.right_arc.max(0.0), a.left_arc.max(0.0));
    }
    [pitch, wrap(base + yaw)]
}

/// Whether a point is inside its arcs from its facing (an automatic turret
/// hunts nothing it could not turn to).
fn in_arcs(eye: [f32; 3], at: [f32; 3], base: f32, a: weapon_t6::facts::TurretFacts) -> bool {
    let d = gsc_t6::math::sub(at, eye);
    let yaw = wrap(d[1].atan2(d[0]).to_degrees() - base);
    let pitch = -d[2].atan2(d[0].hypot(d[1])).to_degrees();
    (-a.right_arc..=a.left_arc).contains(&yaw) && (-a.top_arc..=a.bottom_arc).contains(&pitch)
}

/// A point on its gun in model space (`at`) with its gun tilted `pitch`
/// (down as plus) about its pitch bone at `pivot` (BO2 tilts
/// `tag_aim_pitch`, which carries its muzzle).
fn tilted(at: [f32; 3], pivot: [f32; 3], pitch: f32) -> [f32; 3] {
    let (s, c) = pitch.to_radians().sin_cos();
    let [x, y, z] = gsc_t6::math::sub(at, pivot);
    [pivot[0] + x * c + z * s, pivot[1] + y, pivot[2] - x * s + z * c]
}

/// Its gun's muzzle (`tag_flash`) in the world with its gun tilted `pitch`
/// (down as plus); the entity is turned to its gun's yaw.
fn gun_muzzle(world: &mut World, n: u32, pitch: f32) -> [f32; 3] {
    let at = world
        .resource::<Zm>()
        .ents
        .get(&n)
        .map(|e| (e.origin, e.rot, e.mesh.clone()));
    let Some((spot, rot, mdl)) = at else {
        return super::craft::muzzle(world, n, None);
    };
    let Some(flash) = super::ride::tag_in_model(world, &mdl, "tag_flash") else {
        return super::craft::muzzle(world, n, None);
    };
    let pivot = match super::ride::tag_in_model(world, &mdl, "tag_aim_pitch") {
        Some(p) => p,
        None => super::ride::tag_in_model(world, &mdl, "tag_aim").unwrap_or([0.0; 3]),
    };
    let l = tilted(flash, pivot, pitch);
    let (f, r, u) = gsc_t6::math::facing_axes(rot);
    std::array::from_fn(|i| spot[i] + f[i] * l[0] - r[i] * l[1] + u[i] * l[2])
}

/// Its gun's aim for its picture: its pitch (down as plus) and the facing
/// it was set down with, its legs' (none: drawn as modelled, as carried).
fn show_aim(world: &mut World, n: u32, aim: Option<[f32; 2]>) {
    let shown_id = world.resource::<Zm>().presences.by_ent.get(&n).map(|s| s.id);
    let Some(id) = shown_id else {
        return;
    };
    let mut f = frame(world);
    if let Some(dobj) = f.collision_owner_mut(id).and_then(|r| r.dobj.as_mut()) {
        dobj.bone_role_state.turret_aim = aim;
    }
}

/// His buttons held this tick (his commands').
fn held(game: &World, cid: ClientId) -> u32 {
    game
        .resource::<crate::frame_step::FrameTicket>()
        .input
        .cmds
        .iter()
        .filter(|(ch, _)| *ch == cid)
        .fold(0, |b, (_, cmd)| b | cmd.buttons)
}

/// A driven turret this tick: it turns to where he looks, kept to its arcs,
/// and while his trigger is held fires along its gun each time its fire
/// time has run out (a single-shot gun only on a fresh press), none while
/// it is overheated (turret_track).
fn driven(world: &mut World, n: u32, t: &mut Turret, p: u32) {
    let Some(ps) = frame(world).avatar(ClientId(p)).copied() else {
        return;
    };
    let base = t
        .base_yaw
        .or_else(|| world.resource::<Zm>().ents.get(&n).map(|e| e.rot[1]))
        .unwrap_or(0.0);
    let [pitch, yaw] = kept_to_arcs([ps.look_angles[0], ps.look_angles[1]], base, arcs(world, &t.gun));
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
        e.rot = [0.0, yaw, 0.0];
    }
    t.scan.heading = wrap(yaw - base);
    t.scan.tilt = pitch;
    let down = held(world, ClientId(p)) & playerstate_t6::game::buttons::ATTACK != 0;
    let fresh = !t.trigger_down;
    t.trigger_down = down;
    let w = super::super::weapon(world, &t.gun).unwrap_or(0);
    if w == 0 {
        return;
    }
    let (single, fire) = {
        let f = frame(world);
        let g = f.gun_facts().get(w);
        (g.core.fire_type == weapon_t6::facts::fire_type::SINGLESHOT, g.fire)
    };
    t.heat.step(&fire, down, crate::SIM_STEP_MILLIS as i32);
    let now = world.resource::<Zm>().clock_millis;
    if !down || (single && !fresh) || !t.heat.can_fire() || now < t.next_fire {
        return;
    }
    let fire_gap = frame(world).damage_profile_of(w).map_or(100, |f| f.shot_gap_ms.max(1));
    t.next_fire = now + i64::from((fire_gap + THINK_MS - 1) / THINK_MS * THINK_MS);
    let muzzle = gun_muzzle(world, n, pitch);
    let (dir, _, _) = gsc_t6::math::facing_axes([pitch, yaw, 0.0]);
    let to: [f32; 3] = std::array::from_fn(|i| muzzle[i] + dir[i] * 8192.0);
    super::craft::fire_event(world, n, w, muzzle, to);
    let owner = t.owner.or(Some(p));
    let weapon = t.gun.clone();
    super::super::with_vm(world, |vm, world| {
        super::bullet::fire(
            vm,
            world,
            &super::bullet::Gun {
                ent: n,
                shooter: Some(p),
                owner,
                weapon: &weapon,
                w,
            },
            muzzle,
            dir,
        );
    });
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Its sight (BO2's `turret_SightTrace`): smoke between hides a target
/// from it as a wall does.
fn sees(game: &mut World, from: [f32; 3], goal: [f32; 3]) -> bool {
    !smoke_hides(game, from, goal)
        && frame(game)
        .trace_static_world(
            from,
            goal,
            [0.0; 3],
            [0.0; 3],
            crate::shot_trace::MASK_SHOT,
        )
        .fraction
        >= 1.0
}

/// Is the line hidden by smoke (`fxblocksight`)? A turret's sight goes
/// when it is under a fifth visible, as BO2's does.
fn smoke_hides(game: &World, from: [f32; 3], goal: [f32; 3]) -> bool {
    super::super::natives_fx::fx_visibility(game, from, goal) < 0.2
}

/// Each tick: carried turrets follow their carrier; a driven one turns to
/// its driver's view; automatic ones pick the nearest living enemy they can
/// see (within 2400 units, inside their arcs) and turn to him, or sweep
/// their arcs when there is none.
pub(super) fn advance(world: &mut World) {
    let ids: Vec<u32> = streaks(world).turrets.keys().copied().collect();
    if ids.first().is_none() {
        return;
    }
    let teams: Vec<(u32, String)> = super::super::with_vm(world, |vm, world| {
        let f = vm.intern("team");
        world
            .resource::<Zm>()
            .players
            .iter()
            .map(|(c, p)| (*c, vm.to_text(&vm.raw_field(p.obj, f))))
            .collect()
    })
    .unwrap_or_default();
    let mut events = Vec::default();
    for nn in ids {
        let Some(spot) = world.resource::<Zm>().ents.get(&nn).map(|en| en.origin) else {
            streaks(world).turrets.remove(&nn);
            continue;
        };
        let Some(mut t) = streaks(world).turrets.get(&nn).cloned() else {
            continue;
        };
        if let Some(c) = t.carrier {
            if let Some(pstate) = frame(world).avatar(ClientId(c)).copied() {
                let (sy, cy) = pstate.look_angles[1].to_radians().sin_cos();
                if let Some(en) = world.resource_mut::<Zm>().ents.get_mut(&nn) {
                    let [px, py, pz] = pstate.origin;
                    en.origin = [px + cy * 40.0, py + sy * 40.0, pz];
                    en.rot = [0.0, pstate.look_angles[1], 0.0];
                }
            }
            show_aim(world, nn, None);
            continue;
        }
        if t.base_yaw.is_none() {
            t.base_yaw = world.resource::<Zm>().ents.get(&nn).map(|en| en.rot[1]);
        }
        if let Some(p) = t.remote {
            if t.target.take().is_some() {
                events.push(bare_notify(nn, "turretstatechange"));
                events.push(bare_notify(nn, "turret_target_lost"));
            }
            driven(world, nn, &mut t, p);
            show_aim(world, nn, Some([t.scan.tilt, t.base_yaw.unwrap_or(0.0)]));
            streaks(world).turrets.insert(nn, t);
            continue;
        }
        // Only a placed gun turret hunts players: "auto_nonai" (BO2 puts a
        // carried or stunned one in "auto_ai", which no player draws). The
        // Guardian's microwave never turns: its cone is where it was set
        // down facing.
        if t.setting != "auto_nonai" || t.gun.starts_with("microwave_turret") {
            if t.target.take().is_some() {
                events.push(bare_notify(nn, "turretstatechange"));
                events.push(bare_notify(nn, "turret_target_lost"));
                streaks(world).turrets.insert(nn, t);
            }
            continue;
        }
        let eye_at = [spot[0], spot[1], spot[2] + EYE];
        let lim = arcs(world, &t.gun).zip(t.base_yaw);
        let mut best = Closest::new();
        for (c, team) in &teams {
            if Some(*c) == t.owner || (!t.team.is_empty() && t.team != "free" && *team == t.team) {
                continue;
            }
            let Some(pstate) = frame(world).avatar(ClientId(*c)).copied() else {
                continue;
            };
            let breathing = pstate.health > 0;
            if !breathing {
                continue;
            }
            let centre = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + 40.0];
            let dd = dist(eye_at, centre);
            if dd > 2400.0
                || !best.beaten_by(dd)
                || lim.is_some_and(|(a, base)| !in_arcs(eye_at, centre, base, a))
                || !sees(world, eye_at, centre)
            {
                continue;
            }
            best.offer(dd, *c);
        }
        let now = best.into_item();
        // Its firing state changes with its target (the scripts' burst
        // loop waits for `turretstatechange`).
        if now.is_some() != t.target.is_some() {
            events.push(bare_notify(nn, "turretstatechange"));
        }
        if now != t.target {
            if let Some(c) = now {
                let who = super::craft::world_obj(world, c);
                events.push((nn, "turret_target_aquired", vec![who]));
            } else {
                events.push(bare_notify(nn, "turret_target_lost"));
            }
            t.target = now;
        }
        let mut aim_yaw = None;
        if let Some(c) = now
            && let Some(pstate) = frame(world).avatar(ClientId(c)).copied()
        {
            let dd = gsc_t6::math::sub([pstate.origin[0], pstate.origin[1], pstate.origin[2] + 40.0], eye_at);
            let y = dd[1].atan2(dd[0]).to_degrees();
            events.extend(t.scan.stop().map(|en| (nn, en, Vec::new())));
            t.scan.heading = wrap(y - t.base_yaw.unwrap_or(0.0));
            t.scan.tilt = -dd[2].atan2(dd[0].hypot(dd[1])).to_degrees();
            aim_yaw = Some(y);
        } else if now.is_none()
            && let Some((a, base)) = lim
        {
            let at = world.resource::<Zm>().clock_millis;
            let pitch = t.scanning_pitch;
            events.extend(t.scan.step(a, pitch, at).into_iter().map(|en| (nn, en, Vec::new())));
            aim_yaw = Some(wrap(base + t.scan.heading));
        }
        if let Some(face) = aim_yaw {
            if let Some(en) = world.resource_mut::<Zm>().ents.get_mut(&nn) {
                en.rot = [0.0, face, 0.0];
            }
            show_aim(world, nn, Some([t.scan.tilt, t.base_yaw.unwrap_or(0.0)]));
        }
        streaks(world).turrets.insert(nn, t);
    }
    notify_all(world, events);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// auto_gun_turret_mp's (the Sentry Gun's).
    const SENTRY: weapon_t6::facts::TurretFacts = weapon_t6::facts::TurretFacts {
        left_arc: 65.0,
        right_arc: 65.0,
        top_arc: 45.0,
        bottom_arc: 20.0,
        scan_speed: 60.0,
        scan_accel: 40.0,
        scan_pause_ms: 0,
        barrel_type: 0,
    };

    #[test]
    fn a_driven_sentry_turns_only_as_far_as_its_arcs() {
        // Set down facing north (90): he looks west (180), 90 to its left;
        // it stops at 65.
        assert_eq!(kept_to_arcs([0.0, 180.0], 90.0, Some(SENTRY)), [0.0, 155.0]);
        // Far down and to the right: 20 down, 65 right.
        assert_eq!(kept_to_arcs([60.0, 0.0], 90.0, Some(SENTRY)), [20.0, 25.0]);
        // Inside them it goes where he looks; the wrap round 180 holds.
        assert_eq!(kept_to_arcs([-10.0, -170.0], 170.0, Some(SENTRY)), [-10.0, -170.0]);
        // Without Black Ops II facts it turns all the way round.
        assert_eq!(kept_to_arcs([0.0, -90.0], 90.0, None), [0.0, -90.0]);
    }

    #[test]
    fn a_hunting_sentry_sees_no_one_behind_it() {
        let eye = [0.0, 0.0, 40.0];
        // Facing east: a player 500 ahead and 300 to the left is in.
        assert!(in_arcs(eye, [500.0, 300.0, 40.0], 0.0, SENTRY));
        // Straight behind is out, and so is one far below it.
        assert!(!in_arcs(eye, [-500.0, 0.0, 40.0], 0.0, SENTRY));
        assert!(!in_arcs(eye, [100.0, 0.0, -400.0], 0.0, SENTRY));
    }

    /// Sweeps from its facing until it stops; the yaws it passed, the notifies
    /// and the time it took.
    struct Swept {
        yaws: Vec<f32>,
        told: Vec<&'static str>,
        end: i64,
    }

    fn sweep(scan: &mut Scan, a: weapon_t6::facts::TurretFacts, from: i64) -> Swept {
        let mut out = Swept { yaws: Vec::new(), told: Vec::new(), end: from };
        loop {
            out.told.extend(scan.step(a, 0.0, out.end));
            out.yaws.push(scan.heading);
            out.end += i64::from(THINK_MS);
            if !scan.on {
                return out;
            }
            assert!(out.end < from + 60_000, "it never stopped");
        }
    }

    #[test]
    fn an_idle_sentry_sweeps_its_arcs_and_turns_back() {
        let mut scan = Scan::default();
        let Swept { yaws, told, end } = sweep(&mut scan, SENTRY, 0);
        assert_eq!(told, ["turret_scan_start", "turret_scan_stop"]);
        // It speeds up no faster than its weapon lets it, and stops near its
        // left arc: it aims 4 inside it, and its braking may carry it a
        // little past that.
        assert!(yaws.windows(2).all(|w| w[1] >= w[0]));
        assert!(yaws.windows(2).all(|w| w[1] - w[0] <= 60.0 * 0.05 + 1e-4));
        let top = *yaws.last().unwrap();
        assert!(top > 55.0 && top <= 65.0, "{top}");
        // Then the other way, to near its right arc.
        assert!(scan.back);
        let Swept { yaws, told, .. } = sweep(&mut scan, SENTRY, end);
        assert_eq!(told, ["turret_scan_start", "turret_scan_stop"]);
        assert!(yaws.windows(2).all(|w| w[1] <= w[0]));
        let low = *yaws.last().unwrap();
        assert!(low < -55.0 && low >= -65.0, "{low}");
        assert!(!scan.back);
    }

    #[test]
    fn a_turret_with_a_pause_waits_before_each_sweep() {
        let a = weapon_t6::facts::TurretFacts { scan_pause_ms: 100, ..SENTRY };
        let mut scan = Scan::default();
        let Swept { yaws, .. } = sweep(&mut scan, a, 0);
        assert_eq!(yaws[..2], [0.0, 0.0]);
        assert!((yaws[2] - 0.1).abs() < 1e-5, "{}", yaws[2]);
    }

    #[test]
    fn a_sentry_sweeps_only_once_its_gun_is_at_its_scanning_pitch() {
        let mut scan = Scan { tilt: 20.0, ..Default::default() };
        // 60 a second: 3 degrees a think, so 20 takes 7 thinks.
        for now in 0..6 {
            assert!(scan.step(SENTRY, 0.0, now * 50).is_empty());
        }
        assert_eq!(scan.tilt, 2.0);
        assert_eq!(scan.step(SENTRY, 0.0, 300), ["turret_scan_start"]);
        assert_eq!(scan.tilt, 0.0);
    }

    #[test]
    fn a_target_stops_the_sweep() {
        let mut scan = Scan::default();
        scan.step(SENTRY, 0.0, 0);
        assert_eq!(scan.stop(), Some("turret_scan_stop"));
        assert!(!scan.on && scan.back && scan.deg_per_sec == 0.0);
        // Not sweeping: no notify, but it still turns round.
        assert_eq!(scan.stop(), None);
        assert!(!scan.back);
    }

    #[test]
    fn a_turret_without_a_scan_speed_holds_still() {
        let still = weapon_t6::facts::TurretFacts { scan_speed: 0.0, ..SENTRY };
        let mut scan = Scan::default();
        assert!(scan.step(still, 0.0, 0).is_empty());
        assert_eq!(scan, Scan::default());
    }

    /// Its muzzle tilts with its gun about its pitch bone: level, where it
    /// was modelled; tilted down, lower; up, higher; always as far out.
    #[test]
    fn muzzle_tilts_about_its_pitch_bone() {
        // t6_wpn_turret_sentry_gun's tag_flash and tag_aim_pitch.
        let flash = [41.97, 0.0, 54.76];
        let pivot = [0.0, 0.0, 50.70];
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3);
        assert!(near(tilted(flash, pivot, 0.0), flash));
        let reach = |p: [f32; 3]| {
            let d = gsc_t6::math::sub(p, pivot);
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        };
        let down = tilted(flash, pivot, 20.0);
        assert!(down[2] < flash[2] - 10.0, "{down:?}");
        assert!((reach(down) - reach(flash)).abs() < 1e-3);
        let up = tilted(flash, pivot, -20.0);
        assert!(up[2] > flash[2] + 10.0, "{up:?}");
        assert!((reach(up) - reach(flash)).abs() < 1e-3);
    }

    /// Smoke between it and a player hides him; smoke off to the side, or
    /// none, does not.
    #[test]
    fn smoke_hides_a_player_from_a_sentry() {
        let mut game = World::new();
        game.insert_resource(Zm { mp: true, ..Default::default() });
        let (eye, him) = ([0.0, 0.0, 40.0], [800.0, 0.0, 40.0]);
        assert!(!smoke_hides(&game, eye, him));
        {
            let mut zm = game.resource_mut::<Zm>();
            zm.ents.insert(9, Ent { origin: [400.0, 30.0, 40.0], ..Default::default() });
            zm.vis_blockers.push((9, 64.0));
        }
        assert!(smoke_hides(&game, eye, him));
        game.resource_mut::<Zm>().ents.get_mut(&9).unwrap().origin = [400.0, 200.0, 40.0];
        assert!(!smoke_hides(&game, eye, him));
    }
}
