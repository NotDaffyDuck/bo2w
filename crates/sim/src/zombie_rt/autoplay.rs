//! BO2MP_T6_AUTOPLAY=1: a test player for headless runs. Client 0 aims at
//! the nearest zombie it can see (its head box), fires (pressing and letting
//! go, so a pistol fires again), and backs away from one that comes close;
//! an empty magazine reloads itself when it fires. Its commands replace the
//! client's own for the tick. Every five seconds it logs the round, its
//! points and health and the zombies alive.

use super::pairs::KeepByValue;
use std::sync::OnceLock;

use bevy_ecs::prelude::World;
use playerstate_t6::game::{InputCommand, buttons};

use super::{Zm, frame, with_vm};
use crate::sim_parts::ClientId;

pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_T6_AUTOPLAY").is_ok_and(|v| v == "1" || v == "2"))
}

/// BO2MP_T6_AUTOPLAY=2: the rounds test. The player takes no damage, is
/// given BO2MP_T6_AUTOPLAY_GUN (the M14 unless named) at 8 s and never runs
/// out of ammo, so the rounds go on and the game's rules get exercised.
pub(crate) fn rounds_test() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_T6_AUTOPLAY").is_ok_and(|v| v == "2"))
}

/// Applies `edit` to this tick's queued command for `client` (to every one, should there be several).
fn steer(game: &mut World, peer: ClientId, mut edit: impl FnMut(&mut InputCommand)) {
    let mut request = game.resource_mut::<crate::frame_step::FrameTicket>();
    request.input.cmds.iter_mut().filter(|(owner, _)| *owner == peer).for_each(|(_, cmd)| edit(cmd));
}

/// The numeric words of `text` in order; words that do not parse are skipped.
fn numbers(text: &str) -> Vec<f32> {
    text.split_whitespace().flat_map(str::parse).collect()
}

/// `text` read as exactly `N` numbers, or None when there are more or fewer.
fn tuple_of<const N: usize>(text: &str) -> Option<[f32; N]> {
    numbers(text).try_into().ok()
}

/// Environment variable `key` read as exactly `N` numbers.
fn env_tuple<const N: usize>(key: &str) -> Option<[f32; N]> {
    tuple_of(&std::env::var(key).ok()?)
}

/// Environment variable `key` read as `;`-separated groups of `N` numbers; a group of the wrong size is dropped.
fn env_point_list<const N: usize>(key: &str) -> Vec<[f32; N]> {
    let Ok(spec) = std::env::var(key) else {
        return Vec::new();
    };
    spec.split(';').filter_map(tuple_of).collect()
}

/// BO2MP_AUTOPLAY=1: the multiplayer test player. Client 0 aims at the
/// nearest enemy player it can see and fires, reloading when empty; with
/// none in sight it faces the nearest. It does not move. (So a headless run
/// shows his hit markers, score popups and kills.)
pub(crate) fn mp_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_AUTOPLAY").is_ok_and(|v| v == "1"))
}

/// The other side's living players: their chests.
fn enemies(world: &mut World, me: ClientId) -> Vec<[f32; 3]> {
    let objs: Vec<(u32, gsc_t6::ObjRef)> = world.resource::<Zm>().players.iter().map(|(c, p)| (*c, p.obj)).collect();
    let teams: Vec<(u32, String)> = with_vm(world, |vm, _| {
        let f = vm.intern("team");
        objs.iter().map(|(c, o)| (*c, vm.to_text(&vm.raw_field(*o, f)))).collect()
    })
    .unwrap_or_default();
    let mine = teams.iter().find(|(c, _)| *c == me.0).map(|(_, t)| t.clone()).unwrap_or_default();
    let f = frame(world);
    teams
        .iter()
        .filter(|(c, t)| *c != me.0 && *t != mine && (t == "allies" || t == "axis"))
        .filter_map(|(c, _)| {
            let p = f.avatar(ClientId(*c))?;
            (p.health > 0).then(|| [p.origin[0], p.origin[1], p.origin[2] + 44.0])
        })
        .collect()
}

fn drive_mp(world: &mut World, now: i64) {
    let peer = ClientId(0);
    let Some(pstate) = frame(world).avatar(peer).copied() else {
        return;
    };
    if 0 >= pstate.health {
        return;
    }
    let eye_at = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now];
    let targets = enemies(world, peer);
    let mut met: Vec<(f32, [f32; 3])> = Vec::new();
    for aim in &targets {
        let t = frame(world).trace_static_world(eye_at, *aim, [0.0; 3], [0.0; 3], crate::shot_trace::MASK_SHOT);
        if t.fraction >= 0.98 {
            met.push((flat_dist(*aim, pstate.origin), *aim));
        }
    }
    met.sort_by(|a, b| a.0.total_cmp(&b.0));
    let target = met.first().map(|s| s.1);
    let fire_phase = {
        let mut zm = world.resource_mut::<Zm>();
        zm.autoplay_fire = !zm.autoplay_fire;
        zm.autoplay_fire
    };
    // None in sight: face the nearest one (level), ready when he comes
    // round the corner. (Running at him gets stuck on the houses.)
    let nearest = targets.iter().min_by(|a, b| flat_dist(**a, pstate.origin).total_cmp(&flat_dist(**b, pstate.origin)));
    let view = match (target, nearest) {
        (Some(t), _) => look(eye_at, t),
        (None, Some(n)) => [0.0, look(eye_at, *n)[1], 0.0],
        _ => pstate.look_angles,
    };
    let (clip, stock) = {
        let fl = frame(world);
        (
            crate::client_script::ammo_clip(&fl, peer, pstate.weapon),
            crate::client_script::rounds_held(&fl, peer, pstate.weapon),
        )
    };
    // The hack menu's guest weapons, and one with no sights to aim down (the
    // ballistic knife), fire from the hip.
    let hip = frame(world).damage_profile_of(pstate.weapon).is_some_and(|fl| !fl.scoped_aiming);
    let guest = hip || super::mp_specials::owns_trigger(&super::weapon_text(world, pstate.weapon));
    let mut press = 0;
    if target.is_some() {
        press |= buttons::ADS;
    }
    if clip == 0 && stock > 0 {
        if fire_phase {
            press |= buttons::RELOAD;
        }
    } else if target.is_some() && (pstate.weapon_raise_fraction > 0.9 || guest) && fire_phase {
        press |= buttons::ATTACK;
    }
    let mut last_angles = None;
    {
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        for (id, cmd) in &mut req.input.cmds {
            if *id != peer {
                continue;
            }
            cmd.buttons = press;
            last_angles = Some(cmd.angles);
        }
    }
    if let Some(angles) = last_angles {
        let mut fl = frame(world);
        if let Some(p) = fl.occupant_mut(peer) {
            p.view_offset_angles = std::array::from_fn(|i| {
                let cmd_deg = (angles[i] as u16) as f32 * (360.0 / 65536.0);
                view[i] - cmd_deg
            });
            p.look_angles = view;
        }
    }
    if now % 5000 < i64::from(crate::SIM_STEP_MILLIS) {
        diag::info!(
            Sim,
            "bo2mp autoplay at {}s: health {}, ammo {clip}+{stock}, enemies alive {}, in sight {}",
            now / 1000,
            pstate.health,
            targets.len(),
            met.len()
        );
    }
}

/// BO2MP_T6_AUTOPLAY_SEEK=1: with none in shooting range the player turns
/// to the nearest zombie and walks up to it (Tranzit's depot sleepers).
fn seek() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_T6_AUTOPLAY_SEEK").is_ok_and(|v| v == "1"))
}

/// BO2MP_T6_AUTOPLAY_FROM=secs: the test player only takes over from then,
/// and its gun comes 8 s after (other test aids drive him before that).
fn from_millis_value() -> i64 {
    static MS: OnceLock<i64> = OnceLock::new();
    *MS.get_or_init(|| {
        std::env::var("BO2MP_T6_AUTOPLAY_FROM")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .map_or(0, |s| (s * 1000.0) as i64)
    })
}

/// BO2MP_T6_AUTOPLAY_UNTIL=secs: the test player lets go again then, so other
/// test aids drive him after (TranZit's EMP lamp hops after the tower kills).
fn deadline_millis() -> i64 {
    static MS: OnceLock<i64> = OnceLock::new();
    *MS.get_or_init(|| {
        std::env::var("BO2MP_T6_AUTOPLAY_UNTIL")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .map_or(i64::MAX, |s| (s * 1000.0) as i64)
    })
}

/// BO2MP_T6_AUTOPLAY_RANGE=units: the test player only shoots ones this near
/// (450 by default); TranZit's tower kills count within 420 of the tower.
fn fire_range() -> f32 {
    static R: OnceLock<f32> = OnceLock::new();
    *R.get_or_init(|| {
        std::env::var("BO2MP_T6_AUTOPLAY_RANGE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(450.0)
    })
}

/// BO2MP_T6_AUTOPLAY_KNIFE=1: the test player never shoots, only knifes the
/// ones that reach it (the knife's blood gets seen every run).
fn knife_only() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_T6_AUTOPLAY_KNIFE").is_ok_and(|v| v == "1"))
}

/// BO2MP_T6_AUTOPLAY_KNIFE=0: the test player never knifes, and shoots the
/// ones in reach too (Richtofen's tower step counts only explosive and
/// projectile kills inside the tower, where they crowd him).
fn never_knife() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_T6_AUTOPLAY_KNIFE").is_ok_and(|v| v == "0"))
}

/// BO2MP_T6_AUTOPLAY_FETCH=1: out of Ballistic Knife blades, the test player
/// walks to the nearest one lying about (as a player fetches his), and every
/// 5 s each blade's place, what it is stuck in and his distance is logged.
fn fetch() -> bool {
    static FETCH_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FETCH_ON.get_or_init(|| std::env::var("BO2MP_T6_AUTOPLAY_FETCH").is_ok_and(|v| v == "1"))
}

/// A Ballistic Knife blade lying about (its retrieve model).
struct Blade {
    ent: u32,
    at: [f32; 3],
    /// The entity it is linked to.
    on: Option<u32>,
}

/// The Ballistic Knife blades lying about.
fn knife_blades(game: &World) -> Vec<Blade> {
    game
        .resource::<Zm>()
        .ents
        .iter()
        .where_value(|en| en.mesh.starts_with("t5_weapon_ballistic_knife_blade"))
        .map(|(n, en)| Blade { ent: *n, at: en.origin, on: en.link.map(|l| l.0) })
        .collect()
}

pub(super) fn look(from: [f32; 3], goal: [f32; 3]) -> [f32; 3] {
    let d = [goal[0] - from[0], goal[1] - from[1], goal[2] - from[2]];
    let yaw = d[1].atan2(d[0]).to_degrees();
    let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let tilt = -d[2].atan2(flat).to_degrees();
    [tilt, yaw, 0.0]
}

fn flat_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Where to shoot a zombie: its chest box, else above its feet.
fn head_of(world: &mut World, n: u32, close: bool) -> Option<[f32; 3]> {
    let (id, origin) = {
        let zm = world.resource::<Zm>();
        (
            zm.presences.by_ent.get(&n).map(|s| s.id),
            zm.ents.get(&n)?.origin,
        )
    };
    let head = id.and_then(|id| {
        let f = frame(world);
        let row = f
            .entity_hit_support()
            .iter()
            .find(|row| row.owner.scripted_model() == Some(id))?;
        let bones = &row.dobj.as_ref()?.live_hitbox.as_ref()?.bones;
        // BO2MP_T6_AIM_PART=<part>[,<part>...]: those boxes (BO2's part
        // classes: 2 head, 4-6 torso, 13-16 legs, 17-18 feet).
        static AIM: OnceLock<Vec<u8>> = OnceLock::new();
        let aim = AIM.get_or_init(|| {
            std::env::var("BO2MP_T6_AIM_PART")
                .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
                .unwrap_or_default()
        });
        if let Some(b) = bones.iter().find(|b| aim.contains(&b.body_part_category)) {
            return Some(b.center);
        }
        // The head up close, else the chest (the biggest box, steady
        // while it walks).
        let (first, second) = if close { (2, 4) } else { (4, 2) };
        bones
            .iter()
            .find(|b| b.body_part_category == first)
            .or_else(|| bones.iter().find(|b| b.body_part_category == second))
            .map(|b| b.center)
    });
    Some(head.unwrap_or([origin[0], origin[1], origin[2] + 56.0]))
}

pub(crate) fn drive(world: &mut World, now: i64) {
    if mp_enabled() && world.resource::<Zm>().mp {
        drive_mp(world, now);
        return;
    }
    if !enabled() || now < from_millis_value() || now >= deadline_millis() {
        return;
    }
    // An BO2MP_T6_PLACE entry drives him meanwhile.
    if super::use_test::placing(now) {
        return;
    }
    let peer = ClientId(0);
    // The rounds test's gun and ammo; with BO2MP_T6_AUTOPLAY_GUN named the
    // damage-on player gets them too (lhs soak that lasts past round 9).
    if rounds_test() || std::env::var_os("BO2MP_T6_AUTOPLAY_GUN").is_some() {
        rounds_kit(world, now, peer);
    }
    let Some(pstate) = frame(world).avatar(peer).copied() else {
        return;
    };
    // Fire with equipment in hand sets it down (TranZit's turbine): he
    // holds still until he has lhs gun up again.
    if super::weapon_text(world, pstate.weapon as u32).starts_with("equip_") {
        return;
    }
    let eye = [
        pstate.origin[0],
        pstate.origin[1],
        pstate.origin[2] + pstate.eye_height_now,
    ];
    let living: Vec<(u32, [f32; 3])> = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living && lhs.scripted.is_none())
            .filter_map(|(nn, _)| zm.ents.get(nn).map(|en| (*nn, en.origin)))
            .collect()
    };
    // The nearest one in sight.
    let mut met: Vec<(f32, [f32; 3])> = Vec::new();
    for (nn, origin) in &living {
        let close = flat_dist(*origin, pstate.origin) < 350.0;
        let Some(head) = head_of(world, *nn, close) else {
            continue;
        };
        let t = frame(world).trace_static_world(
            eye,
            head,
            [0.0; 3],
            [0.0; 3],
            crate::shot_trace::MASK_SHOT,
        );
        if t.fraction >= 0.98 {
            met.push((flat_dist(*origin, pstate.origin), head));
        }
    }
    met.sort_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));
    let target = met.first().map(|s| s.1);
    let closest = living
        .iter()
        .map(|(_, o)| (flat_dist(*o, pstate.origin), *o))
        .min_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));
    let fire_phase = {
        let mut zm = world.resource_mut::<Zm>();
        zm.autoplay_fire = !zm.autoplay_fire;
        zm.autoplay_fire
    };
    // None in sight: face the nearest one (Tranzit's depot zombies lie
    // asleep until lhs player comes close; nothing comes to him).
    let mut view = match (target, closest) {
        (Some(t), _) => look(eye, t),
        (None, Some((_, o))) if seek() => {
            let v = look(eye, [o[0], o[1], o[2] + 30.0]);
            [0.0, v[1], 0.0]
        }
        _ => pstate.look_angles,
    };
    let (clip, stock) = {
        let fl = frame(world);
        (
            crate::client_script::ammo_clip(&fl, peer, pstate.weapon),
            crate::client_script::rounds_held(&fl, peer, pstate.weapon),
        )
    };
    // A gun with unlimited ammo (the Paralyzer) fires on an empty clip and is never reloaded.
    let clip = if frame(world).damage_profile_of(pstate.weapon).is_some_and(|fl| fl.unlimited_ammo) {
        clip.max(1)
    } else {
        clip
    };
    // Aim down the sights at one in range; shoot it when near enough to hit.
    let range = met.first().map_or(f32::MAX, |s| s.0);
    // A Jet Gun is held, not tapped: past lhs fifth of its spin it pulls and
    // grinds (`_zm_weap_jetgun`); lhs player lets go before it overheats.
    let jet = super::jetgun::owns_trigger(&super::weapon_text(world, pstate.weapon));
    let mut press = 0;
    // A gun that has no sights to aim down (the Thunder Gun) never gets
    // its ADS fraction up: lhs second into the press he shoots from the hip.
    static ADS_SINCE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);
    let mut aimed = pstate.weapon_raise_fraction > 0.9;
    if range < 700.0 && !jet {
        press |= buttons::ADS;
        let since = ADS_SINCE.load(std::sync::atomic::Ordering::Relaxed);
        if since == i64::MIN {
            ADS_SINCE.store(now, std::sync::atomic::Ordering::Relaxed);
        } else if now - since > 1000 && pstate.weapon_raise_fraction < 0.1 {
            aimed = true;
        }
    } else {
        ADS_SINCE.store(i64::MIN, std::sync::atomic::Ordering::Relaxed);
    }
    let close = closest.is_some_and(|(d, _)| d < 64.0) && !never_knife();
    // BO2MP_T6_AUTOPLAY_HOLD=<ms>: an upgraded staff's fire is held that long
    // to charge it, then let go for 0.4 s (t6::charge_shot).
    let hold = std::env::var("BO2MP_T6_AUTOPLAY_HOLD")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|_| frame(world).damage_profile_of(pstate.weapon).is_some_and(|fl| fl.charge_shot));
    if let Some(h) = hold.filter(|_| target.is_some() && range < fire_range() && aimed && clip > 0) {
        if now % (h + 400) < h {
            press |= buttons::ATTACK;
        }
    } else if jet && target.is_some() && range < fire_range() && super::jetgun::heat(world, peer.0) < 85.0 {
        press |= buttons::ATTACK;
    } else if close {
        // Knife one in reach (lhs fresh press each time).
        if fire_phase {
            press |= weapon_t6::melee_trace::BUTTON_MELEE;
        }
    } else if clip == 0 && stock > 0 {
        // Empty: reload (pressed every other tick, as lhs player taps it).
        if fire_phase {
            press |= buttons::RELOAD;
        }
    } else if target.is_some()
        && range < fire_range()
        && aimed
        && fire_phase
        && !knife_only()
    {
        press |= buttons::ATTACK;
    } else if seek()
        && target.is_none()
        && closest.is_some_and(|(d, _)| d < 2400.0)
        && clip > 0
        && now % 3000 < i64::from(crate::SIM_STEP_MILLIS)
    {
        // Nothing in sight but one near: lhs shot every 3 s, as lhs player
        // would, wakes the sleepers lhs gunshot reaches (2400 units).
        press |= buttons::ATTACK;
    }
    // Back away from one that is close; walk up to the nearest when none
    // is in shooting range (knifed or woken once in reach).
    // BO2MP_T6_AUTOPLAY_STAND=1: never back away (the Jet Gun grinds the ones
    // it pulls in close).
    let stand = std::env::var_os("BO2MP_T6_AUTOPLAY_STAND").is_some();
    // BO2MP_T6_AUTOPLAY_KICK=1: walk straight at the nearest one (the shrink
    // ray's shrunken zombies are kicked by walking into them).
    let kick = std::env::var_os("BO2MP_T6_AUTOPLAY_KICK").is_some();
    let (mut forward, mut right) = (0i8, 0i8);
    let blades = if fetch() { knife_blades(world) } else { Vec::new() };
    if fetch() && now % 5000 < i64::from(crate::SIM_STEP_MILLIS) {
        for Blade { ent, at, on } in &blades {
            diag::info!(
                Sim,
                "bo2zm t6 knife blade ent{ent} at ({:.0} {:.0} {:.0}) on {on:?}, {:.0} from him",
                at[0],
                at[1],
                at[2],
                flat_dist(*at, pstate.origin)
            );
        }
        // Their pickup triggers (BO2's autorecover trigger_radius, 50 x 100)
        // and whether his box touches each.
        let trigs: Vec<(u32, super::Ent)> = world
            .resource::<Zm>()
            .ents
            .iter()
            .where_value(|en| en.class_tag == "trigger_radius" && en.reach == 50.0 && en.height == 100.0)
            .map(|(nn, en)| (*nn, en.clone()))
            .collect();
        let pbox = super::triggers::player_box(world, peer.0);
        for (nn, en) in trigs {
            let touch = pbox.is_some_and(|(lo, hi)| super::triggers::touches(world, &en, lo, hi));
            diag::info!(
                Sim,
                "bo2zm t6 knife trigger ent{nn} at ({:.0} {:.0} {:.0}) on {:?}, {:.0} from him, touches {touch}",
                en.origin[0],
                en.origin[1],
                en.origin[2],
                en.link.map(|l| l.0),
                flat_dist(en.origin, pstate.origin)
            );
        }
    }
    let fetch_to = blades
        .iter()
        .map(|b| (flat_dist(b.at, pstate.origin), b.at))
        .filter(|_| fetch() && clip == 0 && stock == 0)
        .min_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));
    let walk_to = match closest {
        _ if fetch_to.is_some() => fetch_to.map(|(_, o)| (1.0, o)),
        Some((d, o)) if kick && d < 450.0 => Some((1.0, o)),
        Some((d, o)) if seek() && d >= 64.0 && range >= 450.0 => Some((1.0, o)),
        Some((d, o)) if !stand && (64.0..110.0).contains(&d) => Some((-1.0, o)),
        _ => None,
    };
    if let Some((sign, o)) = walk_to {
        let wish = [(pstate.origin[0] - o[0]) * -sign, (pstate.origin[1] - o[1]) * -sign];
        let len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt().max(1e-3);
        let (s, c) = view[1].to_radians().sin_cos();
        let (fw, rt) = ([c, s], [s, -c]);
        forward = (((wish[0] * fw[0] + wish[1] * fw[1]) / len) * 127.0).round() as i8;
        right = (((wish[0] * rt[0] + wish[1] * rt[1]) / len) * 127.0).round() as i8;
    }
    // Mob of the Dead: lhs ghost in Afterlife walks back to its body and
    // holds Use on it.
    if let Some(corpse) = super::autoplay_prison::afterlife_corpse(world, peer) {
        let toward = super::autoplay_prison::body_waypoint(world, pstate.origin, corpse);
        (press, forward, right, view) =
            super::autoplay_prison::revive_keys_via(pstate.origin, eye, corpse, toward);
        if let Some(aim) = super::autoplay_prison::ghost_zap_brutus(world, now, pstate.origin, eye) {
            // Tapped, so lhs one-shot weapon fires again.
            let tap = if now % 400 < 200 { buttons::ATTACK } else { 0 };
            (press, forward, right, view) = (tap, 0, 0, aim);
        }
        if now % 5000 < i64::from(crate::SIM_STEP_MILLIS) {
            diag::info!(
                Sim,
                "bo2zm t6 autoplay afterlife at {}s: at {:?} body {:?} walking to {:?}",
                now / 1000,
                pstate.origin.map(f32::round),
                corpse.map(f32::round),
                toward.map(f32::round)
            );
        }
    }
    // The view turns the way lhs script's setplayerangles turns it: the delta
    // under the peer's own command angles, so the peer's camera follows.
    let mut last_angles = None;
    {
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        for (id, cmd) in &mut req.input.cmds {
            if *id != peer {
                continue;
            }
            cmd.buttons = press;
            cmd.thrust_ahead = forward;
            cmd.thrust_lateral = right;
            last_angles = Some(cmd.angles);
        }
    }
    if let Some(angles) = last_angles {
        let mut fl = frame(world);
        if let Some(p) = fl.occupant_mut(peer) {
            p.view_offset_angles = std::array::from_fn(|i| {
                let cmd_deg = (angles[i] as u16) as f32 * (360.0 / 65536.0);
                view[i] - cmd_deg
            });
            p.look_angles = view;
        }
    }
    // A status line every five seconds.
    if now % 5000 < i64::from(crate::SIM_STEP_MILLIS) {
        let (round, score, threads) = with_vm(world, |vm, world| {
            let level = vm.level;
            let round = {
                let fl = vm.intern("round_number");
                vm.raw_field(level, fl)
            };
            let score = world
                .resource::<Zm>()
                .players
                .get(&0)
                .map(|p| p.obj)
                .map(|o| {
                    let fl = vm.intern("score");
                    vm.get_field(world, o, fl)
                })
                .unwrap_or_default();
            (vm.to_text(&round), vm.to_text(&score), vm.thread_count())
        })
        .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 autoplay at {}s: round {round}, points {score}, health {}, ammo {clip}+{stock}, zombies alive {}, in sight {}, at ({:.0} {:.0} {:.0}), nearest {:.0}, threads {threads}",
            now / 1000,
            pstate.health,
            living.len(),
            met.len(),
            pstate.origin[0], pstate.origin[1], pstate.origin[2],
            closest.map_or(-1.0, |c| c.0)
        );
    }
}

