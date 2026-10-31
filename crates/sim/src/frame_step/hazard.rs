//! What the world does to players on its own: hard landings hurt, and a player kept under water
//! for too long drowns.

use bevy_ecs::prelude::Resource;

use super::is_alive;
use crate::step_view::StepWorldView;
use crate::client_script::Hit;
use crate::sim_parts::{SeatNo, StepNo};

// BO2 breath under water, worked out after the move: from water level 5 up (the surface at 54
// of the 60 eye height) a player holds their breath for `player_swimTime` (16 s), then loses
// `player_swimDamage` (15) every `player_swimDamagerInterval` (1 s) until they come up.
pub(super) const SWIM_TIME_MS: i32 = 16_000;
pub(super) const SWIM_DAMAGE: i32 = 15;
pub(super) const SWIM_DAMAGE_INTERVAL_MS: i32 = 1_000;
/// Above this water level the player has no air.
pub(super) const BREATH_WATER_LEVEL: i32 = 4;
/// Drowning has no real direction; the hit is reported as coming from above.
const STRAIGHT_DOWN: [f32; 3] = [0.0, 0.0, -1.0];
/// BO2 hands the drowning to the damage code with no armour and no knockback.
const DROWN_DAMAGE_FLAGS: i32 = 2 | 4;

/// Health lost to a landing, as `parm` percent of the player's maximum health (a fall from the
/// full height counts as 110%), worked out in floats and rounded down.
pub(super) fn fall_damage(parm: i32, hit_point_ceiling: i32) -> i32 {
    let fraction = if parm < 100 { parm as f32 * 0.01 } else { 1.1 };
    (hit_point_ceiling as f32 * fraction) as i32
}

/// Hurts a living player for a hard landing. BO2's own scripts receive it as `MOD_FALLING`.
pub(super) fn apply_fall_damage(
    world: &mut StepWorldView,
    tick: StepNo,
    id: SeatNo,
    parm: i32,
    health_cap: i32,
    spot: [f32; 3],
) {
    if !is_alive(world, id) {
        return;
    }
    let amount = fall_damage(parm, health_cap);
    if amount < 1 {
        return;
    }
    let hit = environmental_hit(id, amount, 0, "MOD_FALLING", spot, [0.0; 3]);
    crate::gsc_host::player_damage(world.ecs(), tick, &hit);
}

/// A hit with no attacker, no weapon and no hit location: what the level itself deals out.
fn environmental_hit(
    victim: SeatNo,
    harm: i32,
    flags: i32,
    means: &'static str,
    at: [f32; 3],
    dir: [f32; 3],
) -> Hit {
    let attacker = None;
    Hit {
        victim,
        striker: attacker,
        amount: harm,
        bits: flags,
        means,
        gun: 0,
        point: at,
        way: dir,
        hit_part: 0,
        inflictor: None,
        commit: None,
    }
}

/// When each player last had air (BO2's `outWaterTime`), in level milliseconds.
#[derive(Resource, Default)]
pub(super) struct Breath(Vec<(SeatNo, i32)>);

/// One frame of breath for a player at `waterlevel`; true when they take a little drowning
/// damage now. `last_air` is when they last had air.
pub(super) fn breath_step(last_air: &mut i32, waterlevel: i32, now: i32) -> bool {
    if waterlevel <= BREATH_WATER_LEVEL {
        *last_air = now;
        return false;
    }
    if now - *last_air > SWIM_TIME_MS {
        *last_air += SWIM_DAMAGE_INTERVAL_MS;
        return true;
    }
    false
}

/// Runs everybody's breath for this frame and hurts those who ran out.
pub(super) fn drown_players(world: &mut StepWorldView, tick: StepNo) {
    let now = i32::try_from(tick.0.saturating_mul(crate::SIM_STEP_MILLIS)).unwrap_or(i32::MAX);
    let mut swimmers = Vec::new();
    world.for_each_player_mut(|id, ps| swimmers.push((id, ps.waterlevel, ps.origin)));

    let mut drowning = Vec::new();
    {
        let mut breath = world.ecs().get_resource_or_insert_with(Breath::default);
        breath.0.retain(|(id, _)| swimmers.iter().any(|(swimmer, ..)| swimmer == id));
        for &(id, level, origin) in &swimmers {
            let row = match breath.0.iter().position(|(known, _)| *known == id) {
                Some(row) => row,
                None => {
                    breath.0.push((id, now));
                    breath.0.len() - 1
                }
            };
            if breath_step(&mut breath.0[row].1, level, now) {
                drowning.push((id, origin));
            }
        }
    }

    for (id, origin) in drowning {
        if !is_alive(world, id) {
            continue;
        }
        let hit = environmental_hit(
            id,
            SWIM_DAMAGE,
            DROWN_DAMAGE_FLAGS,
            "MOD_DROWN",
            origin,
            STRAIGHT_DOWN,
        );
        crate::gsc_host::player_damage(world.ecs(), tick, &hit);
    }
}
