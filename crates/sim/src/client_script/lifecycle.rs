//! A player's life: spawning into a session state, dying, the corpse left behind and scripted moves to a spawn point.

use crate::harm::FatalityRecord;
use crate::step_view::StepWorldView;
use crate::player_ledger::{PlayerSessionStage, EventRecipients, MatchOccurrence};
use crate::sim_parts::{SeatNo, StepNo, fresh_avatar_at};
use movement_t6::bridge::WORD16_TO_DEGREES;
use playerstate_t6::game::{self, entity_flag_bits};

/// What a client who is not playing is doing: watching, or still picking a class.
pub(crate) fn spectator_lifecycle(team: i32) -> PlayerSessionStage {
    if team == entity_t6::team::SPECTATOR {
        PlayerSessionStage::Spectating
    } else {
        PlayerSessionStage::PickingLoadout
    }
}

/// The spawn point a scripted pick stands for, if one is accepted: where to stand and which way to look.
fn picked_point(world: &mut StepWorldView, seat: SeatNo, pick: crate::SpawnPick) -> Option<([f32; 3], [f32; 3])> {
    let team = world
        .client_meta(seat)
        .map_or(entity_t6::team::FREE, |m| m.seat_side);
    let own_origin = world.avatar(seat).map(|ps| ps.origin);
    let avoid: Vec<[f32; 3]> = world
        .living_positions()
        .into_iter()
        .filter(|origin| own_origin != Some(*origin))
        .collect();
    let report = crate::spawn_pick::decide_forced_spawn(world, pick, &avoid, team);
    crate::frame_step::log_forced_spawn(seat, pick, &report);
    report.accepted.map(|chosen| (chosen.dropped_origin, chosen.authored_angles))
}

/// Puts a client into the named session state ("playing", "dead", "intermission", anything else is spectating) at
/// `origin` and `angles`, or at the point a script forced for this spawn.
pub(crate) fn spawn(
    level: &mut StepWorldView,
    step: StepNo,
    client: SeatNo,
    spot: [f32; 3],
    aim: [f32; 3],
    state_name: &str,
) {
    // A forced point is spent by the spawn it was queued for, used or not.
    let forced = level.client_meta_mut(client).forced_spawn.take();
    let (spot, aim) = forced
        .filter(|_| state_name == "playing")
        .and_then(|pick| picked_point(level, client, pick))
        .unwrap_or((spot, aim));
    level.release_seat_area(client);
    let mut fresh = fresh_avatar_at(spot, aim);
    if let Some(max) = level.client_meta(client).map(|m| m.hit_point_ceiling).filter(|&n| n > 0) {
        fresh.hit_point_ceiling = max;
        fresh.health = max;
    }
    if let Some(cmd) = level.old_cmd_angles(client) {
        fresh.view_offset_angles = std::array::from_fn(|axis| aim[axis] - cmd[axis] as f32 * WORD16_TO_DEGREES);
    }
    // The teleport flag flips on every spawn, so the client sees the jump.
    let was_teleporting = level.avatar(client).map_or(0, |p| p.e_flags & entity_flag_bits::TELEPORT);
    fresh.e_flags = (fresh.e_flags & !entity_flag_bits::TELEPORT) | (was_teleporting ^ entity_flag_bits::TELEPORT);
    // Black Ops II runs the player up to the level time before his first command, so that command moves one frame, as
    // the client predicts it, not the whole time since the clock started.
    fresh.last_cmd_ms = crate::bodies::level_time_ms(step);
    let lifecycle = match state_name {
        "playing" => PlayerSessionStage::Alive,
        "intermission" => {
            fresh.movement_mode = game::MOVE_MODE_INTERLUDE;
            PlayerSessionStage::Intermission
        }
        "dead" => {
            fresh.movement_mode = game::MOVE_KIND_DEAD;
            PlayerSessionStage::Dead
        }
        _ => {
            fresh.movement_mode = game::MOVE_MODE_OBSERVER;
            let team = level
                .client_meta(client)
                .map_or(entity_t6::team::FREE, |m| m.seat_side);
            spectator_lifecycle(team)
        }
    };
    *level.seat_slot_for(client) = fresh;
    diag::info!(
        Sim,
        "spawn: script client={} {state_name} at [{:.1}, {:.1}, {:.1}]",
        client.0,
        spot[0],
        spot[1],
        spot[2]
    );
    let Some(new_life) = begin_life(level, step, client, lifecycle) else {
        return;
    };
    let class_id = level
        .client_meta(client)
        .and_then(|m| m.kit.as_ref().map(|l| l.class_id))
        .unwrap_or(crate::LoadoutClass(0));
    let spawned = MatchOccurrence::Spawned {
        class_id,
        spawn_slot: 0,
        life_sequence: new_life,
    };
    level.log_occurrence(step, EventRecipients::Client(client), spawned);
    level.relink_seat_footing(client);
}