/// Census: a zombie tilted off upright (pitch or roll past 5) or linked to
/// an entity, e.g. " tilt (80 90 0) on 1234".
fn tilt_text(e: &super::Ent) -> String {
    let mut t = String::new();
    if e.rot[0].abs() > 5.0 || e.rot[2].abs() > 5.0 {
        t += &format!(" tilt ({:.0} {:.0} {:.0})", e.rot[0], e.rot[1], e.rot[2]);
    }
    if let Some((p, ..)) = e.link {
        t += &format!(" on {p}");
    }
    t
}

/// BO2MP_T6_CENSUS=1: every 10 s, each living zombie: state, animation,
/// place, distance to the player, path progress and how far it moved in the
/// last 10 s (a stuck one barely moves far from him).
pub(crate) fn census_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    static LAST: std::sync::Mutex<Option<std::collections::BTreeMap<u32, [f32; 3]>>> =
        std::sync::Mutex::new(None);
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_CENSUS").is_some()) {
        return;
    }
    if now % 10_000 >= i64::from(crate::SIM_STEP_MILLIS) {
        return;
    }
    // Once: each move state's animations and their root-motion speeds.
    static SPEEDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !SPEEDS.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let zm = world.resource::<Zm>();
        for (file, def) in &zm.asds {
            for (name, st) in &def.states {
                if !name.starts_with("zm_move_") {
                    continue;
                }
                let anims: Vec<String> = st
                    .substates
                    .iter()
                    .map(|(_, lhs)| {
                        let sp = zm.anims.get(lhs).map_or(-1.0, super::actors::anim_speed);
                        format!("{lhs} {sp:.0}")
                    })
                    .collect();
                diag::info!(
                    Sim,
                    "bo2zm t6 census speeds {file}/{name}: {}",
                    anims.join(", ")
                );
            }
        }
    }
    let Some(me) = frame(world).avatar(ClientId(0)).map(|pt| pt.origin) else {
        return;
    };
    let mut last = LAST.lock().unwrap();
    let prev = last.get_or_insert_with(Default::default);
    // Each zombie's walk/run/sprint (the scripts' zombie_move_speed), and
    // the script fields named in BO2MP_T6_CENSUS_FIELDS=lhs,b,... as [lhs=.. b=..].
    let extra = std::env::var("BO2MP_T6_CENSUS_FIELDS").unwrap_or_default();
    let speeds: std::collections::BTreeMap<u32, String> = with_vm(world, |vm, world| {
        let f = vm.intern("zombie_move_speed");
        let fields: Vec<(&str, _)> = extra
            .split(',')
            .filter(|k| !k.is_empty())
            .map(|k| (k, vm.intern(k)))
            .collect();
        let zm = world.resource::<Zm>();
        let objs: Vec<(u32, _)> = zm
            .actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, _)| Some((*nn, zm.ents.get(nn)?.obj?)))
            .collect();
        objs.into_iter()
            .map(|(nn, o)| {
                let mut t = vm.to_text(&vm.raw_field(o, f));
                if !fields.is_empty() {
                    let kv: Vec<String> = fields
                        .iter()
                        .map(|(k, id)| format!("{k}={}", vm.to_text(&vm.raw_field(o, *id))))
                        .collect();
                    t += &format!("[{}]", kv.join(" "));
                }
                (nn, t)
            })
            .collect()
    })
    .unwrap_or_default();
    let table_rows: Vec<String> = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, lhs)| {
                let en = zm.ents.get(nn)?;
                let moved = prev.get(nn).map_or(-1.0, |pt| flat_dist(*pt, en.origin));
                Some(format!(
                    "{nn} {} {} {} at ({:.0} {:.0} {:.0}) dist {:.0} moved {:.0} path {}/{} goal {:?}",
                    speeds.get(nn).map_or("?", String::as_str),
                    lhs.script,
                    lhs.playing.as_ref().map_or("-", |pt| pt.anim.as_str()),
                    en.origin[0],
                    en.origin[1],
                    en.origin[2],
                    flat_dist(en.origin, me),
                    moved,
                    lhs.path_i,
                    lhs.path.len(),
                    lhs.goal.map(|g| [g[0].round(), g[1].round(), g[2].round()])
                ) + &tilt_text(en)
                    + &if (0.0..24.0).contains(&moved) && lhs.script == "move" {
                    // A stalled one: where it is heading next.
                    let pts: Vec<String> = lhs
                        .path
                        .iter()
                        .skip(lhs.path_i)
                        .take(3)
                        .map(|pt| format!("({:.0} {:.0} {:.0})", pt[0], pt[1], pt[2]))
                        .collect();
                    format!(" next {}", pts.join(" "))
                } else {
                    String::new()
                })
            })
            .collect()
    };
    {
        let zm = world.resource::<Zm>();
        *prev = zm
            .actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, _)| zm.ents.get(nn).map(|en| (*nn, en.origin)))
            .collect();
    }
    diag::info!(
        Sim,
        "bo2zm t6 census {now}: me ({:.0} {:.0} {:.0}), {} alive | {}",
        me[0],
        me[1],
        me[2],
        table_rows.len(),
        table_rows.join(" | ")
    );
}

/// BO2MP_T6_ROAM="x y z;x y z;...": from 12 s the player is moved to the
/// next of those places every BO2MP_T6_ROAM_SECS (20), round and round, as a
/// player who keeps changing where he stands (do the zombies follow).
pub(crate) fn roam_test(world: &mut World, now: i64) {
    static SPOTS: OnceLock<Vec<[f32; 3]>> = OnceLock::new();
    let spots = SPOTS.get_or_init(|| env_point_list("BO2MP_T6_ROAM"));
    if spots.is_empty() || now < 12_000 {
        return;
    }
    let period = std::env::var("BO2MP_T6_ROAM_SECS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(20)
        * 1000;
    let since = now - 12_000;
    if since % period >= i64::from(crate::SIM_STEP_MILLIS) {
        return;
    }
    let spot = spots[(since / period) as usize % spots.len()];
    super::teleport_player(world, ClientId(0), spot);
    diag::info!(
        Sim,
        "bo2zm t6 roam at {}s: player to ({:.0} {:.0} {:.0})",
        now / 1000,
        spot[0],
        spot[1],
        spot[2]
    );
}

/// BO2MP_T6_ONLY_SPAWN="x y": every zombie spawns at the active spawn spot
/// nearest x y (the scripts' spawn list cut to that one each tick).
pub(crate) fn only_spawn_test(world: &mut World) {
    static AT: OnceLock<Option<[f32; 2]>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| env_tuple("BO2MP_T6_ONLY_SPAWN")) else {
        return;
    };
    with_vm(world, |vm, _| {
        let (list_f, origin_f) = (vm.intern("zombie_spawn_locations"), vm.intern("origin"));
        let gsc_t6::ScriptValue::Array(arr) = vm.raw_field(vm.level, list_f) else {
            return;
        };
        let spots: Vec<gsc_t6::ScriptValue> = arr.snapshot().values_in_order().cloned().collect();
        let pick = spots
            .iter()
            .filter_map(|v| {
                let gsc_t6::ScriptValue::Object(o) = v else {
                    return None;
                };
                let p = vm.raw_field(*o, origin_f).as_vec3()?;
                Some(((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2), v.clone()))
            })
            .min_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));
        if let Some(v) = pick.map(|p| p.1)
            && spots.len() > 1
        {
            let mut one = gsc_t6::Array::new();
            one.push(v);
            vm.set_raw_field(vm.level, list_f, gsc_t6::ScriptValue::array(one));
        }
    });
}

/// BO2MP_T6_GLIDE="x y;x y;...": from 12 s the player glides from place to
/// place at 120 units/s over the ground, resting 5 s at each, round and
/// round: a player who walks about the map (do the zombies keep up).
pub(crate) fn glide_test(world: &mut World, now: i64) {
    static SPOTS: OnceLock<Vec<[f32; 2]>> = OnceLock::new();
    static AT: std::sync::Mutex<(usize, i64)> = std::sync::Mutex::new((0, 0));
    let spots = SPOTS.get_or_init(|| env_point_list("BO2MP_T6_GLIDE"));
    if spots.is_empty() || now < 12_000 {
        return;
    }
    let Some(me) = frame(world).avatar(ClientId(0)).map(|p| p.origin) else {
        return;
    };
    let mut at = AT.lock().unwrap();
    if now < at.1 {
        return;
    }
    let target = spots[at.0 % spots.len()];
    let d = [target[0] - me[0], target[1] - me[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let step = 120.0 * crate::SIM_STEP_MILLIS as f32 / 1000.0;
    if len <= step {
        at.0 += 1;
        at.1 = now + 5000;
        diag::info!(
            Sim,
            "bo2zm t6 glide at {}s: reached ({:.0} {:.0})",
            now / 1000,
            target[0],
            target[1]
        );
        return;
    }
    let next = [
        me[0] + d[0] / len * step,
        me[1] + d[1] / len * step,
        me[2] + 40.0,
    ];
    // Onto the ground below (stairs and porches up, yards down).
    let down = [next[0], next[1], next[2] - 400.0];
    let t = frame(world).trace_static_world(
        next,
        down,
        crate::shot_trace::BODY_BOX_LOWER,
        crate::shot_trace::BODY_BOX_UPPER,
        crate::shot_trace::PLAYER_BODY_MASK,
    );
    let feet: [f32; 3] = std::array::from_fn(|i| next[i] + (down[i] - next[i]) * t.fraction);
    super::teleport_player(world, ClientId(0), feet);
}

/// BO2MP_T6_OPENALL=1: at 5 s every door and debris pile opens (the zone
/// flags), so the whole map is in play (test aid).
pub(crate) fn open_all_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_OPENALL").is_some()) {
        return;
    }
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    // BO2MP_T6_OPENALL=buy: each door and debris pile is bought instead, by
    // its own script (paths joined, doors swung, as when he buys them);
    // buy:<flag>,<flag>: only those (the doors' script_flag).
    let mode = std::env::var("BO2MP_T6_OPENALL").unwrap_or_default();
    let buy = mode == "buy" || mode.starts_with("buy:");
    let only: Vec<String> = mode
        .strip_prefix("buy:")
        .map(|l| l.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    if !buy && (5000..5000 + tick).contains(&now) {
        open_all_zones(world);
    }
    if buy && (5000..5000 + tick).contains(&now) {
        give_points(world, 100_000);
    }
    // One a tick from 5.5 s, or once he is in play if later (a debris
    // pile's script refuses a buyer who isn't alive; Origins' intro keeps
    // him a spectator past 5.5 s). Again 15 s on for those whose flag is
    // still clear.
    static START: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
    if buy && now >= 5500 && START.load(std::sync::atomic::Ordering::Relaxed) < 0 {
        let in_game = world
            .resource::<Zm>()
            .players
            .get(&0)
            .is_some_and(|p| p.sessionstate == "playing");
        if in_game {
            START.store(now, std::sync::atomic::Ordering::Relaxed);
            give_points(world, 100_000);
        }
    }
    let first = START.load(std::sync::atomic::Ordering::Relaxed);
    let pass = [first, first + 15000]
        .into_iter()
        .find(|t| first >= 0 && (*t..*t + 3000).contains(&now));
    if let (true, Some(start)) = (buy, pass) {
        // The next one by entity number (a bought debris pile deletes its
        // sibling triggers, so a list index would skip some).
        static MINT_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if now - start < tick {
            MINT_COUNTER.store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let next = MINT_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
        with_vm(world, |vm, world| {
            let (tn, flag_f) = (vm.intern("targetname"), vm.intern("script_flag"));
            let flag_k = vm.intern("flag");
            let flags = match vm.raw_field(vm.level, flag_k) {
                gsc_t6::ScriptValue::Array(a) => Some(a),
                _ => None,
            };
            let set = |vm: &mut gsc_t6::Vm<World>, f: &str| {
                flags.as_ref().is_some_and(|a| {
                    a.get(&gsc_t6::Key::Str(vm.intern(f)))
                        .is_some_and(|v| gsc_t6::truthy(&v))
                })
            };
            let zm = world.resource::<Zm>();
            let mut trigs: Vec<(u32, gsc_t6::ObjRef, String)> = zm
                .ents
                .iter()
                .where_value(|en| en.class_tag.starts_with("trigger_use"))
                .filter_map(|(nn, en)| {
                    let o = en.obj?;
                    let name = vm.to_text(&vm.raw_field(o, tn));
                    let flag = vm.to_text(&vm.raw_field(o, flag_f));
                    ((name == "zombie_door" || name == "zombie_debris")
                        && (only.is_empty() || only.contains(&flag))
                        && (start == first || !set(vm, &flag)))
                    .then_some((*nn, o, flag))
                })
                .collect();
            trigs.sort_by_key(|t| t.0);
            let player = zm.players.get(&0).map(|p| p.obj);
            let state = zm.players.get(&0).map(|p| p.sessionstate.clone());
            if let (Some((nn, t, flag)), Some(p)) =
                (trigs.iter().find(|t| t.0 >= next).cloned(), player)
            {
                let health = frame(world).avatar(ClientId(0)).map(|ps| ps.health);
                diag::info!(
                    Sim,
                    "bo2zm t6 open all: buying ent{nn} ({flag}); player {state:?} health {health:?}"
                );
                MINT_COUNTER.store(nn + 1, std::sync::atomic::Ordering::Relaxed);
                // (who, force): forced, as the scripts' own open-all does.
                vm.notify_str(
                    world,
                    t,
                    "trigger",
                    &[gsc_t6::ScriptValue::Object(p), gsc_t6::ScriptValue::Int(1)],
                );
            }
        });
    }
    if buy && only.is_empty() && (8600..8600 + tick).contains(&now) {
        break_sloth_barricades(world);
    }
}

/// Buried's wooden barricades (`sloth_barricade` triggers: the church,
/// jail, gun store, mansion lawn ...) only break for Leroy. Open them as his
/// break does: the zone flag set, the pieces hidden by the map's own
/// `hide_sloth_barrier`, the trigger's watcher ended.
fn break_sloth_barricades(world: &mut World) {
    with_vm(world, |vm, world| {
        let (tn, tg, flag_f) = (
            vm.intern("targetname"),
            vm.intern("target"),
            vm.intern("script_flag"),
        );
        let objs: Vec<gsc_t6::ObjRef> = world
            .resource::<Zm>()
            .ents
            .values()
            .filter_map(|e| e.obj)
            .collect();
        let named = |vm: &mut gsc_t6::Vm<World>, o, f| vm.to_text(&vm.raw_field(o, f));
        let trigs: Vec<gsc_t6::ObjRef> = objs
            .iter()
            .copied()
            .filter(|&o| named(vm, o, tn) == "sloth_barricade")
            .collect();
        let level = gsc_t6::ScriptValue::Object(vm.level);
        for t in trigs {
            let (flag, target) = (named(vm, t, flag_f), named(vm, t, tg));
            diag::info!(Sim, "bo2zm t6 open all: breaking barricade ({flag})");
            if flag != "undefined" && !flag.is_empty() {
                let name = vm.string(&flag);
                vm.spawn_named(
                    world,
                    "common_scripts/utility",
                    "flag_set",
                    level.clone(),
                    vec![name],
                );
            }
            for &p in &objs {
                if !target.is_empty() && named(vm, p, tn) == target {
                    vm.spawn_named(
                        world,
                        "maps/mp/zombies/_zm_ai_sloth",
                        "hide_sloth_barrier",
                        gsc_t6::ScriptValue::Object(p),
                        Vec::new(),
                    );
                }
            }
            vm.notify_str(world, t, "maxis_minigame_opens_barricade", &[]);
            vm.spawn_named(
                world,
                "maps/mp/zombies/_zm_ai_sloth",
                "hide_sloth_barrier",
                gsc_t6::ScriptValue::Object(t),
                Vec::new(),
            );
        }
    });
}

/// BO2MP_T6_BUY=<wall weapon> (e.g. m14_zm): at 5 s the doors open and he
/// gets 10000 points; at 6 s the player is put in front of that wall buy,
/// looking at it, and presses use at 7, 7.7 and 8.4 s; at 9 s the stub's
/// zone, his weapons and points are logged.
/// `a,b,c` buys each in turn 5 s apart; `@<seconds>` starts the first at
/// that time instead of at 5 s (Mob: after the start Afterlife).
/// `name#n`: the n-th of that name's chalks from 0; `name#stub`: the one
/// nearest its registered use trigger (TranZit's Galvaknuckles: other
/// modes' chalks come first; the Diner roof's is the one spawned).
pub(crate) fn buy_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Option<(Vec<String>, i64)>> = OnceLock::new();
    let Some((want, shift)) = WANT.get_or_init(|| {
            let v = std::env::var("BO2MP_T6_BUY").ok()?;
            let (names, at) = match v.split_once('@') {
                Some((n, s)) => (n, s.parse::<f32>().map_or(0, |s| (s * 1000.0) as i64 - 5000)),
                None => (v.as_str(), 0),
            };
            Some((names.split(',').map(str::to_owned).collect(), at))
        })
        .clone()
    else {
        return;
    };
    for (k, w) in want.iter().enumerate() {
        buy_one(world, now - shift - 5000 * k as i64, w);
    }
}

fn buy_one(world: &mut World, now: i64, want: &str) {
    // `#stub`: the chalk nearest its registered use trigger (the one the
    // map spawned; the others are other modes' chalks).
    let (want, nth) = match want.split_once('#') {
        Some((w, "stub")) => (w, None),
        Some((w, n)) => (w, Some(n.parse::<usize>().unwrap_or(0))),
        None => (want, Some(0)),
    };
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (5000..5000 + tick).contains(&now) {
        open_all_zones(world);
        give_points(world, 10000);
    }
    if (6000..6000 + tick).contains(&now) {
        // The wall buy's struct.
        let spot = with_vm(world, |vm, _| {
            let structs = {
                let f = vm.intern("struct");
                vm.raw_field(vm.level, f)
            };
            let gsc_t6::ScriptValue::Array(arr) = structs else {
                return None;
            };
            let key = vm.intern("zombie_weapon_upgrade");
            let (o, a) = (vm.intern("origin"), vm.intern("angles"));
            let mut chalks = Vec::new();
            for v in arr.snapshot().values_in_order() {
                let gsc_t6::ScriptValue::Object(s) = v else {
                    continue;
                };
                let name = vm.raw_field(*s, key);
                if vm.to_text(&name) == want
                    && let Some(origin) = vm.raw_field(*s, o).as_vec3()
                {
                    let rot = vm.raw_field(*s, a).as_vec3().unwrap_or([0.0; 3]);
                    chalks.push((origin, rot));
                }
            }
            if let Some(n) = nth {
                return chalks.get(n).copied();
            }
            let (ut, ts) = (vm.intern("_unitriggers"), vm.intern("trigger_stubs"));
            let stub = match vm.raw_field(vm.level, ut).as_obj().map(|u| vm.raw_field(u, ts)) {
                Some(gsc_t6::ScriptValue::Array(a)) => a
                    .read()
                    .values_in_order()
                    .filter_map(gsc_t6::ScriptValue::as_obj)
                    .find(|&s| vm.to_text(&vm.raw_field(s, key)) == want)
                    .and_then(|s| vm.raw_field(s, o).as_vec3()),
                _ => None,
            }?;
            let d = |p: &[f32; 3]| (0..3).map(|i| (p[i] - stub[i]).powi(2)).sum::<f32>();
            chalks.into_iter().min_by(|x, y| d(&x.0).total_cmp(&d(&y.0)))
        })
        .flatten();
        let Some((origin, rot)) = spot else {
            let names = with_vm(world, |vm, _| {
                let f = vm.intern("struct");
                let gsc_t6::ScriptValue::Array(arr) = vm.raw_field(vm.level, f) else {
                    return String::new();
                };
                let key = vm.intern("zombie_weapon_upgrade");
                let mut out = Vec::new();
                for v in arr.snapshot().values_in_order() {
                    if let gsc_t6::ScriptValue::Object(s) = v {
                        let n = vm.raw_field(*s, key);
                        if !matches!(n, gsc_t6::ScriptValue::Undefined) {
                            out.push(vm.to_text(&n));
                        }
                    }
                }
                out.join(" ")
            })
            .unwrap_or_default();
            diag::warn!(Sim, "bo2zm t6 buy test: no wall buy {want}; the map has [{names}]");
            return;
        };
        // A wall buy faces its struct's left (the scripts' require_look_from
        // test: the eye must be on the side of -anglestoright).
        let (_, rt, _) = gsc_t6::math::facing_axes([0.0, rot[1], 0.0]);
        // BO2MP_T6_BUY_FLIP=1: from its right instead; BO2MP_T6_BUY_DIST=<u>:
        // that far from it (18).
        let dist = std::env::var("BO2MP_T6_BUY_DIST")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(18.0);
        let side = if std::env::var_os("BO2MP_T6_BUY_FLIP").is_some() {
            -dist
        } else {
            dist
        };
        let stand = [
            origin[0] - rt[0] * side,
            origin[1] - rt[1] * side,
            origin[2] + 8.0,
        ];
        let down = [stand[0], stand[1], stand[2] - 300.0];
        let t = frame(world).trace_static_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
        let eye = [feet[0], feet[1], feet[2] + 60.0];
        let view = look(eye, origin);
        super::teleport_player(world, client, feet);
        super::set_player_view(world, client, view);
        diag::info!(
            Sim,
            "bo2zm t6 buy test: {want} at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0}) looking {:.0} {:.0}",
            origin[0],
            origin[1],
            origin[2],
            feet[0],
            feet[1],
            feet[2],
            view[0],
            view[1]
        );
    }
    // Three fresh presses, as a player presses again when nothing happens:
    // the room's use trigger can reach him only after the first one (the
    // unitrigger manager hands out only stubs in active zones). Not at 7.7 s
    // once he has the gun; the 8.4 s press is always made: on a gun bought
    // with full ammo it must charge nothing (BO2's `ammo_give`).
    if [7000, 7700, 8400].iter().any(|t| (*t..*t + 300).contains(&now)) {
        let has = {
            let f = frame(world);
            crate::client_script::weapons(&f, client, crate::client_script::WeaponList::All)
                .into_iter()
                .any(|w| crate::client_script::weapon_name(&f, w).starts_with(want))
        };
        if has && now < 8400 {
            return;
        }
        steer(world, client, |cmd| cmd.buttons |= buttons::INTERACT_KEY);
    }
    if (9000..9000 + tick).contains(&now) {
        diag::info!(Sim, "bo2zm t6 buy test: {want} {}", wallbuy_stub(world, want));
        let names: Vec<String> = {
            let f = frame(world);
            crate::client_script::weapons(&f, client, crate::client_script::WeaponList::All)
                .into_iter()
                .map(|w| crate::client_script::weapon_name(&f, w))
                .collect()
        };
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        // The gun's clip/stock after the buy and the press on a full gun.
        let ammo = super::weapon(world, want).ok().map(|w| {
            let f = frame(world);
            (
                crate::client_script::ammo_clip(&f, client, w),
                crate::client_script::rounds_held(&f, client, w),
            )
        });
        diag::info!(
            Sim,
            "bo2zm t6 buy test: weapons {names:?}, points {score}, ammo {ammo:?}, hint {hint:?}"
        );
    }
}

/// The wall buy's unitrigger stub as the manager sees it (it only offers
/// stubs whose `in_zone` is in `level.active_zone_names`): zone, place,
/// box size, and the active zones.
fn wallbuy_stub(world: &mut World, want: &str) -> String {
    with_vm(world, |vm, _| {
        let (ut, ts, wu) = (
            vm.intern("_unitriggers"),
            vm.intern("trigger_stubs"),
            vm.intern("zombie_weapon_upgrade"),
        );
        let (iz, o, azn) = (vm.intern("in_zone"), vm.intern("origin"), vm.intern("active_zone_names"));
        let size = [vm.intern("script_length"), vm.intern("script_width"), vm.intern("script_height")];
        let active: Vec<String> = match vm.raw_field(vm.level, azn) {
            gsc_t6::ScriptValue::Array(a) => {
                let vals: Vec<gsc_t6::ScriptValue> = a.read().values_in_order().cloned().collect();
                vals.iter().map(|v| vm.to_text(v)).collect()
            }
            _ => Vec::new(),
        };
        let stubs: Vec<gsc_t6::ObjRef> = match vm.raw_field(vm.level, ut).as_obj().map(|u| vm.raw_field(u, ts)) {
            Some(gsc_t6::ScriptValue::Array(a)) => a.read().values_in_order().filter_map(gsc_t6::ScriptValue::as_obj).collect(),
            _ => Vec::new(),
        };
        let Some(s) = stubs.into_iter().find(|&s| vm.to_text(&vm.raw_field(s, wu)) == want) else {
            return format!("no stub; active zones {active:?}");
        };
        let at = vm.raw_field(s, o).as_vec3().unwrap_or_default();
        let dims: Vec<String> = size.iter().map(|&k| vm.to_text(&vm.raw_field(s, k))).collect();
        format!(
            "stub in_zone {:?} at ({:.0} {:.0} {:.0}) box {}; active zones {active:?}",
            vm.to_text(&vm.raw_field(s, iz)),
            at[0],
            at[1],
            at[2],
            dims.join(" ")
        )
    })
    .unwrap_or_default()
}

/// When the box test's clock starts (its "11.5 s"), if not at 0: Mob's
/// door sweep once it is done.
pub(crate) static BOX_BASE: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(i64::MIN);

/// BO2MP_T6_BOX=1: the magic box. At 12 s the player gets 10000 points and
/// is put on the active box's use side, looking at it; he presses use at
/// 13 s (open, 950) and again at 19.5 s (take the weapon); at 22 s his
/// weapons and points are logged.
pub(crate) fn box_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("BO2MP_T6_BOX").is_ok_and(|v| v != "move" && v != "bear")) {
        return;
    }
    // With Mob's door sweep the clock starts when every door is bought.
    let base = BOX_BASE.load(std::sync::atomic::Ordering::Relaxed);
    if base == i64::MIN && std::env::var_os("BO2MP_T6_DOORS").is_some() {
        return;
    }
    // BO2MP_T6_BOX_AT=<seconds>: the "12 s" step comes then instead
    // (Ascension's intro lander holds the player until about 25 s).
    static AT: OnceLock<i64> = OnceLock::new();
    let shift = *AT.get_or_init(|| {
        std::env::var("BO2MP_T6_BOX_AT")
            .ok()
            .and_then(|text| text.parse::<f32>().ok())
            .map_or(0, |s| (s * 1000.0) as i64 - 12000)
    });
    let now = if base == i64::MIN { now - shift } else { now - base };
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (11000..11000 + tick).contains(&now) {
        open_all_zones(world);
    }
    if (12000..12000 + tick).contains(&now) {
        give_points(world, 10000);
        let Some(origin) = stand_at_box(world, client) else {
            diag::warn!(Sim, "bo2zm t6 box test: no active box");
            return;
        };
        diag::info!(
            Sim,
            "bo2zm t6 box test: box at ({:.0} {:.0} {:.0})",
            origin[0],
            origin[1],
            origin[2]
        );
    }
    // BO2MP_T6_BOX=look: he only stands there (the box's light, closed).
    let look_only = std::env::var("BO2MP_T6_BOX").is_ok_and(|v| v == "look");
    if !look_only && ((13000..13300).contains(&now) || (19500..19800).contains(&now)) {
        press_use(world, client);
    }
    if (22000..22000 + tick).contains(&now) {
        let names: Vec<String> = {
            let f = frame(world);
            crate::client_script::weapons(&f, client, crate::client_script::WeaponList::All)
                .into_iter()
                .map(|w| crate::client_script::weapon_name(&f, w))
                .collect()
        };
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        diag::info!(Sim, "bo2zm t6 box test: weapons {names:?}, points {score}");
    }
}

