//! Actor builtins: spawning, goals and paths, orient/anim modes, the
//! animstatedef queries, scripted animations, melee, damage and death; and
//! the player damage the zombies deal (CodeCallback_PlayerDamage ->
//! finishplayerdamage).

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ScriptValue, Vm};

use super::actors::{self, ANIMSCRIPTS};
use super::{Zm, arg, entnum, frame, int, is_player, list, num, origin_of, text, vec3};
use crate::sim_parts::ClientId;

/// The character an actor wears when the script has set none (or set it badly).
const FIRST_CHARACTER: i32 = 0;

/// An actor's body (radius, height) for the route checks; a zombie's for
/// anything else.
fn body_size(vm: &Vm<World>, world: &World, v: &ScriptValue) -> (f32, f32) {
    entnum(vm, v)
        .and_then(|n| world.resource::<Zm>().actors.get(&n))
        .map_or((15.0, 72.0), |a| (a.reach, a.height))
}

fn actor_n(vm: &Vm<World>, world: &World, s: &ScriptValue) -> Result<u32, String> {
    let n = entnum(vm, s).ok_or("not an entity")?;
    if world.resource::<Zm>().actors.contains_key(&n) {
        Ok(n)
    } else {
        Err("not an actor".into())
    }
}

fn is_actor(vm: &Vm<World>, world: &World, v: &ScriptValue) -> bool {
    entnum(vm, v).is_some_and(|n| world.resource::<Zm>().actors.contains_key(&n))
}

