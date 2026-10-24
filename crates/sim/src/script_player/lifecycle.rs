//! A player's life: spawning into a session state, dying, the corpse left behind and scripted moves to a spawn point.

use crate::damage::DeathCommit;
use crate::frame::FrameWorld;
use crate::match_state::{ClientLifecycle, EventAudience, SimEvent};
use crate::world::{ClientId, Tick, spawn_player_state};
use movement_t6::bridge::SHORT2ANGLE;
use playerstate_t6::game::{self as ps_game, eflags};

/// What a client who is not playing is doing: watching, or still picking a class.
pub(crate) fn spectator_lifecycle(team: i32) -> ClientLifecycle {
    if team == entity_t6::team::SPECTATOR {
        ClientLifecycle::Spectating
    } else {
        ClientLifecycle::ChoosingClass
    }
}

/// The spawn point a scripted pick stands for, if one is accepted: where to stand and which way to look.
fn picked_point(world: &mut FrameWorld, id: ClientId, pick: crate::SpawnPick) -> Option<([f32; 3], [f32; 3])> {
    let team = world
        .client_meta(id)
        .map_or(entity_t6::team::FREE, |m| m.client_state_team);
    let own_origin = world.player(id).map(|ps| ps.origin);
    let avoid: Vec<[f32; 3]> = world
        .alive_origins()
        .into_iter()
        .filter(|origin| own_origin != Some(*origin))
        .collect();
    let report = crate::spawn::decide_forced_spawn(world, pick, &avoid, team);
    crate::step::log_forced_spawn(id, pick, &report);
    report.accepted.map(|chosen| (chosen.traced_origin, chosen.raw_angles))
}

/// Puts a client into `sessionstate` ("playing", "dead", "intermission", anything else is spectating) at `origin` and
/// `angles`, or at the point a script forced for this spawn.
pub(crate) fn spawn(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    origin: [f32; 3],
    angles: [f32; 3],
    sessionstate: &str,
) {
    // A forced point is spent by the spawn it was queued for, used or not.
    let forced = world.client_meta_mut(id).forced_spawn.take();
    let (origin, angles) = forced
        .filter(|_| sessionstate == "playing")
        .and_then(|pick| picked_point(world, id, pick))
        .unwrap_or((origin, angles));
    world.unlink_player_area(id);
    let mut ps = spawn_player_state(origin, angles);
    if let Some(max) = world.client_meta(id).map(|m| m.max_health).filter(|&n| n > 0) {
        ps.max_health = max;
        ps.health = max;
    }
    if let Some(cmd) = world.old_cmd_angles(id) {
        ps.delta_angles = std::array::from_fn(|axis| angles[axis] - cmd[axis] as f32 * SHORT2ANGLE);
    }
    // The teleport flag flips on every spawn, so the client sees the jump.
    let was_teleporting = world.player(id).map_or(0, |p| p.e_flags & eflags::TELEPORT);
    ps.e_flags = (ps.e_flags & !eflags::TELEPORT) | (was_teleporting ^ eflags::TELEPORT);
    // Black Ops II runs the player up to the level time before his first command, so that command moves one frame, as
    // the client predicts it, not the whole time since the clock started.
    ps.command_time = crate::corpse::level_time_ms(tick);
    let lifecycle = match sessionstate {
        "playing" => ClientLifecycle::Alive,
        "intermission" => {
            ps.pm_type = ps_game::PM_TYPE_INTERMISSION;
            ClientLifecycle::Intermission
        }
        "dead" => {
            ps.pm_type = ps_game::PM_TYPE_DEAD;
            ClientLifecycle::Dead
        }
        _ => {
            ps.pm_type = ps_game::PM_TYPE_SPECTATOR;
            let team = world
                .client_meta(id)
                .map_or(entity_t6::team::FREE, |m| m.client_state_team);
            spectator_lifecycle(team)
        }
    };
    *world.ensure_player(id) = ps;
    diag::info!(
        Sim,
        "spawn: script client={} {sessionstate} at [{:.1}, {:.1}, {:.1}]",
        id.0,
        origin[0],
        origin[1],
        origin[2]
    );
    let Some(life) = begin_life(world, tick, id, lifecycle) else {
        return;
    };
    let class_id = world
        .client_meta(id)
        .and_then(|m| m.loadout.as_ref().map(|l| l.class_id))
        .unwrap_or(crate::ClassId(0));
    world.push_event(
        tick,
        EventAudience::Client(id),
        SimEvent::Spawned {
            class_id,
            spawn_index: 0,
            life_sequence: life,
        },
    );
    world.link_player_standing_area(id);
}