/// The doors and debris opened (the box may start behind them): the zone
/// flags Nuketown's own zone init names.
pub(super) fn open_all_zones(world: &mut World) {
    with_vm(world, |vm, world| {
        let level = gsc_t6::ScriptValue::Object(vm.level);
        for f in [
            "culdesac_2_openhouse2_f1",
            "openhouse2_f1_2_openhouse2_backyard",
            "openhouse2_f1_2_openhouse2_f2",
            "openhouse2_backyard_2_openhouse2_f2",
            "culdesac_2_openhouse1_f1",
            "openhouse1_f1_2_openhouse1_backyard",
            "openhouse1_f2_openhouse1_f1",
            "openhouse1_backyard_2_openhouse1_f2",
            "culdesac_2_truck",
            "openhouse2_backyard_2_ammo_door",
        ] {
            let name = vm.string(f);
            vm.spawn_named(
                world,
                "common_scripts/utility",
                "flag_set",
                level.clone(),
                vec![name],
            );
        }
        // Other maps: every zone in `level.zones` enabled by the zone
        // manager's own enable_zone (Tranzit's bus stops, the box spots).
        let zones = vm.intern("zones");
        let names: Vec<gsc_t6::ScriptValue> = match vm.raw_field(vm.level, zones) {
            gsc_t6::ScriptValue::Array(a) => a
                .read()
                .keys()
                .filter_map(|k| match k {
                    gsc_t6::Key::Str(s) => Some(gsc_t6::ScriptValue::Str(s)),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        diag::info!(Sim, "bo2zm t6: open all zones: {} zones enabled", names.len());
        for zone in names {
            vm.spawn_named(world, "maps/mp/zombies/_zm_zonemgr", "enable_zone", level.clone(), vec![zone]);
        }
    });
}

/// Player 0's points set (the scripts' `score`).
pub(super) fn give_points(world: &mut World, points: i32) {
    with_vm(world, |vm, world| {
        if let Some(p) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) {
            let f = vm.intern("score");
            vm.set_field(world, p, f, gsc_t6::ScriptValue::Int(points));
        }
    });
}

/// Use held for this tick.
fn press_use(game: &mut World, peer: ClientId) {
    steer(game, peer, |cmd| cmd.buttons |= buttons::INTERACT_KEY);
}

/// The box that can be used (its zbarrier's script `state` is "initial",
/// "open" or "close", or "arriving" once its lid piece has opened; "away"
/// is the empty spot, "leaving" the move): where it stands.
fn active_box(world: &mut World) -> Option<([f32; 3], [f32; 3])> {
    with_vm(world, |vm, world| {
        let state = vm.intern("state");
        let zm = world.resource::<Zm>();
        zm.ents.values().find_map(|e| {
            let z = e.zbarrier.as_ref()?;
            let s = vm.to_text(&vm.raw_field(e.obj?, state));
            let arrived = s == "arriving" && z.pieces.get(1).is_some_and(|p| p.state == "open");
            (matches!(s.as_str(), "initial" | "open" | "close") || arrived)
                .then_some((e.origin, e.rot))
        })
    })
    .flatten()
}

/// The player put on the showing box's use side, looking at it; the box's
/// place.
fn stand_at_box(world: &mut World, client: ClientId) -> Option<[f32; 3]> {
    stand_at_box_side(world, client, 0.0)
}

/// The showing box's use trigger as BO2's script made it
/// (`level.chests[level.chest_index].unitrigger_stub`): its origin, angles
/// and box (script_width, script_length, script_height).
fn box_stub(world: &mut World) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
    with_vm(world, |vm, _| {
        let (ch, ci, us) = (vm.intern("chests"), vm.intern("chest_index"), vm.intern("unitrigger_stub"));
        let index = usize::try_from(vm.raw_field(vm.level, ci).as_int()?).ok()?;
        let chest = match vm.raw_field(vm.level, ch) {
            gsc_t6::ScriptValue::Array(a) => a.snapshot().values_in_order().nth(index).cloned(),
            _ => None,
        }?;
        let stub = vm.raw_field(chest.as_obj()?, us).as_obj()?;
        let mut field = |name: &str| {
            let f = vm.intern(name);
            vm.raw_field(stub, f)
        };
        let spot = field("origin").as_vec3()?;
        let rot = field("angles").as_vec3().unwrap_or([0.0; 3]);
        let dims = ["script_width", "script_length", "script_height"]
            .map(|n| field(n).as_float().unwrap_or(64.0));
        Some((spot, rot, dims))
    })
    .flatten()
}

/// The player put on the floor inside the showing box's use trigger,
/// looking at it: BO2 offers the box only to a player touching that
/// trigger whose origin + 35 is within 42 units of the trigger's height
/// (`get_closest_unitriggers`). Die Rise's first box stands in the
/// upside-down building, turned over, so the side its turn points to is not
/// its use side and the floor is not under its origin: the trigger is the
/// script's own answer. False: no trigger, or no floor inside it.
fn stand_in_box_stub(game: &mut World, peer: ClientId, box_at: [f32; 3], box_angles: [f32; 3]) -> bool {
    use crate::shot_trace::{PLAYER_BODY_MASK, BODY_BOX_UPPER, BODY_BOX_LOWER};
    use gsc_t6::math::{facing_axes, dot};
    let Some((at, angles, [w, l, h])) = box_stub(game) else {
        return false;
    };
    let (f, r, u) = facing_axes(angles);
    let half_ext: [f32; 3] = std::array::from_fn(|ix| (BODY_BOX_UPPER[ix] - BODY_BOX_LOWER[ix]) * 0.5);
    let reach = |a: [f32; 3]| a[0].abs() * half_ext[0] + a[1].abs() * half_ext[1] + a[2].abs() * half_ext[2];
    // From the trigger's edge away from the box, in towards it and past.
    let out = [at[0] - box_at[0], at[1] - box_at[1], 0.0];
    let sign = if dot(out, r) >= 0.0 { 1.0 } else { -1.0 };
    let edge = l * 0.5 + reach(r) - 2.0;
    let mut spot = None;
    'search: for step in 0..=18 {
        let ro = sign * (edge - step as f32 * 4.0);
        for fo in [0.0, 16.0, -16.0, 32.0, -32.0] {
            let x = at[0] + f[0] * fo + r[0] * ro;
            let y = at[1] + f[1] * fo + r[1] * ro;
            // From just over the trigger, or lower when that starts in the
            // ceiling (the upside-down box hangs from it, a hull's height up).
            let down = [x, y, at[2] - 80.0];
            let Some((top, tt)) = [at[2] + 6.0, at[2] - 30.0].into_iter().find_map(|z| {
                let top = [x, y, z];
                let tt = frame(game).trace_static_world(top, down, BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK);
                (!tt.starts_inside).then_some((top, tt))
            }) else {
                continue;
            };
            if tt.fraction >= 1.0 {
                continue;
            }
            let feet: [f32; 3] = std::array::from_fn(|ix| top[ix] + (down[ix] - top[ix]) * tt.fraction);
            if (feet[2] + 35.0 - at[2]).abs() > 42.0 {
                continue;
            }
            let c: [f32; 3] =
                std::array::from_fn(|ix| feet[ix] + (BODY_BOX_LOWER[ix] + BODY_BOX_UPPER[ix]) * 0.5 - at[ix]);
            if dot(c, f).abs() <= w * 0.5 + reach(f)
                && dot(c, r).abs() <= l * 0.5 + reach(r)
                && dot(c, u).abs() <= h * 0.5 + reach(u)
            {
                spot = Some(feet);
                break 'search;
            }
        }
    }
    diag::info!(
        Sim,
        "bo2zm t6 box stand: box ({:.0} {:.0} {:.0}) turned ({:.0} {:.0} {:.0}); use trigger ({:.0} {:.0} {:.0}) turned ({:.0} {:.0} {:.0}) {w}x{l}x{h}; feet {}",
        box_at[0],
        box_at[1],
        box_at[2],
        box_angles[0],
        box_angles[1],
        box_angles[2],
        at[0],
        at[1],
        at[2],
        angles[0],
        angles[1],
        angles[2],
        spot.map_or("none".to_string(), |p| format!("({:.0} {:.0} {:.0})", p[0], p[1], p[2]))
    );
    let Some(feet) = spot else {
        return false;
    };
    let eye = [feet[0], feet[1], feet[2] + 60.0];
    super::teleport_player(game, peer, feet);
    super::set_player_view(game, peer, look(eye, at));
    true
}

/// [`stand_at_box`] from a side turned `turn` degrees about the box
/// (BO2MP_T6_BOX_SIDES: the move test tries each side in turn).
fn stand_at_box_side(game: &mut World, peer: ClientId, turn: f32) -> Option<[f32; 3]> {
    let (spot, rot) = active_box(game)?;
    if turn == 0.0 && stand_in_box_stub(game, peer, spot, rot) {
        return Some(spot);
    }
    let (_, rt, _) = gsc_t6::math::facing_axes([0.0, rot[1] + turn, 0.0]);
    let stand = [
        spot[0] - rt[0] * 48.0,
        spot[1] - rt[1] * 48.0,
        spot[2] + 20.0,
    ];
    let down = [stand[0], stand[1], stand[2] - 200.0];
    let t = frame(game).trace_static_world(
        stand,
        down,
        crate::shot_trace::BODY_BOX_LOWER,
        crate::shot_trace::BODY_BOX_UPPER,
        crate::shot_trace::PLAYER_BODY_MASK,
    );
    let mut feet: [f32; 3] =
        std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
    // Started inside something (Die Rise's box stands by a wall): the hull
    // stays where it began, in the air. Drop a point instead.
    if t.starts_inside {
        let p = frame(game).trace_static_world(
            stand,
            down,
            [0.0; 3],
            [0.0; 3],
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        feet = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * p.fraction);
        diag::info!(
            Sim,
            "bo2zm t6 box stand: hull started solid at ({:.0} {:.0} {:.0}), point drop to z {:.0}",
            stand[0],
            stand[1],
            stand[2],
            feet[2]
        );
    }
    let eye_at = [feet[0], feet[1], feet[2] + 60.0];
    let view = look(eye_at, [spot[0], spot[1], spot[2] + 20.0]);
    super::teleport_player(game, peer, feet);
    super::set_player_view(game, peer, view);
    Some(spot)
}

/// BO2MP_T6_BOX=move: the box until the teddy bear moves it (run with
/// BO2MP_T6_GOD=1). From 12 s, every 9 s: 10000 points, the player at the
/// box that is showing, use (open), use 6 s later (take); 8.5 s in, the gun
/// in hand, the box's place and the scripts' `level.chest_accessed` /
/// `level.chest_moves` are logged. While the box is away nothing is pressed.
pub(crate) fn box_move_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("BO2MP_T6_BOX").is_ok_and(|v| v == "move" || v == "bear")) {
        return;
    }
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (11000..11000 + tick).contains(&now) {
        open_all_zones(world);
        // BO2MP_T6_BOX=bear: the first use brings the teddy bear (the
        // scripts' rule: eight uses and no move yet).
        if std::env::var("BO2MP_T6_BOX").is_ok_and(|v| v == "bear") {
            with_vm(world, |vm, world| {
                let f = vm.intern("chest_accessed");
                let level = vm.level;
                vm.set_field(world, level, f, gsc_t6::ScriptValue::Int(8));
            });
        }
    }
    if now < 12000 {
        return;
    }
    let t = (now - 12000) % 9000;
    // BO2MP_T6_BOX_SIDES=1: each round of the test stands on the next side
    // of the box (0, 90, 180, 270 degrees from the use side).
    let sides = std::env::var_os("BO2MP_T6_BOX_SIDES").is_some();
    let turn = if sides { ((now - 12000) / 9000 % 4) as f32 * 90.0 } else { 0.0 };
    if t < tick {
        give_points(world, 10000);
        if stand_at_box_side(world, client, turn).is_none() {
            diag::info!(
                Sim,
                "bo2zm t6 box move test at {}s: no box showing",
                now / 1000
            );
        }
    }
    if active_box(world).is_some() && ((1000..1300).contains(&t) || (7000..7300).contains(&t)) {
        if t < 1000 + tick || (7000..7000 + tick).contains(&t) {
            let hint = world.resource::<Zm>().hints.get(&0).cloned().unwrap_or_default();
            let at = frame(world).avatar(client).map(|p| p.origin).unwrap_or_default();
            diag::info!(
                Sim,
                "bo2zm t6 box move test at {:.1}s: side {turn}, player ({:.0} {:.0} {:.0}), hint {hint:?}",
                now as f32 / 1000.0,
                at[0],
                at[1],
                at[2]
            );
        }
        press_use(world, client);
    }
    if (8500..8500 + tick).contains(&t) {
        let (accessed, moves) = with_vm(world, |vm, _| {
            let a = vm.intern("chest_accessed");
            let m = vm.intern("chest_moves");
            (
                vm.raw_field(vm.level, a).as_int(),
                vm.raw_field(vm.level, m).as_int(),
            )
        })
        .unwrap_or((None, None));
        let gun = {
            let f = frame(world);
            f.avatar(client)
                .map(|p| crate::client_script::weapon_name(&f, p.weapon))
                .unwrap_or_default()
        };
        let place =
            active_box(world).map(|(o, _)| format!("({:.0} {:.0} {:.0})", o[0], o[1], o[2]));
        diag::info!(
            Sim,
            "bo2zm t6 box move test at {}s: gun {gun}, box {}, chest_accessed {accessed:?}, chest_moves {moves:?}",
            now / 1000,
            place.unwrap_or_else(|| "away".to_owned())
        );
    }
}

