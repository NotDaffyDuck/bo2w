//! The Jet Gun as BO2's engine gives it to TranZit's `_zm_weap_jetgun`.
//! Its trigger fires no shots. Held, it spins the engine up (the spin the
//! script reads with `getcurrentweaponspinlerp`: past 0.2 it pulls, grinds
//! and flings zombies) and heats it; at full heat it is overheated, which
//! in TranZit blows it up in his hands. Let go, it spins down and cools.

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, ScriptValue, Vm};

use super::{Zm, arg, client, frame, text, with_vm};
use crate::sim_parts::ClientId;

/// One Jet Gun's numbers, read from the game files (`t6m2 --weapons`,
/// zm_transit: iSpinUpTime, iSpinDownTime, overheatRate, cooldownRate,
/// overheatEndVal, iFireTime). Heat runs 0..100; the rates are per second.
/// It does not cool while firing (coolWhileFiring 0).
#[derive(Clone, Copy)]
pub(crate) struct Gun {
    spin_up_ms: f32,
    spin_down_ms: f32,
    heat_rate: f32,
    cool_rate: f32,
    overheat_end: f32,
    fire_ms: i64,
}

const JETGUN: Gun = Gun {
    spin_up_ms: 3000.0,
    spin_down_ms: 5000.0,
    heat_rate: 17.0,
    cool_rate: 1.0,
    overheat_end: 77.0,
    fire_ms: 50,
};
const JETGUN_UPGRADED: Gun = Gun {
    spin_up_ms: 4000.0,
    ..JETGUN
};
const FULL_HEAT: f32 = 100.0;

pub(crate) fn gun(name: &str) -> Option<Gun> {
    match name.strip_suffix("_mp").unwrap_or(name) {
        "jetgun_zm" => Some(JETGUN),
        "jetgun_upgraded_zm" => Some(JETGUN_UPGRADED),
        _ => None,
    }
}

/// The weapon step leaves a Jet Gun's trigger alone: it fires no shots.
pub(crate) fn owns_trigger(weapon_name: &str) -> bool {
    gun(weapon_name).is_some()
}

/// His Jet Gun's engine: the spin of the gun in hand, and the gun's heat
/// (kept while he holds another weapon; the script puts it back when he
/// drops and picks the gun up again).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct JetState {
    pub spin: f32,
    pub heat: f32,
    pub overheated: bool,
    pub last_fired: i64,
}

fn with_state<T>(world: &mut World, c: u32, f: impl FnOnce(&mut JetState) -> T) -> Option<T> {
    world
        .resource_mut::<Zm>()
        .players
        .get_mut(&c)
        .map(|p| f(&mut p.jet))
}

/// His Jet Gun's heat, 0..100 (the autoplay lets go before it blows up).
pub(crate) fn heat(world: &World, c: u32) -> f32 {
    world.resource::<Zm>().players.get(&c).map_or(0.0, |p| p.jet.heat)
}

/// The Jet Gun a native names (its weapon argument), else the one in hand.
fn named_gun(vm: &Vm<World>, game: &mut World, cid: ClientId, lhs: &[ScriptValue], ix: usize) -> bool {
    let name = if matches!(arg(lhs, ix), ScriptValue::Undefined) {
        let w = frame(game).avatar(cid).map_or(0, |ps| ps.weapon);
        super::weapon_text(game, w)
    } else {
        text(vm, lhs, ix)
    };
    gun(&name).is_some()
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // player getcurrentweaponspinlerp(): the gun in hand's spin, 0..1.
    vm.bind("getcurrentweaponspinlerp", true, |vm, world, s, _| {
        let id = client(vm, world, s)?;
        Ok(ScriptValue::Float(with_state(world, id.0, |j| j.spin).unwrap_or(0.0)))
    });
    vm.bind("setcurrentweaponspinlerp", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let v = arg(a, 0).as_float().unwrap_or(0.0).clamp(-1.0, 1.0);
        with_state(world, id.0, |j| j.spin = v);
        Ok(ScriptValue::Undefined)
    });
    // player isweaponoverheating(heat?, weapon?): 0 asks whether it is
    // overheated, 1 how hot it is (0..100). Only the Jet Gun heats.
    vm.bind("isweaponoverheating", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let want_heat = arg(a, 0).as_int().unwrap_or(0) != 0;
        if !named_gun(vm, world, id, a, 1) {
            return Ok(if want_heat { ScriptValue::Float(0.0) } else { ScriptValue::Int(0) });
        }
        let j = with_state(world, id.0, |j| *j).unwrap_or_default();
        Ok(if want_heat {
            ScriptValue::Float(j.heat)
        } else {
            ScriptValue::Int(i32::from(j.overheated))
        })
    });
    // player setweaponoverheating(overheated, heat, weapon?): puts a
    // dropped Jet Gun's heat back when he picks it up.
    vm.bind("setweaponoverheating", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        if !named_gun(vm, world, id, a, 2) {
            return Ok(ScriptValue::Undefined);
        }
        let over = arg(a, 0).as_int().is_some_and(|v| v != 0);
        let heat = arg(a, 1).as_float().unwrap_or(0.0).clamp(0.0, FULL_HEAT);
        with_state(world, id.0, |j| {
            j.overheated = over;
            j.heat = heat;
        });
        Ok(ScriptValue::Undefined)
    });
    // The jet blowing loose physics objects about, and the shredded-zombie
    // spray out of the exhaust: no loose physics objects yet, and drawing
    // only.
    vm.bind("physicsjetthrust", false, |_, _, _, _| Ok(ScriptValue::Undefined));
    vm.bind("weaponplayejectbrass", true, |_, _, _, _| Ok(ScriptValue::Undefined));
}

