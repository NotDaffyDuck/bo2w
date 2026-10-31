//! Damage taken by a player: the record scripts see, the means of death, health loss, last stand and the shove.

use crate::harm::{HarmRequest, HarmResult, FatalityRecord};
use crate::step_view::StepWorldView;
use crate::match_ids::HarmOrigin;
use crate::player_ledger::PlayerSessionStage;
use crate::sim_parts::{SeatNo, StepNo};
use playerstate_t6::game::{self, AvatarState};

/// One hit as the damage callback sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub victim: SeatNo,
    pub striker: Option<SeatNo>,
    pub amount: i32,
    pub bits: i32,
    pub means: &'static str,
    pub gun: u32,
    pub point: [f32; 3],
    pub way: [f32; 3],
    pub hit_part: u8,
    pub inflictor: Option<crate::ThrownId>,
    pub commit: Option<FatalityRecord>,
}

/// The hit came from a blast, not a direct strike.
pub(crate) const IDFLAGS_RADIUS: i32 = 1;
/// An armour-piercing gun's bullet (`level.idflags_no_armor`).
pub(crate) const IDFLAGS_NO_ARMOR: i32 = 2;
/// No shove: scripts mark a hit that has no direction with this.
pub(crate) const IDFLAGS_NO_KNOCKBACK: i32 = 4;
/// The bullet went through something first (`level.idflags_penetration`); the through-wall and FMJ challenges and the
/// FMJ battle chatter read it.
pub(crate) const IDFLAGS_PENETRATION: i32 = 8;

/// The means of death for a damage attempt. A missile that did not go off on the victim (a rocket before it arms, a
/// stuck Semtex, a bolt, a blade) hits as `MOD_IMPACT`, whatever its weapon does once armed.
fn attempt_means(world: &StepWorldView, attempt: &HarmRequest) -> &'static str {
    let projectile = matches!(attempt.source, HarmOrigin::Projectile(_));
    if attempt.impact && !attempt.blast && projectile {
        return "MOD_IMPACT";
    }
    means(world, attempt.source, attempt.gun, attempt.hit_part, attempt.blast)
}

/// The `MOD_*` name for damage from `cause` with `weapon`. `splash` is blast damage; `hitloc` picks the head shot.
pub(crate) fn means(world: &StepWorldView, harm_kind: HarmOrigin, gun: u32, hitloc: u8, splash: bool) -> &'static str {
    let grenade_class = weapon_t6::damage::catalog::GUN_GROUP_THROWABLE;
    let is_grenade = world.damage_profile_of(gun).map(|f| f.gun_kind) == Some(grenade_class);
    // A weapon with no missile row is taken to explode on impact.
    let missile_row = world.gadget_stats_lookup(gun).or_else(|| world.rocket_launch_facts(gun));
    let bursts_on_impact = missile_row.is_none_or(|f| f.shell_detonate_on_hit);
    match harm_kind {
        HarmOrigin::Melee => "MOD_MELEE",
        HarmOrigin::Shot(_) if hud_t6::gamemsg::kill_notice_was_skull(hitloc) => "MOD_HEAD_SHOT",
        // The gun's rifle-bullet flag picks the kind: rifles, LMGs and snipers fire rifle bullets, SMGs and pistols
        // pistol bullets.
        HarmOrigin::Shot(_) => {
            if world.pierce_facts_for(gun).long_gun_round {
                "MOD_RIFLE_BULLET"
            } else {
                "MOD_PISTOL_BULLET"
            }
        }
        HarmOrigin::Projectile(_) if splash => {
            if is_grenade {
                "MOD_GRENADE_SPLASH"
            } else {
                "MOD_PROJECTILE_SPLASH"
            }
        }
        // The missile itself striking something: MOD_IMPACT unless its weapon bursts on impact.
        HarmOrigin::Projectile(_) if !bursts_on_impact => "MOD_IMPACT",
        HarmOrigin::Projectile(_) => {
            if is_grenade {
                "MOD_GRENADE"
            } else {
                "MOD_PROJECTILE"
            }
        }
        HarmOrigin::Radius(_) => "MOD_EXPLOSIVE",
    }
}