/// BO2MP_T6_KNIFE_AT=x,y,z: from 10 s, the nearest damage trigger to that
/// spot (Nuketown's bunker hatch) is knifed once a second for 6 s, from the
/// first clear spot 40 units round it.
pub(crate) fn knife_at_test(world: &mut World, now: i64) {
    static AT: OnceLock<Option<[f32; 3]>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        let v: Vec<f32> = std::env::var("BO2MP_T6_KNIFE_AT")
            .ok()?
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    }) else {
        return;
    };
    if !(10000..16000).contains(&now) {
        return;
    }
    let client = ClientId(0);
    let trig = world
        .resource::<Zm>()
        .ents
        .values()
        .filter(|e| e.class_tag == "trigger_damage")
        .cloned()
        .min_by(|a, b| {
            let d = |e: &super::Ent| gsc_t6::math::length(gsc_t6::math::sub(e.origin, at));
            d(a).total_cmp(&d(b))
        });
    let target = trig.as_ref().map_or(at, |e| super::triggers::center(world, e));
    let Some(ps) = frame(world).avatar(client).copied() else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if now < 10000 + tick {
        diag::info!(Sim, "bo2zm t6 test: knife trigger at {target:?} ({:?})", trig.map(|e| e.origin));
    }
    let mut pick = None;
    for a in (0..16).map(|i| (i as f32 * 22.5).to_radians()) {
        let stand = [target[0] + a.cos() * 40.0, target[1] + a.sin() * 40.0, target[2] + 40.0];
        let down = [stand[0], stand[1], stand[2] - 200.0];
        let f = frame(world);
        let t = f.trace_static_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        if t.starts_inside {
            continue;
        }
        let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
        pick = Some((feet, [feet[0], feet[1], feet[2] + ps.eye_height_now]));
        break;
    }
    let Some((feet, eye)) = pick else { return };
    super::teleport_player(world, client, feet);
    super::set_player_view(world, client, look(eye, target));
    if (now - 10000) % 1000 < tick && now >= 10500 {
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        for (id, user_cmd) in &mut req.input.cmds {
            if client == *id {
                user_cmd.buttons |= weapon_t6::melee_trace::BUTTON_MELEE;
            }
        }
    }
}

/// BO2MP_T6_VIEW=fx,fy,fz,tx,ty,tz (pictures): from 5 s the player stands
/// with his feet at f, looking at t. A tour: `;` between views, each with
/// `@<seconds>` (`...@40;...@48`, in time order); each holds until the next.
/// `off@<seconds>` lets him go (a use test can then move him).
/// `ent:<targetname or script_noteworthy>:<dist>[:<up>]@<seconds>` follows an entity every tick
/// (a perk machine riding a Die Rise car): feet <dist> units in front of it
/// and <up> above its origin, looking at it; its origin is logged every 2 s
/// so the pictures taken while the car stands at a floor can be picked.
/// `door:<name>:<dist>[:<up>]@<seconds>` follows it the same way but stands
/// <dist> units out from the nearest `*_indicator` (the shaft door's floor
/// light), on the landing looking into the car whichever way the machine is
/// turned (Die Rise's machines ride their cars back to front).
pub(crate) fn view_test(world: &mut World, now: i64) {
    enum View {
        Off,
        At([f32; 6]),
        Ent(String, f32, f32, bool),
    }
    static AT: OnceLock<Vec<(View, i64)>> = OnceLock::new();
    let views = AT.get_or_init(|| {
        let Ok(v) = std::env::var("BO2MP_T6_VIEW") else {
            return Vec::new();
        };
        v.split(';')
            .filter_map(|v| {
                let (p, s) = v.split_once('@').unwrap_or((v, "5"));
                let at = s.trim().parse::<f32>().map_or(5000, |s| (s * 1000.0) as i64);
                let p = p.trim();
                if p == "off" {
                    return Some((View::Off, at));
                }
                let door = p.starts_with("door:");
                if let Some(en) = p.strip_prefix("ent:").or_else(|| p.strip_prefix("door:")) {
                    let mut it = en.split(':');
                    let name = it.next()?.to_owned();
                    let dist = it.next().and_then(|x| x.parse().ok()).unwrap_or(170.0);
                    let up = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    return Some((View::Ent(name, dist, up, door), at));
                }
                let n: Vec<f32> = p.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (n.len() == 6).then(|| (View::At([n[0], n[1], n[2], n[3], n[4], n[5]]), at))
            })
            .collect()
    });
    let Some((view, _)) = views.iter().rev().find(|(_, at)| now >= *at) else {
        return;
    };
    let v = match view {
        View::Off => return,
        View::At(v) => *v,
        View::Ent(name, dist, up, door) => {
            let found = with_vm(world, |vm, world| {
                let tn = vm.intern("targetname");
                let sn = vm.intern("script_noteworthy");
                let ents = &world.resource::<Zm>().ents;
                let (spot, rot) = ents.values().find_map(|en| {
                    let o = en.obj?;
                    [tn, sn]
                        .iter()
                        .any(|f| vm.to_text(&vm.raw_field(o, *f)) == *name)
                        .then_some((en.origin, en.rot))
                })?;
                let flat = |p: [f32; 3]| (p[0] - spot[0]).hypot(p[1] - spot[1]);
                let light = if *door {
                    ents.values()
                        .filter(|en| en.obj.is_some_and(|o| vm.to_text(&vm.raw_field(o, tn)).ends_with("_indicator")))
                        .map(|en| en.origin)
                        .min_by(|lhs, rhs| flat(*lhs).total_cmp(&flat(*rhs)))
                } else {
                    None
                };
                Some((spot, rot, light))
            })
            .flatten();
            let Some((spot, rot, light)) = found else {
                return;
            };
            // A machine faces 90 degrees right of its rot (as perk_test);
            // lhs door view stands out from the shaft door's light instead.
            let (mut fw, _, _) = gsc_t6::math::facing_axes([0.0, rot[1] - 90.0, 0.0]);
            let mut from = spot;
            if let Some(l) = light {
                let len = (l[0] - spot[0]).hypot(l[1] - spot[1]).max(1.0);
                fw = [(l[0] - spot[0]) / len, (l[1] - spot[1]) / len, 0.0];
                from = [l[0], l[1], spot[2]];
            }
            if now % 2000 < i64::from(crate::SIM_STEP_MILLIS) {
                diag::info!(
                    Sim,
                    "bo2zm t6 view: {name} at ({:.0} {:.0} {:.0}) yaw {:.0}",
                    spot[0],
                    spot[1],
                    spot[2],
                    rot[1]
                );
            }
            [
                from[0] + fw[0] * dist,
                from[1] + fw[1] * dist,
                spot[2] + up,
                spot[0],
                spot[1],
                spot[2] + 50.0,
            ]
        }
    };
    let client = ClientId(0);
    let Some(ps) = frame(world).avatar(client).copied() else {
        return;
    };
    let feet = [v[0], v[1], v[2]];
    super::teleport_player(world, client, feet);
    let eye = [feet[0], feet[1], feet[2] + ps.eye_height_now];
    super::set_player_view(world, client, look(eye, [v[3], v[4], v[5]]));
}

/// BO2MP_T6_NOHUD=1 (pictures): no HUD at all, through the same flags the
/// game-over screen uses.
pub(crate) fn nohud_test(world: &mut World) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("BO2MP_T6_NOHUD").is_ok_and(|v| v.trim() == "1")) {
        return;
    }
    for key in ["bo2zm_hud_hidden", "bo2zm_ammo_hide", "bo2zm_score_hide"] {
        super::set_client_dvar(world, 0, key, "1");
    }
}

/// BO2MP_T6_BEARS=<secs>: the hidden song's three teddy bears (zm_nuked's
/// sndmusegg2 origins). From then, 8 s at each: the player stands 30 units
/// off it, faces it, and holds Use for half a second 3 s in.
pub(crate) fn bears_test(world: &mut World, now: i64) {
    const BEARS: [[f32; 3]; 3] = [[-1998.0, 632.0, -48.0], [-80.0, 35.0, -18.0], [617.0, 313.0, 152.0]];
    static AT: OnceLock<Option<i64>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| std::env::var("BO2MP_T6_BEARS").ok()?.trim().parse::<i64>().ok())
    else {
        return;
    };
    let t = now - at * 1000;
    if t < 0 || t >= 8000 * BEARS.len() as i64 {
        return;
    }
    let bear = BEARS[(t / 8000) as usize];
    let client = ClientId(0);
    let Some(ps) = frame(world).avatar(client).copied() else {
        return;
    };
    let feet = [bear[0] + 21.0, bear[1] + 21.0, bear[2] - 20.0];
    super::teleport_player(world, client, feet);
    let eye = [feet[0], feet[1], feet[2] + ps.eye_height_now];
    super::set_player_view(world, client, look(eye, bear));
    if (3000..3500).contains(&(t % 8000)) {
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        for (id, user_cmd) in &mut req.input.cmds {
            if client == *id {
                user_cmd.buttons |= buttons::INTERACT_KEY;
            }
        }
    }
}

/// BO2MP_T6_PERK_DROP=1: while a perk machine is flying in, the player
/// stands 100 units in front of where it lands, facing it (inside the
/// landing's 300-unit knockdown).
pub(crate) fn perk_drop_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_PERK_DROP").is_some()) {
        return;
    }
    if now % 500 >= i64::from(crate::SIM_STEP_MILLIS) {
        return;
    }
    let spot = with_vm(world, |vm, world| {
        let (tn, fx, pos, ang) = (
            vm.intern("targetname"),
            vm.intern("fx"),
            vm.intern("original_pos"),
            vm.intern("original_angles"),
        );
        world.resource::<Zm>().ents.values().find_map(|e| {
            let o = e.obj?;
            if !vm.to_text(&vm.raw_field(o, tn)).starts_with("vending_")
                || matches!(vm.raw_field(o, fx), gsc_t6::ScriptValue::Undefined)
            {
                return None;
            }
            let p = vm.raw_field(o, pos).as_vec3()?;
            // Landed: leave the player where he is.
            if (e.origin[2] - p[2]).abs() < 30.0 {
                return None;
            }
            Some((p, vm.raw_field(o, ang).as_vec3().unwrap_or([0.0; 3])))
        })
    })
    .flatten();
    let Some((pos, ang)) = spot else {
        return;
    };
    // The machine's front (the script's anglestoforward(angles - (0, 90, 0))).
    let yaw = (ang[1] - 90.0).to_radians();
    let feet = [pos[0] + yaw.cos() * 100.0, pos[1] + yaw.sin() * 100.0, pos[2]];
    let client = ClientId(0);
    let Some(ps) = frame(world).avatar(client).copied() else {
        return;
    };
    super::teleport_player(world, client, feet);
    let eye = [feet[0], feet[1], feet[2] + ps.eye_height_now];
    super::set_player_view(world, client, look(eye, [pos[0], pos[1], pos[2] + 40.0]));
}

/// BO2MP_T6_EGG1=1 (with the rounds test): the first song's egg. The player
/// leaves the power-up behind the door (`level.door_powerup`, back each time
/// the doomsday clock moves) until the population sign reads 15, then
/// stands on it.
pub(crate) fn egg1_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    static LAST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_EGG1").is_some()) {
        return;
    }
    if now % 500 >= i64::from(crate::SIM_STEP_MILLIS) {
        return;
    }
    let spot = with_vm(world, |vm, world| {
        let (pc, dp) = (vm.intern("population_count"), vm.intern("door_powerup"));
        let pop = vm.raw_field(vm.level, pc).as_int().unwrap_or(-1);
        let o = vm.raw_field(vm.level, dp).as_obj();
        let at = o.and_then(|o| {
            world
                .resource::<Zm>()
                .ents
                .values()
                .find(|e| e.obj == Some(o))
                .map(|e| e.origin)
        });
        (pop, at)
    });
    let Some((pop, at)) = spot else {
        return;
    };
    if LAST.swap(pop, std::sync::atomic::Ordering::Relaxed) != pop {
        diag::info!(Sim, "bo2zm t6 egg1 test: population sign {pop}, door power-up {at:?}");
    }
    let (15, Some(at)) = (pop, at) else {
        return;
    };
    super::teleport_player(world, ClientId(0), at);
}

/// BO2MP_T6_MANNEQUIN=<n> (default 1): the breakable props (map
/// `destructibledef` naming "male": Nuketown's mannequins) in turn. From 10 s, every 4 s
/// the player stands 90 units in front of the next one, looks at its head
/// and fires a tick of attack every 0.3 s for 2 s. BO2MP_T6_MANNEQUIN_HOW=knife
/// stands 45 units off and knifes it twice; =nade stands 160 off and throws a
/// frag at its feet once (give frags with BO2MP_T6_NADES). Past the last one
/// it goes round again, each pass from the next clear spot.
pub(crate) fn mannequin_test(world: &mut World, now: i64) {
    static HOW: OnceLock<String> = OnceLock::new();
    let how = HOW.get_or_init(|| std::env::var("BO2MP_T6_MANNEQUIN_HOW").unwrap_or_default());
    static WANT: OnceLock<Option<usize>> = OnceLock::new();
    let Some(want) = *WANT.get_or_init(|| {
        std::env::var("BO2MP_T6_MANNEQUIN")
            .ok()
            .map(|v| v.parse().unwrap_or(1))
    }) else {
        return;
    };
    if now < 10000 {
        return;
    }
    let slot_no = ((now - 10000) / 4000) as usize;
    let within = (now - 10000) % 4000;
    if slot_no >= want {
        return;
    }
    let client = ClientId(0);
    let targets: Vec<(u32, [f32; 3], f32, String)> = {
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .where_value(|en| en.destructible.as_deref().is_some_and(|dd| dd.contains("male")) && !en.unseen)
            .map(|(nn, en)| (*nn, en.origin, en.rot[1], en.destructible.clone().unwrap_or_default()))
            .collect()
    };
    // Past the last one it goes round again (a second pass shoots from
    // the next clear spot, for a head the first spot's shots missed).
    let pass = slot_no / targets.len().max(1);
    let Some((nn, origin, yw, def)) = targets.get(slot_no % targets.len().max(1)).cloned() else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if within < tick {
        // Its hit boxes (none = bullets pass through it).
        let id = world.resource::<Zm>().presences.by_ent.get(&nn).map(|s| s.id);
        let model = world.resource::<Zm>().ents.get(&nn).map(|en| en.mesh.clone()).unwrap_or_default();
        let (row, destr, bones) = id.map_or((false, false, 0), |id| {
            let f = frame(world);
            let row = f
                .entity_hit_support()
                .iter()
                .find(|row| row.owner.scripted_model() == Some(id));
            let dobj = row.and_then(|r| r.dobj.as_ref());
            if let Some(dd) = dobj {
                diag::info!(
                    Sim,
                    "bo2zm t6 test: mannequin dobj cap={} contents={:?} coll={} err={:?} surfs={:?}",
                    dd.can_do.is_some(),
                    dd.can_do.as_ref().map(|c| c.contents),
                    dd.live_hitbox.is_some(),
                    dd.bake_failure,
                    dd.can_do.as_ref().map(|c| c
                        .collision_faces
                        .iter()
                        .map(|s| (s.bone, s.contents, s.face_property_bits, s.triangle_list.len()))
                        .collect::<Vec<_>>())
                );
            }
            (
                row.is_some(),
                dobj.is_some_and(|dd| dd.breakable.is_some()),
                dobj.and_then(|dd| dd.live_hitbox.as_ref())
                    .map_or(0, |c| c.bones.len()),
            )
        });
        diag::info!(
            Sim,
            "bo2zm t6 test: mannequin {}/{} ent{nn} {def} at ({:.0} {:.0} {:.0}) presence={} row={row} destructible={destr} boxes={bones} model={}",
            slot_no % targets.len().max(1) + 1,
            targets.len(),
            origin[0],
            origin[1],
            origin[2],
            id.is_some(),
            model
        );
    }
    let Some(ps) = frame(world).avatar(client).copied() else {
        return;
    };
    // Its head piece's box (the bone named for the head).
    let head = world
        .resource::<Zm>()
        .presences
        .by_ent
        .get(&nn)
        .map(|s| s.id)
        .and_then(|id| {
            let f = frame(world);
            let dd = f
                .entity_hit_support()
                .iter()
                .find(|row| row.owner.scripted_model() == Some(id))?
                .dobj
                .as_ref()?;
            let names = &dd.can_do.as_ref()?.pose.joint_labels;
            dd.live_hitbox
                .as_ref()?
                .bones
                .iter()
                .find(|b| names.get(usize::from(b.bone)).is_some_and(|nm| nm.contains("head")))
                .map(|b| b.center)
        })
        .or_else(|| head_of(world, nn, true))
        .unwrap_or([origin[0], origin[1], origin[2] + 60.0]);
    // The first spot round it (in front first) with a clear line to its
    // head.
    let mut pick = None;
    let mut clear = 0;
    for (i, off) in [0.0f32, 180.0, 90.0, 270.0, 45.0, 315.0, 135.0, 225.0]
        .into_iter()
        .enumerate()
    {
        let a = (yw + off).to_radians();
        let reach = match how.as_str() {
            "knife" => 45.0,
            "nade" => 160.0,
            _ => 90.0,
        };
        let stand = [origin[0] + a.cos() * reach, origin[1] + a.sin() * reach, origin[2] + 40.0];
        let down = [stand[0], stand[1], stand[2] - 200.0];
        let f = frame(world);
        let t = f.trace_static_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
        let eye = [feet[0], feet[1], feet[2] + ps.eye_height_now];
        let sight = f.trace_static_world(
            eye,
            head,
            [0.0; 3],
            [0.0; 3],
            crate::shot_trace::SHOT_WORLD_FILTER,
        );
        if t.starts_inside || sight.fraction < 0.98 {
            if i == 0 {
                pick = Some((feet, eye));
            }
            continue;
        }
        pick = Some((feet, eye));
        clear += 1;
        if clear > pass {
            break;
        }
    }
    let Some((feet, eye)) = pick else { return };
    super::teleport_player(world, client, feet);
    let aim = if how == "nade" { [origin[0], origin[1], origin[2] + 8.0] } else { head };
    // BO2MP_T6_MANNEQUIN_LOOK=floor: from 2.5 s on, look where its thrown
    // head or arm landed.
    static FLOOR: OnceLock<bool> = OnceLock::new();
    if *FLOOR.get_or_init(|| std::env::var("BO2MP_T6_MANNEQUIN_LOOK").is_ok_and(|v| v == "floor"))
        && within >= 2500
    {
        // The newest thrown piece, else its feet.
        let piece = {
            let zm = world.resource::<Zm>();
            zm.gibs.last().and_then(|(g, _)| zm.ents.get(g)).map(|en| en.origin)
        };
        super::set_player_view(world, client, look(eye, piece.unwrap_or(origin)));
        return;
    }
    super::set_player_view(world, client, look(eye, aim));
    if how == "knife" || how == "nade" {
        let press = if how == "knife" {
            (within % 1000 < tick && (500..2500).contains(&within))
                .then_some(weapon_t6::melee_trace::BUTTON_MELEE)
        } else {
            (500..700).contains(&within).then_some(buttons::SAMPLE_FRAG_ASSET)
        };
        if let Some(press) = press {
            let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
            for (id, user_cmd) in &mut req.input.cmds {
                if client == *id {
                    user_cmd.buttons |= press;
                }
            }
        }
        return;
    }
    if (500..2500).contains(&within) && within % 300 < tick {
        {
            let mut f = frame(world);
            if let Some(w) = f.avatar(client).map(|p| p.weapon) {
                crate::client_script::set_ammo_clip(&mut f, client, w, 8);
            }
        }
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        for (id, user_cmd) in &mut req.input.cmds {
            if client == *id {
                user_cmd.buttons |= buttons::ATTACK;
            }
        }
    }
}

/// BO2MP_T6_DWTEST=<weapon> (e.g. fivesevendw_zm): a dual-wield check. At 8 s
/// the weapon is given; from 12 s, one tick of attack (the left gun) every
/// 0.5 s for 2 s, then one tick of the aim button (the right gun) every
/// 0.5 s for 2 s, then reload at 17 s; at 11.9, 14.1, 16.1 and 19 s the
/// clips (right, left) and stock are logged.
pub(crate) fn dual_wield_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Option<String>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| std::env::var("BO2MP_T6_DWTEST").ok())
        .clone()
    else {
        return;
    };
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (8000..8000 + tick).contains(&now) {
        with_vm(world, |vm, world| {
            if let Some(p) = world.resource::<Zm>().players.get(&client.0).map(|p| p.obj) {
                let g = vm.string(&want);
                vm.spawn_named(
                    world,
                    "maps/mp/zombies/_zm_weapons",
                    "weapon_give",
                    gsc_t6::ScriptValue::Object(p),
                    vec![g],
                );
            }
        });
    }
    let button = if (12000..14000).contains(&now) && (now - 12000) % 500 < tick {
        Some(buttons::ATTACK)
    } else if (14200..16200).contains(&now) && (now - 14200) % 500 < tick {
        Some(buttons::THROW)
    } else if (17000..17000 + tick).contains(&now) {
        Some(buttons::RELOAD)
    } else {
        None
    };
    if let Some(b) = button {
        steer(world, client, |cmd| cmd.buttons |= b);
    }
    // Each change of either hand's animation while the test fires.
    if (11900..19000).contains(&now) {
        static LAST: std::sync::Mutex<(i32, i32)> = std::sync::Mutex::new((-1, -1));
        let anims = frame(world)
            .avatar(client)
            .map(|ps| (ps.weap_anim_t6, ps.weap_anim_left_t6));
        if let (Some(anims), Ok(mut last)) = (anims, LAST.lock())
            && *last != anims
        {
            diag::info!(
                Sim,
                "bo2zm t6 dual test at {now}ms: animations right {:#x} left {:#x}",
                anims.0,
                anims.1
            );
            *last = anims;
        }
    }
    for at in [11900, 14100, 16100, 19000] {
        if (at..at + tick).contains(&now) {
            let f = frame(world);
            let Some(ps) = f.avatar(client) else { return };
            let w = ps.weapon;
            let right = crate::client_script::ammo_clip(&f, client, w);
            let left = crate::client_script::left_clip(&f, client, w);
            let stock = crate::client_script::rounds_held(&f, client, w);
            diag::info!(
                Sim,
                "bo2zm t6 dual test at {}ms: {} hands {} clips right {right} left {left} stock {stock} anims {}/{}",
                now,
                crate::client_script::weapon_name(&f, w),
                ps.previous_gun_hand + 1,
                ps.weap_anim_t6,
                ps.weap_anim_left_t6
            );
        }
    }
}

