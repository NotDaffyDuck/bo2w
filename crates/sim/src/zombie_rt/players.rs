//! Players joining and leaving: `codecallback_playerconnect` when a client
//! shows up, `begin` once it is in the game, `codecallback_playerdisconnect`
//! when it goes.

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{Array, ObjKind, ObjRef, ScriptValue};

use super::{Survivor, Zm, frame, with_vm};

/// Who each spectator watches (BO2's SpectatorClientEndFrame): a player
/// whose `sessionstate` is "spectator" (bled out in co-op, back next round)
/// keeps the teammate he follows while that one is still playing (down
/// counts); else Cmd_FollowCycle_f's next playing client after him, in
/// client order. Only teams his `allowspectateteam` leaves open count
/// (G_ClientCanSpectateTeam), and while free look is allowed he starts
/// following no one. None to watch: he watches no one.
pub(super) fn follow_sync(world: &mut World) {
    let (spectators, playing): (Vec<(u32, u32)>, Vec<u32>) = {
        let zm = world.resource::<Zm>();
        let of = |status: &str| -> Vec<(u32, u32)> {
            zm.players
                .iter()
                .where_value(|pt| pt.sessionstate == status)
                .map(|(c, pt)| (*c, pt.no_spectate))
                .collect()
        };
        (of("spectator"), of("playing").into_iter().map(|(c, _)| c).collect())
    };
    // BO2's G_ClientCanSpectateTeam: the watched one's session team.
    let teams: std::collections::HashMap<u32, u8> = super::with_vm(world, |vm, game| {
        let f = vm.intern("sessionteam");
        let zm = game.resource::<Zm>();
        playing
            .iter()
            .filter_map(|c| {
                let v = vm.raw_field(zm.players.get(c)?.obj, f);
                Some((*c, super::goals::team_number(&vm.to_text(&v)).unwrap_or(0)))
            })
            .collect()
    })
    .unwrap_or_default();
    let was: Vec<(crate::sim_parts::ClientId, crate::sim_parts::ClientId)> = frame(world).follows().to_vec();
    let mut follows = Vec::new();
    for (s, no_spectate) in spectators {
        let watching = was.iter().find(|(v, _)| v.0 == s).map(|(_, f)| f.0);
        if let Some(f) = pick_follow(s, no_spectate, watching, &playing, &teams) {
            if watching != Some(f) {
                diag::info!(Sim, "bo2zm t6 spectator {s} now watches player {f}");
            }
            follows.push((crate::sim_parts::ClientId(s), crate::sim_parts::ClientId(f)));
        }
    }
    if follows != was {
        frame(world).set_follows(follows);
    }
}

/// The one spectator `s` watches next: the one he watches while still
/// playing on a team he may watch, else (StopFollowing) Cmd_FollowCycle_f's
/// next such player after him. Not watching anyone while free look is
/// allowed, he stays free (SpectatorClientEndFrame cycles only without it).
fn pick_follow(
    s: u32,
    no_spectate: u32,
    watching: Option<u32>,
    playing: &[u32],
    teams: &std::collections::HashMap<u32, u8>,
) -> Option<u32> {
    let may_watch = |c: &u32| no_spectate & 1 << teams.get(c).copied().unwrap_or(0) == 0;
    let kept = watching.filter(|f| playing.contains(f) && may_watch(f));
    if kept.is_some() {
        return kept;
    }
    if watching.is_none() && no_spectate & super::natives_player::SPECTATE_FREELOOK == 0 {
        return None;
    }
    let from = watching.unwrap_or(s);
    let open = || playing.iter().copied().filter(|c| *c != s && may_watch(c));
    open().find(|c| *c > from).or_else(|| open().next())
}

