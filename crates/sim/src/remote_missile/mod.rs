//! A missile its rider steers from the screen (BO2's Hellstorm). The rider's stick turns the nose each input; each tick
//! the missile flies along the nose at the speed [`flight`] picks.

mod flight;

use crate::frame::FrameWorld;
use crate::match_state::ClientLifecycle;
use crate::world::{ClientId, Tick};
use flight::{PITCH_LIMITS, boost_cue_due, speed_after, turn_rates};
use playerstate_t6::game::{UserCmd, buttons};

/// Turns the nose `angles` by what the stick asks for over `seconds`: the stick's pitch and yaw as signed bytes, at
/// `rates` degrees a second for a full stick. The pitch is wrapped to +-180 and kept within the missile's limits.
fn turned_nose(angles: [f32; 3], stick: [i8; 2], seconds: f32, rates: (f32, f32)) -> [f32; 3] {
    let turn = game_math::angles_to_axis([
        f32::from(stick[0]) / 127.0 * seconds * rates.0,
        f32::from(stick[1]) / 127.0 * seconds * rates.1,
        0.0,
    ]);
    let undo_turn = core::array::from_fn(|row| core::array::from_fn(|col| turn[col][row]));
    let mut nose = game_math::axis_to_angles(game_math::matrix_multiply(
        undo_turn,
        game_math::angles_to_axis(angles),
    ));
    let turns = nose[0] / 360.0;
    nose[0] = (turns - (turns + 0.5).floor()) * 360.0;
    nose[0] = nose[0].clamp(PITCH_LIMITS.0, PITCH_LIMITS.1);
    nose
}

/// One input from `id`'s rider: turns the missile's nose and reads the control and fire buttons.
pub(crate) fn steer(world: &mut FrameWorld, id: ClientId, cmd: &UserCmd, msec: i32) {
    if !world.publishes_snapshot() {
        return;
    }
    let Some(mut link) = world
        .client_meta(id)
        .and_then(|meta| meta.remote_missile)
        .filter(|link| link.unlink_at_ms.is_none() && !link.coasting)
    else {
        return;
    };
    let seconds = msec as f32 * 0.001;
    let stick = [cmd.remote_control[0] as i8, cmd.remote_control[1] as i8];
    link.angles = turned_nose(link.angles, stick, seconds, turn_rates(link.t6, cmd.last_input));

    let holding_control = cmd.buttons & buttons::REMOTE_CONTROL != 0;
    link.attack = holding_control && cmd.buttons & buttons::ATTACK != 0;
    if link.t6 {
        // BO2's scripts arm the missile when they link the rider (firing drops its bomblets).
        link.held = holding_control;
    } else {
        link.armed |= holding_control && !link.attack;
    }
    world.client_meta_mut(id).remote_missile = Some(link);
}

/// Flies every rider's missile one tick along its nose, and unlinks the riders who died.
pub(crate) fn advance(world: &mut FrameWorld, tick: Tick) {
    if !world.publishes_snapshot() {
        return;
    }
    let now = crate::level_time_ms(tick);
    for id in world.client_ids_sorted() {
        fly_one(world, tick, now, id);
    }
}

fn fly_one(world: &mut FrameWorld, tick: Tick, now: i32, id: ClientId) {
    let Some(mut link) = world.client_meta(id).and_then(|meta| meta.remote_missile) else {
        return;
    };
    let rider_alive = world
        .client_meta(id)
        .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive);
    if !rider_alive {
        world.client_meta_mut(id).remote_missile = None;
        return;
    }
    if link.unlink_at_ms.is_some() {
        return;
    }
    let Some(missile) = world
        .projectile_mut_by_number(link.entnum)
        .filter(|missile| missile.id == link.projectile && missile.live)
    else {
        return;
    };

    let (nose, _, _) = game_math::angle_vectors(link.angles);
    let seconds = crate::MATCH_TICK_MS as f32 * 0.001;
    let along_nose = missile.velocity.iter().zip(nose).map(|(v, n)| v * n).sum::<f32>();
    let before = link;
    let speed = speed_after(&mut link, along_nose, seconds);
    let origin = missile.origin;

    let velocity = crate::entity_local::truncated_tr_delta(nose.map(|axis| axis * speed));
    missile.velocity = velocity;
    missile.pos = entity_t6::Trajectory {
        tr_time: now.saturating_sub(crate::MATCH_TICK_MS as i32),
        tr_type: entity_t6::trajectory::tr_type::LINEAR,
        tr_duration: 0,
        tr_delta: velocity,
        tr_base: missile.origin,
    };
    missile.apos = crate::entity_local::fire_missile_apos(nose);
    world.client_meta_mut(id).remote_missile = Some(link);

    if boost_cue_due(&before, &link) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_still_stick_leaves_the_nose_where_it_was() {
        let nose = turned_nose([45.0, 90.0, 0.0], [0, 0], 0.05, (15.0, 20.0));
        for (got, want) in nose.iter().zip([45.0, 90.0, 0.0]) {
            assert!((got - want).abs() < 1e-3, "{nose:?}");
        }
    }

    #[test]
    fn the_nose_never_flattens_past_the_pitch_limits() {
        let down = turned_nose([2.0, 0.0, 0.0], [-127, 0], 1.0, (35.0, 35.0));
        assert!(down[0] >= PITCH_LIMITS.0, "{down:?}");
        let up = turned_nose([86.0, 0.0, 0.0], [127, 0], 1.0, (35.0, 35.0));
        assert!(up[0] <= PITCH_LIMITS.1, "{up:?}");
    }
}