/// BO2MP_T6_DOOR=<script_flag> (e.g. culdesac_2_openhouse1_f1): a door or
/// debris pile. At 6 s the player gets 10000 points and stands in its use
/// trigger, looking at what it opens; he holds use from 7 to 7.5 s; at 9 s
/// his points, the level flag and the door's place are logged.
pub(crate) fn door_test(game: &mut World, now: i64) {
    static WANT: OnceLock<Option<String>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| std::env::var("BO2MP_T6_DOOR").ok())
        .clone()
    else {
        return;
    };
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    // The trigger and what it moves (its target's place).
    struct DoorFound {
        trig: super::Ent,
        aim: Option<[f32; 3]>,
    }
    let find = |game: &mut World| -> Option<DoorFound> {
        with_vm(game, |vm, game| {
            let (flag_f, tn, target) = (
                vm.intern("script_flag"),
                vm.intern("targetname"),
                vm.intern("target"),
            );
            let zm = game.resource::<Zm>();
            let trig = zm.ents.values().find(|e| {
                e.class_tag.starts_with("trigger_use")
                    && e.obj.is_some_and(|o| {
                        let name = vm.to_text(&vm.raw_field(o, tn));
                        (name == "zombie_door" || name == "zombie_debris")
                            && vm.to_text(&vm.raw_field(o, flag_f)) == want
                    })
            })?;
            let aim = trig.obj.and_then(|o| {
                let t = vm.to_text(&vm.raw_field(o, target));
                zm.ents
                    .values()
                    .find(|e| {
                        e.obj
                            .is_some_and(|eo| vm.to_text(&vm.raw_field(eo, tn)) == t)
                    })
                    .map(|e| e.origin)
            });
            Some(DoorFound { trig: trig.clone(), aim })
        })
        .flatten()
    };
    if (6000..6000 + tick).contains(&now) {
        give_points(game, 10000);
        let Some(DoorFound { trig, aim }) = find(game) else {
            let flags = with_vm(game, |vm, game| {
                let (flag_f, tn) = (vm.intern("script_flag"), vm.intern("targetname"));
                let mut out: Vec<String> = game
                    .resource::<Zm>()
                    .ents
                    .values()
                    .filter(|e| e.class_tag.starts_with("trigger_use"))
                    .filter_map(|e| e.obj)
                    .filter_map(|o| {
                        let name = vm.to_text(&vm.raw_field(o, tn));
                        (name == "zombie_door" || name == "zombie_debris")
                            .then(|| format!("{name}:{}", vm.to_text(&vm.raw_field(o, flag_f))))
                    })
                    .collect();
                out.sort();
                out.join(" ")
            })
            .unwrap_or_default();
            diag::warn!(Sim, "bo2zm t6 door test: no door or debris for {want}; the map has [{flags}]");
            return;
        };
        let c = super::triggers::center(game, &trig);
        let stand = [c[0], c[1], c[2] + 40.0];
        let down = [stand[0], stand[1], stand[2] - 300.0];
        let t = frame(game).trace_static_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
        let eye = [feet[0], feet[1], feet[2] + 60.0];
        let to = aim.unwrap_or(c);
        super::teleport_player(game, client, feet);
        super::set_player_view(game, client, look(eye, [to[0], to[1], to[2] + 40.0]));
        diag::info!(
            Sim,
            "bo2zm t6 door test: {want} trigger at ({:.0} {:.0} {:.0}), opens {:?}; player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            aim.map(|a| a.map(|v| v.round())),
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (7000..7500).contains(&now) {
        press_use(game, client);
    }
    if (9000..9000 + tick).contains(&now) {
        let score = frame(game).client_meta(client).map_or(0, |m| m.score);
        let flag_on = with_vm(game, |vm, _| {
            let f = vm.intern("flag");
            match vm.raw_field(vm.level, f) {
                gsc_t6::ScriptValue::Array(a) => {
                    let k = gsc_t6::Key::Str(vm.intern(&want));
                    a.get(&k).is_some_and(|v| gsc_t6::truthy(&v))
                }
                _ => false,
            }
        })
        .unwrap_or(false);
        let hint = game
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 door test: points {score}, flag {want} {flag_on}, hint {hint:?}"
        );
    }
    // Every piece the door moves (the trigger's target): its place and turn
    // before, during and after the open.
    if [6500, 7100, 7250, 7400, 7600, 8200, 9000, 11000]
        .iter()
        .any(|t| (*t..*t + tick).contains(&now))
    {
        let rows = with_vm(game, |vm, game| {
            let (flag_f, tn, target) = (
                vm.intern("script_flag"),
                vm.intern("targetname"),
                vm.intern("target"),
            );
            let zm = game.resource::<Zm>();
            // A bought debris pile deletes its triggers: the pieces they
            // named are remembered.
            static NAMED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            let mut targets: Vec<String> = zm
                .ents
                .values()
                .filter_map(|e| {
                    let o = e.obj?;
                    (e.class_tag.starts_with("trigger_use")
                        && vm.to_text(&vm.raw_field(o, flag_f)) == want)
                        .then(|| vm.to_text(&vm.raw_field(o, target)))
                })
                .collect();
            if let Ok(mut named) = NAMED.lock() {
                if targets.is_empty() {
                    targets = named.clone();
                } else {
                    *named = targets.clone();
                }
            }
            zm.ents
                .iter()
                .filter(|(_, e)| {
                    e.obj.is_some_and(|o| targets.contains(&vm.to_text(&vm.raw_field(o, tn))))
                })
                .map(|(n, e)| {
                    format!(
                        "ent{n} {} {:?} at ({:.0} {:.0} {:.0}) angles ({:.0} {:.0} {:.0}) hidden {}",
                        e.class_tag,
                        if e.mesh.is_empty() { e.brush.clone().unwrap_or_default() } else { e.mesh.clone() },
                        e.origin[0],
                        e.origin[1],
                        e.origin[2],
                        e.rot[0],
                        e.rot[1],
                        e.rot[2],
                        e.unseen
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        diag::info!(Sim, "bo2zm t6 door test at {now}ms: {}", rows.join(" | "));
    }
}

/// BO2MP_T6_SOLIDWALK=<model>[#<k>][:<seconds>] (with BO2MP_T6_GOD=1): at that
/// time (default 30 s) the player stands 70 units in front of the first (or
/// k-th, 0 = first) entity with that model (a perk machine's
/// zm_collision_perks1), faces it and walks into it for 1.5 s; his distance
/// to it, and how high he stands above the map's own floor, are logged every
/// 100 ms.
pub(crate) fn solid_walk_test(game: &mut World, now: i64) {
    static WANT: OnceLock<Option<(String, i64)>> = OnceLock::new();
    let Some((model, at)) = WANT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_SOLIDWALK").ok()?;
        let (m, s) = v.split_once(':').unwrap_or((v.as_str(), "30"));
        Some((m.to_owned(), s.parse::<i64>().unwrap_or(30) * 1000))
    }) else {
        return;
    };
    let (at, tick) = (*at, i64::from(crate::SIM_STEP_MILLIS));
    if !(at..at + 1700).contains(&now) {
        return;
    }
    let (ident, k) = model
        .split_once('#')
        .map_or((model.as_str(), 0), |(m, k)| (m, k.parse().unwrap_or(0)));
    let Some((nn, spot, yw)) = game
        .resource::<Zm>()
        .ents
        .iter()
        .where_value(|en| en.mesh == ident)
        .nth(k)
        .map(|(nn, en)| (*nn, en.origin, en.rot[1]))
    else {
        if (at..at + tick).contains(&now) {
            diag::info!(Sim, "bo2zm t6 solidwalk: no {model} at {now}ms");
        }
        return;
    };
    let client = ClientId(0);
    if (at..at + tick).contains(&now) {
        // BO2MP_T6_SOLIDSIDE=<degrees>: which side he comes from (0 = along
        // its forward).
        let mut side: f32 = std::env::var("BO2MP_T6_SOLIDSIDE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let (s, c) = (yw + side).to_radians().sin_cos();
        let mut stand = [spot[0] + c * 70.0, spot[1] + s * 70.0, spot[2] + 8.0];
        // BO2MP_T6_SOLIDAUTO=1: he starts from the first side (from that one
        // round in 22.5 degree steps, 70, 56, 90 then 110 units out) where he
        // stands on the map's own floor, not inside it, and the map alone
        // lets him walk to within 10 of its centre.
        if std::env::var("BO2MP_T6_SOLIDAUTO").is_ok_and(|v| v == "1") {
            let mut clear = None;
            'tries: for r in [70.0_f32, 56.0, 90.0, 110.0] {
                for k in 0..16 {
                    let sd = side + k as f32 * 22.5;
                    let (s, c) = (yw + sd).to_radians().sin_cos();
                    let (x, y) = (spot[0] + c * r, spot[1] + s * r);
                    let f = frame(game).trace_static_world(
                        [x, y, spot[2] + 9.0],
                        [x, y, spot[2] - 56.0],
                        crate::shot_trace::BODY_BOX_LOWER,
                        crate::shot_trace::BODY_BOX_UPPER,
                        crate::shot_trace::PLAYER_BODY_MASK,
                    );
                    if f.starts_inside || f.fraction >= 1.0 {
                        continue;
                    }
                    let z = spot[2] + 9.0 - 65.0 * f.fraction;
                    let t = frame(game).trace_static_world(
                        [x, y, z + 1.0],
                        [spot[0], spot[1], z + 1.0],
                        crate::shot_trace::BODY_BOX_LOWER,
                        crate::shot_trace::BODY_BOX_UPPER,
                        crate::shot_trace::PLAYER_BODY_MASK,
                    );
                    if !t.starts_inside && r * (1.0 - t.fraction) <= 10.0 {
                        clear = Some((sd, [x, y, z + 2.0], r));
                        break 'tries;
                    }
                }
            }
            match clear {
                Some((sd, at, r)) => {
                    (side, stand) = (sd, at);
                    diag::info!(Sim, "bo2zm t6 solidwalk: side {sd:.1} at {r:.0} is clear");
                }
                None => diag::info!(Sim, "bo2zm t6 solidwalk: no clear side"),
            }
        }
        super::teleport_player(game, client, stand);
        super::set_player_view(game, client, [0.0, yw + side + 180.0, 0.0]);
        diag::info!(
            Sim,
            "bo2zm t6 solidwalk: ent{nn} {model} at ({:.0} {:.0} {:.0}) yaw {yw:.0}",
            spot[0],
            spot[1],
            spot[2]
        );
        return;
    }
    steer(game, client, |cmd| cmd.thrust_ahead = 127);
    if (now - at) % 100 < tick
        && let Some(me) = frame(game).avatar(client).map(|p| p.origin)
    {
        let d = ((me[0] - spot[0]).powi(2) + (me[1] - spot[1]).powi(2)).sqrt();
        // The map alone: how far toward its centre he could go.
        let to = [spot[0], spot[1], me[2]];
        let t = frame(game).trace_static_world(
            [me[0], me[1], me[2] + 1.0],
            [to[0], to[1], to[2] + 1.0],
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        // The map's own floor under him: standing on a model lifts him above it.
        let f = frame(game).trace_static_world(
            [me[0], me[1], me[2] + 1.0],
            [me[0], me[1], me[2] - 64.0],
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        let above = if f.starts_inside { -1.0 } else { 65.0 * f.fraction - 1.0 };
        diag::info!(
            Sim,
            "bo2zm t6 solidwalk at {now}ms: {d:.0} from ent{nn}; the map alone stops him {:.0} from it; he stands {above:.0} above the map's floor",
            d * (1.0 - t.fraction)
        );
    }
}

/// BO2MP_T6_ZBLOCK=1 (with BO2MP_T6_GOD=1): from 20 s, once a living zombie
/// stands within 100 units of player 0, he turns to it and walks into it
/// for 1.5 s; his distance to it is logged every 100 ms (a zombie stops him
/// ~30 units off, centre to centre; without its box he walks through).
pub(crate) fn zombie_block_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_ZBLOCK").is_some()) || now < 20000 {
        return;
    }
    static AT: std::sync::Mutex<Option<(u32, i64)>> = std::sync::Mutex::new(None);
    let client = ClientId(0);
    let Some(me) = frame(world).avatar(client).map(|p| p.origin) else {
        return;
    };
    let flat = |lhs: [f32; 3], rhs: [f32; 3]| ((lhs[0] - rhs[0]).powi(2) + (lhs[1] - rhs[1]).powi(2)).sqrt();
    let mut at = AT.lock().unwrap_or_else(|en| en.into_inner());
    if at.is_none() {
        let zm = world.resource::<Zm>();
        let near = zm
            .actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, _)| zm.ents.get(nn).map(|en| (*nn, flat(en.origin, me))))
            .filter(|(_, d)| *d < 100.0)
            .min_by(|lhs, rhs| lhs.1.total_cmp(&rhs.1));
        if let Some((nn, d)) = near {
            diag::info!(
                Sim,
                "bo2zm t6 zblock: zombie ent{nn} {d:.0} away at {now}ms, walking into it"
            );
            *at = Some((nn, now));
        }
    }
    let Some((nn, begin)) = *at else {
        return;
    };
    if now > begin + 1500 {
        return;
    }
    let Some(z) = world.resource::<Zm>().ents.get(&nn).map(|en| en.origin) else {
        return;
    };
    let yaw = (z[1] - me[1]).atan2(z[0] - me[0]).to_degrees();
    super::set_player_view(world, client, [0.0, yaw, 0.0]);
    steer(world, client, |cmd| cmd.thrust_ahead = 127);
    if (now - begin) % 100 < i64::from(crate::SIM_STEP_MILLIS) {
        let number = world.resource::<Zm>().presences.by_ent.get(&nn).map(|s| s.num);
        let solid = number
            .and_then(|number| frame(world).script_mover_by_number(number))
            .map(|m| m.state.solid);
        let packed = frame(world).packed_solid_movers().len();
        diag::info!(
            Sim,
            "bo2zm t6 zblock at {now}ms: {:.0} from ent{nn} (me {:.0} {:.0}, it {:.0} {:.0}) mover {number:?} solid {solid:?}, {packed} packed",
            flat(me, z),
            me[0],
            me[1],
            z[0],
            z[1]
        );
    }
}

/// BO2MP_T6_FLOORMAP="x0 x1 y0 y1 step": at 2 s, the ground height under
/// each point of that grid (a straight drop from z 40, under the truck roof), one line per row.
pub(crate) fn floor_map(world: &mut World, now: i64) {
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if !(2000..2000 + tick).contains(&now) {
        return;
    }
    let Some([x0, x1, y0, y1, step]) = env_tuple("BO2MP_T6_FLOORMAP") else {
        return;
    };
    const DROP_FROM: f32 = 40.0;
    const DROP_TO: f32 = -200.0;
    for y in stepped(y0, y1, step) {
        let heights: Vec<String> = stepped(x0, x1, step)
            .map(|x| {
                let hit = frame(world).trace_static_world(
                    [x, y, DROP_FROM],
                    [x, y, DROP_TO],
                    [0.0; 3],
                    [0.0; 3],
                    crate::shot_trace::PLAYER_BODY_MASK,
                );
                format!("{:.0}", DROP_FROM + (DROP_TO - DROP_FROM) * hit.fraction)
            })
            .collect();
        diag::info!(Sim, "bo2zm t6 floor y {y:.0} x {x0:.0}..: {}", heights.join(" "));
    }
}

/// `from`, `from + step`, ... while the running sum stays at or below `to`.
fn stepped(from: f32, to: f32, step: f32) -> impl Iterator<Item = f32> {
    std::iter::successors(Some(from), move |at| Some(at + step)).take_while(move |at| *at <= to)
}

/// BO2MP_T6_HOLESCAN="x0 x1 y0 y1 step [ztop zbot]": at 2 s, every gap in a
/// wall that a small car gets through and a player does not (Nuketown's
/// RC-XD holes): from open ground 24 units each side of a grid point, along
/// x and along y, a small box (8 wide, 2..10 up) passes and a player-tall
/// one is stopped. Each is logged with the widest and tallest box that
/// passes there, and whether the RC-XD's own body (20 wide, 2..14 up) does.
/// Every floor between ztop and zbot counts (default 40 to -200, Nuketown).
pub(crate) fn hole_scan(world: &mut World, now: i64) {
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if !(2000..2000 + tick).contains(&now) {
        return;
    }
    let Ok(spec) = std::env::var("BO2MP_T6_HOLESCAN") else {
        return;
    };
    let (x0, x1, y0, y1, step, ztop, zbot) = match numbers(&spec)[..] {
        [x0, x1, y0, y1, step] => (x0, x1, y0, y1, step, 40.0, -200.0),
        [x0, x1, y0, y1, step, ztop, zbot] => (x0, x1, y0, y1, step, ztop, zbot),
        _ => return,
    };
    // BO2's vehicle clipmask (veh.c: 0x800211, no player clip; the RC-XD's
    // too, ride.rs BODY_MASK) and what stops a player.
    let car: u32 = 0x0080_0211;
    let player = crate::shot_trace::PLAYER_BODY_MASK;
    let facing_x = ((x1 - x0) / step) as usize + 1;
    let facing_y = ((y1 - y0) / step) as usize + 1;
    let clock_start = std::time::Instant::now();
    // Every floor under each point, top down (a roof and the floor under it).
    let mut ground: Vec<Vec<f32>> = vec![Vec::new(); facing_x * facing_y];
    let mut floors = 0usize;
    for j in 0..facing_y {
        for i in 0..facing_x {
            let (x, y) = (x0 + i as f32 * step, y0 + j as f32 * step);
            let (mut top, mut tries) = (ztop, 0);
            while top > zbot && ground[j * facing_x + i].len() < 8 && tries < 64 {
                tries += 1;
                let t = frame(world).trace_static_world(
                    [x, y, top],
                    [x, y, zbot],
                    [0.0; 3],
                    [0.0; 3],
                    crate::shot_trace::PLAYER_BODY_MASK,
                );
                if t.starts_inside {
                    top -= 16.0;
                    continue;
                }
                if t.fraction >= 1.0 {
                    break;
                }
                let z = top - (top - zbot) * t.fraction;
                ground[j * facing_x + i].push(z);
                floors += 1;
                top = z - 16.0;
            }
        }
    }
    let reach = (24.0 / step).round().max(1.0) as isize;
    let passes = |world: &mut World, a: [f32; 3], b: [f32; 3], half: f32, top: f32, mask: u32| {
        let (mins, maxs) = ([-half, -half, 2.0], [half, half, top]);
        let w = frame(world);
        !w.trace_static_world(a, a, mins, maxs, mask).starts_inside
            && !w.trace_static_world(b, b, mins, maxs, mask).starts_inside
            && {
                let t = w.trace_static_world(a, b, mins, maxs, mask);
                !t.starts_inside && t.fraction >= 1.0
            }
    };
    let mut found = 0;
    for j in 0..facing_y as isize {
        for i in 0..facing_x as isize {
            for (axis, di, dj) in [("x", reach, 0), ("y", 0, reach)] {
                let (ia, ja, ib, jb) = (i - di, j - dj, i + di, j + dj);
                let cell = |i: isize, j: isize| {
                    (i >= 0 && j >= 0 && (i as usize) < facing_x && (j as usize) < facing_y)
                        .then(|| ground[j as usize * facing_x + i as usize].as_slice())
                        .unwrap_or(&[])
                };
                let (fa, fb) = (cell(ia, ja), cell(ib, jb));
                for &ga in fa {
                    let Some(&gb) = fb.iter().find(|&&gb| (ga - gb).abs() <= 24.0) else {
                        continue;
                    };
                    let at = |i: isize, j: isize, z: f32| {
                        [x0 + i as f32 * step, y0 + j as f32 * step, z]
                    };
                    let base = ga.max(gb);
                    let Some(z) = [base, base + 8.0].into_iter().find(|&z| {
                        passes(world, at(ia, ja, z), at(ib, jb, z), 4.0, 10.0, car)
                    }) else {
                        continue;
                    };
                    let (a, b) = (at(ia, ja, z), at(ib, jb, z));
                    let (mins, maxs) = ([-4.0, -4.0, 2.0], [4.0, 4.0, 64.0]);
                    let w = frame(world);
                    let tall_ends_free = !w.trace_static_world(a, a, mins, maxs, player).starts_inside
                        && !w.trace_static_world(b, b, mins, maxs, player).starts_inside;
                    if !tall_ends_free || w.trace_static_world(a, b, mins, maxs, player).fraction >= 1.0 {
                        continue;
                    }
                    // A player-tall box the car's mask lets through: only player
                    // clip stops the player there; else it is a low gap.
                    let kind = if w.trace_static_world(a, b, mins, maxs, car).fraction >= 1.0 {
                        "clip"
                    } else {
                        "gap"
                    };
                    let half = [6.0, 8.0, 10.0, 12.0, 16.0]
                        .into_iter()
                        .take_while(|&h| passes(world, a, b, h, 10.0, car))
                        .last()
                        .unwrap_or(4.0);
                    let top = [14.0, 18.0, 24.0, 32.0, 48.0]
                        .into_iter()
                        .take_while(|&t| passes(world, a, b, 4.0, t, car))
                        .last()
                        .unwrap_or(10.0);
                    let rc = passes(world, a, b, 10.0, 14.0, car);
                    found += 1;
                    diag::info!(
                        Sim,
                        "bo2mp hole at {:.0} {:.0} floor {z:.1} (ground {ga:.1} / {gb:.1}) along {axis}: {kind}, widest half {half}, top {top}, rc-xd body passes {rc}",
                        x0 + i as f32 * step,
                        y0 + j as f32 * step
                    );
                }
            }
        }
    }
    diag::info!(
        Sim,
        "bo2mp hole scan: {found} gaps over {facing_x}x{facing_y} points ({floors} floors) in {:.1} s",
        clock_start.elapsed().as_secs_f32()
    );
}

/// BO2MP_T6_WALK="x y z yaw secs [at]": at 12 s (or `at` s) the player
/// stands at x y z facing yaw, then walks forward for secs; his place is
/// logged every 250 ms (does a door or debris stop him). Several walks
/// split by `;` follow one another (each 1.5 s after the last one ends,
/// unless it gives its own `at`); each logs where it ended.
pub(crate) fn walk_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Vec<[f32; 6]>> = OnceLock::new();
    let walks = WANT.get_or_init(|| {
        let mut out: Vec<[f32; 6]> = Vec::new();
        for part in std::env::var("BO2MP_T6_WALK").unwrap_or_default().split(';') {
            let v: Vec<f32> = part.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if v.len() != 5 && v.len() != 6 {
                continue;
            }
            let next = out.last().map_or(12.0, |w| w[5] + 0.3 + w[4] + 1.5);
            out.push([v[0], v[1], v[2], v[3], v[4], v.get(5).copied().unwrap_or(next)]);
        }
        out
    });
    let Some((i, &[x, y, z, yaw, secs, at])) = walks.iter().enumerate().find(|(_, w)| {
        let t0 = (w[5] * 1000.0) as i64;
        (t0..t0 + 300 + (w[4] * 1000.0) as i64 + 1500).contains(&now)
    }) else {
        return;
    };
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let t0 = (at * 1000.0) as i64;
    // "0 0 0": he walks from where he stands.
    if (t0..t0 + tick).contains(&now) && [x, y, z] != [0.0; 3] {
        super::teleport_player(world, client, [x, y, z]);
        super::set_player_view(world, client, [0.0, yaw, 0.0]);
    }
    let end = t0 + 300 + (secs * 1000.0) as i64;
    // BO2MP_T6_WALK_CLIENT: the console's +forward walks him instead (his
    // client predicts the walk: does it clip as the server does).
    let pushed = std::env::var_os("BO2MP_T6_WALK_CLIENT").is_none();
    // BO2MP_T6_WALK_JUMP=<ms>: he presses jump that long into the walk.
    let jump_at = std::env::var("BO2MP_T6_WALK_JUMP")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .map(|ms| t0 + 300 + ms);
    // BO2MP_T6_WALK_HOLD="sprint duck prone ads": he holds those the whole walk.
    static HOLD: OnceLock<u32> = OnceLock::new();
    let hold = *HOLD.get_or_init(|| {
        let names = std::env::var("BO2MP_T6_WALK_HOLD").unwrap_or_default();
        names.split_whitespace().fold(0, |held, name| match name {
            "sprint" => held | buttons::SPRINT,
            "duck" => held | buttons::DUCK_BUTTON_BIT,
            "prone" => held | buttons::LYING_FLAT,
            "ads" => held | buttons::ADS,
            _ => held,
        })
    });
    if pushed && (t0 + 300..end).contains(&now) {
        let jump = jump_at.is_some_and(|t| (t..t + 100).contains(&now));
        steer(world, client, |cmd| {
            if jump {
                cmd.buttons |= buttons::LEAP;
            }
            cmd.buttons |= hold;
            cmd.thrust_ahead = 127;
        });
    }
    // BO2MP_T6_WALK_SPIN=<degrees a second>: he turns that fast as he walks.
    static SPIN: OnceLock<f32> = OnceLock::new();
    let spin = *SPIN.get_or_init(|| {
        std::env::var("BO2MP_T6_WALK_SPIN").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0.0)
    });
    if spin != 0.0 && (t0 + 300..end).contains(&now) {
        let turned = spin * (now - t0 - 300) as f32 / 1000.0;
        super::set_player_view(world, client, [0.0, (yaw + turned).rem_euclid(360.0), 0.0]);
    }
    // BO2MP_T6_WALK_LOOK="x y z": when the walk ends he looks at that point
    // (a wall buy's chalk) and holds use for 500 ms.
    if let Some(at) = env_tuple::<3>("BO2MP_T6_WALK_LOOK")
        && (end + 200..end + 1200).contains(&now)
    {
        if let Some(pstate) = frame(world).avatar(client).copied() {
            let eye = [
                pstate.origin[0],
                pstate.origin[1],
                pstate.origin[2] + pstate.eye_height_now,
            ];
            super::set_player_view(world, client, look(eye, at));
        }
        if (end + 600..end + 1100).contains(&now) {
            steer(world, client, |cmd| cmd.buttons |= buttons::INTERACT_KEY);
        }
    }
    if (t0..end + 1500).contains(&now) && (now - t0) % 250 < tick {
        if let Some(pstate) = frame(world).avatar(client) {
            diag::info!(
                Sim,
                "bo2zm t6 walk test at {now}ms: ({:.0} {:.0} {:.0})",
                pstate.origin[0],
                pstate.origin[1],
                pstate.origin[2]
            );
        }
    }
    if walks.len() > 1 && (end..end + tick).contains(&now) {
        if let Some(pstate) = frame(world).avatar(client) {
            let d = ((pstate.origin[0] - x).powi(2) + (pstate.origin[1] - y).powi(2)).sqrt();
            diag::info!(
                Sim,
                "bo2zm t6 walk {i} yaw {yaw:.0} ended at ({:.0} {:.0} {:.0}) after {d:.0} units",
                pstate.origin[0],
                pstate.origin[1],
                pstate.origin[2]
            );
        }
    }
}

