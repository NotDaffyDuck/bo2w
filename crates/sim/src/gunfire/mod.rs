//! Turning accepted shots into bullets, tracing them through the map and the players, and resolving melee swings.

mod impact;
mod melee;
mod ricochet;
mod trace;

use crate::step_view::StepWorldView;
use crate::match_ids::{LifeCount, MatchRandom, BuckshotId, BulletNo};
use crate::player_ledger::PlayerSessionStage;
use crate::sim_parts::{SeatNo, StepNo};
use std::sync::OnceLock;
use weapon_t6::damage::{ShotOriginKind, fire_weapon_kind};

pub(crate) use impact::{hit_event, on_shield, shows_impact};
pub use melee::MELEE_PARM_SHIELD;
pub(crate) use melee::{MeleeSwing, fire_melee_hit};
pub(crate) use ricochet::shield_bounce;
pub(crate) use trace::trace_shots;

/// One shot the weapon brain let through this tick, before it is split into pellets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedShot {
    pub shot_serial: BulletNo,
    pub attacker: SeatNo,
    pub killer_life: LifeCount,

    pub fist: u8,
    pub gun: u32,
    pub rounds_spent: i32,
    pub spot: [f32; 3],
    pub turn: [f32; 3],
    pub scope_frac: f32,

    pub eye_rise: f32,

    pub sight_spread_factor: f32,

    pub combat_random_seed: u32,
    pub owner_velocity: [f32; 3],
    pub cone_angle_deg: f32,
    /// Scales how deep this shot may penetrate (BO2 brain). Zero leaves the depths as the weapon table has them;
    /// `None` keeps the weapon's own multiplier.
    pub pen_scale: Option<f32>,
}

/// One bullet: a pellet of a shot, or a shot turned aside by a riot shield.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PelletShot {
    pub shot_serial: BulletNo,
    pub pellet: BuckshotId,
    pub attacker: SeatNo,
    pub attacker_life: LifeCount,
    pub fist: u8,
    pub gun: u32,
    pub spot: [f32; 3],
    pub way: [f32; 3],
    pub reach_limit: f32,
    pub raw_hit_harm: i32,
    pub pen_scale: Option<f32>,
    /// A player the bullet flies through: the one whose riot shield it bounced off.
    pub skip_client_hit: Option<SeatNo>,
}

/// Runs the weapon brain for one client's command and returns the shots it fired. A world without weapon facts (no
/// T6 map) has no gun, and a client that is not alive does not fire.
pub(crate) fn progress_weapon_order(
    world: &mut StepWorldView,
    tick: StepNo,
    id: SeatNo,
    cmd: playerstate_t6::game::InputCommand,
    msec: i32,
) -> Vec<ValidatedShot> {
    if world.gun_facts().is_empty() {
        return Vec::new();
    }
    let stage = world.client_meta(id).map(|meta| meta.lifecycle);
    if stage != Some(PlayerSessionStage::Alive) {
        return Vec::new();
    }
    crate::gun_brain::advance(world, tick, id, &cmd, msec.clamp(1, 200))
}

/// `BO2MP_DAMAGE_LOG` set: log every bullet that reaches a player, with its damage at that range, the multiplier
/// left after what it passed through, and the damage dealt (a test aid).
pub(crate) fn damage_log() -> bool {
    static SET: OnceLock<bool> = OnceLock::new();
    env_switch(&SET, "BO2MP_DAMAGE_LOG")
}

/// `BO2MP_SHOT_LOG` set: log every shot's aim (a test aid).
pub(crate) fn shot_log() -> bool {
    static SET: OnceLock<bool> = OnceLock::new();
    env_switch(&SET, "BO2MP_SHOT_LOG")
}

/// Whether environment variable `var` is present, read once and remembered in `cell`.
fn env_switch(cell: &'static OnceLock<bool>, var: &str) -> bool {
    *cell.get_or_init(|| std::env::var_os(var).is_some())
}

/// A pellet's direction: the view's forward, spread inside a cone of `cone_angle_deg`.
pub fn pellet_heading(angles: [f32; 3], cone_angle_deg: f32, rng: &mut MatchRandom) -> [f32; 3] {
    spread_direction_on_plane(angles, cone_angle_deg, rng, 1.0)
}

/// Black Ops II Zombies' flat spread (Bullet_Endpos, a weapon with the accuracy perk: the Blundergat): pellet 0
/// straight ahead, the rest out to either side in turn along the aim's right, never up or down. Pellet i sits
/// `spread * 0.5 / (count / 2) * ((i - 1) / 2 + 1)` cone widths out, right for even i, left for odd.
pub fn flat_pellet_direction(angles: [f32; 3], cone_angle_deg: f32, pellet_no: u16, count: u16) -> [f32; 3] {
    let (ahead, right, _) = game_math::euler_basis(angles);
    if cone_angle_deg <= 0.0 || pellet_no == 0 || count <= 1 {
        return ahead;
    }
    let side = if pellet_no % 2 == 0 { 1.0 } else { -1.0 };
    let step = (i32::from(pellet_no - 1) >> 1) + 1;
    let out = cone_angle_deg * 0.5 / f32::from(count >> 1) * step as f32 * side;
    let lateral = cone_angle_deg.to_radians().tan() * out;
    let q: [f32; 3] = std::array::from_fn(|k| ahead[k] + lateral * right[k]);
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    if len > 0.0 { [q[0] / len, q[1] / len, q[2] / len] } else { ahead }
}