/// Records the new lifecycle and clears what a fresh life starts without. Only a client who is now alive starts a new
/// life; the life number it got is returned.
fn begin_life(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    lifecycle: ClientLifecycle,
) -> Option<crate::LifeSequence> {
    let meta = world.client_meta_mut(id);
    meta.lifecycle = lifecycle;
    meta.controls.switch_to = 0;
    meta.controls.linked = false;
    // The player state is cleared by a spawn, and the allow* switches (kept in its movement flags) with it.
    meta.controls.prone_disabled = false;
    meta.controls.crouch_disabled = false;
    meta.controls.stand_disabled = false;
    meta.controls.sprint_disabled = false;
    meta.controls.melee_disabled = false;
    meta.controls.ads_disabled = false;
    if lifecycle != ClientLifecycle::Alive {
        return None;
    }
    meta.life_sequence = meta.life_sequence.next();
    meta.dead_since_tick = None;
    meta.item_use_spawn_ms = crate::corpse::level_time_ms(tick);
    meta.item_use_entity = None;
    meta.clear_ammo_inventory();
    meta.weapon_shot_count = 0;
    meta.burst_latch = false;
    meta.burst_latch_secondary = false;
    meta.rechamber_pending = false;
    meta.rechamber_pending_secondary = false;
    meta.t6_gun = None;
    Some(meta.life_sequence)
}

/// Brings a downed (last stand) player back to his feet.
pub(crate) fn revive(world: &mut FrameWorld, id: ClientId) {
    if let Some(ps) = world.player_mut(id) {
        ps.pm_type = 0;
        ps.view_height_target = movement_t6::bridge::view_height::CROUCH;
    }
}

/// Ends the life of a living client: marks him dead, plays his death, announces it and drops his grenade. Does nothing
/// to a client who is not alive.
pub(crate) fn kill(
    world: &mut FrameWorld,
    tick: Tick,
    victim: ClientId,
    attacker: Option<ClientId>,
    commit: Option<DeathCommit>,
) {
    let alive = world
        .client_meta(victim)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive);
    if !alive {
        return;
    }
    let meta = world.client_meta_mut(victim);
    meta.lifecycle = ClientLifecycle::Dead;
    meta.dead_since_tick = Some(tick.0);
    meta.controls.switch_to = 0;
    let life_sequence = meta.life_sequence;
    crate::damage::play_death(world, victim, attacker);
    if let Some(ps) = world.player_mut(victim) {
        ps.health = 0;
    }
    world.unlink_player_area(victim);
    let attacker_life = attacker.and_then(|a| world.client_meta(a).map(|m| m.life_sequence));
    world.push_event(
        tick,
        EventAudience::All,
        SimEvent::Died {
            victim,
            life_sequence,
            attacker,
            attacker_life,
            source: commit.map(|c| c.source),
            weapon: commit.map_or(0, |c| c.weapon),
            killcam_entity_start_time: commit.map_or(0, |c| c.killcam_entity_start_time),
        },
    );
    crate::t6_gun::death_grenade_drop(world, tick, victim);
}

/// Announces a death in the kill feed. `means` is the `MOD_*` name; only the head shot, melee and suicide have a
/// feed icon of their own.
pub(crate) fn obituary(
    world: &mut FrameWorld,
    tick: Tick,
    victim: ClientId,
    attacker: Option<ClientId>,
    weapon: u32,
    means: &str,
) {
    let (weap_type, weap_class) = world
        .combat_facts_for(weapon)
        .map_or((0, 0), |f| (f.weap_type, f.weap_class));
    let icon = match means {
        "MOD_HEAD_SHOT" => hud_t6::gamemsg::MOD_HEAD_SHOT,
        "MOD_MELEE" => hud_t6::gamemsg::MOD_MELEE,
        "MOD_SUICIDE" => hud_t6::gamemsg::MOD_SUICIDE,
        _ => 0,
    };
    // A death with no attacker is shown as the victim's own.
    let credited = attacker.unwrap_or(victim);
    world.push_entity_event(
        tick,
        EventAudience::All,
        entity_t6::EntityEventKind::OBITUARY,
        crate::EntityEventPayload {
            number: victim.0 as i32,
            other_entity_num: victim.0 as i32,
            attacker_entity_num: credited.0 as i32,
            event_parm: hud_t6::gamemsg::pack_obituary_event_parm(weapon, icon, weap_type, weap_class),
            weapon,
            ..Default::default()
        },
    );
}