/// BO2MP_T6_FACE=1: the player keeps facing the nearest living zombie
/// within 800 units (its chest), to watch one come (test aid).
pub(crate) fn face_test(world: &mut World) {
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_FACE").is_some()) {
        return;
    }
    // BO2MP_T6_FACE_STOP=<seconds>: then the view holds where it was (to
    // watch what happens to the zombie it was on).
    static STOP: OnceLock<Option<i64>> = OnceLock::new();
    let stop = *STOP.get_or_init(|| {
        let s = std::env::var("BO2MP_T6_FACE_STOP").ok()?;
        Some((s.trim().parse::<f32>().ok()? * 1000.0) as i64)
    });
    if stop.is_some_and(|s| world.resource::<Zm>().clock_millis >= s) {
        return;
    }
    let client = ClientId(0);
    let Some(pstate) = frame(world).avatar(client).copied() else {
        return;
    };
    let nearest = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, lhs)| {
                zm.ents
                    .get(nn)
                    .filter(|e| flat_dist(e.origin, pstate.origin) < 800.0)
                    .map(|e| (*nn, e.origin, e.rot[1], lhs.clone()))
            })
            .min_by(|lhs, rhs| flat_dist(lhs.1, pstate.origin).total_cmp(&flat_dist(rhs.1, pstate.origin)))
    };
    // Every 100 ms: that zombie's state, animation, facing against the way
    // to him, and distance.
    let now = world.resource::<Zm>().clock_millis;
    if let Some((nn, o, yaw, lhs)) = &nearest
        && now % 100 < i64::from(crate::SIM_STEP_MILLIS)
    {
        let to_me = (pstate.origin[1] - o[1])
            .atan2(pstate.origin[0] - o[0])
            .to_degrees();
        diag::info!(
            Sim,
            "bo2zm t6 face test {now}: zombie {nn} {} {} yaw {yaw:.0} to me {to_me:.0} dist {:.0} path {}/{}",
            lhs.script,
            lhs.playing.as_ref().map_or("-", |p| p.anim.as_str()),
            flat_dist(*o, pstate.origin),
            lhs.path_i,
            lhs.path.len()
        );
    }
    let target = nearest.map(|t| t.1);
    if let Some(o) = target {
        let eye = [
            pstate.origin[0],
            pstate.origin[1],
            pstate.origin[2] + pstate.eye_height_now,
        ];
        super::set_player_view(world, client, look(eye, [o[0], o[1], o[2] + 45.0]));
    }
}

/// BO2MP_T6_KITE=1 (with BO2MP_T6_GOD=1): once a zombie is swinging at him
/// (combat, within 80 units), the player turns away from it and walks for
/// 6 s, then stands 6 s; every 500 ms his place and each zombie within 400
/// units (state, animation, place, goal) are logged: does a zombie that
/// swung follow him.
pub(crate) fn kite_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    static START: std::sync::Mutex<Option<(i64, f32)>> = std::sync::Mutex::new(None);
    if !*ON.get_or_init(|| std::env::var("BO2MP_T6_KITE").is_ok()) {
        return;
    }
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let Some(pstate) = frame(world).avatar(client).copied() else {
        return;
    };
    let from_pt = *START.lock().unwrap();
    let Some((t_at, yaw_at)) = from_pt else {
        // A zombie swinging close by: turn away from it and go.
        let found = {
            let zm = world.resource::<Zm>();
            zm.actors
                .iter()
                .where_value(|lhs| lhs.living && lhs.script == "combat")
                .filter_map(|(nn, _)| zm.ents.get(nn).map(|en| en.origin))
                .find(|o| flat_dist(*o, pstate.origin) < 80.0)
        };
        if let Some(o) = found {
            let away = (pstate.origin[1] - o[1])
                .atan2(pstate.origin[0] - o[0])
                .to_degrees();
            *START.lock().unwrap() = Some((now, away));
            super::set_player_view(world, client, [0.0, away, 0.0]);
            diag::info!(
                Sim,
                "bo2zm t6 kite test: zombie swinging at ({:.0} {:.0}); walking away at yaw {away:.0}",
                o[0],
                o[1]
            );
        }
        return;
    };
    if now - t_at < 6000 {
        super::set_player_view(world, client, [0.0, yaw_at, 0.0]);
        steer(world, client, |cmd| cmd.thrust_ahead = 127);
    }
    if now - t_at <= 12_000 && (now - t_at) % 500 < tick {
        let rows: Vec<String> = {
            let zm = world.resource::<Zm>();
            zm.actors
                .iter()
                .where_value(|lhs| lhs.living)
                .filter_map(|(nn, lhs)| {
                    let en = zm.ents.get(nn)?;
                    (flat_dist(en.origin, pstate.origin) < 400.0).then(|| {
                        format!(
                            "{nn} {} {} at ({:.0} {:.0}) goal {:?}",
                            lhs.script,
                            lhs.playing.as_ref().map_or("-", |p| p.anim.as_str()),
                            en.origin[0],
                            en.origin[1],
                            lhs.goal.map(|g| [g[0].round(), g[1].round()])
                        )
                    })
                })
                .collect()
        };
        diag::info!(
            Sim,
            "bo2zm t6 kite test +{}ms: me ({:.0} {:.0}) | {}",
            now - t_at,
            pstate.origin[0],
            pstate.origin[1],
            rows.join(" | ")
        );
    }
}

/// BO2MP_T6_GAMEOVER=<seconds>: then the level's `end_game` (the scripts'
/// game over: GAME OVER, the map's intermission shot, a new game).
pub(crate) fn game_over_test(world: &mut World, now: i64) {
    static AT: OnceLock<Option<i64>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        std::env::var("BO2MP_T6_GAMEOVER")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
    }) else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (at * 1000..at * 1000 + tick).contains(&now) {
        with_vm(world, |vm, world| {
            let level = vm.level;
            vm.notify_str(world, level, "end_game", &[]);
        });
        diag::info!(Sim, "bo2zm t6 game over test: end_game at {at}s");
    }
}

/// BO2MP_T6_SPRINTLOG=1: every start and end of his sprint (how long it
/// ran, whether sprint must be pressed again, the flags then).
pub(crate) fn sprint_log(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    static WAS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_SPRINTLOG").is_some()) {
        return;
    }
    let f = frame(world);
    let Some(pstate) = f.avatar(ClientId(0)) else {
        return;
    };
    // And each change of his dive state (dive, slide, stay prone).
    static DIVE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dive = pstate.movement_state_bits & 0x0700_0000;
    if DIVE.swap(dive, std::sync::atomic::Ordering::Relaxed) != dive {
        diag::info!(Sim, "bo2zm t6 dive state {dive:#x} at {now}ms, pm_flags {:#x}", pstate.movement_state_bits);
    }
    let on = pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::SPRINTING != 0;
    let was = WAS.swap(u8::from(on), std::sync::atomic::Ordering::Relaxed);
    if was != 2 && was != u8::from(on) {
        diag::info!(
            Sim,
            "bo2zm t6 sprint {} at {now}ms: ran {}ms, press again {}, pm_flags {:#x}, weapon state {:#x}, max {}ms",
            if on { "on" } else { "off" },
            pstate.sprint_state.sprint_stop_ms.wrapping_sub(pstate.sprint_state.sprint_begin_ms),
            pstate.sprint_state.sprint_needs_release,
            pstate.movement_state_bits, pstate.weaponstate_primary,
            pstate.sprint_state.sprint_launch_cap
        );
    }
}

/// BO2MP_T6_PRESS="<frag|smoke|attack> <seconds>[, ...]": then each button
/// is held for 200 ms (the throw a player's G or 4 key makes, a click).
pub(crate) fn press_test(world: &mut World, now_ms: i64) {
    static WANT: OnceLock<Vec<(u32, i64)>> = OnceLock::new();
    let want = WANT.get_or_init(|| {
        let Ok(v) = std::env::var("BO2MP_T6_PRESS") else {
            return Vec::new();
        };
        v.split(',')
            .filter_map(|one| {
                let (b, t) = one.trim().split_once(' ')?;
                let b = match b {
                    "frag" => buttons::SAMPLE_FRAG_ASSET,
                    "smoke" => buttons::OBSCURANT,
                    "attack" => buttons::ATTACK,
                    _ => return None,
                };
                Some((b, (t.trim().parse::<f32>().ok()? * 1000.0) as i64))
            })
            .collect()
    });
    let button = want
        .iter()
        .where_value(|at| (*at..*at + 200).contains(&now_ms))
        .fold(0, |held, (b, _)| held | b);
    if button != 0 {
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        for (id, cmd) in &mut req.input.cmds {
            if *id == ClientId(0) {
                cmd.buttons |= button;
            }
        }
    }
}

/// BO2MP_T6_GIVE="<weapon>@<seconds>[,<weapon>@<seconds>...]": then the
/// scripts give him that weapon the way a wall buy or the box does
/// (`weapon_give`: the Time Bomb's own give runs).
pub(crate) fn give_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Vec<(String, i64)>> = OnceLock::new();
    let want = WANT.get_or_init(|| {
        std::env::var("BO2MP_T6_GIVE")
            .unwrap_or_default()
            .split(',')
            .filter_map(|item| {
                let (w, t) = item.trim().split_once('@')?;
                Some((w.to_owned(), (t.trim().parse::<f32>().ok()? * 1000.0) as i64))
            })
            .collect()
    });
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    // A `+name` (the engine's bare `giveweapon`), a `melee:name` and an
    // `egg:name` are use_test.rs's to give.
    for (name, at) in want
        .iter()
        .filter(|(n, at)| !n.starts_with('+') && !n.starts_with("melee:") && !n.starts_with("egg:") && (*at..*at + tick).contains(&now))
    {
        with_vm(world, |vm, world| {
            if let Some(p) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) {
                super::cheats::give_special(vm, world, p, name);
            }
        });
        diag::info!(Sim, "bo2zm t6 give test: {name} at {}s", at / 1000);
    }
}

/// BO2MP_T6_NADES=<seconds>: then two frag grenades in his hand (BO2 gives
/// none on round 1; a test of the throw key without playing a round).
pub(crate) fn nades_test(world: &mut World, now: i64) {
    static AT: OnceLock<Option<i64>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| std::env::var("BO2MP_T6_NADES").ok().and_then(|s| s.parse::<i64>().ok()))
    else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (at * 1000..at * 1000 + tick).contains(&now)
        && let Ok(w) = super::weapon(world, "frag_grenade_zm")
    {
        crate::client_script::set_ammo_clip(&mut frame(world), ClientId(0), w, 2);
        diag::info!(Sim, "bo2zm t6 nades test: 2 frag grenades at {at}s");
    }
}

/// BO2MP_T6_DOWN=<seconds>[@<client>]: when the down test's blow lands,
/// and on whom (client 0, the host, unless `@1` names player 2).
fn down_at() -> Option<(i64, u32)> {
    static AT: OnceLock<Option<(i64, u32)>> = OnceLock::new();
    *AT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_DOWN").ok()?;
        let (s, who) = v.split_once('@').unwrap_or((v.as_str(), "0"));
        Some((s.trim().parse().ok()?, who.trim().parse().ok()?))
    })
}

/// A party's test clock (BO2MP_T6_PLAYERS 2+): from when every expected
/// player is first playing, since the second can still be loading a minute
/// after the host and a blow before he is in is a solo game over. Solo, the
/// match's own clock. None until they are all in.
fn party_clock(world: &mut World, now: i64) -> Option<i64> {
    let want = super::expected_players();
    if want < 2 {
        return Some(now);
    }
    if let Some(t) = world.resource::<Zm>().party_in_at {
        return Some(now - t);
    }
    let playing = world
        .resource::<Zm>()
        .players
        .values()
        .filter(|p| p.sessionstate == "playing")
        .count();
    if (playing as i32) < want {
        return None;
    }
    world.resource_mut::<Zm>().party_in_at = Some(now);
    diag::info!(Sim, "bo2zm t6 party: all {want} playing at {}s", now / 1000);
    Some(0)
}

/// BO2MP_T6_DOWN=<seconds>[@<client>]: then a zombie's blow takes all his
/// health (he goes down; solo with no Quick Revive that is the game over),
/// even under BO2MP_T6_GOD (so a Quick Revive test can buy it first). Every
/// second after, his eye height and view are logged (down-glitch test). In
/// a party the seconds count from when everyone is playing.
pub(crate) fn down_test(world: &mut World, now: i64) {
    let Some((at, who)) = down_at() else {
        return;
    };
    let Some(now) = party_clock(world, now) else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (at * 1000..at * 1000 + tick).contains(&now) {
        // What decides a solo death (_zm player_damage_override): his
        // perks, his `lives`, and the game's cached `using_solo_revive`.
        let (perks, obj) = world
            .resource::<Zm>()
            .players
            .get(&who)
            .map_or((Vec::new(), None), |p| (p.perks.iter().cloned().collect::<Vec<_>>(), Some(p.obj)));
        let (lives, solo) = with_vm(world, |vm, world| {
            let lf = vm.intern("lives");
            let sf = vm.intern("using_solo_revive");
            let lives = obj.map(|o| vm.get_field(world, o, lf));
            let solo = vm.get_field(world, vm.level, sf);
            (lives.map_or("-".to_owned(), |v| vm.to_text(&v)), vm.to_text(&solo))
        })
        .unwrap_or_default();
        diag::info!(Sim, "bo2zm t6 down test: before the blow perks {perks:?} lives {lives} using_solo_revive {solo}");
        with_vm(world, |vm, world| {
            let point = frame(world).avatar(ClientId(who)).map_or([0.0; 3], |pstate| pstate.origin);
            super::natives_ai::TEST_BLOW.store(true, std::sync::atomic::Ordering::Relaxed);
            super::natives_ai::player_damage(
                vm,
                world,
                who,
                gsc_t6::ScriptValue::Undefined,
                gsc_t6::ScriptValue::Undefined,
                1000,
                0,
                "MOD_MELEE",
                "none",
                point, [0.0, 0.0, -1.0],
                "torso_upper",
            );
        });
        diag::info!(Sim, "bo2zm t6 down test: 1000 damage at {at}s");
    }
    if now > at * 1000 && now < at * 1000 + 60_000 && now % 250 < tick {
        let (down, linked) = world
            .resource::<Zm>()
            .players
            .get(&who)
            .map_or((false, None), |p| (p.laststand, p.linked));
        if let Some(pstate) = frame(world).avatar(ClientId(who)) {
            diag::info!(
                Sim,
                "bo2zm t6 down test +{}ms: down {down} linked {linked:?} z {:.1} view_h {:.1} pm_type {} eflags {:#x} pm_flags {:#x}",
                now - at * 1000,
                pstate.origin[2], pstate.eye_height_now,
                pstate.movement_mode,
                pstate.e_flags,
                pstate.movement_state_bits
            );
        }
    }
    // The 1.5 s after he is up again, every tick: the stance target, its
    // lerp, his stance buttons and the last events (stood-up-prone trace).
    static UP_AT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);
    static WAS_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    use std::sync::atomic::Ordering::Relaxed;
    let down = world.resource::<Zm>().players.get(&who).is_some_and(|p| p.laststand);
    if down {
        WAS_DOWN.store(true, Relaxed);
    } else if WAS_DOWN.swap(false, Relaxed) {
        UP_AT.store(now, Relaxed);
    }
    let up = UP_AT.load(Relaxed);
    if up != i64::MIN && now - up <= 1500 {
        let buttons = frame(world).last_tick_buttons(ClientId(who)).unwrap_or(0);
        if let Some(pstate) = frame(world).avatar(ClientId(who)) {
            diag::info!(
                Sim,
                "bo2zm t6 up trace +{}ms: pm_type {} pm_flags {:#x} target {} lerp_time {} lerp_target {} view_h {:.1} buttons {:#x} seq {} ev [{} {} {} {}]",
                now - up,
                pstate.movement_mode,
                pstate.movement_state_bits,
                pstate.eye_height_goal,
                pstate.eye_blend_clock,
                pstate.eye_blend_goal, pstate.eye_height_now,
                buttons,
                pstate.predictable_event_sequence,
                pstate.predictable_events[0],
                pstate.predictable_events[1],
                pstate.predictable_events[2],
                pstate.predictable_events[3]
            );
        }
    }
}