/// Records the new lifecycle and clears what a fresh life starts without. Only a client who is now alive starts a new
/// life; the life number it got is returned.
fn begin_life(
    world: &mut StepWorldView,
    tick: StepNo,
    id: SeatNo,
    lifecycle: PlayerSessionStage,
) -> Option<crate::LifeCount> {
    let slot = world.client_meta_mut(id);
    slot.lifecycle = lifecycle;
    let controls = &mut slot.controls;
    controls.switch_to = 0;
    controls.paired = false;
    // The player state is cleared by a spawn, and the allow* switches (kept in its movement flags) with it.
    controls.prone_disabled = false;
    controls.crouch_disabled = false;
    controls.stand_disabled = false;
    controls.sprint_disabled = false;
    controls.melee_disabled = false;
    controls.ads_disabled = false;
    if lifecycle != PlayerSessionStage::Alive {
        return None;
    }
    slot.life_sequence = slot.life_sequence.next();
    slot.died_at_step = None;
    slot.item_ready_ms = crate::bodies::level_time_ms(tick);
    slot.use_target = None;
    slot.wipe_ammo_stock();
    slot.shots_per_gun = 0;
    slot.burst_held = false;
    slot.burst_latch_secondary = false;
    slot.bolt_due = false;
    slot.rechamber_pending_secondary = false;
    slot.t6_gun = None;
    Some(slot.life_sequence)
}

/// Brings a downed (last stand) player back to his feet.
pub(crate) fn revive(world: &mut StepWorldView, id: SeatNo) {
    if let Some(body) = world.occupant_mut(id) {
        body.movement_mode = 0;
        body.eye_height_goal = movement_t6::bridge::eye_level::DUCK_BUTTON_BIT;
    }
}

/// Ends the life of a living client: marks him dead, plays his death, announces it and drops his grenade. Does nothing
/// to a client who is not alive.
pub(crate) fn kill(
    level: &mut StepWorldView,
    step: StepNo,
    victim: SeatNo,
    killer: Option<SeatNo>,
    commit: Option<FatalityRecord>,
) {
    let was_alive = matches!(level.client_meta(victim), Some(m) if m.lifecycle == PlayerSessionStage::Alive);
    if !was_alive {
        return;
    }
    let record = level.client_meta_mut(victim);
    record.lifecycle = PlayerSessionStage::Dead;
    record.died_at_step = Some(step.0);
    record.controls.switch_to = 0;
    let victim_life = record.life_sequence;
    crate::harm::play_death(level, victim, killer);
    if let Some(body) = level.occupant_mut(victim) {
        body.health = 0;
    }
    level.release_seat_area(victim);
    let killer_life = killer.and_then(|a| level.client_meta(a).map(|m| m.life_sequence));
    let died = MatchOccurrence::Died {
        victim,
        life_sequence: victim_life,
        attacker: killer,
        attacker_life: killer_life,
        source: commit.map(|c| c.source),
        gun: commit.map_or(0, |c| c.gun),
        cam_ent_since: commit.map_or(0, |c| c.cam_ent_since),
    };
    level.log_occurrence(step, EventRecipients::All, died);
    crate::gun_brain::death_grenade_drop(level, step, victim);
}

/// Announces a death in the kill feed. `means` is the `MOD_*` name. Head shot, crush, falling and suicide always
/// show an icon of their own; melee and impact do unless the weapon is a riot shield or a blade (BO2 `GScr_Obituary`).
pub(crate) fn obituary(
    world: &mut StepWorldView,
    tick: StepNo,
    victim: SeatNo,
    killer: Option<SeatNo>,
    gun: u32,
    means: &str,
) {
    let (riot_shield, blade_impact) = world.damage_profile_of(gun).map_or((false, false), |f| {
        (
            f.gun_family_code == weapon_t6::damage::catalog::WEAPTYPE_RIOTSHIELD,
            f.impact_kind_id == weapon_t6::damage::catalog::IMPACT_TYPE_BLADE,
        )
    });
    let icon = match means {
        "MOD_HEAD_SHOT" => hud_t6::gamemsg::MOD_HEADSHOT,
        "MOD_MELEE" => hud_t6::gamemsg::MOD_MELEE,
        "MOD_IMPACT" => hud_t6::gamemsg::MOD_IMPACT,
        "MOD_CRUSH" => hud_t6::gamemsg::MOD_CRUSH,
        "MOD_FALLING" => hud_t6::gamemsg::MOD_FALLING,
        "MOD_SUICIDE" => hud_t6::gamemsg::MOD_SUICIDE,
        _ => 0,
    };
    // A death with no attacker is shown as the victim's own.
    let credited = killer.unwrap_or(victim);
    let victim_num = victim.0 as i32;
    world.post_entity_event(
        tick,
        EventRecipients::All,
        entity_t6::EventCode::OBITUARY,
        crate::HappeningBody {
            number: victim_num,
            other_entity_num: victim_num,
            attacker_entity_num: credited.0 as i32,
            event_parm: hud_t6::gamemsg::encode_kill_notice_arg(gun, icon, riot_shield, blade_impact),
            weapon: gun,
            ..Default::default()
        },
    );
}

