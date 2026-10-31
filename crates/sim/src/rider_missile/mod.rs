//! A missile its rider steers from the screen (BO2's Hellstorm). The rider's stick turns the nose each input; each tick
//! the missile flies along the nose at the speed [`flight`] picks.

mod flight;

use crate::step_view::StepWorldView;
use crate::player_ledger::PlayerSessionStage;
use crate::sim_parts::{SeatNo, StepNo};
use crate::HappeningBody;
use flight::{PITCH_LIMITS, boost_cue_due, speed_after, turn_rates};
use playerstate_t6::game::{InputCommand, buttons};

/// Turns the nose `angles` by what the stick asks for over `seconds`: the stick's pitch and yaw as signed bytes, at
/// `rates` degrees a second for a full stick. The pitch is wrapped to +-180 and kept within the missile's limits.
fn turned_nose(angles: [f32; 3], stick: [i8; 2], seconds: f32, rates: (f32, f32)) -> [f32; 3] {
    let turn = game_math::euler_to_rows([
        f32::from(stick[0]) / 127.0 * seconds * rates.0,
        f32::from(stick[1]) / 127.0 * seconds * rates.1,
        0.0,
    ]);
    let undo_turn = core::array::from_fn(|row| core::array::from_fn(|col| turn[col][row]));
    let mut nose = game_math::rows_to_euler(game_math::axis_product(
        undo_turn,
        game_math::euler_to_rows(angles),
    ));
    let turns = nose[0] / 360.0;
    nose[0] = (turns - (turns + 0.5).floor()) * 360.0;
    nose[0] = nose[0].clamp(PITCH_LIMITS.0, PITCH_LIMITS.1);
    nose
}

/// One input from `rider`: turns the missile's nose and reads the control and fire buttons.
pub(crate) fn steer(world: &mut StepWorldView, rider: SeatNo, cmd: &InputCommand, msec: i32) {
    let snapshotting = world.emits_state_snapshot();
    if !snapshotting {
        return;
    }
    let Some(mut link) = world
        .client_meta(rider)
        .and_then(|meta| meta.remote_missile)
        .filter(|link| link.unlink_at_ms.is_none() && !link.coasting)
    else {
        return;
    };
    let seconds = msec as f32 * 0.001;
    let stick = [cmd.remote_operated[0] as i8, cmd.remote_operated[1] as i8];
    link.turn = turned_nose(link.turn, stick, seconds, turn_rates(link.t6, cmd.last_input));

    let holding_control = cmd.buttons & buttons::REMOTE_CONTROL != 0;
    link.attack = holding_control && cmd.buttons & buttons::ATTACK != 0;
    if link.t6 {
        // BO2's scripts arm the missile when they link the rider (firing drops its bomblets).
        link.holding = holding_control;
    } else {
        link.armed |= holding_control && !link.attack;
    }
    world.client_meta_mut(rider).remote_missile = Some(link);
}

/// Flies every rider's missile one tick along its nose, and unlinks the riders who died.
pub(crate) fn advance(world: &mut StepWorldView, step: StepNo) {
    if world.emits_state_snapshot() {
        let now = crate::level_time_ms(step);
        for rider in world.ordered_player_ids() {
            fly_one(world, step, now, rider);
        }
    }
}

fn fly_one(world: &mut StepWorldView, step: StepNo, now: i32, rider: SeatNo) {
    let Some(mut link) = world.client_meta(rider).and_then(|meta| meta.remote_missile) else {
        return;
    };
    let rider_alive = world
        .client_meta(rider)
        .is_some_and(|meta| meta.lifecycle == PlayerSessionStage::Alive);
    if !rider_alive {
        world.client_meta_mut(rider).remote_missile = None;
        return;
    }
    if link.unlink_at_ms.is_some() {
        return;
    }
    let Some(missile) = world
        .projectile_in_slot_mut(link.ent_no)
        .filter(|missile| missile.id == link.projectile && missile.in_flight)
    else {
        return;
    };

    let (nose, _, _) = game_math::euler_basis(link.turn);
    let seconds = crate::SIM_STEP_MILLIS as f32 * 0.001;
    let along_nose = missile.motion.iter().zip(nose).map(|(v, n)| v * n).sum::<f32>();
    let before = link;
    let speed = speed_after(&mut link, along_nose, seconds);
    let origin = missile.origin;

    let velocity = crate::ent_links::clipped_shift(nose.map(|axis| axis * speed));
    missile.motion = velocity;
    missile.at = entity_t6::Trajectory {
        began_at: now.saturating_sub(crate::SIM_STEP_MILLIS as i32),
        tr_type: entity_t6::flight::path_kind::STRAIGHT,
        span_ms: Default::default(),
        rate: missile.motion,
        base_point: missile.origin,
    };
    missile.turn_path = crate::ent_links::fire_missile_apos(nose);
    world.client_meta_mut(rider).remote_missile = Some(link);

    if boost_cue_due(&before, &link) {
        let audience = crate::EventRecipients::All;
        world.post_entity_event(
            step,
            audience,
            entity_t6::EventCode::MISSILE_REMOTE_BOOST,
            HappeningBody {
                number: link.ent_no,
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
