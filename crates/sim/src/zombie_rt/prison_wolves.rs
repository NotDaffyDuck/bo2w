//! Mob of the Dead's wolf heads (feeding the Cerberus soul catchers). BO2
//! plays them from a client script (zm_prison_weap_quest.csc) our engine
//! does not run: one world client field per `wolf_position` struct (named
//! by its `script_parameters`) says what the head does, and the client
//! shows, hides and animates the map's head (the struct's `script_label`),
//! its rune on the wall (`script_noteworthy`) and the body model it keeps
//! for the bite (`script_friendname`). The engine plays that half here:
//! 0 gone (the rune shows), 1 arriving (the portal opens, the intro),
//! 2 idle (the idle, then a twitch, over and over), 3 / 4 / 5 eating in
//! front / right / left (pre-eat while the body is pulled in, then the
//! bite), 6 leaving (the outro, the portal closes, the rune shows), 7 done
//! (the rune turns into the dream catcher and glows). The client also
//! takes the dead zombie the server marked `make_client_clone` (ghosted at
//! its `portal_death` notetrack) and pulls it into the head's mouth for the
//! bite: here that dead actor itself is shown again and moved.

use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::Zm;
use crate::EventRecipients;

const INTRO: &str = "o_zombie_dreamcatcher_intro";
const OUTRO: &str = "o_zombie_dreamcatcher_outtro";
const IDLE: &str = "o_zombie_dreamcatcher_idle";
const TWITCHES: [&str; 3] = [
    "o_zombie_dreamcatcher_idle_twitch_scan",
    "o_zombie_dreamcatcher_idle_twitch_shake",
    "o_zombie_dreamcatcher_idle_twitch_yawn",
];
const PORTAL_FX: &str = "maps/zombie_alcatraz/fx_alcatraz_portal_hell";
const PORTAL_CLOSE_FX: &str = "maps/zombie_alcatraz/fx_alcatraz_portal_hell_close";
const CHARGED_FX: &str = "maps/zombie_alcatraz/fx_alcatraz_soul_charged";
const DONE_MODEL: &str = "p6_zm_al_dream_catcher";
/// The client pulls the body in before the bite: 1.5 s, then 0.5 s (its
/// two `moveto`s); the head plays its pre-eat meanwhile.
const PULL_MS: i64 = 2000;
const RISE_MS: i64 = 1500;
/// The first move goes a fifth of the way to the head, the second to 36
/// in front of it.
const RISE_SHARE: f32 = 0.2;
const FRONT: f32 = 36.0;
const RISE: &str = "ai_zombie_dreamcatch_rise";
const SHRINK: &str = "ai_zombie_dreamcatch_shrink_a";
const SOUL_START_FX: &str = "maps/zombie_alcatraz/fx_alcatraz_soul_charge_start";
/// The bite's blood at the mouth, on the head clip's `blood` / `blood_sm`
/// notetracks.
const BITE_FX: &str = "maps/zombie_alcatraz/fx_alcatraz_soul_charge_impact";
const BITE_SMALL_FX: &str = "maps/zombie_alcatraz/fx_alcatraz_soul_charge_impact_sm";

