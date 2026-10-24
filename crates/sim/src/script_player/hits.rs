//! Damage taken by a player: the record scripts see, the means of death, health loss, last stand and the shove.

use crate::damage::{DamageAttempt, DamageOutcome, DeathCommit};
use crate::frame::FrameWorld;
use crate::identities::DamageSource;
use crate::match_state::ClientLifecycle;
use crate::world::{ClientId, Tick};
use playerstate_t6::game::{self as ps_game, PlayerState};

/// One hit as the damage callback sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub victim: ClientId,
    pub attacker: Option<ClientId>,
    pub amount: i32,
    pub flags: i32,
    pub means: &'static str,
    pub weapon: u32,
    pub point: [f32; 3],
    pub dir: [f32; 3],
    pub hitloc: u8,
    pub inflictor: Option<crate::ProjectileId>,
    pub commit: Option<DeathCommit>,
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
fn attempt_means(world: &FrameWorld, attempt: &DamageAttempt) -> &'static str {
    let projectile = matches!(attempt.source, DamageSource::Projectile(_));
    if attempt.impact && !attempt.blast && projectile {
        return "MOD_IMPACT";
    }
    means(world, attempt.source, attempt.weapon, attempt.hitloc, attempt.blast)
}

/// The `MOD_*` name for damage from `source` with `weapon`. `splash` is blast damage; `hitloc` picks the head shot.
pub(crate) fn means(
    world: &FrameWorld,
    source: DamageSource,
    weapon: u32,
    hitloc: u8,
    splash: bool,
) -> &'static str {
    let is_grenade = world
        .combat_facts_for(weapon)
        .is_some_and(|f| f.weap_class == weapon_t6::damage::catalog::WEAPCLASS_GRENADE);
    // A weapon with no missile row is taken to explode on impact.
    let bursts_on_impact = world
        .equipment_facts_for(weapon)
        .or_else(|| world.missile_launch_facts(weapon))
        .is_none_or(|f| f.proj_impact_explode);
    match source {
        DamageSource::Melee => "MOD_MELEE",
        DamageSource::Shot(_) if hud_t6::gamemsg::obituary_is_headshot(hitloc) => "MOD_HEAD_SHOT",
        // The gun's rifle-bullet flag picks the kind: rifles, LMGs and snipers fire rifle bullets, SMGs and pistols
        // pistol bullets.
        DamageSource::Shot(_) => {
            if world.bullet_pen_facts_for(weapon).rifle_bullet {
                "MOD_RIFLE_BULLET"
            } else {
                "MOD_PISTOL_BULLET"
            }
        }
        DamageSource::Projectile(_) if splash => {
            if is_grenade {
                "MOD_GRENADE_SPLASH"
            } else {
                "MOD_PROJECTILE_SPLASH"
            }
        }
        // The missile itself striking something: MOD_IMPACT unless its weapon bursts on impact.
        DamageSource::Projectile(_) if !bursts_on_impact => "MOD_IMPACT",
        DamageSource::Projectile(_) => {
            if is_grenade {
                "MOD_GRENADE"
            } else {
                "MOD_PROJECTILE"
            }
        }
        DamageSource::Radius(_) => "MOD_EXPLOSIVE",
    }
}

/// A bullet from a gun whose definition says armour-piercing hits through armour.
fn pierces_armor(world: &FrameWorld, attempt: &DamageAttempt) -> bool {
    matches!(attempt.source, DamageSource::Shot(_))
        && !world.gun_facts().is_empty()
        && world.gun_facts().get(attempt.weapon).core.armor_piercing
}

fn unit_or_zero(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 {
        v.map(|c| c / len)
    } else {
        [0.0; 3]
    }
}