/// Leaves a corpse where the player stands and returns the clone slot it took.
pub(crate) fn clone_corpse(world: &mut FrameWorld, tick: Tick, id: ClientId) -> Option<u8> {
    let body = world.player(id).copied()?;
    let slot = crate::corpse::occupy_player_clone(world, id, &body, crate::corpse::level_time_ms(tick));
    if let Some(ps) = world.player_mut(id) {
        ps.corpse_index = i32::from(slot);
        ps.e_flags |= eflags::DEAD;
    }
    Some(slot)
}

/// Moves a living player to the spawn point a script picked, as a fresh life: full health, no speed, a new life number.
pub(crate) fn move_to_forced_spawn(world: &mut FrameWorld, id: ClientId, pick: crate::SpawnPick) {
    let Some((origin, angles)) = picked_point(world, id, pick) else {
        return;
    };
    world.unlink_player_area(id);
    world.set_origin(id, origin);
    world.set_viewangles(id, angles);
    if let Some(ps) = world.player_mut(id) {
        ps.velocity = [0.0; 3];
        ps.health = ps.max_health;
        ps.e_flags ^= eflags::TELEPORT;
    }
    let meta = world.client_meta_mut(id);
    meta.life_sequence = meta.life_sequence.next();
    world.link_player_standing_area(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_state::ScriptControls;

    #[test]
    fn a_spectator_waits_on_the_class_screen_unless_on_the_spectator_team() {
        assert_eq!(spectator_lifecycle(entity_t6::team::SPECTATOR), ClientLifecycle::Spectating);
        assert_eq!(spectator_lifecycle(entity_t6::team::FREE), ClientLifecycle::ChoosingClass);
    }

    #[test]
    fn a_spawn_allows_everything_again_except_jumping() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let id = ClientId(0);
        world.client_meta_mut(id).controls = ScriptControls {
            stand_disabled: true,
            crouch_disabled: true,
            prone_disabled: true,
            sprint_disabled: true,
            melee_disabled: true,
            ads_disabled: true,
            jump_disabled: true,
            ..Default::default()
        };
        spawn(&mut world, Tick(1), id, [0.0; 3], [0.0; 3], "playing");
        let jump_only = ScriptControls {
            jump_disabled: true,
            ..Default::default()
        };
        assert_eq!(world.client_meta(id).unwrap().controls, jump_only);
    }

    #[test]
    fn each_session_state_gives_its_lifecycle() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let id = ClientId(0);
        for (state, want) in [
            ("playing", ClientLifecycle::Alive),
            ("dead", ClientLifecycle::Dead),
            ("intermission", ClientLifecycle::Intermission),
        ] {
            spawn(&mut world, Tick(1), id, [0.0; 3], [0.0; 3], state);
            assert_eq!(world.client_meta(id).unwrap().lifecycle, want, "{state}");
        }
        spawn(&mut world, Tick(1), id, [0.0; 3], [0.0; 3], "spectator");
        let team = world.client_meta(id).unwrap().client_state_team;
        assert_eq!(world.client_meta(id).unwrap().lifecycle, spectator_lifecycle(team));
    }

    #[test]
    fn only_a_spawn_into_play_starts_a_new_life() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let id = ClientId(0);
        spawn(&mut world, Tick(1), id, [0.0; 3], [0.0; 3], "playing");
        let first = world.client_meta(id).unwrap().life_sequence;
        spawn(&mut world, Tick(2), id, [0.0; 3], [0.0; 3], "dead");
        assert_eq!(world.client_meta(id).unwrap().life_sequence, first);
        spawn(&mut world, Tick(3), id, [0.0; 3], [0.0; 3], "playing");
        assert_eq!(world.client_meta(id).unwrap().life_sequence, first.next());
    }

    #[test]
    fn the_teleport_flag_flips_on_every_spawn() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let id = ClientId(0);
        spawn(&mut world, Tick(1), id, [0.0; 3], [0.0; 3], "playing");
        let before = world.player(id).unwrap().e_flags & eflags::TELEPORT;
        spawn(&mut world, Tick(2), id, [0.0; 3], [0.0; 3], "playing");
        let after = world.player(id).unwrap().e_flags & eflags::TELEPORT;
        assert_ne!(before, after);
    }

    #[test]
    fn revive_stands_a_downed_player_up() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let id = ClientId(0);
        world.ensure_player(id).pm_type = ps_game::PM_TYPE_LAST_STAND;
        revive(&mut world, id);
        let ps = world.player(id).unwrap();
        assert_eq!(ps.pm_type, 0);
        assert_eq!(ps.view_height_target, movement_t6::bridge::view_height::CROUCH);
    }
}