/// A bullet from a gun whose definition says armour-piercing hits through armour.
fn pierces_armor(world: &StepWorldView, attempt: &HarmRequest) -> bool {
    matches!(attempt.source, HarmOrigin::Shot(_))
        && !world.gun_facts().is_empty()
        && world.gun_facts().get(attempt.gun).core.armor_piercing
}

fn unit_or_zero(v: [f32; 3]) -> [f32; 3] {
    let norm = v.iter().map(|c| c * c).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.map(|c| c / norm)
    } else {
        [0.0; 3]
    }
}

/// Hands one damage attempt to the scripts' damage callback and reports whether the victim died of it.
pub(crate) fn damage(world: &mut StepWorldView, tick: StepNo, attempt: &HarmRequest, amount: i32) -> HarmResult {
    let victim_at = world.avatar(attempt.target).map_or([0.0; 3], |ps| ps.origin);
    let source_at = attempt
        .attacker_source_pos
        .or_else(|| world.avatar(attempt.attacker).map(|ps| ps.origin))
        .unwrap_or(victim_at);
    // A rocket or bolt that strikes direct is not radius damage; its blast is.
    let blast = matches!(attempt.source, HarmOrigin::Radius(_)) || attempt.blast;
    let mut flags = 0;
    if blast {
        flags |= IDFLAGS_RADIUS;
    }
    if attempt.passed_through_wall {
        flags |= IDFLAGS_PENETRATION;
    }
    if pierces_armor(world, attempt) {
        flags |= IDFLAGS_NO_ARMOR;
    }
    let death = FatalityRecord {
        victim: attempt.target,
        victim_life: attempt.victim_life_index,
        attacker: attempt.attacker,
        killer_life: attempt.killer_life,
        source: attempt.source,
        pellet: attempt.pellet,
        gun: attempt.gun,
        much: amount,
        cam_ent_since: attempt.cam_ent_since,
        attacker_source_pos: attempt.attacker_source_pos,
        hit_part: attempt.hit_part,
    };
    let hit = Hit {
        victim: attempt.target,
        striker: Some(attempt.attacker),
        amount,
        bits: flags,
        means: attempt_means(world, attempt),
        gun: attempt.gun,
        point: attempt.attacker_source_pos.unwrap_or(victim_at),
        way: unit_or_zero([
            victim_at[0] - source_at[0],
            victim_at[1] - source_at[1],
            victim_at[2] - source_at[2],
        ]),
        hit_part: attempt.hit_part,
        inflictor: match attempt.source {
            HarmOrigin::Projectile(id) => Some(id),
            _ => None,
        },
        commit: Some(death),
    };
    crate::gsc_host::player_damage(world.ecs(), tick, &hit);
    let died_this_life = world
        .client_meta(attempt.target)
        .is_some_and(|m| m.lifecycle == PlayerSessionStage::Dead && m.life_sequence == attempt.victim_life_index);
    if died_this_life {
        HarmResult::Died(death)
    } else {
        HarmResult::Survived {
            health_after: world.avatar(attempt.target).map_or(0, |ps| ps.health),
        }
    }
}

/// Runs the damage callback for `amount` damage that has no attacker and no cause (a console command).
pub(crate) fn debug_damage(world: &mut StepWorldView, tick: StepNo, id: SeatNo, qty: i32) {
    let here = world.avatar(id).map_or([0.0; 3], |ps| ps.origin);
    let hit = Hit {
        victim: id,
        striker: None,
        amount: qty,
        bits: 0,
        means: "MOD_UNKNOWN",
        gun: 0,
        point: here,
        way: [0.0; 3],
        hit_part: 0,
        inflictor: None,
        commit: None,
    };
    crate::gsc_host::player_damage(world.ecs(), tick, &hit);
}

/// What a damage step left of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Finish {
    Hurt,
    DownedGrace,
    Killed,
}