/// BO2MP_T6_COREVIVE=1 (two players, with BO2MP_T6_DOWN): once the down
/// test's player is down, the other is put 40 units from him (on his own
/// side), looks at him and holds Use till he is up: BO2's revive asks him to
/// touch the 75-unit revive trigger, face him and hold Use (_zm_laststand
/// can_revive). Both are logged each second.
pub(crate) fn corevive_test(world: &mut World, now: i64) {
    static ON: OnceLock<bool> = OnceLock::new();
    static PUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    use std::sync::atomic::Ordering::Relaxed;
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_COREVIVE").is_some()) {
        return;
    }
    let Some(hurt) = down_at().map(|hit| hit.1) else {
        return;
    };
    let aid = u32::from(hurt == 0);
    let (Some(body), Some(me)) = (
        frame(world).avatar(ClientId(hurt)).map(|p| p.origin),
        frame(world).avatar(ClientId(aid)).map(|p| p.origin),
    ) else {
        return;
    };
    let (down, down2) = {
        let zm = world.resource::<Zm>();
        let down = |c| zm.players.get(&c).is_some_and(|p| p.laststand);
        (down(hurt), down(aid))
    };
    let d = flat_dist(body, me);
    let (n, m) = (hurt + 1, aid + 1);
    if now % 1000 < i64::from(crate::SIM_STEP_MILLIS) {
        diag::info!(
            Sim,
            "bo2zm t6 corevive at {}s: player {n} down {down}, player {m} down {down2} at {:?}, {d:.0} from him",
            now / 1000,
            me.map(f32::round)
        );
    }
    if !down {
        if PUT.swap(false, Relaxed) {
            diag::info!(Sim, "bo2zm t6 corevive: player {n} up again at {}s", now / 1000);
        }
        return;
    }
    if !PUT.swap(true, Relaxed) && d > 40.0 {
        let k = 40.0 / d;
        let spot = [
            body[0] + (me[0] - body[0]) * k,
            body[1] + (me[1] - body[1]) * k,
            body[2] + 8.0,
        ];
        super::teleport_player(world, ClientId(aid), spot);
        diag::info!(Sim, "bo2zm t6 corevive: player {m} put beside him at {:?}", spot.map(f32::round));
    }
    let eye = [me[0], me[1], me[2] + 60.0];
    super::set_player_view(world, ClientId(aid), look(eye, body));
    let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
    for (id, cmd) in &mut req.input.cmds {
        if *id == ClientId(aid) {
            cmd.buttons |= buttons::INTERACT_KEY;
            cmd.thrust_ahead = 0;
            cmd.thrust_lateral = 0;
        }
    }
}

/// When the perk test's clock starts (its "42 s"), if not at 0: `landed`, or
/// Mob's shock test once the box is on and he is up again.
pub(crate) static PERK_BASE: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(i64::MIN);

/// BO2MP_T6_PERK=<machine targetname>[@<seconds>] (e.g. vending_revive, or
/// `landed` for the lowest machine): at 42 s (the
/// first perk has landed) the player gets 10000 points and stands 40 units
/// in front of the machine, looking at it; he presses use at 43 s; at 50 s
/// his perks, weapons and points are logged. `@<seconds>` starts it then
/// instead of at 42 s (after a power test). A comma list buys each in turn
/// (`vending_revive@60,vending_jugg@80`: Tranzit's six machines in one run).
pub(crate) fn perk_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Vec<(String, i64)>> = OnceLock::new();
    let wants = WANT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_PERK").unwrap_or_default();
        v.split(',')
            .filter(|one| !one.is_empty())
            .map(|one| match one.split_once('@') {
                Some((n, s)) => (
                    n.to_owned(),
                    s.parse::<f32>().map_or(0, |s| (s * 1000.0) as i64 - 42000),
                ),
                None => (one.to_owned(), 0),
            })
            .collect()
    });
    for (want, shift) in wants {
        perk_test_one(world, now, want, *shift);
    }
}

fn perk_test_one(world: &mut World, now: i64, want: &str, shift: i64) {
    let now = now - shift;
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    // "landed": the test starts once lhs perk machine is on the ground
    // (Nuketown drops them from the sky, the first some time into round 1):
    // its times count from then as if it were 42 s.
    let base = &PERK_BASE;
    // With Mob's shock test the clock starts when that test is done.
    if std::env::var_os("BO2MP_T6_SHOCK").is_some()
        && base.load(std::sync::atomic::Ordering::Relaxed) == i64::MIN
    {
        return;
    }
    if want == "landed" && base.load(std::sync::atomic::Ordering::Relaxed) == i64::MIN {
        if now < 42000 || now % 1000 >= tick {
            return;
        }
        let low = with_vm(world, |vm, world| {
            let tn = vm.intern("targetname");
            world
                .resource::<Zm>()
                .ents
                .values()
                .filter(|e| e.obj.is_some_and(|o| vm.to_text(&vm.raw_field(o, tn)).starts_with("vending_")))
                .map(|e| e.origin[2])
                .reduce(f32::min)
        })
        .flatten()
        .unwrap_or(f32::MAX);
        // On the ground (Nuketown's streets are near 0; lhs falling one
        // passes through 500).
        if low > 150.0 {
            return;
        }
        base.store(now - 42000, std::sync::atomic::Ordering::Relaxed);
    }
    let base = base.load(std::sync::atomic::Ordering::Relaxed);
    let now = if base == i64::MIN { now } else { now - base };
    if (42000..42000 + tick).contains(&now) {
        // BO2MP_T6_PERK_OPEN=1: every zone opened first (Mob's Speed Cola is
        // past doors; standing in an unopened zone gets him killed).
        if std::env::var_os("BO2MP_T6_PERK_OPEN").is_some() {
            open_all_zones(world);
        }
        let machine = with_vm(world, |vm, world| {
            let tn = vm.intern("targetname");
            let zm = world.resource::<Zm>();
            // "landed": whichever perk machine is lowest.
            if want == "landed" {
                return zm
                    .ents
                    .values()
                    .filter(|e| e.obj.is_some_and(|o| vm.to_text(&vm.raw_field(o, tn)).starts_with("vending_")))
                    .min_by(|lhs, rhs| lhs.origin[2].total_cmp(&rhs.origin[2]))
                    .map(|e| (e.origin, e.rot));
            }
            zm.ents.values().find_map(|e| {
                let o = e.obj?;
                (vm.to_text(&vm.raw_field(o, tn)) == want).then_some((e.origin, e.rot))
            })
        })
        .flatten();
        let Some((spot, rot)) = machine else {
            diag::warn!(Sim, "bo2zm t6 perk test: no {want}");
            return;
        };
        with_vm(world, |vm, world| {
            if let Some(p) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) {
                let f = vm.intern("score");
                vm.set_field(world, p, f, gsc_t6::ScriptValue::Int(10000));
            }
        });
        // A machine faces 90 degrees right of its rot (bring_perk pushes
        // it back along forward(rot - 90)).
        let (fw, _, _) = gsc_t6::math::facing_axes([0.0, rot[1] - 90.0, 0.0]);
        let stand = [
            spot[0] + fw[0] * stand_dist(),
            spot[1] + fw[1] * stand_dist(),
            spot[2] + 30.0,
        ];
        let down = [stand[0], stand[1], stand[2] - 200.0];
        // trace_world, not the static world: Die Rise's machines ride elevator
        // cars, whose floors move (the static trace fell through to the floor below).
        let t = frame(world).trace_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        // A car's floor is lhs turned box (highrise.rs platform_slabs), which
        // trace_world leaves out: take the nearer of the two.
        let mut fraction = t.fraction;
        for rhs in frame(world).oriented_blockers() {
            let brush = crate::sim_parts::SolidBrush {
                sides: rhs.planes().to_vec(),
                solid_bits: crate::shot_trace::HARD_SURFACE_FLAG,
                plane_material_flags: vec![rhs.face_bits; 6],
                glass_hit_code: 0,
                vertex_points: Vec::new(),
                hit_surface_flags: 0,
            };
            let o = crate::sim_parts::t6_brushes_trace(
                &[&brush],
                stand,
                down,
                crate::shot_trace::BODY_BOX_LOWER,
                crate::shot_trace::BODY_BOX_UPPER,
                crate::shot_trace::PLAYER_BODY_MASK,
            );
            if !o.starts_inside && !o.fully_embedded {
                fraction = fraction.min(o.fraction);
            }
        }
        let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * fraction);
        let eye_at = [feet[0], feet[1], feet[2] + 60.0];
        let view = look(eye_at, [spot[0], spot[1], spot[2] + 40.0]);
        super::teleport_player(world, client, feet);
        super::set_player_view(world, client, view);
        diag::info!(
            Sim,
            "bo2zm t6 perk test: {want} at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
            spot[0],
            spot[1],
            spot[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (43000..43300).contains(&now) {
        steer(world, client, |cmd| cmd.buttons |= buttons::INTERACT_KEY);
    }
    if (50000..50000 + tick).contains(&now) {
        let perks: Vec<String> = world
            .resource::<Zm>()
            .players
            .get(&0)
            .map(|p| p.perks.iter().cloned().collect())
            .unwrap_or_default();
        let names: Vec<String> = {
            let f = frame(world);
            crate::client_script::weapons(&f, client, crate::client_script::WeaponList::All)
                .into_iter()
                .map(|w| crate::client_script::weapon_name(&f, w))
                .collect()
        };
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        let hit_point_ceiling = frame(world).avatar(client).map_or(0, |ps| ps.hit_point_ceiling);
        diag::info!(
            Sim,
            "bo2zm t6 perk test: {want}: perks {perks:?}, weapons {names:?}, points {score}, max health {hit_point_ceiling}, hint {hint:?}"
        );
    }
}

/// An entity found by a key: its object, where it stands and how it is turned.
pub(crate) struct Found {
    pub(crate) obj: gsc_t6::ObjRef,
    pub(crate) at: [f32; 3],
    pub(crate) rot: [f32; 3],
}

/// An entity's object by a key's value (`targetname`, `script_noteworthy`).
pub(crate) fn ent_by(
    game: &mut World, lookup_key: &str, val: &str,
) -> Option<Found> {
    with_vm(game, |vm, game| {
        let k = vm.intern(lookup_key);
        let zm = game.resource::<Zm>();
        zm.ents.values().find_map(|e| {
            let o = e.obj?;
            (vm.to_text(&vm.raw_field(o, k)) == val).then_some(Found { obj: o, at: e.origin, rot: e.rot })
        })
    })
    .flatten()
}

/// BO2MP_T6_PAP=1: Pack-a-Punch. At 25 s Nuketown's machine is brought down
/// by the game's own `bring_perk`; at 90 s the player, given 10000 points,
/// stands at it and uses it with his gun; at 100 s he takes the upgraded
/// gun back; at 106 s his weapons and points are logged.
/// BO2MP_T6_PAP=@<seconds>: a machine already there (Tranzit's is built):
/// no bringing, he uses it at that time instead of 90 s.
pub(crate) fn pap_test(game: &mut World, now: i64) {
    static ON: OnceLock<Option<i64>> = OnceLock::new();
    let Some(shift) = *ON.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_PAP").ok()?;
        Some(
            v.strip_prefix('@')
                .and_then(|text| text.parse::<f32>().ok())
                .map_or(0, |s| (s * 1000.0) as i64 - 90000),
        )
    }) else {
        return;
    };
    let now = now - shift;
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let machine = |game: &mut World| {
        let trigger = ent_by(game, "script_noteworthy", "specialty_weapupgrade")?.obj;
        let target = with_vm(game, |vm, _| {
            let t = vm.intern("target");
            vm.to_text(&vm.raw_field(trigger, t))
        })?;
        let Found { obj: m, at: spot, rot } = ent_by(game, "targetname", &target)?;
        Some((trigger, m, spot, rot))
    };
    if shift == 0 && (25000..25000 + tick).contains(&now) {
        let Some((trigger, m, _, _)) = machine(game) else {
            diag::warn!(Sim, "bo2zm t6 pap test: no machine");
            return;
        };
        with_vm(game, |vm, game| {
            let level = gsc_t6::ScriptValue::Object(vm.level);
            vm.spawn_named(
                game,
                "maps/mp/zm_nuked_perks",
                "bring_perk",
                level,
                vec![gsc_t6::ScriptValue::Object(m), gsc_t6::ScriptValue::Object(trigger)],
            );
        });
        diag::info!(Sim, "bo2zm t6 pap test: bringing the machine");
    }
    if (90000..90000 + tick).contains(&now) {
        let Some((spot, rot)) = machine(game).map(|m| (m.2, m.3)) else {
            return;
        };
        with_vm(game, |vm, game| {
            if let Some(p) = game.resource::<Zm>().players.get(&0).map(|p| p.obj) {
                let f = vm.intern("score");
                vm.set_field(game, p, f, gsc_t6::ScriptValue::Int(10000));
            }
        });
        stand_at(game, client, spot, rot);
    }
    if (91000..91300).contains(&now) || (100000..100300).contains(&now) {
        steer(game, client, |cmd| cmd.buttons |= buttons::INTERACT_KEY);
    }
    if [92000, 96000, 101000, 106000]
        .iter()
        .any(|t| (*t..*t + tick).contains(&now))
    {
        let names: Vec<String> = {
            let f = frame(game);
            crate::client_script::weapons(&f, client, crate::client_script::WeaponList::All)
                .into_iter()
                .map(|w| crate::client_script::weapon_name(&f, w))
                .collect()
        };
        let score = frame(game).client_meta(client).map_or(0, |m| m.score);
        let hint = game
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 pap test at {}s: weapons {names:?}, points {score}, hint {hint:?}",
            now / 1000
        );
    }
}

/// BO2MP_T6_POWERUP=<name>[,<name>...]: from 30 s, one every 12 s, that
/// power-up drops 60 units in front of the player (the game's own
/// specific_powerup_drop) and he walks onto it at the next second.
pub(crate) fn powerup_test(world: &mut World, now: i64) {
    // `@<seconds>` after the list: the first drops then instead of at 30 s.
    static WANT: OnceLock<(Vec<String>, i64)> = OnceLock::new();
    let (want, shift) = WANT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_POWERUP").unwrap_or_default();
        let (names, at) = match v.split_once('@') {
            Some((n, s)) => (n, s.parse::<f32>().map_or(0, |s| (s * 1000.0) as i64 - 30000)),
            None => (v.as_str(), 0),
        };
        (names.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect(), at)
    });
    // BO2MP_T6_POWERUP_AT=<secs> (30): when the first one drops (Carpenter
    // wants the zombies to have torn some boards first).
    static AT: OnceLock<i64> = OnceLock::new();
    let at = *AT.get_or_init(|| {
        std::env::var("BO2MP_T6_POWERUP_AT")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .map_or(30000, |s| s * 1000)
    });
    // `@<seconds>` shifts the whole list.
    let now = now - shift;
    if want.is_empty() || now < at {
        return;
    }
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let step = (now - at) / 12000;
    let into = (now - at) % 12000;
    let Some(name) = want.get(step as usize) else {
        return;
    };
    let peer = ClientId(0);
    let Some(pstate) = frame(world).avatar(peer).copied() else {
        return;
    };
    // BO2MP_T6_POWERUP_DIST=<units> (60): how far in front it drops;
    // BO2MP_T6_POWERUP_WALK=0: he stays put (a look at it, not a grab).
    static DIST: OnceLock<(f32, bool)> = OnceLock::new();
    let (dist, walk) = *DIST.get_or_init(|| {
        (
            std::env::var("BO2MP_T6_POWERUP_DIST").ok().and_then(|text| text.parse().ok()).unwrap_or(60.0),
            std::env::var("BO2MP_T6_POWERUP_WALK").map_or(true, |s| s != "0"),
        )
    });
    // In front of him, or behind or to a side when a wall is in the way (a
    // step into a wall is out of the playable area: the game kills him and
    // takes his Quick Revive life).
    let spot = {
        let up = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + 18.0];
        let mut best = (-1.0f32, pstate.origin);
        for turn in [0.0f32, 180.0, 90.0, 270.0] {
            let (fl, _, _) = gsc_t6::math::facing_axes([0.0, pstate.look_angles[1] + turn, 0.0]);
            let to = [up[0] + fl[0] * dist, up[1] + fl[1] * dist, up[2]];
            let t = frame(world).trace_static_world(
                up,
                to,
                crate::shot_trace::BODY_BOX_LOWER,
                crate::shot_trace::BODY_BOX_UPPER,
                crate::shot_trace::PLAYER_BODY_MASK,
            );
            if !t.starts_inside && t.fraction > best.0 {
                best = (t.fraction, [pstate.origin[0] + fl[0] * dist, pstate.origin[1] + fl[1] * dist, pstate.origin[2]]);
            }
            if best.0 >= 1.0 {
                break;
            }
        }
        best.1
    };
    if into < tick {
        diag::info!(Sim, "bo2zm t6 powerup test: before {name}: {}", boards_and_score(world, peer));
        with_vm(world, |vm, world| {
            let n = vm.string(name);
            vm.spawn_named(
                world,
                "maps/mp/zombies/_zm_powerups",
                "specific_powerup_drop",
                gsc_t6::ScriptValue::Object(vm.level),
                vec![n, gsc_t6::ScriptValue::Vec3(spot)],
            );
        });
        diag::info!(Sim, "bo2zm t6 powerup test: {name} dropped");
    }
    if walk && (1000..1000 + tick).contains(&into) {
        super::teleport_player(world, peer, spot);
        diag::info!(Sim, "bo2zm t6 powerup test: walked onto {name}");
    }
    // A second later: the team's power-up state (`zombie_vars["allies"]`:
    // *_on flags, point scalar) the grab should have changed.
    if (2000..2000 + tick).contains(&into) {
        let state = with_vm(world, |vm, _| {
            let zv = vm.intern("zombie_vars");
            let gsc_t6::ScriptValue::Array(vars) = vm.raw_field(vm.level, zv) else {
                return String::new();
            };
            let allies = vars
                .read()
                .map
                .iter()
                .find(|(k, _)| vm.to_text(&k.value()) == "allies")
                .map(|(_, v)| v.clone());
            let Some(gsc_t6::ScriptValue::Array(team)) = allies else {
                return "no allies vars".to_owned();
            };
            let mut out: Vec<String> = team
                .read()
                .map
                .iter()
                .map(|(k, v)| (vm.to_text(&k.value()), vm.to_text(v)))
                .filter(|(k, _)| k.ends_with("_on") || k.contains("scalar") || k.contains("insta"))
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            out.sort();
            out.join(" ")
        })
        .unwrap_or_default();
        let ammo = {
            let fl = frame(world);
            format!(
                "{}+{}",
                crate::client_script::ammo_clip(&fl, peer, pstate.weapon),
                crate::client_script::rounds_held(&fl, peer, pstate.weapon)
            )
        };
        diag::info!(Sim, "bo2zm t6 powerup test: after {name}: {state}; ammo {ammo}");
    }
    // Carpenter puts a board back every ~0.1 s, then pays 200 points.
    if [5000, 10000].iter().any(|t| (*t..*t + tick).contains(&into)) {
        diag::info!(
            Sim,
            "bo2zm t6 powerup test: {} s after {name}: {}",
            into / 1000,
            boards_and_score(world, peer)
        );
    }
}

/// Window boards (pieces of typed zbarriers, the box has no board type)
/// not in the "closed" state, of all, and the player's points.
fn boards_and_score(world: &mut World, client: ClientId) -> String {
    let (mut torn, mut all) = (0, 0);
    for z in world.resource::<Zm>().ents.values().filter_map(|e| e.zbarrier.as_ref()) {
        if z.def.as_ref().is_none_or(|d| d.boards.is_empty()) {
            continue;
        }
        all += z.pieces.len();
        torn += z.pieces.iter().filter(|p| p.state != "closed").count();
    }
    let score = frame(world).client_meta(client).map_or(0, |m| m.score);
    format!("boards torn {torn}/{all}, points {score}")
}

/// BO2MP_T6_GIBTEST=<secs>[,<gib_ref>]: then the living zombie nearest the
/// player loses that piece (right_arm by default; head, left_leg, ...) by
/// the scripts' own zm_death::do_gib (a gore test without the aim).
pub(crate) fn gib_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Option<(i64, String)>> = OnceLock::new();
    let Some((at, gib_ref)) = WANT.get_or_init(|| {
            let v = std::env::var("BO2MP_T6_GIBTEST").ok()?;
            let (s, r) = v.split_once(',').unwrap_or((v.as_str(), "right_arm"));
            Some((s.trim().parse::<i64>().ok()?, r.trim().to_owned()))
        })
        .clone()
    else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if !(at * 1000..at * 1000 + tick).contains(&now) {
        return;
    }
    let me = frame(world).avatar(ClientId(0)).map_or([0.0; 3], |ps| ps.origin);
    let tgt = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, lhs)| {
                let o = zm.ents.get(nn)?.origin;
                let d = (o[0] - me[0]).powi(2) + (o[1] - me[1]).powi(2);
                Some((d, *nn, lhs.obj?))
            })
            .min_by(|x, y| x.0.total_cmp(&y.0))
    };
    let Some((nn, obj)) = tgt.map(|t| (t.1, t.2)) else {
        diag::info!(Sim, "bo2zm t6 gib test: no living zombie at {at}s");
        return;
    };
    diag::info!(Sim, "bo2zm t6 gib test: zombie {nn} loses {gib_ref}");
    with_vm(world, |vm, world| {
        let lhs = vm.intern("a");
        if let gsc_t6::ScriptValue::Object(st) = vm.raw_field(obj, lhs) {
            let f = vm.intern("gib_ref");
            let v = vm.string(&gib_ref);
            vm.set_raw_field(st, f, v);
        }
        vm.spawn_named(
            world,
            "maps/mp/animscripts/zm_death",
            "do_gib",
            gsc_t6::ScriptValue::Object(obj),
            Vec::new(),
        );
    });
    diag::info!(Sim, "bo2zm t6 gib test: do_gib returned");
}