/// The direction of a point picked in a disc on the plane `plane` units ahead of the eye, as the unit vector from the
/// eye to it. Two draws are taken, the radius first and then the angle; no draw is taken without spread.
pub fn spread_direction_on_plane(
    aim: [f32; 3],
    cone_angle_deg: f32,
    rng: &mut MatchRandom,
    plane: f32,
) -> [f32; 3] {
    let (ahead, right, up) = game_math::euler_basis(aim);
    if cone_angle_deg <= 0.0 {
        // No cone: straight ahead, and the random stream is left untouched.
        ahead
    } else {
        offset_in_disc(plane, cone_angle_deg, rng, [ahead, right, up])
    }
}

fn offset_in_disc(
    plane: f32,
    cone_angle_deg: f32,
    rng: &mut MatchRandom,
    basis: [[f32; 3]; 3],
) -> [f32; 3] {
    let [forward, right, up] = basis;
    let fraction = |draw: u32| draw as f32 / u32::MAX as f32;
    let radius = fraction(rng.draw_word());
    let theta = fraction(rng.draw_word()) * core::f32::consts::TAU;
    let (sin, cos) = (theta.sin(), theta.cos());
    let lateral = plane * cone_angle_deg.to_radians().tan() * radius;
    let aim: [f32; 3] = core::array::from_fn(|k| plane * forward[k] + lateral * (cos * right[k] + sin * up[k]));
    let length = (aim[0] * aim[0] + aim[1] * aim[1] + aim[2] * aim[2]).sqrt();
    if length > 0.0 {
        [aim[0] / length, aim[1] / length, aim[2] / length]
    } else {
        forward
    }
}

/// Splits each bullet shot into its pellets, one `PelletShot` each. Shots that are not bullets, or whose weapon has no
/// range, make none. BO2 fires every bullet weapon whatever its damage: a 0-damage one (the Paralyzer, whose script
/// does its harm) still hits the models and triggers it crosses, which take at least 1; players keep their own
/// `scaled > 0` gate in the trace.
pub(crate) fn emit_shot_events(world: &StepWorldView, shots: &[ValidatedShot]) -> Vec<PelletShot> {
    let mut bullets = Vec::new();
    let with_facts = shots.iter().filter_map(|fired| Some((fired, world.damage_profile_of(fired.gun)?)));
    for (fired, stats) in with_facts {
        let is_bullet = matches!(
            fire_weapon_kind(stats.gun_family_code, stats.gun_kind),
            Some(ShotOriginKind::Bullet)
        );
        if !is_bullet || stats.shot_reach() <= 0.0 {
            continue;
        }
        // Each shot's pellets draw from their own stream, seeded by the shot.
        let mut rng = MatchRandom::new(u64::from(fired.combat_random_seed));
        // bo2zm: Black Ops II Zombies' Double Tap Root Beer II fires every bullet twice, shotgun pellets too
        // (Bullet_Fire: PERK_RATEOFFIRE).
        let doubled = if crate::zombie_rt::has_perk(world, fired.attacker, "specialty_rof") { 2 } else { 1 };
        let pellets = (stats.shot_pellet_total() * doubled).clamp(1, i32::from(u16::MAX)) as u16;
        if doubled > 1 && std::env::var_os("BO2MP_T6_WEAPLOG").is_some() {
            diag::info!(Sim, "bo2zm t6 double tap: weapon {} fires {} bullets", fired.gun, pellets);
        }
        let reach = stats.shot_reach();
        bullets.extend((0..pellets).map(|pellet| PelletShot {
            shot_serial: fired.shot_serial,
            pellet: BuckshotId(pellet),
            attacker: fired.attacker,
            attacker_life: fired.killer_life,
            fist: fired.fist,
            gun: fired.gun,
            spot: fired.spot,
            way: if stats.flat_spread {
                flat_pellet_direction(fired.turn, fired.cone_angle_deg, pellet, pellets)
            } else {
                pellet_heading(fired.turn, fired.cone_angle_deg, &mut rng)
            },
            reach_limit: reach,
            raw_hit_harm: stats.damage,
            pen_scale: fired.pen_scale,
            skip_client_hit: None,
        }));
    }
    bullets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner(u: [f32; 3], v: [f32; 3]) -> f32 {
        u.iter().zip(v).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn no_spread_is_the_view_forward_and_takes_no_draw() {
        let angles = [10.0, 70.0, 0.0];
        let mut rng = MatchRandom::new(5);
        let mut untouched = MatchRandom::new(5);
        let aim = pellet_heading(angles, 0.0, &mut rng);
        assert_eq!(aim, game_math::euler_basis(angles).0);
        assert_eq!(rng.draw_word(), untouched.draw_word());
    }

    #[test]
    fn spread_stays_inside_the_cone_and_is_a_unit_vector() {
        let angles = [-5.0, 120.0, 0.0];
        let forward = game_math::euler_basis(angles).0;
        let limit = 4.0f32.to_radians().cos();
        let mut rng = MatchRandom::new(99);
        for _ in 0..200 {
            let aim = pellet_heading(angles, 4.0, &mut rng);
            assert!((inner(aim, aim) - 1.0).abs() < 1e-4);
            assert!(inner(aim, forward) >= limit - 1e-4, "{aim:?} outside the cone");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_pellets() {
        let mut first = MatchRandom::new(1234);
        let mut second = MatchRandom::new(1234);
        for _ in 0..10 {
            assert_eq!(
                pellet_heading([0.0, 0.0, 0.0], 6.0, &mut first),
                pellet_heading([0.0, 0.0, 0.0], 6.0, &mut second)
            );
        }
    }
}