/// Takes `amount` health off a living player and says how it ended. With `bo2` the hit flinches and widens the aim
/// as Black Ops II does, sized by the share of health the frame's hits took; without it each hit is one small flinch.
pub(crate) fn finish_damage(
    world: &mut StepWorldView,
    id: SeatNo,
    harm: i32,
    push: Option<[f32; 3]>,
    bo2: bool,
) -> Finish {
    if !matches!(world.client_meta(id), Some(m) if m.lifecycle == PlayerSessionStage::Alive) {
        return Finish::Hurt;
    }
    if bo2 {
        frame_flinch(world, id, harm, push);
    }
    let Some(body) = world.occupant_mut(id) else {
        return Finish::Hurt;
    };
    movement_t6::bridge::update_damage_timer(body, harm, push);
    body.health = (body.health - harm.max(0)).max(0);
    if !bo2 {
        legacy_flinch(body, push);
    }
    if body.health > 0 {
        return Finish::Hurt;
    }
    // Last stand lives in movement_mode alone (PM_LASTSTAND).
    let not_yet_down = body.movement_mode != game::MOVE_MODE_DOWNED;
    if not_yet_down && body.has_perk_bit(playerstate_t6::perks::PERK_PISTOLDEATH) {
        body.health = 1;
        body.movement_mode = game::MOVE_MODE_DOWNED;
        body.eye_height_goal = movement_t6::bridge::eye_level::DOWNED_GRACE;
        return Finish::DownedGrace;
    }
    Finish::Killed
}

/// MW2's flinch rules: one event and one count a hit, and the HUD's damage direction as a byte angle (256 = a turn).
fn legacy_flinch(body: &mut AvatarState, heading: Option<[f32; 3]>) {
    body.hurt_amount = body.hurt_amount.saturating_add(1);
    body.damage_event = body.damage_event.wrapping_add(1);
    if let Some(d) = heading.filter(|d| d[0] != 0.0 || d[1] != 0.0) {
        let yaw = game_math::heading_of(d[0], d[1]);
        body.damage_yaw = ((yaw * 256.0 / 360.0) as u32) & 255;
    }
}

/// Black Ops II sends one flinch a frame for all of that frame's hits: their share of his max health along the last
/// hit's direction. The same share widens his aim.
fn frame_flinch(world: &mut StepWorldView, id: SeatNo, qty: i32, heading: Option<[f32; 3]>) {
    let frame = world.entity_kernel().frame_serial();
    let record = world.client_meta_mut(id);
    if record.hit_frame != Some(frame) {
        record.hit_frame = Some(frame);
        record.hit_blood = 0;
        record.hit_sent = None;
    }
    record.hit_blood = record.hit_blood.saturating_add(qty.max(0));
    let (taken, already) = (record.hit_blood, record.hit_sent);
    let Some(body) = world.occupant_mut(id) else {
        return;
    };
    let Some((count, yaw, pitch)) = weapon_t6::view_bob::damage_feedback_bo2(taken, body.hit_point_ceiling, heading) else {
        return;
    };
    let extra = (count - already.unwrap_or(0)) as f32;
    body.sight_spread_factor = (body.sight_spread_factor + extra).min(weapon_t6::recoil::SPREAD_MAX);
    if already.is_none() {
        body.damage_event = body.damage_event.wrapping_add(1);
    }
    body.hurt_amount = count;
    body.damage_yaw = yaw;
    body.damage_pitch = pitch;
    world.client_meta_mut(id).hit_sent = Some(count);
}

/// `g_knockback`, Black Ops II's default.
const KNOCKBACK_SCALE: f32 = 1000.0;
/// movement_state_bits 0x100 (PMF_TIME_KNOCKBACK): shoved, so the ground's grip is off until pm_time runs out.
const PMF_TIME_KNOCKBACK: u32 = 0x100;

