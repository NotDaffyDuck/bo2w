//! bo2mp: the AGR's client script (clientscripts/mp/_ai_tank.csc): its
//! scanner light, green to its side and red to the rest (none to whoever
//! drives it); the stun and its shutdown sound while an EMP or a shock holds
//! it (client flag 3); its lights out as it dies; its rocket pods' firing and
//! reload animations; the dirt or dust its treads throw and its driver's
//! rumble while it moves.
//!
//! Not ported: `rebooting` and the hack demo's `ai_tank_hack_spawned`
//! (multiplayer's scripts never set those two fields), and the lights the
//! killcam's own copy shows.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, Value};

use super::{Pending, Step, Zm, rc_sides};
use crate::EventAudience;
use crate::world::ClientId;

/// Its vehicle type (`_callbacks.csc` threads `spawned` for it).
pub(super) const VEHICLE: &str = "ai_tank_drone_mp";
const LIGHT_FRIEND: &str = "light/fx_vlight_talon_eye_grn";
const LIGHT_FOE: &str = "light/fx_vlight_talon_eye_red";
const LIGHT_TAG: &str = "tag_scanner";
const STUN: &str = "weapon/talon/fx_talon_emp_stun";
const STUN_SOUND: &str = "veh_talon_shutdown";
const TREAD_DIRT: &str = "vehicle/treadfx/fx_treadfx_talon_dirt";
const TREAD_CONCRETE: &str = "vehicle/treadfx/fx_treadfx_talon_concrete";
/// Its client flag for `tank_stun`.
pub(super) const STUN_FLAG: i32 = 3;
/// `getspeed()` from which its treads throw dirt and its driver feels it.
const MOVING: f32 = 40.0;
/// `play_driving_fx`: a puff each half second while it moves, else it looks
/// again in a tenth.
const TREAD_WAIT_MS: i64 = 500;
const TREAD_LOOK_MS: i64 = 100;
/// `play_driving_rumble`: `earthquake( 0.1, 0.1, origin, 200 )` each 0.05 s.
const RUMBLE_SCALE: f32 = 0.1;
const RUMBLE_SECS: f32 = 0.1;
const RUMBLE_RADIUS: f32 = 200.0;
/// Each snapshot (`waitforclient` in `play_light_fx`'s loop).
const TICK_MS: i64 = 50;
/// The surfaces (Black Ops II's surface type numbers) its treads throw dirt
/// on: dirt, foliage, grass, gravel, mud, sand and water; concrete dust on
/// the rest.
const DIRTY: [u32; 7] = [6, 8, 10, 11, 14, 18, 20];
const SURF_TYPE_SHIFT: u32 = 20;
const SURF_TYPE_BITS: u32 = 0x3f;