pub(super) fn actor_alive(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Option<bool> {
    let n = entnum(vm, v)?;
    world.resource::<Zm>().actors.get(&n).map(|a| a.living)
}

/// Set by the down test: its one blow lands even under BO2MP_T6_GOD.
pub(crate) static TEST_BLOW: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `CodeCallback_PlayerDamage` on player `victim` (a zombie's swing, a
/// fall, an explosion...).
#[allow(clippy::too_many_arguments)]
pub(crate) fn player_damage(
    vm: &mut Vm<World>, game: &mut World,
    struck: u32, inflictor: ScriptValue, hitter: ScriptValue, qty: i32, flags: i32,
    means: &str, gun: &str, place: [f32; 3], aim: [f32; 3], hitloc: &str,
) {
    player_damage_from(
        vm,
        game,
        struck,
        inflictor,
        hitter,
        qty,
        flags,
        means,
        gun,
        ScriptValue::Vec3(place),
        ScriptValue::Vec3(aim),
        hitloc,
    );
}

/// The same, the point and direction undefined where BO2's G_Damage got
/// none (a trigger_hurt's).
#[allow(clippy::too_many_arguments)]
pub(crate) fn player_damage_from(
    vm: &mut Vm<World>, game: &mut World,
    struck: u32, inflictor: ScriptValue, hitter: ScriptValue, qty: i32, flags: i32,
    means: &str, gun: &str, place: ScriptValue, aim: ScriptValue, hitloc: &str,
) {
    // BO2MP_T6_GOD=1 (tests): players take no damage; BO2MP_T6_GOD=melee:
    // none from zombies' hits only (TranZit's lava still burns him).
    static GOD: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let forced = TEST_BLOW.swap(false, std::sync::atomic::Ordering::Relaxed);
    let god = match GOD.get_or_init(|| std::env::var("BO2MP_T6_GOD").ok()).as_deref() {
        Some("melee") => means == "MOD_MELEE",
        Some(_) => true,
        None => false,
    };
    // bo2zm: the cheat menu's God Mode: no hurt and no going down, from
    // anything (zombies, falls, explosions, his own grenades).
    if super::cheats::cheat_on(super::cheats::Cheat::GodMode) {
        return;
    }
    if !forced && (god || super::autoplay::rounds_test()) {
        return;
    }
    // bo2zm: Origins' staffs on another map (zm_tomb's tomb_player_damage_callback).
    let staffs = vm.intern("bo2zm_foreign_staffs");
    if gun.contains("staff") && gsc_t6::truthy(&vm.raw_field(vm.level, staffs)) {
        return;
    }
    let Some(obj) = game.resource::<Zm>().players.get(&struck).map(|p| p.obj) else {
        return;
    };
    let means = vm.string(means);
    let gun = vm.string(gun);
    let hitloc = vm.string(hitloc);
    let args = vec![
        inflictor,
        hitter,
        ScriptValue::Int(qty),
        ScriptValue::Int(flags),
        means,
        gun,
        place,
        aim,
        hitloc,
        ScriptValue::Int(0),
        ScriptValue::Int(0),
    ];
    let cb = super::mp::callbacks(game);
    vm.spawn_named(
        game,
        &cb,
        "codecallback_playerdamage",
        ScriptValue::Object(obj),
        args,
    );
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
    m!("spawnactor", |vm, sim, s, _| {
        let Some(sp) = s.as_obj() else {
            return Ok(ScriptValue::Undefined);
        };
        Ok(actors::spawn_actor(vm, sim, sp).map_or(ScriptValue::Undefined, ScriptValue::Object))
    });
    f!("getfreeactorcount", |_, sim, _, _| {
        // The engine's actor table holds 32.
        let living = sim.resource::<Zm>().actors.values().filter(|actor| actor.living).count() as i32;
        Ok(ScriptValue::Int((32 - living).max(0)))
    });
    for name in ["getaiarray", "getaispeciesarray"] {
        vm.bind(name, false, |vm, sim, _, lhs| {
            let team = match arg(lhs, 0) {
                ScriptValue::Undefined => None,
                v => Some(vm.to_text(v)),
            };
            let team_s = vm.intern("team");
            let actors: Vec<gsc_t6::ObjRef> = sim
                .resource::<Zm>()
                .actors
                .values()
                .filter(|xs| xs.living)
                .filter_map(|xs| xs.obj)
                .collect();
            // A deleted actor's object is freed at once; its actor entry
            // goes on the next think, so skip it here.
            let out = actors
                .into_iter()
                .filter(|o| vm.alive(*o))
                .filter(|o| match &team {
                    None => true,
                    Some(t) if t == "all" => true,
                    Some(t) => vm.to_text(&vm.raw_field(*o, team_s)) == *t,
                })
                .map(ScriptValue::Object)
                .collect();
            Ok(list(out))
        });
    }
    // Every corpse entity: the dead actors and the players' bodies (`cloneplayer`),
    // which the map scripts clear off lhs moving train, crane, cable car or gondola.
    f!("getcorpsearray", |_, sim, _, _| {
        let zm = sim.resource::<Zm>();
        let actors = zm.actors.values().filter(|xs| !xs.living).filter_map(|xs| xs.obj);
        let bodies = zm
            .ents
            .values()
            .filter(|en| en.class_tag == "player_corpse")
            .filter_map(|en| en.obj);
        Ok(list(actors.chain(bodies).map(ScriptValue::Object).collect()))
    });
    f!("isai", |vm, sim, _, lhs| Ok(ScriptValue::bool(
        actor_alive(vm, sim, arg(lhs, 0)).is_some()
    )));
    f!("isalive", |vm, sim, _, lhs| {
        let v = arg(lhs, 0);
        if !vm.is_defined(v) {
            return Ok(ScriptValue::Int(0));
        }
        if let Some(alive) = actor_alive(vm, sim, v) {
            return Ok(ScriptValue::bool(alive));
        }
        if let Some(n) = entnum(vm, v)
            && let Some(p) = sim.resource::<Zm>().players.get(&n).cloned()
        {
            let h = frame(sim).avatar(ClientId(n)).map_or(0, |ps| ps.health);
            return Ok(ScriptValue::bool(p.sessionstate == "playing" && h > 0));
        }
        Ok(ScriptValue::bool(entnum(vm, v).is_some()))
    });
    f!("issentient", |vm, sim, _, lhs| {
        let v = arg(lhs, 0);
        Ok(ScriptValue::bool(
            is_player(vm, sim, v) || actor_alive(vm, sim, v) == Some(true),
        ))
    });
    m!("setgoalpos", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let goal = vec3(lhs, 0)?;
        set_goal(vm, sim, n, goal);
        if let Some(ang) = arg(lhs, 1).as_vec3()
            && let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n)
        {
            xs.goal_yaw = Some(ang[1]);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setgoalnode", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let Some(goal) = arg(lhs, 0).as_obj().map(|o| {
            let f = vm.intern("origin");
            vm.raw_field(o, f)
        }) else {
            return Ok(ScriptValue::Undefined);
        };
        if let Some(goal) = goal.as_vec3() {
            set_goal(vm, sim, n, goal);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setgoalentity", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let target = arg(lhs, 0).clone();
        if let Some(goal) = origin_of(vm, sim, &target) {
            set_goal(vm, sim, n, goal);
        }
        Ok(ScriptValue::Undefined)
    });
    // 0 when no path joins them, as BO2's engine says: the scripts test
    // `== 0` / `!= 0` (Buried's ghosts skip such lhs player, Buried's and
    // Origins' distance tracking respawn lhs zombie that can reach no one).
    // Before it was -1, so an unreachable player read as the closest one.
    m!("calcpathlength", |vm, sim, s, lhs| {
        let from = origin_of(vm, sim, s).unwrap_or([0.0; 3]);
        let to = vec3(lhs, 0)?;
        let (r, h) = body_size(vm, sim, s);
        Ok(ScriptValue::Float(actors::path_length(sim, from, to, r, h).unwrap_or(0.0)))
    });
    // getpathlength: how far this actor still has to go on its route (0
    // with none). Buried's mansion ghosts read 0 as no way to lhs player who
    // left the mansion and run back (_zm_ai_ghost should_runaway).
    m!("getpathlength", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        let zm = sim.resource::<Zm>();
        let (Some(xs), Some(en)) = (zm.actors.get(&n), zm.ents.get(&n)) else {
            return Ok(ScriptValue::Float(0.0));
        };
        let mut at = en.origin;
        let mut len = 0.0;
        for p in xs.path.iter().skip(xs.path_i) {
            len += ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2)).sqrt();
            at = *p;
        }
        Ok(ScriptValue::Float(len))
    });
    // findpath(start, end, ent, allow_negotiation, allow_partial): whether
    // the path nodes join the two points (Mob's brutus_stuck_watcher counts
    // lhs false as lhs stuck Brutus and teleports him after 4).
    f!("findpath", |vm, sim, _, lhs| {
        let from = arg(lhs, 0)
            .as_vec3()
            .or_else(|| origin_of(vm, sim, arg(lhs, 0)));
        let to = arg(lhs, 1)
            .as_vec3()
            .or_else(|| origin_of(vm, sim, arg(lhs, 1)));
        let (r, h) = body_size(vm, sim, arg(lhs, 2));
        Ok(ScriptValue::bool(match (from, to) {
            (Some(from), Some(to)) => actors::path_length(sim, from, to, r, h).is_some(),
            _ => false,
        }))
    });
    m!("isingoal", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let p = vec3(lhs, 0)?;
        let obj = s.as_obj().unwrap();
        let gr = vm.intern("goalradius");
        let radius = vm.raw_field(obj, gr).as_float().unwrap_or(32.0);
        let goal = sim.resource::<Zm>().actors.get(&n).and_then(|xs| xs.goal);
        Ok(ScriptValue::bool(goal.is_none_or(|g| {
            let d = [g[0] - p[0], g[1] - p[1]];
            (d[0] * d[0] + d[1] * d[1]).sqrt() <= radius.max(64.0)
        })))
    });
    // maymovetopoint(point, ignore_actors) / maymovefrompointtopoint(from,
    // to): lhs straight walk with this actor's body (actors::may_move).
    m!("maymovetopoint", |vm, sim, s, lhs| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Int(1));
        };
        let to = vec3(lhs, 0)?;
        let body = {
            let zm = sim.resource::<Zm>();
            zm.actors.get(&n).map(|xs| (xs.reach, xs.height)).zip(zm.ents.get(&n).map(|en| en.origin))
        };
        let Some(((r, h), from)) = body else {
            return Ok(ScriptValue::Int(1));
        };
        Ok(ScriptValue::bool(actors::may_move(sim, from, to, r, h)))
    });
    m!("maymovefrompointtopoint", |vm, sim, s, lhs| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Int(1));
        };
        let from = vec3(lhs, 0)?;
        let to = vec3(lhs, 1)?;
        let body = sim.resource::<Zm>().actors.get(&n).map(|xs| (xs.reach, xs.height));
        let Some((r, h)) = body else {
            return Ok(ScriptValue::Int(1));
        };
        Ok(ScriptValue::bool(actors::may_move(sim, from, to, r, h)))
    });
    for name in ["cansee", "canshoot"] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Int(1)));
    }
    m!("forceteleport", |vm, sim, s, lhs| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let mut p = vec3(lhs, 0)?;
        let ang = arg(lhs, 1).as_vec3();
        // A walking actor lands on the floor under the spot and plans its
        // goal again from there (its old route started where it was).
        let walker = {
            let zm = sim.resource::<Zm>();
            zm.actors
                .get(&n)
                .filter(|xs| xs.living && xs.scripted.is_none() && xs.traverse.is_none())
                .filter(|_| zm.ents.get(&n).is_some_and(|en| en.link.is_none()))
                .map(|xs| (xs.reach, xs.height))
        };
        let asked = p;
        if let Some((r, h)) = walker {
            p = actors::floor_below(sim, p, r, h);
        }
        let goal = {
            let mut zm = sim.resource_mut::<Zm>();
            if let Some(en) = zm.ents.get_mut(&n) {
                en.origin = p;
                if let Some(ang) = ang {
                    en.rot = ang;
                }
            }
            match (walker, zm.actors.get_mut(&n)) {
                (Some(_), Some(xs)) => {
                    xs.path.clear();
                    xs.path_nodes.clear();
                    xs.path_i = 0;
                    xs.goal_node = None;
                    xs.last_gain = None;
                    xs.stalled_on = None;
                    xs.goal
                }
                _ => None,
            }
        };
        if walker.is_some() && std::env::var_os("BO2MP_T6_ACTORS").is_some() {
            diag::info!(
                Sim,
                "bo2zm t6 actor {n} forceteleport to ({:.0} {:.0} {:.0}) stands at ({:.0} {:.0} {:.0}), replans {}",
                asked[0],
                asked[1],
                asked[2],
                p[0],
                p[1],
                p[2],
                goal.map_or("nothing".to_owned(), |g| format!("({:.0} {:.0} {:.0})", g[0], g[1], g[2]))
            );
        }
        if let Some(g) = goal {
            set_goal(vm, sim, n, g);
        }
        Ok(ScriptValue::Undefined)
    });
    // teleport(origin): animscripts/traverse/shared::teleportthread lifts lhs
    // zombie mid-climb by the height its animation misses. Ours crosses
    // node to node already (actors' traverse), so lhs climbing one stays on
    // it; any other moves there.
    m!("teleport", |vm, sim, s, lhs| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let p = vec3(lhs, 0)?;
        let mut zm = sim.resource_mut::<Zm>();
        if zm.actors.get(&n).is_some_and(|xs| xs.traverse.is_some()) {
            return Ok(ScriptValue::Undefined);
        }
        if let Some(en) = zm.ents.get_mut(&n) {
            en.origin = p;
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: stun it or clear the stun, as Black Ops II's Actor_SetFlashed
    // does: on keeps the stronger strength (1 unless given), off clears it.
    m!("setflashbanged", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let strength = match lhs.len() {
            1 => 1.0,
            2 => num(lhs, 1)?,
            _ => return Ok(ScriptValue::Undefined),
        };
        let on = int(lhs, 0)? != 0;
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.flash_banged = on;
            xs.flash_strength = if on {
                xs.flash_strength.max(strength)
            } else {
                0.0
            };
        }
        Ok(ScriptValue::Undefined)
    });
    m!("getflashbangedstrength", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        let zm = sim.resource::<Zm>();
        Ok(ScriptValue::Float(
            zm.actors.get(&n).map_or(0.0, |xs| xs.flash_strength),
        ))
    });
    // bo2mp: Black Ops II's getdeltaturnyaw (the dog's turn animscript).
    m!("getdeltaturnyaw", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        Ok(ScriptValue::Float(actors::delta_turn_yaw(sim, n)))
    });
    // bo2zm M3 fix list 2: tilt to the slope (actors::pitch_to_ground).
    m!("setpitchorient", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.pitch_orient = true;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("clearpitchorient", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.pitch_orient = false;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("orientmode", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let mode = text(vm, lhs, 0);
        let yw = arg(lhs, 1).as_float();
        let point = arg(lhs, 1).as_vec3();
        let mut zm = sim.resource_mut::<Zm>();
        if let Some(xs) = zm.actors.get_mut(&n) {
            xs.orientmode = mode;
            if let Some(y) = yw {
                xs.orient_yaw = y;
            }
            if let Some(p) = point {
                xs.orient_point = p;
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("animmode", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let mode = text(vm, lhs, 0);
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.animmode = mode;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setphysparams", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let r = num(lhs, 0).unwrap_or(15.0);
        let h = num(lhs, 2).unwrap_or(72.0);
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.reach = r;
            xs.height = h;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setanimstatefromasd", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let state = text(vm, lhs, 0);
        let sub = arg(lhs, 1).clone();
        actors::set_state(vm, sim, n, &state, &sub);
        Ok(ScriptValue::Undefined)
    });
    m!("getanimstatefromasd", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        let st = sim
            .resource::<Zm>()
            .actors
            .get(&n)
            .and_then(|xs| xs.playing.as_ref().map(|p| p.state.clone()));
        Ok(st.map_or(ScriptValue::Undefined, |st| vm.string(&st)))
    });
    m!("getanimsubstatefromasd", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        // (state, alias) -> that alias's index; (state) -> lhs pick; () ->
        // the current substate.
        if let ScriptValue::Str(_) = arg(lhs, 0) {
            let state = text(vm, lhs, 0);
            let zm = sim.resource::<Zm>();
            let Some(xs) = zm.actors.get(&n) else {
                return Ok(ScriptValue::Int(0));
            };
            let Some(def) = zm.asds.get(&xs.asd) else {
                return Ok(ScriptValue::Int(0));
            };
            let Some(st) = def.state(&state, xs.missing_legs) else {
                return Ok(ScriptValue::Int(0));
            };
            if let ScriptValue::Str(_) = arg(lhs, 1) {
                let alias = vm.to_text(arg(lhs, 1)).to_ascii_lowercase();
                return Ok(ScriptValue::Int(
                    st.substates
                        .iter()
                        .position(|(n, _)| *n == alias)
                        .unwrap_or(0) as i32,
                ));
            }
            let count = st.substates.len().max(1) as u32;
            return Ok(ScriptValue::Int((vm.rand_u32() % count) as i32));
        }
        let cur = sim
            .resource::<Zm>()
            .actors
            .get(&n)
            .and_then(|xs| xs.playing.as_ref().map(|p| p.substate));
        Ok(ScriptValue::Int(cur.unwrap_or(0) as i32))
    });
    m!("hasanimstatefromasd", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let state = text(vm, lhs, 0);
        let zm = sim.resource::<Zm>();
        let has = zm
            .actors
            .get(&n)
            .and_then(|xs| zm.asds.get(&xs.asd))
            .is_some_and(|d| d.state(&state, false).is_some());
        Ok(ScriptValue::bool(has))
    });
    m!("getanimlengthfromasd", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        let ms = sim
            .resource::<Zm>()
            .actors
            .get(&n)
            .and_then(|xs| xs.playing.as_ref().map(|p| p.length_ms))
            .unwrap_or(1000);
        Ok(ScriptValue::Float(ms as f32 / 1000.0))
    });
    m!("getanimhasnotetrackfromasd", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let note = text(vm, lhs, 0);
        let zm = sim.resource::<Zm>();
        let has = zm
            .actors
            .get(&n)
            .and_then(|xs| xs.playing.as_ref())
            .and_then(|p| zm.anims.get(&p.anim))
            .is_some_and(|an| {
                an.notifies
                    .iter()
                    .any(|(nn, _)| nn.eq_ignore_ascii_case(&note))
            });
        Ok(ScriptValue::bool(has))
    });
    // An ASD state's animation, for the scripts that measure it
    // (Die Rise's Jumping Jacks: `_zm_ai_leaper::leaper_calc_anim_offsets`
    // takes getmovedelta and getnotetracktimes of `zm_wall_up` ...). The
    // value is the animation's name; the anim builtins below take it.
    m!("getanimfromasd", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let state = text(vm, lhs, 0);
        let zm = sim.resource::<Zm>();
        let anim = zm
            .actors
            .get(&n)
            .and_then(|xs| zm.asds.get(&xs.asd))
            .and_then(|d| d.state(&state, false))
            .and_then(|st| {
                let ix = int(lhs, 1).unwrap_or(0).max(0) as usize;
                st.substates.get(ix).or(st.substates.first())
            })
            .map(|(_, anim)| anim.to_ascii_lowercase());
        Ok(anim.map_or(ScriptValue::Undefined, |anim| vm.string(&anim)))
    });
    f!("getmovedelta", |vm, sim, _, lhs| {
        let Some(anim) = anim_of(vm, sim, arg(lhs, 0)) else {
            return Ok(ScriptValue::Vec3([0.0; 3]));
        };
        let (from, to) = (num(lhs, 1).unwrap_or(0.0), num(lhs, 2).unwrap_or(1.0));
        let (p, q) = (actors::anim_root(&anim, from), actors::anim_root(&anim, to));
        Ok(ScriptValue::Vec3(std::array::from_fn(|ix| q[ix] - p[ix])))
    });
    // Where an aligned animation (window board tear, Leroy's table eat)
    // starts when played at `origin`/`angles`: its first root key, turned
    // by the yw (scripted anims play from there by root motion). Only
    // translation is captured, so the start angles are the align angles.
    f!("getstartorigin", |vm, sim, _, lhs| {
        let origin = vec3(lhs, 0).unwrap_or([0.0; 3]);
        let yw = vec3(lhs, 1).map_or(0.0, |v| v[1]);
        let root = anim_of(vm, sim, arg(lhs, 2)).map_or([0.0; 3], |anim| actors::anim_root(&anim, 0.0));
        let (s, c) = yw.to_radians().sin_cos();
        Ok(ScriptValue::Vec3([
            origin[0] + root[0] * c - root[1] * s,
            origin[1] + root[0] * s + root[1] * c,
            origin[2] + root[2],
        ]))
    });
    f!("getstartangles", |_vm, _world, _, lhs| {
        Ok(ScriptValue::Vec3(vec3(lhs, 1).unwrap_or([0.0; 3])))
    });
    f!("getnotetracktimes", |vm, sim, _, lhs| {
        let note = text(vm, lhs, 1);
        let times = anim_of(vm, sim, arg(lhs, 0))
            .map(|anim| {
                anim.notifies
                    .iter()
                    .filter(|(n, _)| n.eq_ignore_ascii_case(&note))
                    .map(|(_, f)| ScriptValue::Float(*f))
                    .collect()
            })
            .unwrap_or_default();
        Ok(list(times))
    });
    f!("getanimlength", |vm, sim, _, lhs| {
        let ms = anim_of(vm, sim, arg(lhs, 0)).map_or(1000, |anim| actors::anim_length_ms(&anim));
        Ok(ScriptValue::Float(ms as f32 / 1000.0))
    });
    // The function runs as the actor's animscript: its AI picks no script
    // until the thread ends.
    m!("animcustom", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let Some(obj) = s.as_obj() else {
            return Ok(ScriptValue::Undefined);
        };
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.script = "custom".to_owned();
            xs.path.clear();
        }
        vm.notify_str(sim, obj, "killanimscript", &[]);
        let ScriptValue::Func(gsc_t6::FuncRef::Script(f)) = arg(lhs, 0).clone() else {
            return Ok(ScriptValue::Undefined);
        };
        let at = sim.resource::<Zm>().ents.get(&n).map_or([0.0; 3], |en| en.origin);
        diag::info!(
            Sim,
            "bo2zm t6 animcustom ent{n} starts at ({:.0} {:.0} {:.0})",
            at[0],
            at[1],
            at[2]
        );
        let t = vm.spawn(sim, f, ScriptValue::Object(obj), vec![]);
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.custom = Some(t);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("animscripted", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        vec3(lhs, 0)?;
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.scripted = Some(actors::Scripted);
            xs.scripted_starts += 1;
            xs.path.clear();
            // Each call starts zm_scripted afresh, even back to back (the
            // window boards' tear in, loop and out).
            xs.script.clear();
        }
        // The engine hands the arguments to zm_scripted::init, then runs
        // zm_scripted::main (which calls startscriptedanim).
        let obj = s.as_obj().unwrap();
        vm.spawn_named(
            sim,
            &format!("{ANIMSCRIPTS}/zm_scripted"),
            "init",
            ScriptValue::Object(obj),
            lhs.to_vec(),
        );
        actors::run_script(vm, sim, n, "scripted");
        Ok(ScriptValue::Undefined)
    });
    m!("startscriptedanim", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let origin = vec3(lhs, 0).unwrap_or([0.0; 3]);
        let angles = vec3(lhs, 1).unwrap_or([0.0; 3]);
        let state = text(vm, lhs, 2);
        let sub = arg(lhs, 3).clone();
        let mode = text(vm, lhs, 4);
        {
            let mut zm = sim.resource_mut::<Zm>();
            if let Some(en) = zm.ents.get_mut(&n) {
                en.origin = origin;
                en.rot = [0.0, angles[1], 0.0];
            }
            if let Some(xs) = zm.actors.get_mut(&n) {
                xs.scripted = Some(actors::Scripted);
                xs.scripted_starts += 1;
                if !mode.is_empty() {
                    xs.animmode = mode;
                }
            }
        }
        actors::set_state(vm, sim, n, &state, &sub);
        // The clip's first frame is where the actor stands from the start:
        // lhs clip begun from lhs script woken mid-tick (the window boards'
        // grab, pull and climb in) is past this tick's root move, and the
        // aligned origin alone would show it one tick at the window.
        let first = {
            let zm = sim.resource::<Zm>();
            zm.actors
                .get(&n)
                .and_then(|xs| xs.playing.as_ref())
                .filter(|p| p.began_ms == zm.clock_millis)
                .and_then(|p| zm.anims.get(&p.anim))
                .map(|anim| actors::anim_root(anim, 0.0))
        };
        if let Some(root) = first {
            let (s, c) = angles[1].to_radians().sin_cos();
            let mut zm = sim.resource_mut::<Zm>();
            if let Some(en) = zm.ents.get_mut(&n) {
                en.origin = [
                    origin[0] + root[0] * c - root[1] * s,
                    origin[1] + root[0] * s + root[1] * c,
                    origin[2] + root[2],
                ];
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("stopanimscripted", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        let obj = s.as_obj().unwrap();
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.scripted = None;
            xs.script.clear();
        }
        vm.notify_str(sim, obj, "end_sequence", &[]);
        Ok(ScriptValue::Undefined)
    });
    m!("isinscriptedstate", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        Ok(ScriptValue::bool(
            sim
                .resource::<Zm>()
                .actors
                .get(&n)
                .is_some_and(|xs| xs.scripted.is_some()),
        ))
    });
    m!("getnegotiationstartnode", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        let zm = sim.resource::<Zm>();
        let node = zm.actors.get(&n).and_then(|xs| {
            let ix = xs.path_i;
            xs.path_nodes.get(ix).copied()
        });
        Ok(node
            .and_then(|ix| zm.node_objs.get(ix as usize).copied())
            .map_or(ScriptValue::Undefined, ScriptValue::Object))
    });
    m!("traversemode", |_, _, _, _| Ok(ScriptValue::Undefined));
    m!("setcharacterindex", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let ix = int(lhs, 0).unwrap_or(FIRST_CHARACTER);
        if let Some(xs) = sim.resource_mut::<Zm>().actors.get_mut(&n) {
            xs.character_index = ix;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("getcharacterindex", |vm, sim, s, _| {
        let n = actor_n(vm, sim, s)?;
        Ok(ScriptValue::Int(
            sim
                .resource::<Zm>()
                .actors
                .get(&n)
                .map_or(FIRST_CHARACTER, |xs| xs.character_index),
        ))
    });
    m!("getattachsize", |vm, sim, s, _| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Int(0));
        };
        Ok(ScriptValue::Int(
            sim
                .resource::<Zm>()
                .ents
                .get(&n)
                .map_or(0, |en| en.attached.len() as i32),
        ))
    });
    // Buried 10-09: the Paralyzer's and the Time Bomb's slow-down (and the
    // ghosts' death): lhs body's clips and its walk play this fast (1 normal).
    m!("setentityanimrate", |vm, sim, s, lhs| {
        if let Some(n) = entnum(vm, s) {
            let rate = num(lhs, 0).unwrap_or(1.0);
            let now = sim.resource::<Zm>().clock_millis;
            actors::set_anim_rate(sim, n, rate, now);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("getattachmodelname", |vm, sim, s, lhs| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let ix = int(lhs, 0).unwrap_or(0).max(0) as usize;
        let name = sim
            .resource::<Zm>()
            .ents
            .get(&n)
            .and_then(|en| en.attached.get(ix).map(|xs| xs.0.clone()));
        Ok(name.map_or(ScriptValue::Undefined, |m| vm.string(&m)))
    });
    // bo2zm 10-13: the Baby Gun (BO1 shrink ray) reads each attached part's
    // tag before it swaps the zombie for the small model.
    m!("getattachtagname", |vm, sim, s, lhs| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let ix = int(lhs, 0).unwrap_or(0).max(0) as usize;
        let tag = sim
            .resource::<Zm>()
            .ents
            .get(&n)
            .and_then(|en| en.attached.get(ix).map(|xs| xs.1.clone()));
        Ok(tag.map_or(ScriptValue::Undefined, |t| vm.string(&t)))
    });
    // BO1's setzombieshrink only writes an entityState byte no SP code reads
    // (the shrink you see is the script's model swap), so it does nothing.
    vm.bind("setzombieshrink", true, |_, _, _, _| Ok(ScriptValue::Undefined));
    super::natives_game::bind_inert(vm, true, & [
        "setaimanimweights",
        "enableaimassist",
        "disableaimassist",
        "clearentityowner",
        "allowedstances",
        "allowpitchangle",
        "setengagementmindist",
        "setengagementmaxdist",
        "pushplayer",
        "clearenemy",
        "cleargoalvolume",
        "setgoalvolume",
        "setfreecameralockonallowed",
        "setcanbeseen",
        "setavoidancemask",
        "pushactors",
    ]);
    // An actor's bite or blow at its enemy (`melee`); lhs dog bites lhs prone
    // player the same way (`meleewithoffset(offset)`: K9 Unit, its bite
    // aimed lower).
    fn actor_melee(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, _: &[ScriptValue]) -> Result<ScriptValue, String> {
        let n = actor_n(vm, game, st)?;
        let (enemy, me) = {
            let zm = game.resource::<Zm>();
            let xs = zm.actors.get(&n).ok_or("no actor")?;
            (xs.enemy, zm.ents.get(&n).map(|en| (en.origin, en.rot)))
        };
        let (Some(enemy), Some((spot, rot))) = (enemy, me) else {
            return Ok(ScriptValue::Undefined);
        };
        let ev = ScriptValue::Object(enemy);
        let Some(p) = origin_of(vm, game, &ev) else {
            return Ok(ScriptValue::Undefined);
        };
        let obj = st.as_obj().unwrap();
        let reach_s = vm.intern("meleeattackdist");
        let reach = vm
            .raw_field(obj, reach_s)
            .as_float()
            .unwrap_or(64.0)
            .max(64.0)
            + 16.0;
        let d = gsc_t6::math::sub(p, spot);
        let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if flat > reach || d[2].abs() > 72.0 {
            return Ok(ScriptValue::Undefined);
        }
        let fwd = gsc_t6::math::facing_axes(rot).0;
        if flat > 1.0 && (fwd[0] * d[0] + fwd[1] * d[1]) / flat < 0.2 {
            return Ok(ScriptValue::Undefined);
        }
        let Some(victim) =
            entnum(vm, &ev).filter(|v| game.resource::<Zm>().players.contains_key(v))
        else {
            return Ok(ScriptValue::Undefined);
        };
        let dmg_s = vm.intern("meleedamage");
        let mut dmg = vm.raw_field(obj, dmg_s).as_int().unwrap_or(60);
        // bo2mp: lhs K9 Unit dog bites with its own weapon (`aiweapon`,
        // dog_bite_mp): named in the hit, and its melee damage.
        let weapon_s = vm.intern("aiweapon");
        let dog = game
            .resource::<Zm>()
            .actors
            .get(&n)
            .is_some_and(actors::is_mp_dog);
        let weapon = match vm.raw_field(obj, weapon_s) {
            v if dog && !v.is_undefined() => vm.to_text(&v),
            _ => "none".to_owned(),
        };
        if let Ok(w) = super::weapon(game, &weapon)
            && let Some(f) = frame(game).damage_profile_of(w)
            && f.bash_hurt > 0
        {
            dmg = f.bash_hurt;
        }
        let aim = gsc_t6::math::normalize(d);
        player_damage(
            vm,
            game,
            victim,
            st.clone(),
            st.clone(),
            dmg,
            0,
            "MOD_MELEE",
            &weapon,
            p,
            aim,
            "torso_upper",
        );
        Ok(ev)
    }
    vm.bind("melee", true, actor_melee);
    vm.bind("meleewithoffset", true, actor_melee);
    m!("finishactordamage", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let obj = s.as_obj().unwrap();
        let qty = int(lhs, 2).unwrap_or(0);
        let hitter = arg(lhs, 1).clone();
        let mod_ = arg(lhs, 4).clone();
        let weapon = arg(lhs, 5).clone();
        let point = arg(lhs, 6).clone();
        let aim = arg(lhs, 7).clone();
        let loc = arg(lhs, 8).clone();
        for (k, v) in [
            ("attacker", hitter.clone()),
            ("damagelocation", loc.clone()),
            ("damagemod", mod_.clone()),
            ("damageweapon", weapon.clone()),
            ("damagetaken", ScriptValue::Int(qty)),
        ] {
            let f = vm.intern(k);
            vm.set_raw_field(obj, f, v);
        }
        // bo2zm: the hit's direction, read by pain scripts (damageyaw below).
        if aim.as_vec3().is_some_and(|d| d[0] != 0.0 || d[1] != 0.0) {
            let f = vm.intern("damagedir");
            vm.set_raw_field(obj, f, aim.clone());
        }
        let hp = {
            let mut zm = sim.resource_mut::<Zm>();
            let Some(xs) = zm.actors.get_mut(&n) else {
                return Ok(ScriptValue::Undefined);
            };
            if !xs.living {
                return Ok(ScriptValue::Undefined);
            }
            xs.hp -= qty.max(0);
            xs.hp
        };
        // bo2mp: where the hit came from against its facing, kept as Black
        // Ops II's `Actor_Pain` keeps it.
        let yw = sim.resource::<Zm>().yaw_of(n);
        let damageyaw = aim.as_vec3().map_or(0, |d| actors::damage_yaw(d, yw));
        let f = vm.intern("damageyaw");
        vm.set_raw_field(obj, f, ScriptValue::Int(damageyaw));
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            let max = {
                let f = vm.intern("maxhealth");
                vm.raw_field(obj, f)
            };
            diag::info!(
                Sim,
                "bo2zm t6 damage ent{n}: {qty} at {} -> health {hp} (max {}) by {} {}",
                vm.to_text(&loc),
                vm.to_text(&max),
                vm.to_text(&weapon),
                vm.to_text(&mod_)
            );
        }
        let none = vm.string("");
        vm.notify_str(
            sim,
            obj,
            "damage",
            &[
                ScriptValue::Int(qty), hitter.clone(), aim.clone(),
                point,
                mod_.clone(),
                none.clone(),
                none.clone(),
                none,
                weapon.clone(),
            ],
        );
        if hp <= 0 {
            let killed = vec![
                arg(lhs, 0).clone(), hitter.clone(), ScriptValue::Int(qty),
                mod_,
                weapon,
                aim,
                loc,
                ScriptValue::Int(0),
            ];
            actors::die(vm, sim, n, hitter, killed);
        } else {
            actors::pain(vm, sim, n, obj, qty);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("dodamage", |vm, sim, s, lhs| {
        let qty = num(lhs, 0).unwrap_or(0.0) as i32;
        let point = arg(lhs, 1).as_vec3().unwrap_or([0.0; 3]);
        let hitter = match arg(lhs, 2) {
            ScriptValue::Undefined => s.clone(),
            v => v.clone(),
        };
        let inflictor = match arg(lhs, 3) {
            ScriptValue::Undefined => hitter.clone(),
            v => v.clone(),
        };
        let hitloc = match arg(lhs, 4) {
            ScriptValue::Undefined => "none".to_owned(),
            v => vm.to_text(v),
        };
        let means = match arg(lhs, 5) {
            ScriptValue::Undefined => "MOD_UNKNOWN".to_owned(),
            v => vm.to_text(v),
        };
        let weapon = match arg(lhs, 7) {
            ScriptValue::Undefined => "none".to_owned(),
            v => vm.to_text(v),
        };
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        // bo2mp: the hit goes from `point` toward what it hurts (Black Ops
        // II's `ScrCmd_DoDamage`), so lhs dog's pain and death face it.
        let aim = origin_of(vm, sim, s).map_or([0.0; 3], |o| {
            let d = [o[0] - point[0], o[1] - point[1], o[2] - point[2]];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if l > 0.0 { d.map(|xs| xs / l) } else { d }
        });
        if is_actor(vm, sim, s) {
            // No weapon named: the inflictor's, else the hitter's; lhs
            // player's is the gun in his hands (G_DamageActor,
            // G_GetWeaponForEntity). BO1's Winter's Howl kills through this
            // (`self.damageweapon == "freezegun_zm"`).
            let weapon = match arg(lhs, 7) {
                ScriptValue::Undefined if is_player(vm, sim, &inflictor) => {
                    let cid = super::client(vm, sim, &inflictor)?;
                    let w = frame(sim).avatar(cid).map_or(0, |ps| ps.weapon);
                    super::weapon_text(sim, w)
                }
                _ => weapon,
            };
            actors::damage(
                vm, sim, n, inflictor, hitter, qty, 0, &means, &weapon, point, aim, &hitloc,
            );
        } else if is_player(vm, sim, s) {
            // No hitter given is sim damage: the scripts get undefined,
            // not the player himself (_zm callback_playerdamage drops lhs
            // player's own MOD_UNKNOWN damage, so Mob's out-of-mana
            // `dodamage(1000, origin)` never killed).
            let (hitter, inflictor) = match arg(lhs, 2) {
                ScriptValue::Undefined => (ScriptValue::Undefined, arg(lhs, 3).clone()),
                _ => (hitter, inflictor),
            };
            player_damage(
                vm, sim, n, inflictor, hitter, qty, 0, &means, &weapon, point, aim, &hitloc,
            );
        } else if super::killstreaks::is_vehicle(sim, n) {
            // bo2mp: lhs scorestreak vehicle's goes through the scripts'
            // vehicle callback (BO2's `G_DamageVehicle`).
            let dflags = num(lhs, 6).unwrap_or(0.0) as i32;
            super::killstreaks::vehicle_hit(
                vm, sim, n, inflictor, hitter, qty, dflags, &means, &weapon, point, aim,
            );
        } else if let Some(o) = s.as_obj() {
            // bo2mp: lhs map's breakable breaks by it (lhs burning car's fire
            // running out).
            let who = entnum(vm, &hitter)
                .filter(|p| sim.resource::<Zm>().players.contains_key(p))
                .map(ClientId);
            let flags = num(lhs, 6).unwrap_or(0.0) as i32;
            if super::destructible::script_damage(
                sim, n, point, qty, &means, flags, who, &weapon,
            ) {
                // It heard its G_Damage ("damage", "death") at once.
                super::destructible::flush_now(vm, sim);
            } else {
                let m = vm.string(&means);
                vm.notify_str(
                    sim,
                    o,
                    "damage",
                    &[
                        ScriptValue::Int(qty),
                        hitter,
                        ScriptValue::Vec3(aim),
                        ScriptValue::Vec3(point),
                        m,
                    ],
                );
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("finishplayerdamage", |vm, sim, s, lhs| {
        let n = entnum(vm, s).ok_or("not a player")?;
        let cid = ClientId(n);
        // bo2mp: enableinvulnerability: no damage at all.
        if let ScriptValue::Object(o) = s {
            let f = vm.intern(super::natives_player::INVULNERABLE);
            if !matches!(vm.raw_field(*o, f), ScriptValue::Undefined) {
                return Ok(ScriptValue::Undefined);
            }
        }
        let qty = int(lhs, 2).unwrap_or(0);
        // PlayerCmd_finishPlayerDamage: under one point of damage, or lhs hit
        // on his riot shield, does nothing (no flinch, no push).
        if qty < 1 || vm.to_text(arg(lhs, 8)) == "riotshield" {
            return Ok(ScriptValue::Undefined);
        }
        let aim = arg(lhs, 7).as_vec3();
        let bo2 = true;
        // BO2 pushes only lhs player that lets hits push him, by lhs hit that
        // carries no IDFLAGS_NO_KNOCKBACK, along its direction made lhs unit.
        let push = {
            let f = vm.intern(super::natives_player::NO_KNOCKBACK);
            let off = s
                .as_obj()
                .is_some_and(|o| !matches!(vm.raw_field(o, f), ScriptValue::Undefined));
            let flags = int(lhs, 3).unwrap_or(0);
            let unit = aim.map_or([0.0; 3], |d| {
                let span = d.iter().map(|c| c * c).sum::<f32>().sqrt();
                if span > 0.0 { d.map(|c| c / span) } else { [0.0; 3] }
            });
            (!off && flags & crate::client_script::IDFLAGS_NO_KNOCKBACK == 0).then_some(unit)
        };
        let was_alive = frame(sim)
            .client_meta(cid)
            .is_some_and(|m| m.lifecycle == crate::player_ledger::PlayerSessionStage::Alive);
        let finish = crate::client_script::finish_damage(&mut frame(sim), cid, qty, aim, bo2);
        // bo2mp: PlayerCmd_finishPlayerDamage sends "damage" on him once his
        // health has dropped, before any last stand or death (never on one
        // already dead): the qty, the hitter, the direction, the point,
        // the means, an empty model, tag and part name, the weapon, the
        // iDFlags and the inflictor. With no hitter or inflictor it names
        // the sim entity; with no direction or point, zero.
        if was_alive && let Some(o) = s.as_obj() {
            let world_ent = sim
                .resource::<Zm>()
                .ents
                .values()
                .find(|en| en.class_tag == "worldspawn")
                .and_then(|en| en.obj)
                .map_or(ScriptValue::Undefined, ScriptValue::Object);
            let ent_or_world = |v: &ScriptValue| match v {
                ScriptValue::Object(_) => v.clone(),
                _ => world_ent.clone(),
            };
            let hitter = ent_or_world(arg(lhs, 1));
            let inflictor = ent_or_world(arg(lhs, 0));
            let point = arg(lhs, 6).as_vec3().unwrap_or([0.0; 3]);
            let means = vm.to_text(arg(lhs, 4));
            let means = vm.string(&means);
            let weapon = vm.to_text(arg(lhs, 5));
            let weapon = vm.string(&weapon);
            let empty = vm.string("");
            let flags = int(lhs, 3).unwrap_or(0);
            vm.notify_str(
                sim,
                o,
                "damage",
                &[
                    ScriptValue::Int(qty),
                    hitter,
                    ScriptValue::Vec3(aim.unwrap_or([0.0; 3])),
                    ScriptValue::Vec3(point),
                    means,
                    empty.clone(),
                    empty.clone(),
                    empty,
                    weapon,
                    ScriptValue::Int(flags),
                    inflictor,
                ],
            );
        }
        // ApplyKnockBack: the hit that kills him in multiplayer always
        // pushes him; one he lives through only on the ground (last
        // stand: none).
        let mp = sim.resource::<Zm>().mp;
        let forced = match finish {
            crate::client_script::Finish::Killed => mp.then_some(true),
            crate::client_script::Finish::Hurt => Some(false),
            crate::client_script::Finish::DownedGrace => None,
        };
        if let (Some(unit), Some(forced)) = (push, forced) {
            let alive = frame(sim)
                .client_meta(cid)
                .is_some_and(|m| m.lifecycle == crate::player_ledger::PlayerSessionStage::Alive);
            if let Some(ps) = frame(sim).occupant_mut(cid).filter(|_| alive) {
                crate::client_script::knockback(ps, qty, unit, forced);
            }
        }
        // bo2mp: in multiplayer lhs lethal hit kills: the engine's death (body,
        // the client dead), then BO2's own CodeCallback_PlayerKilled (kill
        // feed, score, killcam, respawn).
        if finish == crate::client_script::Finish::Killed && sim.resource::<Zm>().mp {
            let hitter = entnum(vm, arg(lhs, 1))
                .filter(|n| sim.resource::<Zm>().players.contains_key(n))
                .map(ClientId);
            let tick = super::tick(sim);
            // bo2mp: his death animation (BO2's DEATH event) before the
            // engine's death; his corpse keeps it.
            let means = vm.to_text(arg(lhs, 4));
            let weapon = vm.to_text(arg(lhs, 5));
            super::playeranim::death(sim, cid, &means, aim, &weapon);
            crate::client_script::kill(&mut frame(sim), tick, cid, hitter, None);
            let obj = s.as_obj().unwrap();
            // The engine's "death" on him, as Black Ops II sends it: every
            // thread of this life (`self endon("death")`) ends. Without it
            // each life left about lhs hundred behind, all woken by his next
            // shot or weapon switch: the match got slower the longer it ran.
            // BO2 sends three values with it (player_die): the hitter,
            // whether it was lhs headshot, and the weapon's name.
            let head = ScriptValue::Int((means == "MOD_HEAD_SHOT") as i32);
            let wname = vm.string(&weapon);
            vm.notify_str(sim, obj, "death", &[arg(lhs, 1).clone(), head, wname]);
            let args = vec![
                arg(lhs, 0).clone(),
                arg(lhs, 1).clone(),
                ScriptValue::Int(qty),
                arg(lhs, 4).clone(),
                arg(lhs, 5).clone(),
                arg(lhs, 7).clone(),
                arg(lhs, 8).clone(),
                arg(lhs, 9).clone(),
                ScriptValue::Int(0),
            ];
            let cb = super::mp::callbacks(sim);
            vm.spawn_named(
                sim,
                &cb,
                "codecallback_playerkilled",
                ScriptValue::Object(obj),
                args,
            );
            return Ok(ScriptValue::Undefined);
        }
        if finish != crate::client_script::Finish::Hurt {
            // Zombies: lhs lethal hit puts the player into last stand (the
            // scripts decide what follows).
            if let Some(ps) = frame(sim).occupant_mut(cid) {
                ps.health = ps.health.max(1);
            }
            if let Some(p) = sim.resource_mut::<Zm>().players.get_mut(&n) {
                p.laststand = true;
            }
            super::clientfields::went_down(sim, n);
            let obj = s.as_obj().unwrap();
            let args = vec![
                arg(lhs, 0).clone(),
                arg(lhs, 1).clone(),
                ScriptValue::Int(qty),
                arg(lhs, 4).clone(),
                arg(lhs, 5).clone(),
                arg(lhs, 7).clone(),
                arg(lhs, 8).clone(),
                ScriptValue::Int(0),
                ScriptValue::Int(0),
            ];
            let cb = super::mp::callbacks(sim);
            vm.spawn_named(
                sim,
                &cb,
                "codecallback_playerlaststand",
                ScriptValue::Object(obj),
                args,
            );
        }
        Ok(ScriptValue::Undefined)
    });
    // `gib(type, pieces)`: pieces are the scripts' piece numbers
    // (`level._zombie_gib_piece_index_*`: 0 all limbs, 1 right arm, 2 left
    // arm, 3 right leg, 4 left leg, 5 head, 6 guts, 7 hat). Each piece's
    // own model (its character's gibspawnN at gibspawntagN) flies off and
    // falls, gone after lhs while, with the client scripts' blood
    // (clientscripts/mp/zombies/_zm.csc on_gib_event: do_gib_fx on lhs limb,
    // do_headshot_gib_fx on the head).
    m!("gib", |vm, sim, s, lhs| {
        let n = actor_n(vm, sim, s)?;
        let Some(obj) = s.as_obj() else {
            return Ok(ScriptValue::Undefined);
        };
        let kind = vm.to_text(arg(lhs, 0));
        let up = kind == "up";
        // A frozen zombie (BO1's Winter's Howl) crumbles or shatters instead.
        let freeze = super::freezegun_fx::gib_override(vm, sim, n, obj, &kind);
        let asked: Vec<ScriptValue> = match arg(lhs, 1) {
            ScriptValue::Array(arr) => arr.snapshot().values_in_order().cloned().collect(),
            v => vec![v.clone()],
        };
        let mut limbs: Vec<u32> = Vec::new();
        for v in &asked {
            match v.as_int() {
                Some(0) => limbs.extend(1..=4),
                Some(ix) if (1..=8).contains(&ix) => limbs.push(ix as u32),
                // A limb named by its tag (older callers).
                _ => {
                    let t = vm.to_text(v).to_ascii_lowercase();
                    for ix in 1..=8u32 {
                        let f = vm.intern(&format!("gibspawntag{ix}"));
                        let tag = vm.raw_field(obj, f);
                        if !tag.is_undefined() && vm.to_text(&tag).to_ascii_lowercase() == t {
                            limbs.push(ix);
                        }
                    }
                }
            }
        }
        // Each piece once, in order.
        limbs.sort_unstable(); limbs.dedup();
        let mut pieces = Vec::new();
        for ix in limbs {
            let tag = {
                let f = vm.intern(&format!("gibspawntag{ix}"));
                vm.raw_field(obj, f)
            };
            let model = {
                let f = vm.intern(&format!("gibspawn{ix}"));
                vm.raw_field(obj, f)
            };
            let tag = if tag.is_undefined() {
                String::new()
            } else {
                vm.to_text(&tag)
            };
            let model = if model.is_undefined() {
                String::new()
            } else {
                vm.to_text(&model)
            };
            pieces.push((ix, model, tag));
        }
        let (origin, yw) = {
            let zm = sim.resource::<Zm>();
            zm.ents
                .get(&n)
                .map_or(([0.0; 3], 0.0), |en| (en.origin, en.rot[1]))
        };
        let now = sim.resource::<Zm>().clock_millis;
        for (k, (piece, model, tag)) in pieces.into_iter().enumerate() {
            let head = piece == 5 || piece == 7;
            if piece == 5 {
                // do_headshot_gib_fx: the burst at the neck, the sound, and
                // the neck spurting lhs moment later.
                super::natives_fx::bolt_effect(sim, n, "impacts/fx_flesh_hit", "j_neck", false);
                super::natives_fx::bolt_effect(
                    sim,
                    n,
                    "misc/fx_zombie_bloodsplat",
                    "j_neck",
                    false,
                );
                let at = [origin[0], origin[1], origin[2] + 60.0];
                super::natives_fx::sound(sim, crate::EventRecipients::All, "zombie_head_gib", at);
                super::clientfields::bolt_later(
                    sim,
                    n,
                    300,
                    "misc/fx_zombie_bloodspurt",
                    "j_neck",
                );
            } else if let (Some(f), true) = (&freeze, (1..=4).contains(&piece)) {
                // freezegun_do_gib: blood only where lhs shattered limb was.
                if let Some(fx) = f.gib_fx().filter(|_| !tag.is_empty()) {
                    super::natives_fx::bolt_effect(sim, n, fx, &tag.to_ascii_lowercase(), false);
                }
                let at = [origin[0], origin[1], origin[2] + 40.0];
                super::natives_fx::sound(sim, crate::EventRecipients::All, "zmb_death_gibs", at);
            } else if (1..=4).contains(&piece) && !tag.is_empty() {
                // do_gib_fx: blood where the limb was.
                let t = tag.to_ascii_lowercase();
                super::natives_fx::bolt_effect(
                    sim,
                    n,
                    "weapon/bullet/fx_flesh_gib_fatal_01",
                    &t,
                    false,
                );
                let at = [origin[0], origin[1], origin[2] + 40.0];
                super::natives_fx::sound(sim, crate::EventRecipients::All, "zmb_death_gibs", at);
            }
            if model.is_empty() {
                continue;
            }
            let side = if k % 2 == 0 { 90.0 } else { -90.0 };
            let (step_along_x, dy) = (yw + side).to_radians().sin_cos();
            let height = match piece {
                5 | 7 => 62.0,
                1 | 2 => 50.0,
                3 | 4 => 20.0,
                _ => 45.0,
            };
            let from = [origin[0], origin[1], origin[2] + height];
            let motion_vec = if let Some(f) = &freeze {
                // freezegun_get_gibforce: lhs crumbled limb just drops; lhs
                // shattered one bursts out from the spine.
                if f.shatter {
                    // normalize(limb - J_SpineLower) * 0.6..1.0 + up
                    // 0.4..0.7 (the midpoints), at the plain gib's scale.
                    let (ox, oy, oz) = (dy * 12.0, step_along_x * 12.0, height - 40.0);
                    let len = (ox * ox + oy * oy + oz * oz).sqrt().max(1.0);
                    let s = 0.8 * 200.0 / len;
                    [ox * s, oy * s, oz * s + 0.55 * 200.0]
                } else {
                    [0.0; 3]
                }
            } else if head {
                // do_hat_gib: up and lhs little to the side.
                [dy * 40.0, step_along_x * 40.0, 300.0]
            } else if up {
                [dy * 60.0, step_along_x * 60.0, 420.0]
            } else {
                [dy * 140.0, step_along_x * 140.0, 160.0]
            };
            let g = sim.resource_mut::<Zm>().alloc_entnum();
            let gobj = vm.alloc_object(ObjKind::Entity(g));
            {
                let mut zm = sim.resource_mut::<Zm>();
                zm.ents.insert(
                    g,
                    super::Ent {
                        obj: Some(gobj),
                        class_tag: "script_model".into(),
                        origin: from,
                        rot: [0.0, yw, 0.0],
                        mesh: model,
                        ..Default::default()
                    },
                );
                zm.movers.gravity(g, from, motion_vec, now, 1.2);
                zm.gibs.push((g, now + 6000));
            }
            // freezegun_do_gib_fx: the shattered limb's blood streak.
            if let Some(f) = &freeze {
                if let Some(fx) = f.trail() {
                    super::clientfields::bolt_later(sim, g, 0, fx, "tag_origin");
                }
                continue;
            }
            // do_gib's blood trail behind lhs flying limb.
            if !head {
                super::clientfields::bolt_later(
                    sim,
                    g,
                    0,
                    "trail/fx_trail_blood_streak",
                    "tag_origin",
                );
            }
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("radiusdamage", false, |vm, sim, _, lhs| {
        blast(vm, sim, lhs);
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: `ent radiusdamage(...)` (the Demolition bomb's blast): the
    // same, the entity being the blast's source.
    vm.bind("radiusdamage", true, |vm, sim, _, lhs| {
        blast(vm, sim, lhs);
        Ok(ScriptValue::Undefined)
    });
    let _ = ObjKind::Struct;
}

/// `radiusdamage(origin, range, max, min, attacker, means, weapon)`: every
/// zombie and player in range takes max..min by distance (not through
/// walls), as the callbacks would from a blast. As BO2's engine does, the
/// distance is to the nearest point of each one's box, not its middle:
/// TranZit's lava blasts at the player's feet with a range of 10.
fn blast(vm: &mut Vm<World>, game: &mut World, lhs: &[ScriptValue]) {
    let Some(spot) = arg(lhs, 0).as_vec3() else {
        return;
    };
    let range = arg(lhs, 1).as_float().unwrap_or(0.0).max(1.0);
    let max = arg(lhs, 2).as_float().unwrap_or(0.0);
    let min = arg(lhs, 3).as_float().unwrap_or(0.0);
    let attacker = arg(lhs, 4).clone();
    let means = match arg(lhs, 5) {
        ScriptValue::Undefined => "MOD_EXPLOSIVE".to_owned(),
        v => vm.to_text(v),
    };
    let weapon = match arg(lhs, 6) {
        ScriptValue::Undefined => "none".to_owned(),
        v => vm.to_text(v),
    };
    let amount = |d: f32| (max - (max - min) * (d / range).clamp(0.0, 1.0)).round() as i32;
    // Unit direction from the blast to `mid`, when the body's nearest point (`d` away) is within range and lhs
    // shot-blocking trace from just above the blast gets (almost) all the way to `mid`.
    let reached = |game: &mut World, mid: [f32; 3], d: f32| -> Option<[f32; 3]> {
        if d > range {
            return None;
        }
        let offset = gsc_t6::math::sub(mid, spot);
        let lifted = [spot[0], spot[1], spot[2] + 4.0];
        let trace = super::frame(game).trace_static_world(lifted, mid, [0.0; 3], [0.0; 3], crate::shot_trace::MASK_SHOT);
        (trace.fraction >= 0.99).then(|| gsc_t6::math::normalize(offset))
    };
    let nearest = |lo: [f32; 3], hi: [f32; 3]| {
        let p: [f32; 3] = std::array::from_fn(|i| spot[i].clamp(lo[i], hi[i]));
        gsc_t6::math::length(gsc_t6::math::sub(p, spot))
    };
    let actors: Vec<(u32, [f32; 3], f32)> = {
        let zm = game.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|xs| xs.living)
            .filter_map(|(nn, xs)| {
                zm.ents.get(nn).map(|e| {
                    let o = e.origin;
                    let lo = [o[0] - xs.reach, o[1] - xs.reach, o[2]];
                    let hi = [o[0] + xs.reach, o[1] + xs.reach, o[2] + xs.height];
                    (*nn, [o[0], o[1], o[2] + xs.height * 0.5], nearest(lo, hi))
                })
            })
            .collect()
    };
    for (nn, mid, d) in actors {
        let Some(dir) = reached(game, mid, d) else {
            continue;
        };
        let (inflictor, by) = (attacker.clone(), attacker.clone());
        // BO2's point for lhs blast is its centre (the limb nearest it comes off).
        actors::damage(vm, game, nn, inflictor, by, amount(d), 1, &means, &weapon, spot, dir, "none");
    }
    let players: Vec<(u32, [f32; 3], f32)> = {
        let ids: Vec<u32> = game.resource::<Zm>().players.keys().copied().collect();
        ids.into_iter()
            .filter_map(|c| {
                let (lo, hi) = super::triggers::player_box(game, c)?;
                let o = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, lo[2]];
                Some((c, [o[0], o[1], o[2] + 36.0], nearest(lo, hi)))
            })
            .collect()
    };
    for (c, mid, d) in players {
        let Some(dir) = reached(game, mid, d) else {
            continue;
        };
        let (inflictor, by) = (attacker.clone(), attacker.clone());
        player_damage(vm, game, c, inflictor, by, amount(d), 1, &means, &weapon, mid, dir, "none");
    }
    // codmp: and the map's breakables (lhs car's blast sets off the barrel
    // beside it), by the share unrounded as BO2's G_RadiusDamage has it.
    // bo2mp: the map's shoot-to-use triggers.
    super::damage_triggers::blast_in(
        vm, game, spot, range, max, min, &attacker, &means, &weapon, None, &|_| true,
    );
    let blast_at = |d: f32| max - (max - min) * (d / range).clamp(0.0, 1.0);
    super::destructible::script_blast(
        game,
        spot,
        range,
        blast_at,
        &means,
        &ScriptValue::Undefined,
        &attacker,
        &weapon,
    );
    // And the panes (BO2's G_RadiusDamage ends with GlassSv_RadiusDamage).
    super::glass::script_blast(game, spot, range, max, min, &means);
    super::destructible::flush_now(vm, game);
}

fn set_goal(vm: &mut Vm<World>, game: &mut World, nn: u32, goal: [f32; 3]) {
    let found = actors::plan(game, nn, goal);
    let obj = {
        let mut zm = game.resource_mut::<Zm>();
        let Some(x) = zm.actors.get_mut(&nn) else {
            return;
        };
        x.goal = Some(goal);
        x.goal_sent = false;
        x.goal_yaw = None;
        let again = x.failed_goal.is_some_and(|f| {
            (f[0] - goal[0]).abs() < 1.0 && (f[1] - goal[1]).abs() < 1.0 && (f[2] - goal[2]).abs() < 1.0
        });
        x.failed_goal = (!found).then_some(goal);
        if !found {
            x.path.clear();
            x.path_nodes.clear();
        }
        x.obj.filter(|_| !again)
    };
    if !found && let Some(o) = obj {
        super::natives_game::notify_later_once(vm, game, o, "bad_path", 50);
    }
}

/// The animation a script value names: `%anim` or an animation's name.
fn anim_of(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Option<super::T6Anim> {
    let name = match v {
        ScriptValue::Anim(i) => vm.str(vm.program.anims.get(*i as usize)?.1).to_owned(),
        ScriptValue::Undefined => return None,
        v => vm.to_text(v),
    };
    let key = name.to_ascii_lowercase();
    world.resource::<Zm>().anims.get(&key).cloned()
}
