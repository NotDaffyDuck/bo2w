use crate::frame::FrameWorld;
use crate::match_state::ClientLifecycle;
use crate::world::{ClientId, Tick};

const PITCH_RANGE: [f32; 2] = [1.0, 87.0];
const PITCH_RATE: f32 = 15.0;
const YAW_RATE: f32 = 20.0;
const SPEED_RANGE: [f32; 2] = [3_000.0, 6_000.0];
const SPEED_UP: f32 = 2_000.0;
const SPEED_DOWN: f32 = 500.0;
/// bo2mp: Black Ops II turns a mouse-steered Hellstorm this fast both ways.
const T6_MOUSE_RATE: f32 = 35.0;
/// bo2mp: BO2 counts a boost only when the missile was this much slower than
/// boosted.
const T6_BOOST_FELT: f32 = 500.0;
/// bo2mp: a coasting missile drifts on this slowly (its bomblets are out).
const COAST_SPEED: f32 = 750.0;

/// How fast his view turns the missile, pitch and yaw: BO2 turns a
/// mouse-steered one faster than a pad-steered one; MW2 turns both alike.
fn rates(t6: bool, last_input: u8) -> (f32, f32) {
    if t6 && last_input != playerstate_t6::game::last_input::GAMEPAD {
        (T6_MOUSE_RATE, T6_MOUSE_RATE)
    } else {
        (PITCH_RATE, YAW_RATE)
    }
}

/// The missile's speed along its nose this tick, from `speed`: coasting it
/// drifts; armed, its rider boosts it once to full speed; otherwise it
/// settles back to cruising speed.
fn next_speed(link: &mut crate::RemoteMissile, speed: f32, seconds: f32) -> f32 {
    if link.coasting {
        link.armed = false;
        return COAST_SPEED;
    }
    let boost = if link.t6 {
        link.held && link.armed && !link.boosted
    } else {
        link.armed && !link.boosted && link.attack
    };
    if boost {
        if !link.t6 || SPEED_RANGE[1] - speed > T6_BOOST_FELT {
            link.boosted = true;
        }
        return SPEED_RANGE[1];
    }
    let target = SPEED_RANGE[0];
    if speed < target {
        (speed + SPEED_UP * seconds).min(target)
    } else {
        (speed - SPEED_DOWN * seconds).max(target)
    }
}

/// bo2mp: whether this tick sends Black Ops II's boost event (its 111, to
/// everyone, on the missile): a counted boost, or the missile starting to
/// coast while still armed. MW2 sends none.
fn boost_event(before: &crate::RemoteMissile, after: &crate::RemoteMissile) -> bool {
    after.t6
        && ((!before.boosted && after.boosted)
            || (after.coasting && before.armed && !after.armed))
}

pub(crate) fn steer(
    world: &mut FrameWorld,
    id: ClientId,
    cmd: &playerstate_t6::game::UserCmd,
    msec: i32,
) {
    if !world.publishes_snapshot() {
        return;
    }
    let Some(mut link) = world
        .client_meta(id)
        .and_then(|m| m.remote_missile)
        .filter(|link| link.unlink_at_ms.is_none())
    else {
        return;
    };
    if link.coasting {
        return;
    }
    let seconds = msec as f32 * 0.001;
    let (pitch_rate, yaw_rate) = rates(link.t6, cmd.last_input);
    let delta = game_math::angles_to_axis([
        f32::from(cmd.remote_control[0] as i8) / 127.0 * seconds * pitch_rate,
        f32::from(cmd.remote_control[1] as i8) / 127.0 * seconds * yaw_rate,
        0.0,
    ]);
    let transposed = core::array::from_fn(|row| core::array::from_fn(|col| delta[col][row]));
    let mut angles = game_math::axis_to_angles(game_math::matrix_multiply(
        transposed,
        game_math::angles_to_axis(link.angles),
    ));
    angles[0] = (angles[0] / 360.0 - (angles[0] / 360.0 + 0.5).floor()) * 360.0;
    angles[0] = angles[0].clamp(PITCH_RANGE[0], PITCH_RANGE[1]);
    link.angles = angles;
    let held = cmd.buttons & playerstate_t6::game::buttons::REMOTE_CONTROL != 0;
    link.attack = held && cmd.buttons & playerstate_t6::game::buttons::ATTACK != 0;
    if link.t6 {
        // BO2's scripts arm it when they link him (fire drops its bomblets).
        link.held = held;
    } else {
        link.armed |= held && !link.attack;
    }
    world.client_meta_mut(id).remote_missile = Some(link);
}