/// Hands one damage attempt to the scripts' damage callback and reports whether the victim died of it.
pub(crate) fn damage(world: &mut FrameWorld, tick: Tick, attempt: &DamageAttempt, amount: i32) -> DamageOutcome {
    let victim_at = world.player(attempt.target).map_or([0.0; 3], |ps| ps.origin);
    let source_at = attempt
        .inflictor_origin
        .or_else(|| world.player(attempt.attacker).map(|ps| ps.origin))
        .unwrap_or(victim_at);
    // A rocket or bolt that strikes direct is not radius damage; its blast is.
    let blast = matches!(attempt.source, DamageSource::Radius(_)) || attempt.blast;
    let mut flags = 0;
    if blast {
        flags |= IDFLAGS_RADIUS;
    }
    if attempt.penetrated {
        flags |= IDFLAGS_PENETRATION;
    }
    if pierces_armor(world, attempt) {
        flags |= IDFLAGS_NO_ARMOR;
    }
    let commit = DeathCommit {
        victim: attempt.target,
        victim_life: attempt.target_life,
        attacker: attempt.attacker,
        attacker_life: attempt.attacker_life,
        source: attempt.source,
        pellet: attempt.pellet,
        weapon: attempt.weapon,
        amount,
        killcam_entity_start_time: attempt.killcam_entity_start_time,
        inflictor_origin: attempt.inflictor_origin,
        hitloc: attempt.hitloc,
    };
    let hit = Hit {
        victim: attempt.target,
        attacker: Some(attempt.attacker),
        amount,
        flags,
        means: attempt_means(world, attempt),
        weapon: attempt.weapon,
        point: attempt.inflictor_origin.unwrap_or(victim_at),
        dir: unit_or_zero([
            victim_at[0] - source_at[0],
            victim_at[1] - source_at[1],
            victim_at[2] - source_at[2],
        ]),
        hitloc: attempt.hitloc,
        inflictor: match attempt.source {
            DamageSource::Projectile(id) => Some(id),
            _ => None,
        },
        commit: Some(commit),
    };
    crate::script::player_damage(world.ecs(), tick, &hit);
    let died_this_life = world
        .client_meta(attempt.target)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Dead && m.life_sequence == attempt.target_life);
    if died_this_life {
        DamageOutcome::Died(commit)
    } else {
        DamageOutcome::Nonlethal {
            health_after: world.player(attempt.target).map_or(0, |ps| ps.health),
        }
    }
}

/// Runs the damage callback for `amount` damage that has no attacker and no cause (a console command).
pub(crate) fn debug_damage(world: &mut FrameWorld, tick: Tick, id: ClientId, amount: i32) {
    let here = world.player(id).map_or([0.0; 3], |ps| ps.origin);
    let hit = Hit {
        victim: id,
        attacker: None,
        amount,
        flags: 0,
        means: "MOD_UNKNOWN",
        weapon: 0,
        point: here,
        dir: [0.0; 3],
        hitloc: 0,
        inflictor: None,
        commit: None,
    };
    crate::script::player_damage(world.ecs(), tick, &hit);
}

/// What a damage step left of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Finish {
    Hurt,
    LastStand,
    Killed,
}

/// Takes `amount` health off a living player and says how it ended. With `bo2` the hit flinches and widens the aim
/// as Black Ops II does, sized by the share of health the frame's hits took; without it each hit is one small flinch.
pub(crate) fn finish_damage(
    world: &mut FrameWorld,
    id: ClientId,
    amount: i32,
    dir: Option<[f32; 3]>,
    bo2: bool,
) -> Finish {
    let alive = world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive);
    if !alive {
        return Finish::Hurt;
    }
    if bo2 {
        frame_flinch(world, id, amount, dir);
    }
    let Some(ps) = world.player_mut(id) else {
        return Finish::Hurt;
    };
    movement_t6::bridge::update_damage_timer(ps, amount, dir);
    ps.health = (ps.health - amount.max(0)).max(0);
    if !bo2 {
        legacy_flinch(ps, dir);
    }
    if ps.health > 0 {
        return Finish::Hurt;
    }
    // Last stand lives in pm_type alone (PM_LASTSTAND).
    let can_last_stand =
        ps.perks[0] & ps_game::PERK_PISTOLDEATH != 0 && ps.pm_type != ps_game::PM_TYPE_LAST_STAND;
    if can_last_stand {
        ps.health = 1;
        ps.pm_type = ps_game::PM_TYPE_LAST_STAND;
        ps.view_height_target = movement_t6::bridge::view_height::LAST_STAND;
        return Finish::LastStand;
    }
    Finish::Killed
}

/// MW2's flinch rules: one event and one count a hit, and the HUD's damage direction as a byte angle (256 = a turn).
fn legacy_flinch(ps: &mut PlayerState, dir: Option<[f32; 3]>) {
    ps.damage_count = ps.damage_count.saturating_add(1);
    ps.damage_event = ps.damage_event.wrapping_add(1);
    if let Some(d) = dir.filter(|d| d[0] != 0.0 || d[1] != 0.0) {
        let yaw = game_math::vec_to_yaw(d[0], d[1]);
        ps.damage_yaw = ((yaw * 256.0 / 360.0) as u32) & 255;
    }
}