/// Each tick: a held trigger on a ready Jet Gun spins it up and heats it,
/// and fires `weapon_fired` every shot's time (the script's pull loop
/// starts on it); otherwise it spins down and cools.
pub(super) fn tick(world: &mut World, dt_ms: f32) {
    let now = world.resource::<Zm>().clock_millis;
    let clients: Vec<(u32, ObjRef)> = world
        .resource::<Zm>()
        .players
        .iter()
        .map(|(c, p)| (*c, p.obj))
        .collect();
    for (c, obj) in clients {
        let id = ClientId(c);
        let Some(pstate) = frame(world).avatar(id).copied() else {
            continue;
        };
        let label = super::weapon_text(world, pstate.weapon);
        let Some(g) = gun(&label) else {
            // Spin belongs to the gun in hand; heat stays with the Jet Gun
            // until it is gone (taken, or blown up).
            let owned = pstate
                .weapons
                .iter()
                .any(|&w| w > 0 && gun(&super::weapon_text(world, w as u32)).is_some());
            with_state(world, c, |j| {
                j.spin = 0.0;
                if !owned {
                    *j = JetState::default();
                }
            });
            continue;
        };
        let held = crate::client_script::buttons(&mut frame(world), id)
            & playerstate_t6::game::buttons::ATTACK
            != 0;
        let ready = matches!(pstate.weaponstate_primary, weapon_t6::state::READY | weapon_t6::state::FIRING);
        let fired = with_state(world, c, |j| {
            let firing = held && ready && !j.overheated;
            if firing {
                j.spin = (j.spin + dt_ms / g.spin_up_ms).min(1.0);
                j.heat = (j.heat + g.heat_rate * dt_ms / 1000.0).min(FULL_HEAT);
                if j.heat >= FULL_HEAT {
                    j.overheated = true;
                }
            } else {
                j.spin = (j.spin - dt_ms / g.spin_down_ms).max(0.0);
                j.heat = (j.heat - g.cool_rate * dt_ms / 1000.0).max(0.0);
                if j.overheated && j.heat <= g.overheat_end {
                    j.overheated = false;
                }
            }
            let fire = firing && now - j.last_fired >= g.fire_ms;
            if fire {
                j.last_fired = now;
            }
            (fire, *j)
        })
        .unwrap_or_default();
        let (fired, j) = fired;
        if std::env::var_os("BO2MP_T6_PLACE").is_some()
            && (held || j.spin > 0.0)
            && now % 1000 < i64::from(crate::SIM_STEP_MILLIS)
        {
            diag::info!(
                Sim,
                "bo2zm t6 jetgun: {label} held {held} ready {ready} spin {:.2} heat {:.0} overheated {}",
                j.spin,
                j.heat,
                j.overheated
            );
        }
        if fired && std::env::var_os("BO2MP_T6_JETLOG").is_some() && j.spin > 0.2 {
            near_log(world, &pstate);
        }
        if fired {
            with_vm(world, |vm, world| {
                let n = vm.string(&label);
                vm.notify_str(world, obj, "weapon_fired", &[n]);
            });
        }
    }
}

/// BO2MP_T6_JETLOG: at each pull, the three zombies nearest his muzzle as
/// `_zm_weap_jetgun` measures them (muzzle to body middle, the aim's dot)
/// and the state the script checks (`ai_state`, `drag_state`).
fn near_log(world: &mut World, pstate: &playerstate_t6::game::AvatarState) {
    let (f, _, _) = gsc_t6::math::facing_axes(pstate.look_angles);
    let eye_at = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now];
    let muzzle: [f32; 3] = std::array::from_fn(|i| eye_at[i] + f[i] * 16.0);
    let rows = with_vm(world, |vm, world| {
        let zm = world.resource::<Zm>();
        let field = |o: ObjRef, k: &str| -> String {
            match vm.strings.find(k).map(|k| vm.raw_field(o, k)) {
                Some(v) => vm.to_text(&v),
                None => "-".to_owned(),
            }
        };
        let mut zs: Vec<(f32, String)> = zm
            .actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(k, lhs)| {
                let e = zm.ents.get(k)?;
                let o = lhs.obj?;
                let c = [e.origin[0], e.origin[1], e.origin[2] + 36.0];
                let v: [f32; 3] = std::array::from_fn(|i| c[i] - muzzle[i]);
                let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                let dot = if d > 0.001 { (v[0] * f[0] + v[1] * f[1] + v[2] * f[2]) / d } else { 0.0 };
                Some((
                    d,
                    format!("ent{k} {d:.0} dot {dot:.2} ai_state {} drag {}", field(o, "ai_state"), field(o, "drag_state")),
                ))
            })
            .collect();
        zs.sort_by(|lhs, b| lhs.0.total_cmp(&b.0));
        zs.into_iter().take(3).map(|z| z.1).collect::<Vec<_>>()
    });
    diag::info!(Sim, "bo2zm t6 jetgun near: {rows:?}");
}