pub(crate) fn advance(world: &mut FrameWorld, tick: Tick) {
    if !world.publishes_snapshot() {
        return;
    }
    let now = crate::level_time_ms(tick);
    for id in world.client_ids_sorted() {
        let Some(mut link) = world.client_meta(id).and_then(|m| m.remote_missile) else {
            continue;
        };
        let alive = world
            .client_meta(id)
            .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive);
        if !alive {
            world.client_meta_mut(id).remote_missile = None;
            continue;
        }
        if link.unlink_at_ms.is_some() {
            continue;
        }
        let Some(projectile) = world
            .projectile_mut_by_number(link.entnum)
            .filter(|p| p.id == link.projectile && p.live)
        else {
            continue;
        };
        let (dir, _, _) = game_math::angle_vectors(link.angles);
        let seconds = crate::MATCH_TICK_MS as f32 * 0.001;
        let speed = projectile
            .velocity
            .iter()
            .zip(dir)
            .map(|(v, d)| v * d)
            .sum::<f32>();
        let before = link;
        let speed = next_speed(&mut link, speed, seconds);
        let origin = projectile.origin;
        let velocity =
            crate::entity_local::truncated_tr_delta([dir[0] * speed, dir[1] * speed, dir[2] * speed]);
        projectile.velocity = velocity;
        projectile.pos = entity_t6::Trajectory {
            tr_time: now.saturating_sub(crate::MATCH_TICK_MS as i32),
            tr_type: entity_t6::trajectory::tr_type::LINEAR,
            tr_duration: 0,
            tr_delta: velocity,
            tr_base: projectile.origin,
        };
        projectile.apos = crate::entity_local::fire_missile_apos(dir);
        world.client_meta_mut(id).remote_missile = Some(link);
        if boost_event(&before, &link) {
            world.push_entity_event(
                tick,
                crate::EventAudience::All,
                entity_t6::EntityEventKind::MISSILE_REMOTE_BOOST,
                crate::EntityEventPayload {
                    number: link.entnum,
                    correlation: link.projectile.0,
                    origin,
                    ..Default::default()
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bo2(armed: bool) -> crate::RemoteMissile {
        crate::RemoteMissile { t6: true, held: true, armed, ..Default::default() }
    }

    #[test]
    fn a_mouse_turns_a_bo2_missile_faster() {
        let mouse = playerstate_t6::game::last_input::MOUSECLICK;
        let pad = playerstate_t6::game::last_input::GAMEPAD;
        assert_eq!(rates(true, mouse), (35.0, 35.0));
        assert_eq!(rates(true, pad), (PITCH_RATE, YAW_RATE));
        assert_eq!(rates(false, mouse), (PITCH_RATE, YAW_RATE));
    }

    #[test]
    fn an_armed_bo2_missile_boosts_once_by_itself() {
        let mut link = bo2(true);
        assert_eq!(next_speed(&mut link, 1000.0, 0.05), SPEED_RANGE[1]);
        assert!(link.boosted);
        // Then it settles back toward cruising speed.
        assert_eq!(next_speed(&mut link, 6000.0, 0.05), 5975.0);
        // Not armed, or the control button up: no boost.
        let mut link = bo2(false);
        assert_eq!(next_speed(&mut link, 1000.0, 0.05), 1100.0);
        let mut link = crate::RemoteMissile { held: false, ..bo2(true) };
        assert_eq!(next_speed(&mut link, 1000.0, 0.05), 1100.0);
        assert!(!link.boosted);
    }

    #[test]
    fn a_bo2_boost_near_full_speed_is_not_counted() {
        let mut link = bo2(true);
        assert_eq!(next_speed(&mut link, 5800.0, 0.05), SPEED_RANGE[1]);
        assert!(!link.boosted);
    }

    #[test]
    fn mw2_boosts_on_fire_once_armed() {
        let mut link = crate::RemoteMissile { armed: true, ..Default::default() };
        assert_eq!(next_speed(&mut link, 3000.0, 0.05), 3000.0);
        link.attack = true;
        assert_eq!(next_speed(&mut link, 3000.0, 0.05), SPEED_RANGE[1]);
        assert!(link.boosted);
    }

    #[test]
    fn a_bo2_boost_or_an_armed_coast_sends_the_boost_event() {
        let before = bo2(true);
        let mut link = before;
        next_speed(&mut link, 1000.0, 0.05);
        assert!(boost_event(&before, &link));
        // An uncounted boost near full speed sends none.
        let mut link = before;
        next_speed(&mut link, 5800.0, 0.05);
        assert!(!boost_event(&before, &link));
        // Starting to coast still armed sends it, once.
        let before = crate::RemoteMissile { coasting: true, ..bo2(true) };
        let mut link = before;
        next_speed(&mut link, 3000.0, 0.05);
        assert!(boost_event(&before, &link));
        let again = link;
        next_speed(&mut link, COAST_SPEED, 0.05);
        assert!(!boost_event(&again, &link));
        // MW2's Predator sends none.
        let before = crate::RemoteMissile { armed: true, attack: true, ..Default::default() };
        let mut link = before;
        next_speed(&mut link, 3000.0, 0.05);
        assert!(link.boosted && !boost_event(&before, &link));
    }

    #[test]
    fn a_coasting_missile_drifts_slowly_and_disarms() {
        let mut link = crate::RemoteMissile { coasting: true, ..bo2(true) };
        assert_eq!(next_speed(&mut link, 3000.0, 0.05), COAST_SPEED);
        assert!(!link.armed && !link.boosted);
    }
}