/// Black Ops II sends one flinch a frame for all of that frame's hits: their share of his max health along the last
/// hit's direction. The same share widens his aim.
fn frame_flinch(world: &mut FrameWorld, id: ClientId, amount: i32, dir: Option<[f32; 3]>) {
    let frame = world.entity_kernel().frame_serial();
    let meta = world.client_meta_mut(id);
    if meta.hit_frame != Some(frame) {
        meta.hit_frame = Some(frame);
        meta.hit_blood = 0;
        meta.hit_sent = None;
    }
    meta.hit_blood = meta.hit_blood.saturating_add(amount.max(0));
    let (taken, already) = (meta.hit_blood, meta.hit_sent);
    let Some(ps) = world.player_mut(id) else {
        return;
    };
    let Some((count, yaw, pitch)) = weapon_t6::view_bob::damage_feedback_bo2(taken, ps.max_health, dir) else {
        return;
    };
    let extra = (count - already.unwrap_or(0)) as f32;
    ps.aim_spread_scale = (ps.aim_spread_scale + extra).min(weapon_t6::recoil::SPREAD_MAX);
    if already.is_none() {
        ps.damage_event = ps.damage_event.wrapping_add(1);
    }
    ps.damage_count = count;
    ps.damage_yaw = yaw;
    ps.damage_pitch = pitch;
    world.client_meta_mut(id).hit_sent = Some(count);
}

/// `g_knockback`, Black Ops II's default.
const KNOCKBACK_SCALE: f32 = 1000.0;
/// pm_flags 0x100 (PMF_TIME_KNOCKBACK): shoved, so the ground's grip is off until pm_time runs out.
const PMF_TIME_KNOCKBACK: u32 = 0x100;
/// On a turret (our e_flags; Black Ops II's 0x300).
const EF_TURRET: u32 = 0xc00;