/// One AGR as its client script keeps it.
#[derive(Clone, Debug)]
pub(super) struct Tank {
    obj: ObjRef,
    /// Who sees which of its lights now.
    lit: Vec<(u32, &'static str)>,
    /// `light_disable`: stunned (flag 3) or dead (`ai_tank_death`).
    stunned: bool,
    dead: bool,
    /// `ai_tank_missile_fire` as its client last saw it (a field's
    /// callback runs when its value changes).
    missiles: i64,
    next_tread: i64,
}

/// `spawned`: its lights, tread dirt and driver's rumble from its first
/// showing until it is gone.
pub(super) fn spawned(world: &mut World, n: u32) {
    let Some(obj) = world.resource::<Zm>().ents.get(&n).and_then(|e| e.obj) else {
        return;
    };
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    zm.client_fx.pending.retain(|p| p.ent != n || !matches!(p.step, Step::Tank(_)));
    zm.client_fx.pending.push(Pending {
        due: now,
        until: i64::MAX,
        ent: n,
        step: Step::Tank(Box::new(Tank {
            obj,
            lit: Vec::new(),
            stunned: false,
            dead: false,
            missiles: 0,
            next_tread: now,
        })),
    });
}

/// Runs `f` on AGR `n`'s state, if it has one.
fn with_tank(world: &mut World, n: u32, f: impl FnOnce(&mut World, u32, &mut Tank)) {
    let p = {
        let mut zm = world.resource_mut::<Zm>();
        let pending = &mut zm.client_fx.pending;
        let Some(i) = pending.iter().position(|p| p.ent == n && matches!(p.step, Step::Tank(_))) else {
            return;
        };
        pending.swap_remove(i)
    };
    let Pending { due, until, ent, step: Step::Tank(mut t) } = p else { return };
    f(world, n, &mut t);
    world.resource_mut::<Zm>().client_fx.pending.push(Pending { due, until, ent, step: Step::Tank(t) });
}

/// `stop_light_fx`: every light it shows, taken off.
fn lights_off(world: &mut World, n: u32, t: &mut Tank) {
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for (c, fx) in std::mem::take(&mut t.lit) {
        if players.contains(&c) {
            super::super::natives_fx::bolt_effect_to(world, EventAudience::Client(ClientId(c)), n, fx, LIGHT_TAG, true);
        }
    }
}

/// `tank_stun`: set, its lights go out (`light_disable`) and the stun
/// shows on `tag_origin` with the shutdown sound; cleared, the stun goes and
/// `play_light_fx` lights it again.
pub(super) fn stun(world: &mut World, n: u32, on: bool) {
    with_tank(world, n, |world, n, t| {
        super::super::natives_fx::bolt_effect(world, n, STUN, "tag_origin", true);
        t.stunned = on;
        if on {
            lights_off(world, n, t);
            super::super::natives_fx::bolt_effect(world, n, STUN, "tag_origin", false);
            if let Some((_, _, _, origin)) = super::super::natives_fx::shown(world, n) {
                super::super::natives_fx::sound(world, EventAudience::All, STUN_SOUND, origin);
            }
        }
    });
}

/// `death` (`ai_tank_death`): its lights and stun off for good.
pub(super) fn death(world: &mut World, n: u32) {
    with_tank(world, n, |world, n, t| {
        lights_off(world, n, t);
        super::super::natives_fx::bolt_effect(world, n, STUN, "tag_origin", true);
        t.dead = true;
    });
}

/// `missile_fire`: its pods' animation from the start
/// (`setanimrestart( anim, 1, 0, rate )`): the rockets left after one is
/// fired (2, 1, 0) at half speed, or the full reload (4).
pub(super) fn missile_fire(world: &mut World, n: u32, value: i64) {
    let clip = match value {
        2 => Some(("o_drone_tank_missile1_fire", 0.5)),
        1 => Some(("o_drone_tank_missile2_fire", 0.5)),
        0 => Some(("o_drone_tank_missile3_fire", 0.5)),
        4 => Some(("o_drone_tank_missile_full_reload", 1.0)),
        _ => None,
    };
    with_tank(world, n, |world, n, t| {
        if t.missiles == value {
            return;
        }
        t.missiles = value;
        if let Some((clip, rate)) = clip {
            super::super::mp_fxanims::restart_at(world, n, clip, rate);
        }
    });
}

/// Who sees which light: its side green, the rest red, its driver none
/// (`start_light_fx`, `play_light_fx`).
fn lights_wanted(world: &mut World, obj: ObjRef, driver: Option<u32>) -> Vec<(u32, &'static str)> {
    let sides = super::super::with_vm(world, |vm, world| rc_sides(vm, world, &Value::Object(obj)));
    let Some((EventAudience::Clients(friends), EventAudience::Clients(foes))) = sides else {
        return Vec::new();
    };
    let side = |cs: Vec<ClientId>, fx: &'static str| cs.into_iter().map(move |c| (c.0, fx));
    side(friends, LIGHT_FRIEND)
        .chain(side(foes, LIGHT_FOE))
        .filter(|(c, _)| Some(*c) != driver)
        .collect()
}

/// Whether the ground under it throws dirt (`getwheelsurface`).
fn dirty(world: &mut World, origin: [f32; 3]) -> bool {
    let start = [origin[0], origin[1], origin[2] + 16.0];
    let end = [origin[0], origin[1], origin[2] - 32.0];
    let tr = super::super::frame(world).trace_static_world(
        start,
        end,
        [0.0; 3],
        [0.0; 3],
        crate::bullet_collision::MASK_PLAYER_SOLID,
    );
    tr.fraction < 1.0 && DIRTY.contains(&((tr.sflags >> SURF_TYPE_SHIFT) & SURF_TYPE_BITS))
}

/// Each snapshot: its lights to who should see them, its tread dirt and its
/// driver's rumble. None once it is gone (what it showed went with it).
pub(super) fn tick(world: &mut World, n: u32, mut t: Box<Tank>, now: i64) -> Option<(i64, Box<Tank>)> {
    let alive = world.resource::<Zm>().ents.get(&n).is_some_and(|e| e.obj == Some(t.obj));
    if !alive {
        super::super::mp_fxanims::forget(world, n);
        return None;
    }
    // Not drawn (yet): its client has no dobj to light.
    let Some((_, _, angles, origin)) = super::super::natives_fx::shown(world, n) else {
        return Some((now + TICK_MS, t));
    };
    let (speed, driver) = world
        .resource::<super::super::killstreaks::Streaks>()
        .craft
        .get(&n)
        .map_or((0.0, None), |c| (c.cur, c.rider));
    let want = if t.stunned || t.dead { Vec::new() } else { lights_wanted(world, t.obj, driver) };
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for &(c, fx) in &t.lit {
        if !want.contains(&(c, fx)) && players.contains(&c) {
            super::super::natives_fx::bolt_effect_to(world, EventAudience::Client(ClientId(c)), n, fx, LIGHT_TAG, true);
        }
    }
    for &(c, fx) in &want {
        if !t.lit.contains(&(c, fx)) {
            super::super::natives_fx::bolt_effect_to(world, EventAudience::Client(ClientId(c)), n, fx, LIGHT_TAG, false);
        }
    }
    t.lit = want;
    if now >= t.next_tread {
        t.next_tread = if speed >= MOVING {
            let fx = if dirty(world, origin) { TREAD_DIRT } else { TREAD_CONCRETE };
            let (forward, _, up) = gsc_t6::math::angle_vectors(angles);
            super::super::natives_fx::effect_axis(world, fx, origin, forward, up);
            now + TREAD_WAIT_MS
        } else {
            now + TREAD_LOOK_MS
        };
    }
    if let Some(d) = driver
        && speed.abs() >= MOVING
    {
        let _ = super::super::killstreaks::shake_to(world, d, RUMBLE_SCALE, RUMBLE_SECS, origin, RUMBLE_RADIUS);
    }
    Some((now + TICK_MS, t))
}
