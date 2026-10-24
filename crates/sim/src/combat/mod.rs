//! Turning accepted shots into bullets, tracing them through the map and the players, and resolving melee swings.

mod impact;
mod melee;
mod ricochet;
mod trace;

use crate::frame::FrameWorld;
use crate::identities::{LifeSequence, MatchRng, PelletId, ShotId};
use crate::match_state::ClientLifecycle;
use crate::world::{ClientId, Tick};
use weapon_t6::damage::{FireWeaponKind, fire_weapon_kind};

pub(crate) use impact::{hit_event, on_shield, shows_impact};
pub use melee::MELEE_PARM_SHIELD;
pub(crate) use melee::{MeleeSwing, fire_melee_hit};
pub(crate) use ricochet::shield_bounce;
pub(crate) use trace::phase_trace;

/// One shot the weapon brain let through this tick, before it is split into pellets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcceptedShot {
    pub shot_id: ShotId,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,

    pub hand: u8,
    pub weapon: u32,
    pub ammo_used: i32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub ads_frac: f32,

    pub view_height_current: f32,

    pub aim_spread_scale: f32,

    pub perks0: u32,
    pub combat_seed: u32,
    pub owner_velocity: [f32; 3],
    pub spread_degrees: f32,
    /// Scales how deep this shot may penetrate (BO2 brain). Zero leaves the depths as the weapon table has them;
    /// `None` keeps the weapon's own multiplier.
    pub pen_scale: Option<f32>,
}

/// One bullet: a pellet of a shot, or a shot turned aside by a riot shield.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emission {
    pub shot_id: ShotId,
    pub pellet: PelletId,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,
    pub hand: u8,
    pub weapon: u32,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub max_range: f32,
    pub base_damage: i32,
    pub pen_scale: Option<f32>,
    /// A player the bullet flies through: the one whose riot shield it bounced off.
    pub ignore_hit: Option<ClientId>,
}

/// Runs the weapon brain for one client's command and returns the shots it fired. A world without weapon facts (no
/// T6 map) has no gun, and a client that is not alive does not fire.
pub(crate) fn advance_weapon_command(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    cmd: playerstate_t6::game::UserCmd,
    msec: i32,
) -> Vec<AcceptedShot> {
    let armed = !world.gun_facts().is_empty();
    let alive = world
        .client_meta(id)
        .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive);
    if !armed || !alive {
        return Vec::new();
    }
    crate::t6_gun::advance(world, tick, id, &cmd, msec.clamp(1, 200))
}

/// `BO2MP_DAMAGE_LOG` set: log every bullet that reaches a player, with its damage at that range, the multiplier
/// left after what it passed through, and the damage dealt (a test aid).
pub(crate) fn damage_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_DAMAGE_LOG").is_some())
}

/// `BO2MP_SHOT_LOG` set: log every shot's aim (a test aid).
pub(crate) fn shot_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some())
}

/// A pellet's direction: the view's forward, spread inside a cone of `spread_degrees`.
pub fn spread_pellet_direction(angles: [f32; 3], spread_degrees: f32, rng: &mut MatchRng) -> [f32; 3] {
    spread_direction_on_plane(angles, spread_degrees, rng, 1.0)
}

/// The direction of a point picked in a disc on the plane `plane` units ahead of the eye, as the unit vector from the
/// eye to it. Two draws are taken, the radius first and then the angle; no draw is taken without spread.
pub fn spread_direction_on_plane(
    angles: [f32; 3],
    spread_degrees: f32,
    rng: &mut MatchRng,
    plane: f32,
) -> [f32; 3] {
    let (forward, right, up) = game_math::angle_vectors(angles);
    if spread_degrees <= 0.0 {
        return forward;
    }
    let fraction = |draw: u32| draw as f32 / u32::MAX as f32;
    let radius = fraction(rng.next_u32());
    let theta = fraction(rng.next_u32()) * core::f32::consts::TAU;
    let (sin, cos) = (theta.sin(), theta.cos());
    let lateral = plane * spread_degrees.to_radians().tan() * radius;
    let mut aim = [0.0f32; 3];
    for axis in 0..3 {
        aim[axis] = plane * forward[axis] + lateral * (cos * right[axis] + sin * up[axis]);
    }
    let length = (aim[0] * aim[0] + aim[1] * aim[1] + aim[2] * aim[2]).sqrt();
    if length > 0.0 {
        [aim[0] / length, aim[1] / length, aim[2] / length]
    } else {
        forward
    }
}

/// Splits each bullet shot into its pellets, one `Emission` each. Shots that are not bullets, or whose weapon does no
/// damage or has no range, make none.
pub(crate) fn phase_emit(world: &FrameWorld, shots: &[AcceptedShot]) -> Vec<Emission> {
    let mut bullets = Vec::new();
    for shot in shots {
        let Some(facts) = world.combat_facts_for(shot.weapon) else {
            continue;
        };
        let is_bullet = matches!(
            fire_weapon_kind(facts.weap_type, facts.weap_class),
            Some(FireWeaponKind::Bullet)
        );
        if !is_bullet || facts.damage <= 0 || facts.bullet_range() <= 0.0 {
            continue;
        }
        let mut rng = MatchRng::new(u64::from(shot.combat_seed));
        let pellets = facts.pellet_count().clamp(1, i32::from(u16::MAX)) as u16;
        for pellet in 0..pellets {
            bullets.push(Emission {
                shot_id: shot.shot_id,
                pellet: PelletId(pellet),
                attacker: shot.attacker,
                attacker_life: shot.attacker_life,
                hand: shot.hand,
                weapon: shot.weapon,
                origin: shot.origin,
                direction: spread_pellet_direction(shot.angles, shot.spread_degrees, &mut rng),
                max_range: facts.bullet_range(),
                base_damage: facts.damage,
                pen_scale: shot.pen_scale,
                ignore_hit: None,
            });
        }
    }
    bullets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn no_spread_is_the_view_forward_and_takes_no_draw() {
        let angles = [10.0, 70.0, 0.0];
        let mut rng = MatchRng::new(5);
        let mut untouched = MatchRng::new(5);
        let aim = spread_pellet_direction(angles, 0.0, &mut rng);
        assert_eq!(aim, game_math::angle_vectors(angles).0);
        assert_eq!(rng.next_u32(), untouched.next_u32());
    }

    #[test]
    fn spread_stays_inside_the_cone_and_is_a_unit_vector() {
        let angles = [-5.0, 120.0, 0.0];
        let forward = game_math::angle_vectors(angles).0;
        let limit = 4.0f32.to_radians().cos();
        let mut rng = MatchRng::new(99);
        for _ in 0..200 {
            let aim = spread_pellet_direction(angles, 4.0, &mut rng);
            assert!((dot(aim, aim) - 1.0).abs() < 1e-4);
            assert!(dot(aim, forward) >= limit - 1e-4, "{aim:?} outside the cone");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_pellets() {
        let mut first = MatchRng::new(1234);
        let mut second = MatchRng::new(1234);
        for _ in 0..10 {
            assert_eq!(
                spread_pellet_direction([0.0, 0.0, 0.0], 6.0, &mut first),
                spread_pellet_direction([0.0, 0.0, 0.0], 6.0, &mut second)
            );
        }
    }
}