/// How far in front of a machine the tests stand (BO2MP_T6_STAND, 40).
fn stand_dist() -> f32 {
    static D: OnceLock<f32> = OnceLock::new();
    *D.get_or_init(|| {
        std::env::var("BO2MP_T6_STAND")
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or(40.0)
    })
}

/// The player stands 40 units in front of a machine, looking at it.
fn stand_at(game: &mut World, peer: ClientId, spot: [f32; 3], rot: [f32; 3]) {
    // A machine faces 90 degrees right of its rot (bring_perk pushes
    // it back along forward(rot - 90)).
    let (fw, _, _) = gsc_t6::math::facing_axes([0.0, rot[1] - 90.0, 0.0]);
    let stand = [
        spot[0] + fw[0] * stand_dist(),
        spot[1] + fw[1] * stand_dist(),
        spot[2] + 30.0,
    ];
    let down = [stand[0], stand[1], stand[2] - 200.0];
    let t = frame(game).trace_static_world(
        stand,
        down,
        crate::shot_trace::BODY_BOX_LOWER,
        crate::shot_trace::BODY_BOX_UPPER,
        crate::shot_trace::PLAYER_BODY_MASK,
    );
    let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
    let eye_at = [feet[0], feet[1], feet[2] + 60.0];
    let view = look(eye_at, [spot[0], spot[1], spot[2] + 40.0]);
    super::teleport_player(game, peer, feet);
    super::set_player_view(game, peer, view);
    diag::info!(
        Sim,
        "bo2zm t6 test: machine at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
        spot[0],
        spot[1],
        spot[2],
        feet[0],
        feet[1],
        feet[2]
    );
}

/// The rounds test's kit: the gun at 8 s, ammo topped up every 2 s. Mob
/// starts him in the Afterlife, and leaving it gives back what he had going
/// in (`afterlife_give_loadout`), so there the gun waits until 2 s after he
/// is back in his body.
fn rounds_kit(game: &mut World, now_ms: i64, peer: ClientId) {
    use std::sync::atomic::{AtomicI64, Ordering};
    static GIVE_AT: AtomicI64 = AtomicI64::new(8000);
    static RAISE_AT: AtomicI64 = AtomicI64::new(i64::MAX);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let now_ms = now_ms - from_millis_value();
    let give_at = GIVE_AT.load(Ordering::Relaxed);
    if give_at != i64::MAX && now_ms >= give_at {
        if super::autoplay_prison::afterlife_corpse(game, peer).is_some() {
            GIVE_AT.store(now_ms + 2000, Ordering::Relaxed);
            return;
        }
        GIVE_AT.store(i64::MAX, Ordering::Relaxed);
        let guns = std::env::var("BO2MP_T6_AUTOPLAY_GUN").unwrap_or_else(|_| "m14_zm".to_owned());
        // BO2MP_T6_AUTOPLAY_GUN=a,b: a is given; every 2 s he swaps between
        // it and b, a gun he holds already (a Winter's Howl freeze, then
        // the start pistol to shatter the frozen body).
        for gun in guns.split(',').take(1) {
            with_vm(game, |vm, game| {
                if let Some(p) = game.resource::<Zm>().players.get(&peer.0).map(|p| p.obj) {
                    let g = vm.string(gun);
                    vm.spawn_named(
                        game,
                        "maps/mp/zombies/_zm_weapons",
                        "weapon_give",
                        gsc_t6::ScriptValue::Object(p),
                        vec![g],
                    );
                }
            });
            diag::info!(Sim, "bo2zm t6 autoplay: given {gun}");
        }
        RAISE_AT.store(now_ms + 1000, Ordering::Relaxed);
    }
    let guns = std::env::var("BO2MP_T6_AUTOPLAY_GUN").unwrap_or_default();
    if let Some((a, b)) = guns.split_once(',')
        && give_at == i64::MAX
        && now_ms % 2000 < tick
    {
        let want = if (now_ms / 2000) % 2 == 0 { a } else { b };
        let held = frame(game).avatar(peer).map(|p| p.weapon as u32);
        if let Ok(w) = super::weapon(game, want)
            && held != Some(w)
        {
            crate::client_script::switch_to_weapon(&mut frame(game), peer, w);
            diag::info!(Sim, "bo2zm t6 autoplay: swapped to {want}");
        }
    }
    // Equipment (TranZit's Jet Gun) goes on a d-pad slot and weapon_give
    // does not raise it; a second later he presses that slot, as a player
    // would.
    let raise_at = RAISE_AT.load(Ordering::Relaxed);
    if raise_at != i64::MAX && now_ms >= raise_at {
        RAISE_AT.store(i64::MAX, Ordering::Relaxed);
        let guns = std::env::var("BO2MP_T6_AUTOPLAY_GUN").unwrap_or_else(|_| "m14_zm".to_owned());
        let gun = guns.split(',').next().unwrap_or_default().to_owned();
        let held = frame(game).avatar(peer).map(|p| p.weapon as u32);
        if let Ok(w) = super::weapon(game, &gun)
            && held != Some(w)
        {
            crate::client_script::switch_to_weapon(&mut frame(game), peer, w);
            diag::info!(Sim, "bo2zm t6 autoplay: raised {gun} from its d-pad slot");
        }
    }
    if now_ms % 2000 < tick {
        let mut f = frame(game);
        if let Some(w) = f.avatar(peer).map(|p| p.weapon) {
            crate::client_script::set_ammo_stock(&mut f, peer, w, 400);
        }
    }
}

/// BO2MP_T6_FXTEST=<effect name>: at 6 s that effect plays (held, turned
/// with a forward and an up) 80 units (BO2MP_T6_FXTEST_DIST) in front of
/// the player, facing him along its left (a wall buy's chalk faces its struct's left).
pub(crate) fn fx_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Option<String>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| std::env::var("BO2MP_T6_FXTEST").ok())
        .clone()
    else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if !(6000..6000 + tick).contains(&now) {
        return;
    }
    let Some(ps) = frame(world).avatar(ClientId(0)).copied() else {
        return;
    };
    let yaw = ps.look_angles[1];
    let (f, _, _) = gsc_t6::math::facing_axes([0.0, yaw, 0.0]);
    // BO2MP_T6_FXTEST_DIST=<units>: that far out instead, on the floor (a
    // thrown grenade's effect, seen from across the room).
    let dist = std::env::var("BO2MP_T6_FXTEST_DIST").ok();
    let (dist, up) = match dist.and_then(|d| d.parse::<f32>().ok()) {
        Some(d) => (d, 0.0),
        None => (80.0, 50.0),
    };
    let at = [
        ps.origin[0] + f[0] * dist,
        ps.origin[1] + f[1] * dist,
        ps.origin[2] + up,
    ];
    // The effect's forward runs across his view; its left faces him.
    // BO2MP_T6_FXTEST_UP=1: its forward points up (as the map places its
    // fires), its up away from him.
    let fx_forward = gsc_t6::math::facing_axes([0.0, yaw + 90.0, 0.0]).0;
    if std::env::var_os("BO2MP_T6_FXTEST_UP").is_some() {
        super::natives_fx::fx_basis(world, &want, at, [0.0, 0.0, 1.0], f);
    } else {
        super::natives_fx::fx_basis(world, &want, at, fx_forward, [0.0, 0.0, 1.0]);
    }
    diag::info!(
        Sim,
        "bo2zm t6 fx test: {want} at ({:.0} {:.0} {:.0})",
        at[0],
        at[1],
        at[2]
    );
}

/// BO2MP_T6_LOOKAT=1 (pictures): the player's view follows the nearest
/// living zombie's head every tick. BO2MP_T6_LOOKAT=orbit<deg> (e.g.
/// orbit180 = behind it): the player stands 70 units from that zombie at
/// that angle from the way it faces, looking at its head.
/// BO2MP_T6_LOOKAT=down<deg>: he keeps his yaw and looks <deg> down.
pub(crate) fn look_test(game: &mut World) {
    static ON: OnceLock<Option<String>> = OnceLock::new();
    let Some(mode) = ON
        .get_or_init(|| std::env::var("BO2MP_T6_LOOKAT").ok())
        .clone()
    else {
        return;
    };
    if let Some(deg) = mode
        .strip_prefix("orbit")
        .and_then(|d| d.parse::<f32>().ok())
    {
        orbit_look(game, deg);
        return;
    }
    let peer = ClientId(0);
    // BO2MP_T6_LOOKAT=down<deg>: keep the way he faces, pitch <deg> down (what
    // he planted at his feet: the claymore's lasers).
    if let Some(deg) = mode.strip_prefix("down").and_then(|d| d.parse::<f32>().ok()) {
        if let Some(pstate) = frame(game).avatar(peer).copied() {
            super::set_player_view(game, peer, [deg, pstate.look_angles[1], 0.0]);
        }
        return;
    }
    let Some(pstate) = frame(game).avatar(peer).copied() else {
        return;
    };
    let eye = [
        pstate.origin[0],
        pstate.origin[1],
        pstate.origin[2] + pstate.eye_height_now,
    ];
    let nearest = {
        let zm = game.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, _)| zm.ents.get(nn).map(|en| (*nn, flat_dist(en.origin, pstate.origin))))
            .min_by(|lhs, rhs| lhs.1.total_cmp(&rhs.1))
            .map(|(nn, _)| nn)
    };
    let Some(nn) = nearest else { return };
    let Some(head) = head_of(game, nn, true) else {
        return;
    };
    super::set_player_view(game, peer, look(eye, head));
}

/// BO2MP_T6_LOOKAT=orbit<deg>: see `look_test`.
fn orbit_look(world: &mut World, deg: f32) {
    let client = ClientId(0);
    let Some(ps) = frame(world).avatar(client).copied() else {
        return;
    };
    let nearest = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, _)| {
                zm.ents
                    .get(nn)
                    .map(|e| (*nn, e.origin, e.rot[1], flat_dist(e.origin, ps.origin)))
            })
            .min_by(|lhs, rhs| lhs.3.total_cmp(&rhs.3))
    };
    let Some((nn, spot, yw, _)) = nearest else {
        return;
    };
    let Some(head) = head_of(world, nn, true) else {
        return;
    };
    let lhs = (yw + deg).to_radians();
    let stand = [
        spot[0] + lhs.cos() * 70.0,
        spot[1] + lhs.sin() * 70.0,
        spot[2] + 40.0,
    ];
    let down = [stand[0], stand[1], stand[2] - 200.0];
    let t = frame(world).trace_static_world(
        stand,
        down,
        crate::shot_trace::BODY_BOX_LOWER,
        crate::shot_trace::BODY_BOX_UPPER,
        crate::shot_trace::PLAYER_BODY_MASK,
    );
    let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
    let eye = [feet[0], feet[1], feet[2] + ps.eye_height_now];
    super::teleport_player(world, client, feet);
    super::set_player_view(world, client, look(eye, head));
}

/// BO2MP_T6_USE=<targetname>[:<seconds>] (default 10 s): at that time the
/// player gets 10000 points and stands 40 units from the first use trigger
/// with that targetname (on the first open side with a floor), looking at
/// it; he holds use 1 s later for 0.5 s. `<seconds>+<again>+...`: he holds
/// use again <again> s after he was placed, fires 3 s later for 2 s, and his
/// weapons with their clips are logged before and after the shots (Pack-a-
/// Punch: take the upgraded gun, fire it). BO2MP_T6_USE_FLAG=<flag>: that
/// level flag is logged 1 s before and 3 s after (a power switch's power_on).
pub(crate) fn use_test(world: &mut World, now: i64) {
    /// One use the variable asks for.
    struct Want {
        name: String,
        note: Option<String>,
        at: i64,
        again: Vec<i64>,
    }
    static WANT: OnceLock<Vec<Want>> = OnceLock::new();
    let list = WANT.get_or_init(|| {
        std::env::var("BO2MP_T6_USE")
            .unwrap_or_default()
            .split(',')
            .filter(|v| !v.is_empty())
            .map(|v| {
                let (name, secs) = v.split_once(':').unwrap_or((v, "10"));
                let (name, note) = match name.split_once('/') {
                    Some((n, k)) => (n.to_owned(), Some(k.to_owned())),
                    None => (name.to_owned(), None),
                };
                let ms = |v: &str| (v.parse::<f32>().unwrap_or(10.0) * 1000.0) as i64;
                let mut secs = secs.split('+');
                let at = ms(secs.next().unwrap_or("10"));
                Want { name, note, at, again: secs.map(ms).collect() }
            })
            .collect()
    });
    for Want { name, note, at, again } in list {
        use_one(world, now, name, note.as_deref(), *at, again);
    }
}

fn use_one(world: &mut World, now: i64, want: &str, note: Option<&str>, at: i64, again: &[i64]) {
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let log_flag = |world: &mut World, when: &str| {
        let Ok(flag) = std::env::var("BO2MP_T6_USE_FLAG") else {
            return;
        };
        let on = with_vm(world, |vm, _| {
            let (f, k) = (vm.intern("flag"), vm.intern(&flag));
            match vm.raw_field(vm.level, f) {
                gsc_t6::ScriptValue::Array(a) => a.get(&gsc_t6::Key::Str(k)).is_some_and(|v| gsc_t6::truthy(&v)),
                _ => false,
            }
        })
        .unwrap_or(false);
        diag::info!(Sim, "bo2zm t6 use test {when}: flag {flag} {on}");
    };
    if (at - 1000..at - 1000 + tick).contains(&now) {
        log_flag(world, "before");
    }
    // When he was put by it (it may be moving, as in an elevator: tried each
    // tick for 15 s).
    static PLACED: std::sync::Mutex<Vec<(String, i64)>> = std::sync::Mutex::new(Vec::new());
    let key = format!("{want}/{}", note.unwrap_or(""));
    let placed = |key: &str| {
        PLACED.lock().ok().and_then(|l| l.iter().find(|(k, _)| k == key).map(|p| p.1))
    };
    if (at..at + tick).contains(&now) {
        give_points(world, 10000);
    }
    if (at..at + 15000).contains(&now) && placed(&key).is_none() {
        let trig = with_vm(world, |vm, world| {
            let (tn, sn) = (vm.intern("targetname"), vm.intern("script_noteworthy"));
            world
                .resource::<Zm>()
                .ents
                .values()
                .find(|e| {
                    // Any trigger: a use press in a plain one does nothing
                    // (Die Rise's escape pod: standing in it is the test).
                    super::triggers::kind(&e.class_tag).is_some()
                        && e.obj.is_some_and(|o| {
                            vm.to_text(&vm.raw_field(o, tn)) == want
                                && note.is_none_or(|k| vm.to_text(&vm.raw_field(o, sn)) == k)
                        })
                })
                .cloned()
        })
        .flatten();
        let Some(trig) = trig else {
            if now + tick < at + 15000 {
                return;
            }
            diag::warn!(Sim, "bo2zm t6 use test: no use trigger {want}");
            return;
        };
        let c = super::triggers::center(world, &trig);
        let (mins, maxs, mask) = (
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        // Its centre, else the nearest of 8 sides (16..56 units out) the
        // player fits on, over a floor (an elevator's too), touching the
        // trigger (Die Rise's escape pod trigger: only its inside has room).
        let feet = [0.0f32, 16.0, 24.0, 32.0, 40.0, 56.0].into_iter().find_map(|r| {
            (0..8).find_map(|i| {
                let a = (i as f32) * std::f32::consts::FRAC_PI_4;
                let from = [c[0], c[1], c[2] + 10.0];
                let to = [c[0] + r * a.cos(), c[1] + r * a.sin(), c[2] + 10.0];
                let down = [to[0], to[1], to[2] - 160.0];
                let feet: [f32; 3] = {
                    let f = frame(world);
                    // A centre inside something solid (a perk machine's
                    // own collision) only rules out walls between.
                    let side = f.trace_world(from, to, mins, maxs, mask);
                    if side.fraction < 1.0 && !side.starts_inside {
                        return None;
                    }
                    let t = f.trace_world(to, down, mins, maxs, mask);
                    if t.fraction >= 1.0 || t.starts_inside {
                        return None;
                    }
                    std::array::from_fn(|k| to[k] + (down[k] - to[k]) * t.fraction)
                };
                let lo = std::array::from_fn(|k| feet[k] + mins[k]);
                let hi = std::array::from_fn(|k| feet[k] + maxs[k]);
                super::triggers::touches(world, &trig, lo, hi).then_some(feet)
            })
        });
        let Some(feet) = feet else {
            if now + tick < at + 15000 {
                return;
            }
            diag::warn!(Sim, "bo2zm t6 use test: no room by {want} at ({:.0} {:.0} {:.0})", c[0], c[1], c[2]);
            return;
        };
        if let Ok(mut l) = PLACED.lock() {
            l.push((key.clone(), now));
        }
        let eye = [feet[0], feet[1], feet[2] + 60.0];
        super::teleport_player(world, client, feet);
        super::set_player_view(world, client, look(eye, c));
        diag::info!(
            Sim,
            "bo2zm t6 use test: {want} {} brush {:?} radius {} box {:?} at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
            trig.class_tag,
            trig.brush,
            trig.reach,
            trig.box_dims,
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    let Some(at) = placed(&key) else {
        return;
    };
    if (at + 800..at + 800 + tick).contains(&now) {
        let hint = world.resource::<Zm>().hints.get(&0).cloned().unwrap_or_default();
        diag::info!(Sim, "bo2zm t6 use test: hint {hint:?}");
    }
    if (at + 1000..at + 1500).contains(&now) {
        press_use(world, client);
    }
    if [4000, 8000].iter().any(|d| (at + d..at + d + tick).contains(&now)) {
        log_flag(world, "after");
        let names: Vec<String> = {
            let f = frame(world);
            crate::client_script::weapons(&f, client, crate::client_script::WeaponList::All)
                .into_iter()
                .map(|w| crate::client_script::weapon_name(&f, w))
                .collect()
        };
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        diag::info!(Sim, "bo2zm t6 use test {want} after: points {score}, weapons {names:?}");
    }
    for &d in again {
        if (at + d - 200..at + d - 200 + tick).contains(&now) {
            let hint = world.resource::<Zm>().hints.get(&0).cloned().unwrap_or_default();
            diag::info!(Sim, "bo2zm t6 use test: hint again {hint:?}");
        }
        if (at + d..at + d + 500).contains(&now) {
            press_use(world, client);
        }
        for (dt, when) in [(2800, "before shots"), (5500, "after shots")] {
            if (at + d + dt..at + d + dt + tick).contains(&now) {
                let clips: Vec<(String, i32, i32)> = {
                    let f = frame(world);
                    crate::client_script::weapons(&f, client, crate::client_script::WeaponList::All)
                        .into_iter()
                        .map(|w| {
                            let n = crate::client_script::weapon_name(&f, w);
                            // A dual-wield gun: its left clip too.
                            let left = crate::client_script::left_clip(&f, client, w);
                            (n, crate::client_script::ammo_clip(&f, client, w), left)
                        })
                        .collect()
                };
                diag::info!(Sim, "bo2zm t6 use test {want} again {when}: clips (right, left) {clips:?}");
            }
        }
        if (at + d + 3000..at + d + 5000).contains(&now) {
            let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
            for (id, user_cmd) in &mut req.input.cmds {
                if client == *id {
                    // Taps (a pistol fires once a press).
                    if (now / 200) % 2 == 0 {
                        user_cmd.buttons |= buttons::ATTACK;
                    }
                }
            }
        }
    }
}

/// BO2MP_T6_ENTLOG=<targetname>[,...]: every 5 s each entity with that
/// targetname logs its origin (Die Rise's escape pod going down and back).
/// BO2MP_T6_ENTLOG_FIELDS=<field>[,...] adds those script fields (a car
/// body's `current_level`, `force_starting_floor`). The name "player" logs
/// player 0's place as well.
pub(crate) fn ent_log(world: &mut World, now: i64) {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    static FIELDS: OnceLock<Vec<String>> = OnceLock::new();
    let list = |var: &str| -> Vec<String> {
        std::env::var(var)
            .unwrap_or_default()
            .split(',')
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let names = NAMES.get_or_init(|| list("BO2MP_T6_ENTLOG"));
    let fields = FIELDS.get_or_init(|| list("BO2MP_T6_ENTLOG_FIELDS"));
    if names.is_empty() || now % 5000 >= i64::from(crate::SIM_STEP_MILLIS) {
        return;
    }
    let found = with_vm(world, |vm, world| {
        let tn = vm.intern("targetname");
        let ids: Vec<_> = fields.iter().map(|f| (f, vm.intern(f))).collect();
        world
            .resource::<Zm>()
            .ents
            .iter()
            .filter_map(|(n, e)| {
                let obj = e.obj?;
                let name = vm.to_text(&vm.raw_field(obj, tn));
                names.contains(&name).then(|| {
                    let extra: String = ids
                        .iter()
                        .map(|(f, id)| format!(" {f}={}", vm.to_text(&vm.raw_field(obj, *id))))
                        .collect();
                    (name, *n, e.origin, extra)
                })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    for (name, n, o, extra) in found {
        diag::info!(
            Sim,
            "bo2zm t6 entlog {}s: {name} ent{n} at ({:.0} {:.0} {:.0}){extra}",
            now / 1000,
            o[0],
            o[1],
            o[2]
        );
    }
    // "player" in the list: player 0's place too (riding a car).
    if names.iter().any(|n| n == "player")
        && let Some(o) = frame(world).avatar(ClientId(0)).map(|p| p.origin)
    {
        diag::info!(
            Sim,
            "bo2zm t6 entlog {}s: player at ({:.0} {:.0} {:.0})",
            now / 1000,
            o[0],
            o[1],
            o[2]
        );
    }
}