pub(super) fn sync(world: &mut World) {
    if !world.resource::<Zm>().underway {
        return;
    }
    let clients: Vec<(u32, bool)> = {
        let f = frame(world);
        f.ordered_player_ids()
            .into_iter()
            .map(|cid| {
                let joined = f
                    .client_meta(cid)
                    .is_some_and(|m| m.lifecycle != crate::PlayerSessionStage::Connecting);
                (cid.0, joined)
            })
            .collect()
    };
    let known: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let now = world.resource::<Zm>().clock_millis;
    for n in known {
        // bo2mp: a bot made by `addtestclient` waits for its client (up to
        // 15 s, then it is dropped like a disconnect).
        let waiting = world
            .resource::<Zm>()
            .players
            .get(&n)
            .and_then(|pt| pt.pending_since)
            .is_some_and(|t| now - t < 15_000);
        if waiting {
            continue;
        }
        if !clients.iter().any(|(ch, _)| *ch == n) {
            world.resource_mut::<Zm>().bots.remove(&n);
            // The next client in this seat numbers its sound notifies from 1.
            world.resource_mut::<Zm>().snd_notify.remove(&n);
            let pt = world.resource_mut::<Zm>().players.remove(&n);
            if let Some(pt) = pt {
                with_vm(world, |vm, world| {
                    let cb = super::mp::callbacks(world);
                    vm.spawn_named(
                        world,
                        &cb,
                        "codecallback_playerdisconnect",
                        ScriptValue::Object(pt.obj),
                        vec![],
                    );
                    vm.free_object(world, pt.obj);
                });
            }
        }
    }
    for (ch, joined) in clients {
        let existing = world.resource::<Zm>().players.get(&ch).cloned();
        match existing {
            None => {
                with_vm(world, |vm, world| {
                    let obj = vm.alloc_object(ObjKind::Entity(ch));
                    // bo2mp: back from a round restart (`map_restart( 1 )`):
                    // his pers[] and whether he is a bot carry over.
                    let back = world.resource_mut::<Zm>().rejoin.remove(&ch);
                    let mut player = Survivor::new(obj);
                    player.test_client = back.as_ref().is_some_and(|(_, bot)| *bot);
                    world.resource_mut::<Zm>().players.insert(ch, player);
                    let pers = vm.intern("pers");
                    let kept = back.map(|(v, _)| v).filter(|v| !matches!(v, ScriptValue::Undefined));
                    vm.set_raw_field(obj, pers, kept.unwrap_or_else(|| ScriptValue::array(Array::new())));
                    diag::info!(Sim, "bo2zm t6: player {ch} connects");
                    let cb = super::mp::callbacks(world);
                    if vm
                        .spawn_named(
                            world,
                            &cb,
                            "codecallback_playerconnect",
                            ScriptValue::Object(obj),
                            vec![],
                        )
                        .is_none()
                    {
                        diag::warn!(Sim, "bo2zm t6: no {cb}::codecallback_playerconnect");
                    }
                });
            }
            // bo2mp: a bot's client came: it connects now (its entity and
            // pers[] are the ones its script already marked).
            Some(pt) if pt.pending_since.is_some() => {
                if let Some(pp) = world.resource_mut::<Zm>().players.get_mut(&ch) {
                    pp.pending_since = None;
                }
                with_vm(world, |vm, world| {
                    diag::info!(Sim, "bo2mp bot: client {ch} connects");
                    let cb = super::mp::callbacks(world);
                    vm.spawn_named(
                        world,
                        &cb,
                        "codecallback_playerconnect",
                        ScriptValue::Object(pt.obj),
                        vec![],
                    );
                });
            }
            Some(pt) if joined && !pt.begun => {
                if let Some(pp) = world.resource_mut::<Zm>().players.get_mut(&ch) {
                    pp.begun = true;
                }
                // A new game starts in the map's own fog (the last one's
                // game over may have changed it).
                super::set_client_dvar(world, ch, "bo2zm_fogbank", "");
                with_vm(world, |vm, world| vm.notify_str(world, pt.obj, "begin", &[]));
            }
            Some(_) => {}
        }
    }
    // A player down crawls until he is revived: Black Ops II's movement
    // keeps last stand in the move type alone (the low view, the crawl, and
    // his client predicts the same). Its pm flag's bit is slide ground, set
    // by the movement itself, so it is never touched here.
    let states: Vec<(u32, bool)> = world.resource::<Zm>().players.iter().map(|(ch, pt)| (*ch, pt.laststand)).collect();
    for (ch, down) in states {
        if let Some(pstate) = frame(world).occupant_mut(crate::sim_parts::ClientId(ch)) {
            use playerstate_t6::game::MOVE_MODE_DOWNED;
            if down {
                if pstate.movement_mode != playerstate_t6::game::MOVE_MODE_INTERLUDE {
                    pstate.movement_mode = MOVE_MODE_DOWNED;
                }
            } else if pstate.movement_mode == MOVE_MODE_DOWNED {
                pstate.movement_mode = 0;
            }
        }
    }
    // Down, his screen fades to the last stand's vision.
    super::clientfields::laststand_visions(world);
    // Near an open Gersh Device, his screen fades to the black hole's.
    super::blackhole_fx::tick(world);
    // A script camera: his eye at the camera entity, looking its way; he
    // stands frozen (intermission) and his gun is put away.
    let cams: Vec<(u32, [f32; 3], [f32; 3], bool)> = {
        let zm = world.resource::<Zm>();
        zm.players
            .iter()
            .where_value(|pt| pt.camera_on)
            .filter_map(|(ch, pt)| {
                let en = zm.ents.get(&pt.camera?)?;
                Some((*ch, en.origin, en.rot, false))
            })
            .collect()
    };
    for (ch, origin, angles, _) in cams {
        let cid = crate::sim_parts::ClientId(ch);
        let first = frame(world)
            .avatar(cid)
            .is_some_and(|pstate| pstate.movement_mode != playerstate_t6::game::MOVE_MODE_INTERLUDE);
        // The camera turns as it moves (the rocket shot tilts down onto the
        // town): his view turns with it the way his client follows a turn
        // (its delta angles), not just the server's copy.
        super::set_player_view(world, cid, angles);
        if let Some(pstate) = frame(world).occupant_mut(cid) {
            pstate.movement_mode = playerstate_t6::game::MOVE_MODE_INTERLUDE;
            pstate.velocity = [0.0; 3];
            pstate.origin = [origin[0], origin[1], origin[2] - pstate.eye_height_now];
            if first {
                pstate.e_flags ^= playerstate_t6::game::entity_flag_bits::TELEPORT;
                diag::info!(
                    Sim,
                    "bo2zm t6: client {ch} views from a camera at ({:.0} {:.0} {:.0}) facing ({:.0} {:.0} {:.0})",
                    origin[0],
                    origin[1],
                    origin[2],
                    angles[0],
                    angles[1],
                    angles[2]
                );
            }
        }
    }
    // Linked players go where their entity goes - not one viewing from a
    // script camera (a player down is linked where he fell; the game-over
    // shot must still take his view to the rocket).
    let links: Vec<(u32, [f32; 3])> = {
        let zm = world.resource::<Zm>();
        zm.players
            .iter()
            .where_value(|pt| !(pt.camera_on && pt.camera.is_some()))
            .filter_map(|(ch, pt)| {
                pt.linked
                    .and_then(|en| zm.ents.get(&en))
                    .map(|en| (*ch, en.origin))
            })
            .collect()
    };
    // The movement code's linked mode (no walking; his client takes the
    // server's position as it is) and the entity's position each tick - a
    // teleport each tick snapped his view every time (the up-and-down
    // glitch while he was lowered to the ground, down).
    let linked: std::collections::BTreeSet<u32> = links.iter().map(|(ch, _)| *ch).collect();
    for (ch, origin) in links {
        let cid = crate::sim_parts::ClientId(ch);
        let mut f = frame(world);
        let first = f.avatar(cid).is_some_and(|pstate| pstate.movement_mode != playerstate_t6::game::MOVE_MODE_WALK_ATTACHED);
        f.place_at(cid, origin);
        if let Some(pstate) = f.occupant_mut(cid) {
            if pstate.movement_mode == 0 {
                pstate.movement_mode = playerstate_t6::game::MOVE_MODE_WALK_ATTACHED;
            }
            pstate.velocity = [0.0; 3];
            if first {
                pstate.e_flags ^= playerstate_t6::game::entity_flag_bits::TELEPORT;
            }
        }
    }
    // Unlinked: back to walking.
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().filter(|ch| !linked.contains(ch)).collect();
    for ch in clients {
        if let Some(pstate) = frame(world).occupant_mut(crate::sim_parts::ClientId(ch))
            && pstate.movement_mode == playerstate_t6::game::MOVE_MODE_WALK_ATTACHED
        {
            pstate.movement_mode = 0;
        }
    }
}