/// The eat direction a field value names (3 front, 4 right, 5 left), as
/// the clip names' suffix.
fn side(val: i64) -> Option<&'static str> {
    match val {
        3 => Some("f"),
        4 => Some("r"),
        5 => Some("l"),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
enum Next {
    Idle,
    Twitch,
    Bite(&'static str),
}

#[derive(Clone, Debug)]
struct Head {
    field: String,
    /// The struct's place: the client picks a body's head by it.
    at: [f32; 3],
    head: Option<u32>,
    rune: Option<u32>,
    body: Option<u32>,
    value: i64,
    /// The head's next clip and when (the idle cycle, or the bite once the
    /// body is in).
    next: Option<(i64, Next)>,
}

/// A dead zombie being pulled into head `head` (index) since `start`.
#[derive(Clone, Debug)]
struct Pull {
    n: u32,
    head: usize,
    side: &'static str,
    start: i64,
    from: [f32; 3],
    mid: [f32; 3],
    to: [f32; 3],
    shrinking: bool,
    /// When the bite began and its length (ms).
    bite: Option<(i64, i64)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Wolves {
    heads: Vec<Head>,
    seed: u64,
    /// Dead zombies marked `make_client_clone`, oldest first.
    clones: Vec<u32>,
    pulls: Vec<Pull>,
}

impl Wolves {
    fn rand(&mut self) -> u64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.seed >> 33
    }
}

fn text_field(vm: &mut Vm<World>, o: gsc_t6::ObjRef, ident: &str) -> String {
    let k = vm.intern(ident);
    let v = vm.raw_field(o, k);
    vm.to_text(&v)
}

/// The map entity named `targetname`.
fn map_ent(vm: &mut Vm<World>, world: &World, targetname: &str) -> Option<u32> {
    if targetname.is_empty() {
        return None;
    }
    let k = vm.intern("targetname");
    world.resource::<Zm>().ents.iter().find_map(|(n, e)| {
        let o = e.obj?;
        (e.map && vm.to_text(&vm.raw_field(o, k)) == targetname).then_some(*n)
    })
}

/// The heads from `level.a_wolf_structs` (set by the server's weapon quest
/// before it sets their fields).
fn find(vm: &mut Vm<World>, world: &mut World) {
    let lvl = vm.level;
    let k = vm.intern("a_wolf_structs");
    let ScriptValue::Array(a) = vm.raw_field(lvl, k) else {
        return;
    };
    let mut heads = Vec::new();
    for i in 0..8 {
        let Some(ScriptValue::Object(s)) = a.get(&gsc_t6::Key::Int(i)) else {
            break;
        };
        let field = text_field(vm, s, "script_parameters");
        let ko = vm.intern("origin");
        let at = match vm.raw_field(s, ko) {
            ScriptValue::Vec3(v) => v,
            _ => [0.0; 3],
        };
        let head = text_field(vm, s, "script_label");
        let rune = text_field(vm, s, "script_noteworthy");
        let body = text_field(vm, s, "script_friendname");
        let (head, rune, body) = (
            map_ent(vm, world, &head),
            map_ent(vm, world, &rune),
            map_ent(vm, world, &body),
        );
        diag::info!(
            Sim,
            "bo2zm t6 wolf head {field}: head {head:?} rune {rune:?} body {body:?} at ({:.0} {:.0} {:.0})",
            at[0],
            at[1],
            at[2]
        );
        heads.push(Head {
            field,
            at,
            head,
            rune,
            body,
            value: -1,
            next: None,
        });
    }
    let mut zm = world.resource_mut::<Zm>();
    zm.wolves.heads = heads;
    zm.wolves.seed = 0x0d06_f00d;
}

fn show(game: &mut World, nn: Option<u32>, on: bool) {
    let mut zm = game.resource_mut::<Zm>();
    if let Some(e) = nn.and_then(|nn| zm.ents.get_mut(&nn)) {
        e.unseen = !on;
        e.ghosted = false;
    }
}

/// Start `clip` on the head from its first frame (`clearanim` + `setanim`);
/// its length in ms.
fn play(game: &mut World, nn: Option<u32>, clip_no: &str, now_ms: i64) -> i64 {
    let mut zm = game.resource_mut::<Zm>();
    let len = zm.anims.get(clip_no).map(super::actors::anim_length_ms);
    if len.is_none() {
        diag::info!(Sim, "bo2zm t6 wolf head: animation {clip_no} not loaded");
    }
    if let Some(e) = nn.and_then(|nn| zm.ents.get_mut(&nn)) {
        e.anim = Some((clip_no.to_owned(), now_ms));
    }
    len.unwrap_or(1000).max(1)
}

fn origin(world: &World, n: Option<u32>) -> Option<[f32; 3]> {
    n.and_then(|n| world.resource::<Zm>().ents.get(&n).map(|e| e.origin))
}

/// Where the client puts the portal: just off the rune and 24 below it,
/// facing out of the wall (the rune's forward turned 90 degrees).
fn portal(world: &World, rune: Option<u32>) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
    let e = world.resource::<Zm>().ents.get(&rune?)?.clone();
    let (fwd, _, _) = game_math::euler_basis([e.rot[0], e.rot[1] + 90.0, e.rot[2]]);
    let (_, _, up) = game_math::euler_basis(e.rot);
    let at = std::array::from_fn(|i| e.origin[i] - fwd[i] * 2.5 - up[i] * 24.0);
    Some((at, fwd, up))
}

fn sound(game: &mut World, nn: Option<u32>, snd_alias: &str) {
    if let Some(at) = origin(game, nn) {
        super::natives_fx::sound(game, EventRecipients::All, snd_alias, at);
    }
}

fn repeating_audio(game: &mut World, nn: Option<u32>, snd_alias: Option<&str>) {
    let mut zm = game.resource_mut::<Zm>();
    if let Some(e) = nn.and_then(|nn| zm.ents.get_mut(&nn)) {
        e.repeating_audio = snd_alias.map(str::to_owned);
    }
}

/// `make_client_clone` set on a zombie dying in a head's area: its body is
/// the one that head eats next.
pub(super) fn clone_body(world: &mut World, n: u32) {
    let mut zm = world.resource_mut::<Zm>();
    diag::info!(Sim, "bo2zm t6 wolf head: body {n} marked t{}", zm.clock_millis);
    // Entity numbers come back: keep only bodies still lying dead.
    let Zm { wolves, actors, .. } = &mut *zm;
    wolves
        .clones
        .retain(|c| *c != n && actors.get(c).is_some_and(|a| !a.living));
    wolves.clones.push(n);
}

fn toward(from: [f32; 3], goal: [f32; 3], share: f32) -> [f32; 3] {
    std::array::from_fn(|i| from[i] + (goal[i] - from[i]) * share)
}

fn separation_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

/// Head `i` starts eating: the newest marked body nearest that head (as
/// the client picks it) is shown again and pulled in.
fn start_pull(game: &mut World, i: usize, side: &'static str, now: i64) {
    let (heads, clones) = {
        let zm = game.resource::<Zm>();
        (zm.wolves.heads.clone(), zm.wolves.clones.clone())
    };
    let nearest_head = |at: [f32; 3]| {
        (0..heads.len()).min_by(|a, b| separation_sq(heads[*a].at, at).total_cmp(&separation_sq(heads[*b].at, at)))
    };
    // Only a dead one: its number may since belong to a new zombie.
    let pick = clones.iter().rev().copied().find(|nn| {
        let zm = game.resource::<Zm>();
        zm.actors.get(nn).is_some_and(|a| !a.living)
            && zm
                .ents
                .get(nn)
                .is_some_and(|e| nearest_head(e.origin) == Some(i))
    });
    let Some(nn) = pick else {
        diag::info!(
            Sim,
            "bo2zm t6 wolf head {}: no marked body to pull",
            heads[i].field
        );
        return;
    };
    let (Some(from), Some(head)) = (origin(game, Some(nn)), heads[i].head) else {
        return;
    };
    let Some(e) = game.resource::<Zm>().ents.get(&head).cloned() else {
        return;
    };
    let (fwd, _, _) = game_math::euler_basis(e.rot);
    let mid = toward(from, e.origin, RISE_SHARE);
    let to = std::array::from_fn(|k| e.origin[k] + fwd[k] * FRONT);
    // What it played before the ghost: the server's `zm_portal_death`,
    // waited on to its `end` (`donotetracks("portal_death")`); a short
    // play here means the script went on early (task check, local log).
    let before = game
        .resource::<Zm>()
        .actors
        .get(&nn)
        .and_then(|a| a.playing.clone());
    if let Some(p) = before {
        diag::info!(
            Sim,
            "bo2zm t6 wolf head {}: body {nn} played {} {} for {} of {} ms",
            heads[i].field,
            p.state,
            p.anim,
            p.elapsed(now),
            p.length_ms
        );
    }
    show(game, Some(nn), true);
    super::actors::show_clip(game, nn, RISE, now);
    sound(game, Some(nn), "evt_soulsuck_body");
    super::clientfields::bolt_later(game, nn, 0, SOUL_START_FX, "j_spinelower");
    diag::info!(
        Sim,
        "bo2zm t6 wolf head {}: pulling body {nn} from ({:.0} {:.0} {:.0})",
        heads[i].field,
        from[0],
        from[1],
        from[2]
    );
    let mut zm = game.resource_mut::<Zm>();
    zm.wolves.clones.retain(|c| *c != nn);
    zm.wolves.pulls.push(Pull {
        n: nn,
        head: i,
        side,
        start: now,
        from,
        mid,
        to,
        shrinking: false,
        bite: None,
    });
}

/// Each tick: the pulled bodies move in (each `moveto` speeds up all the
/// way, its speed_up_rate time being its whole time), then ride the head's mouth
/// through the bite and are hidden at its end (the server deletes them).
fn advance_pulls(game: &mut World, now: i64) {
    let pulls = std::mem::take(&mut game.resource_mut::<Zm>().wolves.pulls);
    let mut keep = Vec::new();
    for mut pt in pulls {
        let dead = game
            .resource::<Zm>()
            .actors
            .get(&pt.n)
            .is_some_and(|a| !a.living);
        if !dead || !game.resource::<Zm>().ents.contains_key(&pt.n) {
            continue;
        }
        let t = now - pt.start;
        let head = game
            .resource::<Zm>()
            .wolves
            .heads
            .get(pt.head)
            .and_then(|h| h.head);
        if t < RISE_MS {
            let k = t as f32 / RISE_MS as f32;
            set_origin(game, pt.n, toward(pt.from, pt.mid, k * k));
        } else if t < PULL_MS {
            if !pt.shrinking {
                pt.shrinking = true;
                super::actors::show_clip(game, pt.n, SHRINK, now);
            }
            let k = (t - RISE_MS) as f32 / (PULL_MS - RISE_MS) as f32;
            set_origin(game, pt.n, toward(pt.mid, pt.to, k * k));
        } else {
            if pt.bite.is_none() {
                super::natives_fx::bolt_effect(game, pt.n, SOUL_START_FX, "j_spinelower", true);
                let clip = format!("ai_zombie_dreamcatcher_wallconsume_align_{}", pt.side);
                let len = super::actors::show_clip(game, pt.n, &clip, now);
                pt.bite = Some((now, len));
            }
            // Linked to the head's `tag_mouth_fx` for the bite.
            let mouth = head.and_then(|h| super::tags::tag_world(game, h, "tag_mouth_fx"));
            match mouth {
                Some((at, axes)) => {
                    set_origin(game, pt.n, at);
                    if let Some(e) = game.resource_mut::<Zm>().ents.get_mut(&pt.n) {
                        e.rot = super::tags::axis_to_angles(axes);
                    }
                }
                None => set_origin(game, pt.n, pt.to),
            }
            if let Some((since, len)) = pt.bite
                && now - since >= len
            {
                show(game, Some(pt.n), false);
                continue;
            }
        }
        keep.push(pt);
    }
    game.resource_mut::<Zm>().wolves.pulls.extend(keep);
}

fn set_origin(world: &mut World, n: u32, at: [f32; 3]) {
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
        e.origin = at;
    }
}