/// ApplyKnockBack: a hit of `amount` shoves the player along `dir` (a unit vector) by 0.3 of the damage standing, 0.15
/// crouched or 0.02 prone, at most 60, times `g_knockback` / 250 in units a second. Unless `forced` (the hit that kills)
/// nobody in the air or mid-jump is shoved, and nobody on a turret ever is. With no other move timer running the shove
/// lifts the ground's grip for twice its size in ms (50 to 200).
pub(crate) fn knockback(body: &mut AvatarState, qty: i32, dir: [f32; 3], forced: bool) {
    use game::movement_state_bits::{DUCK_BUTTON_BIT, JUMPING, LYING_FLAT};
    let airborne = body.movement_state_bits & JUMPING != 0 || body.standing_on == game::NO_ENTITY_SLOT;
    if airborne && !forced {
        return;
    }
    let share = if body.movement_state_bits & LYING_FLAT != 0 {
        0.02
    } else if body.movement_state_bits & DUCK_BUTTON_BIT != 0 {
        0.15
    } else {
        0.3
    };
    let points = ((qty as f32 * share) as i32).min(60);
    if points == 0 || body.e_flags & game::entity_flag_bits::TURRET_MASK != 0 {
        return;
    }
    let speed = KNOCKBACK_SCALE * points as f32 * 0.004;
    for (v, d) in body.velocity.iter_mut().zip(dir) {
        *v += d * speed;
    }
    if body.move_timer == 0 {
        body.move_timer = (points * 2).clamp(50, 200);
        body.movement_state_bits |= PMF_TIME_KNOCKBACK;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::GadgetLiveStats;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    fn missile_attempt(impact: bool, blast: bool) -> HarmRequest {
        HarmRequest {
            source: HarmOrigin::Projectile(crate::ThrownId(1)),
            pellet: crate::BuckshotId(0),
            attacker: SeatNo(0),
            killer_life: crate::LifeCount(0),
            target: SeatNo(1),
            victim_life_index: crate::LifeCount(0),
            gun: 7,
            much: 50,
            cam_ent_since: 0,
            attacker_source_pos: None,
            hit_part: 0,
            blast,
            passed_through_wall: false,
            impact,
        }
    }

    #[test]
    fn a_missile_that_bursts_on_impact_keeps_its_own_means() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let thrown = GadgetLiveStats {
            missile_velocity: 1000,
            collision_harm: 125,
            ..Default::default()
        };
        let rocket = GadgetLiveStats {
            shell_detonate_on_hit: true,
            ..thrown
        };
        let mut content = crate::SimContentBuilder::default();
        content.with_gear_table(vec![GadgetLiveStats::default(), thrown, rocket]);
        world.install_content(content.finish());
        let strike = HarmOrigin::Projectile(crate::ThrownId(1));
        // A tomahawk, bolt, knife or frag striking him is an impact.
        assert_eq!(means(&world, strike, 1, 0, false), "MOD_IMPACT");
        assert_eq!(means(&world, strike, 2, 0, false), "MOD_PROJECTILE");
        // The blast of either is splash.
        assert_eq!(means(&world, strike, 1, 0, true), "MOD_PROJECTILE_SPLASH");
        assert_eq!(means(&world, strike, 2, 0, true), "MOD_PROJECTILE_SPLASH");
        // A weapon with no missile row is taken to burst.
        assert_eq!(means(&world, strike, 7, 0, false), "MOD_PROJECTILE");
    }

    #[test]
    fn a_dud_missile_hits_as_an_impact() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let world = sim.frame();
        assert_eq!(attempt_means(&world, &missile_attempt(true, false)), "MOD_IMPACT");
        assert_eq!(attempt_means(&world, &missile_attempt(false, false)), "MOD_PROJECTILE");
        assert_eq!(attempt_means(&world, &missile_attempt(false, true)), "MOD_PROJECTILE_SPLASH");
    }

    #[test]
    fn a_frames_hits_send_one_flinch_sized_by_their_share() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let id = SeatNo(0);
        world.client_meta_mut(id).lifecycle = PlayerSessionStage::Alive;
        let pstate = world.seat_slot_for(id);
        pstate.hit_point_ceiling = 150;
        pstate.health = 150;
        pstate.sight_spread_factor = 0.0;
        let event = pstate.damage_event;
        // Two hits in one frame: one flinch for 45 of 150 health, along the last hit's travel (+y = yaw byte 64).
        world.entity_kernel_mut().open_tick(50);
        finish_damage(&mut world, id, 15, Some([1.0, 0.0, 0.0]), true);
        finish_damage(&mut world, id, 30, Some([0.0, 1.0, 0.0]), true);
        let pstate = world.avatar(id).unwrap();
        assert_eq!(pstate.damage_event, event.wrapping_add(1));
        assert_eq!((pstate.hurt_amount, pstate.damage_yaw, pstate.damage_pitch), (30, 64, 0));
        assert_eq!(pstate.sight_spread_factor, 30.0);
        assert_eq!(pstate.health, 105);
        // The next frame's hit flinches on its own; no direction reads 255/255.
        world.entity_kernel_mut().open_tick(100);
        finish_damage(&mut world, id, 3, None, true);
        let pstate = world.avatar(id).unwrap();
        assert_eq!(pstate.damage_event, event.wrapping_add(2));
        assert_eq!((pstate.hurt_amount, pstate.damage_yaw, pstate.damage_pitch), (2, 255, 255));
        assert_eq!(pstate.sight_spread_factor, 32.0);
        // MW2's rules: one small flinch a hit.
        finish_damage(&mut world, id, 3, None, false);
        assert_eq!(world.avatar(id).unwrap().damage_event, event.wrapping_add(3));
    }

    #[test]
    fn a_hit_shoves_by_its_damage_and_stance() {
        let mut target = AvatarState::ZERO;
        // Standing, 50 damage: 15 points, 60 units a second, the grip off 50 ms.
        knockback(&mut target, 50, [1.0, 0.0, 0.0], false);
        assert!(near(target.velocity, [60.0, 0.0, 0.0]), "{:?}", target.velocity);
        assert_eq!((target.move_timer, target.movement_state_bits & 0x100), (50, 0x100));
        // Crouched, 1000 damage: held to 60 points; the timer is already running.
        target.velocity = [0.0; 3];
        target.movement_state_bits = game::movement_state_bits::DUCK_BUTTON_BIT;
        knockback(&mut target, 1000, [0.0, 1.0, 0.0], false);
        assert!(near(target.velocity, [0.0, 240.0, 0.0]), "{:?}", target.velocity);
        assert_eq!((target.move_timer, target.movement_state_bits & 0x100), (50, 0));
        // Prone, 40 damage: under a point, no shove.
        target.velocity = [0.0; 3];
        target.movement_state_bits = game::movement_state_bits::LYING_FLAT;
        knockback(&mut target, 40, [0.0, 1.0, 0.0], false);
        assert_eq!(target.velocity, [0.0; 3]);
    }

    #[test]
    fn only_the_killing_hit_shoves_him_in_the_air() {
        let mut ps = AvatarState::ZERO;
        ps.standing_on = game::NO_ENTITY_SLOT;
        knockback(&mut ps, 100, [0.0, 0.0, 1.0], false);
        assert_eq!(ps.velocity, [0.0; 3]);
        knockback(&mut ps, 100, [0.0, 0.0, 1.0], true);
        assert!(near(ps.velocity, [0.0, 0.0, 120.0]), "{:?}", ps.velocity);
        // Nobody on a turret is shoved.
        for turret in [game::entity_flag_bits::TURRET_STAND, game::entity_flag_bits::TURRET_CROUCH] {
            let mut ps = AvatarState::ZERO;
            ps.e_flags = turret;
            knockback(&mut ps, 100, [1.0, 0.0, 0.0], true);
            assert_eq!(ps.velocity, [0.0; 3]);
        }
        // 0x400 and 0x800 are not turret bits in Black Ops II (0x800 is aim assist).
        let mut ps = AvatarState::ZERO;
        ps.e_flags = 0xc00;
        knockback(&mut ps, 100, [1.0, 0.0, 0.0], true);
        assert!(ps.velocity[0] > 0.0);
    }

    #[test]
    fn a_zero_vector_has_no_direction() {
        assert_eq!(unit_or_zero([0.0; 3]), [0.0; 3]);
        assert!(near(unit_or_zero([0.0, 3.0, 4.0]), [0.0, 0.6, 0.8]));
    }
}