/// A shellshock that has run its time ends (the engine clears it; the
/// script's `stopshellshock` does the same earlier).
pub(super) fn shock_tick(world: &mut World) {
    let level = crate::level_time_ms(super::tick(world));
    let ended: Vec<u32> = world
        .resource::<Zm>()
        .players
        .iter()
        .where_value(|pt| pt.shock.as_ref().is_some_and(|(_, finish)| *finish < level))
        .map(|(c, _)| *c)
        .collect();
    for c in ended {
        super::natives_player::end_shock(world, c, "time ran out");
    }
}

/// `dtp_start` when he dives, `dtp_end` when the dive's slide ends (PhD
/// Flopper, the dive challenges, the bots); `sprint_begin` / `sprint_end`
/// (sprinting into a step trigger plays its sound); `mantle_start` with the
/// climb's end time when he starts a climb (the challenges).
pub(super) fn movement_events(game: &mut World) {
    let clients: Vec<(u32, ObjRef, bool, bool, i32)> = game
        .resource::<Zm>()
        .players
        .iter()
        .map(|(c, p)| (*c, p.obj, p.last_dive, p.last_sprint, p.last_mantle_end))
        .collect();
    for (c, obj, was_diving, was_sprinting, last_mantle_end) in clients {
        let Some(ps) = frame(game).avatar(crate::sim_parts::ClientId(c)).copied() else {
            continue;
        };
        let diving = ps.movement_mode <= 9 && movement_t6::bridge::dive_to_prone(&ps);
        let sprinting = ps.sprint_state.sprint_begin_ms != 0 && ps.sprint_state.sprint_stop_ms < ps.sprint_state.sprint_begin_ms;
        let mantle_end = frame(game).mantle_end_time(c as u16).filter(|t| *t != last_mantle_end);
        if let Some(p) = game.resource_mut::<Zm>().players.get_mut(&c) {
            p.last_dive = diving;
            p.last_sprint = sprinting;
            if let Some(t) = mantle_end {
                p.last_mantle_end = t;
            }
        }
        if let Some(t) = mantle_end {
            with_vm(game, |vm, game| {
                vm.notify_str(game, obj, "mantle_start", &[ScriptValue::Int(t)]);
            });
        }
        let mut events: Vec<&str> = Vec::new();
        if diving != was_diving {
            events.push(if diving { "dtp_start" } else { "dtp_end" });
        }
        if sprinting != was_sprinting {
            events.push(if sprinting { "sprint_begin" } else { "sprint_end" });
        }
        if events.is_empty() {
            continue;
        }
        with_vm(game, |vm, game| {
            for e in events {
                vm.notify_str(game, obj, e, &[]);
            }
        });
    }
}