/// `codesetworldclientfield` / `level setclientfield`: a wolf head's field
/// changed (other fields are left alone).
pub(super) fn world_field(vm: &mut Vm<World>, game: &mut World, ident: &str, val: i64) {
    if game.resource::<Zm>().map != "zm_prison" {
        return;
    }
    if game.resource::<Zm>().wolves.heads.is_empty() {
        find(vm, game);
    }
    let Some(i) = game
        .resource::<Zm>()
        .wolves
        .heads
        .iter()
        .position(|h| h.field == ident)
    else {
        return;
    };
    let now = game.resource::<Zm>().clock_millis;
    let h = game.resource::<Zm>().wolves.heads[i].clone();
    let old = h.value;
    diag::info!(Sim, "bo2zm t6 wolf head {ident}: {old} -> {val} t{now}");
    show(game, h.head, !matches!(val, 0 | 7));
    show(game, h.rune, matches!(val, 0 | 6 | 7));
    show(game, h.body, false);
    let mut next = None;
    match val {
        1 => {
            if let Some((at, fwd, up)) = portal(game, h.rune) {
                super::natives_fx::effect_held(game, PORTAL_FX, at, fwd, Some(up));
            }
            sound(game, h.head, "evt_wolfhead_spawn");
            repeating_audio(game, h.head, Some("evt_wolfhead_fire_loop"));
            play(game, h.head, INTRO, now);
        }
        2 => {
            if (3..=5).contains(&old) {
                sound(game, h.head, "evt_wolfhead_body_count");
            }
            let len = play(game, h.head, IDLE, now);
            next = Some((now + len, Next::Twitch));
        }
        3..=5 => {
            let s = side(val).unwrap_or("f");
            play(
                game,
                h.head,
                &format!("o_zombie_dreamcatcher_wallconsume_pre_eat_{s}"),
                now,
            );
            next = Some((now + PULL_MS, Next::Bite(s)));
            start_pull(game, i, s, now);
        }
        6 => {
            play(game, h.head, OUTRO, now);
            if let Some((at, fwd, up)) = portal(game, h.rune) {
                super::natives_fx::effect_stop_world(game, PORTAL_FX, at);
                super::natives_fx::fx_basis(game, PORTAL_CLOSE_FX, at, fwd, up);
            }
            sound(game, h.head, "evt_wolfhead_depart");
            repeating_audio(game, h.head, None);
            repeating_audio(game, h.rune, None);
        }
        7 => {
            if let Some(n) = h.rune
                && let Some(e) = game.resource_mut::<Zm>().ents.get_mut(&n)
            {
                e.mesh = DONE_MODEL.to_owned();
            }
            if let Some(n) = h.rune {
                super::clientfields::bolt_later(game, n, 0, CHARGED_FX, "tag_origin");
            }
            repeating_audio(game, h.rune, Some("evt_runeglow_loop"));
        }
        _ => {}
    }
    let mut zm = game.resource_mut::<Zm>();
    let h = &mut zm.wolves.heads[i];
    h.value = val;
    h.next = next;
}

