//! Origins (zm_tomb) Panzer Soldat test aid.
//!
//! BO2MP_T6_MECHZ=1: every 2 s, each living Panzer (`is_mechz`, spawned by
//! the game's own `mechz_round_tracker` from round 8 once No Man's Land is
//! open): where it is, its `ai_state`, health, the helmet / power-plant
//! cover / power-plant damage it has taken, and how far the player is.
//!
//! BO2MP_T6_MECHZ_VIEW=<dist>[:<deg>] (pictures): the player looks at the
//! nearest Panzer's chest every tick; while it is more than 1.5 x <dist>
//! away he is put <dist> units from it at <deg> from the way it faces
//! (0, in front, unless given), so it can still walk up, grab and burn him.
//! BO2MP_T6_MECHZ_VIEW=<dist>:<deg>:shoot: he also aims at its head
//! (`j_head`, posed) and fires every 0.3 s, clip refilled (the helmet comes
//! off, then head hits kill it).

use super::pairs::KeepByValue;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, ScriptValue};

use super::{Zm, frame, with_vm};
use crate::sim_parts::ClientId;

/// Living Panzers: entity number, script object, origin, yaw.
fn panzers(world: &mut World) -> Vec<(u32, ObjRef, [f32; 3], f32)> {
    let alive: Vec<(u32, ObjRef, [f32; 3], f32)> = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, lhs)| {
                let e = zm.ents.get(nn)?;
                Some((*nn, lhs.obj?, e.origin, e.rot[1]))
            })
            .collect()
    };
    with_vm(world, |vm, world| {
        let f = vm.intern("is_mechz");
        alive
            .into_iter()
            .filter(|c| {
                let v = vm.get_field(world, c.1, f);
                gsc_t6::truthy(&v)
            })
            .collect()
    })
    .unwrap_or_default()
}

fn flat_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Shoot mode: the other zombies within 300 units of him die (as a team
/// mate would shoot them), so his bullets reach the Panzer's head.
fn clear_the_way(world: &mut World, panzers: &[u32], feet: [f32; 3]) {
    let near: Vec<(u32, i32, [f32; 3])> = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .filter(|(nn, lhs)| lhs.living && !panzers.contains(nn))
            .filter_map(|(nn, lhs)| {
                let en = zm.ents.get(nn)?;
                (flat_dist(en.origin, feet) < 300.0).then_some((*nn, lhs.hp, en.origin))
            })
            .collect()
    };
    if near.is_empty() {
        return;
    }
    with_vm(world, |vm, world| {
        for (nn, health, origin) in near {
            let head = [origin[0], origin[1], origin[2] + 60.0];
            super::actors::damage(
                vm,
                world,
                nn,
                ScriptValue::Undefined,
                ScriptValue::Undefined,
                health + 1,
                0,
                "MOD_RIFLE_BULLET",
                "none",
                head,
                [0.0, 0.0, -1.0],
                "head",
            );
        }
    });
}

pub(crate) fn mechz_test(world: &mut World, now: i64) {
    static LOG: OnceLock<bool> = OnceLock::new();
    static VIEW: OnceLock<Option<(f32, f32, bool)>> = OnceLock::new();
    let log = *LOG.get_or_init(|| std::env::var_os("BO2MP_T6_MECHZ").is_some());
    let view = *VIEW.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_MECHZ_VIEW").ok()?;
        let mut it = v.split(':');
        let d = it.next()?.parse().ok()?;
        let deg = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        Some((d, deg, it.next() == Some("shoot")))
    });
    if !log && view.is_none() {
        return;
    }
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let client = ClientId(0);
    let Some(pstate) = frame(world).avatar(client).copied() else {
        return;
    };
    let stock = panzers(world);
    if let Some((dist, deg, fire)) = view {
        if let Some(&(n, _, origin, yaw)) = stock
            .iter()
            .min_by(|lhs, rhs| flat_dist(lhs.2, pstate.origin).total_cmp(&flat_dist(rhs.2, pstate.origin)))
        {
            let mut feet = pstate.origin;
            if flat_dist(origin, pstate.origin) > dist * 1.5 {
                let lhs = (yaw + deg).to_radians();
                let stand = [
                    origin[0] + lhs.cos() * dist,
                    origin[1] + lhs.sin() * dist,
                    origin[2] + 60.0,
                ];
                let down = [stand[0], stand[1], stand[2] - 300.0];
                let t = frame(world).trace_static_world(
                    stand,
                    down,
                    crate::shot_trace::BODY_BOX_LOWER,
                    crate::shot_trace::BODY_BOX_UPPER,
                    crate::shot_trace::PLAYER_BODY_MASK,
                );
                if !t.starts_inside && t.fraction < 1.0 {
                    feet = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
                    super::teleport_player(world, client, feet);
                }
            }
            let eye = [feet[0], feet[1], feet[2] + pstate.eye_height_now];
            let chest = [origin[0], origin[1], origin[2] + 70.0];
            let aim = if fire {
                super::tags::tag_world(world, n, "j_head").map_or([origin[0], origin[1], origin[2] + 90.0], |(o, _)| o)
            } else {
                chest
            };
            super::set_player_view(world, client, super::autoplay::look(eye, aim));
            if fire && now % 300 < tick {
                let panzers: Vec<u32> = stock.iter().map(|c| c.0).collect();
                clear_the_way(world, &panzers, feet);
                {
                    let mut f = frame(world);
                    if let Some(w) = f.avatar(client).map(|p| p.weapon) {
                        crate::client_script::set_ammo_clip(&mut f, client, w, 8);
                    }
                }
                let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
                let mine = req.input.cmds.iter_mut().filter(|(who, _)| *who == client);
                for (_, cmd) in mine {
                    cmd.buttons |= playerstate_t6::game::buttons::ATTACK;
                }
            }
        }
    }
    if !log || now % 2000 >= tick {
        return;
    }
    static LAST: AtomicUsize = AtomicUsize::new(0);
    if stock.is_empty() {
        if LAST.swap(0, Ordering::Relaxed) > 0 {
            diag::info!(Sim, "bo2zm t6 mechz test {}s: no Panzer left", now / 1000);
        }
        return;
    }
    LAST.store(stock.len(), Ordering::Relaxed);
    for (n, obj, origin, _) in stock {
        let health = world.resource::<Zm>().actors.get(&n).map_or(0, |lhs| lhs.hp);
        let line = with_vm(world, |vm, world| {
            let mut field = |name: &str| {
                let f = vm.intern(name);
                let v = vm.get_field(world, obj, f);
                vm.to_text(&v)
            };
            format!(
                "ai_state {} helmet_dmg {}/{} cover_dmg {}/{} powerplant_dmg {}/{}",
                field("ai_state"),
                field("helmet_dmg"),
                field("helmet_dmg_for_removal"),
                field("powerplant_cover_dmg"),
                field("powerplant_cover_dmg_for_removal"),
                field("powerplant_dmg"),
                field("powerplant_dmg_for_destroy"),
            )
        })
        .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 mechz test {}s: #{n} at ({:.0} {:.0} {:.0}) health {health} {line}; player {:.0} away",
            now / 1000,
            origin[0],
            origin[1],
            origin[2],
            flat_dist(origin, pstate.origin)
        );
    }
}