/// What his weapon did since last tick, as the engine tells scripts:
/// `weapon_switch_started`, `weapon_change` (a new weapon in hand),
/// `weapon_change_complete` (raised), `weapon_fired`, `reload_start`,
/// `reload` (the magazine went in).
// TODO(rw-zm): not called any more; gun_script::frame_notifies/frame_watch
// (run from mod.rs for MP and Zombies) raise these notifies. Delete once nothing needs it.
#[allow(dead_code)]
pub(super) fn weapon_events(game: &mut World) {
    use weapon_t6::state;
    let clients: Vec<(u32, ObjRef, u32, i32, i32)> = game
        .resource::<Zm>()
        .players
        .iter()
        .map(|(c, p)| (*c, p.obj, p.last_weapon, p.last_wstate, p.last_clip))
        .collect();
    for (c, obj, last_w, last_s, last_clip) in clients {
        let Some(ps) = frame(game).avatar(crate::sim_parts::ClientId(c)).copied() else {
            continue;
        };
        let (w, st) = (ps.weapon, ps.weapon_state_t6);
        let clip = crate::client_script::ammo_clip(&frame(game), crate::sim_parts::ClientId(c), w);
        // BO2's gun states; TODO(fact): the swim in and out as a drop and raise,
        // as our old MW2 mirror read them.
        let raising = state::is_raising_or_swim_out;
        let dropping = state::is_dropping_or_swim_in;
        let reloading = state::is_any_reload;
        let mut events: Vec<&str> = Vec::new();
        if dropping(st) && !dropping(last_s) {
            events.push("weapon_switch_started");
        }
        if w != last_w && w != 0 {
            events.push("weapon_change");
        }
        if raising(last_s) && !raising(st) && !dropping(st) {
            events.push("weapon_change_complete");
        }
        if w == last_w && clip < last_clip && !reloading(st) {
            events.push("weapon_fired");
        }
        if reloading(st) && !reloading(last_s) {
            events.push("reload_start");
        }
        if reloading(last_s) && !reloading(st) && clip > last_clip {
            events.push("reload");
        }
        if let Some(p) = game.resource_mut::<Zm>().players.get_mut(&c) {
            p.last_weapon = w;
            p.last_wstate = st;
            p.last_clip = clip;
        }
        if events.is_empty() {
            continue;
        }
        let name = super::weapon_text(game, w);
        // bo2mp: BO2's `weapon_change` carries the new weapon, then the one
        // it replaced ("none" on his first frame after a spawn).
        let old = match last_w {
            0 => "none".to_owned(),
            _ => super::weapon_text(game, last_w),
        };
        with_vm(game, |vm, game| {
            let n = vm.string(&name);
            let o = vm.string(&old);
            for e in events {
                if e == "weapon_change" {
                    vm.notify_str(game, obj, e, &[n.clone(), o.clone()]);
                } else {
                    vm.notify_str(game, obj, e, &[n.clone()]);
                }
            }
        });
    }
}