/// Each tick: the heads' idle cycle and their bites once the body is in.
pub(super) fn advance(game: &mut World, now: i64) {
    advance_pulls(game, now);
    let due: Vec<(usize, Next, Option<u32>)> = {
        let zm = game.resource::<Zm>();
        zm.wolves
            .heads
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                h.next
                    .filter(|(t, _)| now >= *t)
                    .map(|(_, n)| (i, n, h.head))
            })
            .collect()
    };
    for (i, step, head) in due {
        let next = match step {
            Next::Idle => {
                let len = play(game, head, IDLE, now);
                Some((now + len, Next::Twitch))
            }
            Next::Twitch => {
                let pick = (game.resource_mut::<Zm>().wolves.rand() % 3) as usize;
                let len = play(game, head, TWITCHES[pick], now);
                Some((now + len, Next::Idle))
            }
            Next::Bite(s) => {
                sound(game, head, "evt_wolfhead_eat");
                let clip = format!("o_zombie_dreamcatcher_wallconsume_align_{s}");
                let len = play(game, head, &clip, now);
                let notes = game
                    .resource::<Zm>()
                    .anims
                    .get(&clip)
                    .map(|a| a.notifies.clone())
                    .unwrap_or_default();
                for (note, at) in notes {
                    let fx = match note.as_str() {
                        "blood" => BITE_FX,
                        "blood_sm" => BITE_SMALL_FX,
                        _ => continue,
                    };
                    if let Some(n) = head {
                        let after = (at * len as f32) as i64;
                        super::clientfields::bolt_later(game, n, after, fx, "tag_mouth_fx");
                    }
                }
                None
            }
        };
        game.resource_mut::<Zm>().wolves.heads[i].next = next;
    }
}