/// Leaves a corpse where the player stands and returns the clone slot it took.
pub(crate) fn clone_corpse(world: &mut StepWorldView, step: StepNo, id: SeatNo) -> Option<u8> {
    let body = world.avatar(id).copied()?;
    let cell = crate::bodies::fill_clone_slot(world, id, &body, crate::bodies::level_time_ms(step));
    if let Some(body) = world.occupant_mut(id) {
        body.body_index = cell.into();
        body.e_flags |= entity_flag_bits::DEAD;
    }
    Some(cell)
}

/// Moves a living player to the spawn point a script picked, as a fresh life: full health, no speed, a new life number.
pub(crate) fn move_to_forced_spawn(world: &mut StepWorldView, client: SeatNo, pick: crate::SpawnPick) {
    let Some((spot, angles)) = picked_point(world, client, pick) else {
        return;
    };
    world.release_seat_area(client);
    world.place_at(client, spot);
    world.write_aim(client, angles);
    if let Some(body) = world.occupant_mut(client) {
        body.velocity = [0.0; 3];
        body.health = body.hit_point_ceiling;
        body.e_flags ^= entity_flag_bits::TELEPORT;
    }
    let record = world.client_meta_mut(client);
    record.life_sequence = record.life_sequence.next();
    world.relink_seat_footing(client);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_ledger::ScriptControls;

    #[test]
    fn a_spectator_waits_on_the_class_screen_unless_on_the_spectator_team() {
        assert_eq!(spectator_lifecycle(entity_t6::team::SPECTATOR), PlayerSessionStage::Spectating);
        assert_eq!(spectator_lifecycle(entity_t6::team::FREE), PlayerSessionStage::PickingLoadout);
    }

    #[test]
    fn a_spawn_allows_everything_again_except_jumping() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let id = SeatNo(0);
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
        spawn(&mut world, StepNo(1), id, [0.0; 3], [0.0; 3], "playing");
        let jump_only = ScriptControls {
            jump_disabled: true,
            ..Default::default()
        };
        assert_eq!(world.client_meta(id).unwrap().controls, jump_only);
    }

    #[test]
    fn each_session_state_gives_its_lifecycle() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let id = SeatNo(0);
        for (state, want) in [
            ("playing", PlayerSessionStage::Alive),
            ("dead", PlayerSessionStage::Dead),
            ("intermission", PlayerSessionStage::Intermission),
        ] {
            spawn(&mut world, StepNo(1), id, [0.0; 3], [0.0; 3], state);
            assert_eq!(world.client_meta(id).unwrap().lifecycle, want, "{state}");
        }
        spawn(&mut world, StepNo(1), id, [0.0; 3], [0.0; 3], "spectator");
        let team = world.client_meta(id).unwrap().seat_side;
        assert_eq!(world.client_meta(id).unwrap().lifecycle, spectator_lifecycle(team));
    }

    #[test]
    fn only_a_spawn_into_play_starts_a_new_life() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let id = SeatNo(0);
        spawn(&mut world, StepNo(1), id, [0.0; 3], [0.0; 3], "playing");
        let first = world.client_meta(id).unwrap().life_sequence;
        spawn(&mut world, StepNo(2), id, [0.0; 3], [0.0; 3], "dead");
        assert_eq!(world.client_meta(id).unwrap().life_sequence, first);
        spawn(&mut world, StepNo(3), id, [0.0; 3], [0.0; 3], "playing");
        assert_eq!(world.client_meta(id).unwrap().life_sequence, first.next());
    }

    #[test]
    fn the_teleport_flag_flips_on_every_spawn() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let id = SeatNo(0);
        spawn(&mut world, StepNo(1), id, [0.0; 3], [0.0; 3], "playing");
        let before = world.avatar(id).unwrap().e_flags & entity_flag_bits::TELEPORT;
        spawn(&mut world, StepNo(2), id, [0.0; 3], [0.0; 3], "playing");
        let after = world.avatar(id).unwrap().e_flags & entity_flag_bits::TELEPORT;
        assert_ne!(before, after);
    }

    #[test]
    fn revive_stands_a_downed_player_up() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let id = SeatNo(0);
        world.seat_slot_for(id).movement_mode = game::MOVE_MODE_DOWNED;
        revive(&mut world, id);
        let ps = world.avatar(id).unwrap();
        assert_eq!(ps.movement_mode, 0);
        assert_eq!(ps.eye_height_goal, movement_t6::bridge::eye_level::DUCK_BUTTON_BIT);
    }
}