#[cfg(test)]
mod follow_tests {
    use super::pick_follow;
    use crate::zombie_rt::natives_player::SPECTATE_FREELOOK;

    /// Clients 1 and 3 on allies (team 1), 2 on axis (team 2).
    fn teams() -> std::collections::HashMap<u32, u8> {
        [(1, 1), (2, 2), (3, 1)].into_iter().collect()
    }

    /// TDM's spectatetype 1 (_spectating.gsc): his own team only, no free look.
    #[test]
    fn a_shut_team_is_never_followed() {
        let axis_shut = SPECTATE_FREELOOK | 1 << 2;
        assert_eq!(pick_follow(0, axis_shut, None, &[1, 2, 3], &teams()), Some(1));
        assert_eq!(pick_follow(0, axis_shut, Some(1), &[1, 2, 3], &teams()), Some(1));
        // The one he watched went to a shut team: on to the next open one.
        assert_eq!(pick_follow(0, axis_shut, Some(2), &[1, 2, 3], &teams()), Some(3));
        assert_eq!(pick_follow(0, SPECTATE_FREELOOK | 1 << 1 | 1 << 2, Some(1), &[1, 2, 3], &teams()), None);
    }

    /// Free look allowed: he starts following no one, but keeps whom he follows.
    #[test]
    fn free_look_starts_no_follow() {
        assert_eq!(pick_follow(0, 0, None, &[1, 2, 3], &teams()), None);
        assert_eq!(pick_follow(0, 0, Some(2), &[1, 2, 3], &teams()), Some(2));
        assert_eq!(pick_follow(0, SPECTATE_FREELOOK, None, &[1, 2, 3], &teams()), Some(1));
        assert_eq!(pick_follow(0, SPECTATE_FREELOOK, Some(2), &[1, 3], &teams()), Some(3));
    }
}