/// ApplyKnockBack: a hit of `amount` shoves the player along `dir` (a unit vector) by 0.3 of the damage standing, 0.15
/// crouched or 0.02 prone, at most 60, times `g_knockback` / 250 in units a second. Unless `forced` (the hit that kills)
/// nobody in the air or mid-jump is shoved, and nobody on a turret ever is. With no other move timer running the shove
/// lifts the ground's grip for twice its size in ms (50 to 200).
pub(crate) fn knockback(ps: &mut PlayerState, amount: i32, dir: [f32; 3], forced: bool) {
    use ps_game::pm_flags::{CROUCH, JUMPING, PRONE};
    let airborne = ps.pm_flags & JUMPING != 0 || ps.ground_entity_num == ps_game::ENTITYNUM_NONE;
    if airborne && !forced {
        return;
    }
    let share = if ps.pm_flags & PRONE != 0 {
        0.02
    } else if ps.pm_flags & CROUCH != 0 {
        0.15
    } else {
        0.3
    };
    let points = ((amount as f32 * share) as i32).min(60);
    if points == 0 || ps.e_flags & EF_TURRET != 0 {
        return;
    }
    let speed = KNOCKBACK_SCALE * points as f32 * 0.004;
    for (v, d) in ps.velocity.iter_mut().zip(dir) {
        *v += d * speed;
    }
    if ps.pm_time == 0 {
        ps.pm_time = (points * 2).clamp(50, 200);
        ps.pm_flags |= PMF_TIME_KNOCKBACK;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equipment::EquipmentRuntimeFacts;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    fn missile_attempt(impact: bool, blast: bool) -> DamageAttempt {
        DamageAttempt {
            source: DamageSource::Projectile(crate::ProjectileId(1)),
            pellet: crate::PelletId(0),
            attacker: ClientId(0),
            attacker_life: crate::LifeSequence(0),
            target: ClientId(1),
            target_life: crate::LifeSequence(0),
            weapon: 7,
            amount: 50,
            killcam_entity_start_time: 0,
            inflictor_origin: None,
            hitloc: 0,
            blast,
            penetrated: false,
            impact,
        }
    }

    #[test]
    fn a_missile_that_bursts_on_impact_keeps_its_own_means() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let thrown = EquipmentRuntimeFacts {
            projectile_speed: 1000,
            impact_damage: 125,
            ..Default::default()
        };
        let rocket = EquipmentRuntimeFacts {
            proj_impact_explode: true,
            ..thrown
        };
        let mut content = crate::SimContentBuilder::default();
        content.set_equipment_runtime_table(vec![EquipmentRuntimeFacts::default(), thrown, rocket]);
        world.install_content(content.finish());
        let strike = DamageSource::Projectile(crate::ProjectileId(1));
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
        let mut sim = crate::carrier::SimWorld::default();
        let world = sim.frame();
        assert_eq!(attempt_means(&world, &missile_attempt(true, false)), "MOD_IMPACT");
        assert_eq!(attempt_means(&world, &missile_attempt(false, false)), "MOD_PROJECTILE");
        assert_eq!(attempt_means(&world, &missile_attempt(false, true)), "MOD_PROJECTILE_SPLASH");
    }

    #[test]
    fn a_frames_hits_send_one_flinch_sized_by_their_share() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let id = ClientId(0);
        world.client_meta_mut(id).lifecycle = ClientLifecycle::Alive;
        let ps = world.ensure_player(id);
        ps.max_health = 150;
        ps.health = 150;
        ps.aim_spread_scale = 0.0;
        let event = ps.damage_event;
        // Two hits in one frame: one flinch for 45 of 150 health, along the last hit's travel (+y = yaw byte 64).
        world.entity_kernel_mut().begin_frame(50);
        finish_damage(&mut world, id, 15, Some([1.0, 0.0, 0.0]), true);
        finish_damage(&mut world, id, 30, Some([0.0, 1.0, 0.0]), true);
        let ps = world.player(id).unwrap();
        assert_eq!(ps.damage_event, event.wrapping_add(1));
        assert_eq!((ps.damage_count, ps.damage_yaw, ps.damage_pitch), (30, 64, 0));
        assert_eq!(ps.aim_spread_scale, 30.0);
        assert_eq!(ps.health, 105);
        // The next frame's hit flinches on its own; no direction reads 255/255.
        world.entity_kernel_mut().begin_frame(100);
        finish_damage(&mut world, id, 3, None, true);
        let ps = world.player(id).unwrap();
        assert_eq!(ps.damage_event, event.wrapping_add(2));
        assert_eq!((ps.damage_count, ps.damage_yaw, ps.damage_pitch), (2, 255, 255));
        assert_eq!(ps.aim_spread_scale, 32.0);
        // MW2's rules: one small flinch a hit.
        finish_damage(&mut world, id, 3, None, false);
        assert_eq!(world.player(id).unwrap().damage_event, event.wrapping_add(3));
    }

    #[test]
    fn a_hit_shoves_by_its_damage_and_stance() {
        let mut ps = PlayerState::ZERO;
        // Standing, 50 damage: 15 points, 60 units a second, the grip off 50 ms.
        knockback(&mut ps, 50, [1.0, 0.0, 0.0], false);
        assert!(near(ps.velocity, [60.0, 0.0, 0.0]), "{:?}", ps.velocity);
        assert_eq!((ps.pm_time, ps.pm_flags & 0x100), (50, 0x100));
        // Crouched, 1000 damage: held to 60 points; the timer is already running.
        ps.velocity = [0.0; 3];
        ps.pm_flags = ps_game::pm_flags::CROUCH;
        knockback(&mut ps, 1000, [0.0, 1.0, 0.0], false);
        assert!(near(ps.velocity, [0.0, 240.0, 0.0]), "{:?}", ps.velocity);
        assert_eq!((ps.pm_time, ps.pm_flags & 0x100), (50, 0));
        // Prone, 40 damage: under a point, no shove.
        ps.velocity = [0.0; 3];
        ps.pm_flags = ps_game::pm_flags::PRONE;
        knockback(&mut ps, 40, [0.0, 1.0, 0.0], false);
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn only_the_killing_hit_shoves_him_in_the_air() {
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = ps_game::ENTITYNUM_NONE;
        knockback(&mut ps, 100, [0.0, 0.0, 1.0], false);
        assert_eq!(ps.velocity, [0.0; 3]);
        knockback(&mut ps, 100, [0.0, 0.0, 1.0], true);
        assert!(near(ps.velocity, [0.0, 0.0, 120.0]), "{:?}", ps.velocity);
        // Nobody on a turret is shoved.
        let mut ps = PlayerState::ZERO;
        ps.e_flags = 0x400;
        knockback(&mut ps, 100, [1.0, 0.0, 0.0], true);
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn a_zero_vector_has_no_direction() {
        assert_eq!(unit_or_zero([0.0; 3]), [0.0; 3]);
        assert!(near(unit_or_zero([0.0, 3.0, 4.0]), [0.0, 0.6, 0.8]));
    }
}
