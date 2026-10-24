//! Carrying a model's animation on from the snapshot's time to the client's.
//! The pose is made from the snapshot, which is 20 Hz, while the model's
//! place glides at the frame rate; without this its body moves in steps.

use xmodel_runtime::DObjSemanticState;

/// The longest the client's clock may be taken to run past the snapshot.
const MAX_AHEAD_S: f32 = 0.1;

fn ahead_secs(clock_ms: i32, snapshot_ms: i32) -> f32 {
    ((clock_ms - snapshot_ms) as f32 / 1000.0).clamp(0.0, MAX_AHEAD_S)
}

/// How far the client's clock is past the newest snapshot, 0 to 0.1 s.
pub(crate) fn anim_ahead_secs(
    clock: Option<&net::FrameClock>,
    presented: Option<&net::PresentedSnapshot>,
) -> f32 {
    match (clock, presented.and_then(|p| p.snapshot())) {
        (Some(clock), Some(snapshot)) if clock.started() => {
            ahead_secs(clock.time(), sim::level_time_ms(snapshot.tick))
        }
        _ => 0.0,
    }
}

/// `state` with every playing clip carried on by `ahead_s`: wrapped when the
/// clip loops (`looping` says), held at its end when not. `None` when
/// nothing would move.
pub(crate) fn advance_script_anim(
    state: &DObjSemanticState,
    ahead_s: f32,
    looping: impl Fn(&str) -> Option<bool>,
) -> Option<DObjSemanticState> {
    let playing = |node: &xanim_t6::XAnimSemanticNode| node.clip.is_some() && node.state.rate != 0.0;
    if ahead_s <= 0.0 || !state.tree.as_ref()?.nodes.iter().any(playing) {
        return None;
    }
    let mut advanced = state.clone();
    for node in advanced.tree.as_mut()?.nodes.iter_mut().filter(|node| playing(node)) {
        let wraps = node.clip.as_deref().and_then(&looping).unwrap_or(false);
        let time = node.state.time + node.state.rate * ahead_s;
        node.state.time = if wraps { time.rem_euclid(1.0) } else { time.min(1.0) };
        node.state.old_time = node.state.time;
    }
    Some(advanced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(time: f32) -> DObjSemanticState {
        DObjSemanticState::one_leaf("zombie".to_owned(), "walk".to_owned(), 1, 1, time)
    }

    fn time_of(state: &DObjSemanticState) -> f32 {
        state.tree.as_ref().expect("tree").nodes[0].state.time
    }

    #[test]
    fn the_clock_may_be_ahead_by_a_tenth_of_a_second_no_more() {
        assert_eq!(ahead_secs(1000, 1050), 0.0);
        assert!((ahead_secs(1050, 1000) - 0.05).abs() < 1e-6);
        assert_eq!(ahead_secs(1500, 1000), 0.1);
    }

    #[test]
    fn nothing_is_ahead_without_a_clock() {
        assert_eq!(anim_ahead_secs(None, None), 0.0);
    }

    #[test]
    fn a_clip_is_carried_on_by_its_rate() {
        let moved = advance_script_anim(&playing(0.25), 0.1, |_| Some(true)).expect("moves");
        assert!((time_of(&moved) - 0.35).abs() < 1e-6);
        assert_eq!(moved.tree.as_ref().unwrap().nodes[0].state.old_time, time_of(&moved));
    }

    #[test]
    fn a_looping_clip_wraps_and_a_one_shot_stops_at_its_end() {
        let wrapped = advance_script_anim(&playing(0.95), 0.1, |_| Some(true)).expect("moves");
        assert!((time_of(&wrapped) - 0.05).abs() < 1e-5);
        let held = advance_script_anim(&playing(0.95), 0.1, |_| Some(false)).expect("moves");
        assert_eq!(time_of(&held), 1.0);
        let unknown = advance_script_anim(&playing(0.95), 0.1, |_| None).expect("moves");
        assert_eq!(time_of(&unknown), 1.0);
    }

    #[test]
    fn nothing_moves_without_time_a_tree_or_a_rate() {
        assert!(advance_script_anim(&playing(0.5), 0.0, |_| Some(true)).is_none());
        let bind = DObjSemanticState::bind_pose("zombie".to_owned(), 1, 1);
        assert!(advance_script_anim(&bind, 0.1, |_| Some(true)).is_none());
        let mut still = playing(0.5);
        still.tree.as_mut().unwrap().nodes[0].state.rate = 0.0;
        assert!(advance_script_anim(&still, 0.1, |_| Some(true)).is_none());
    }
}
