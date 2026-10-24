//! The ledge climb's shared half: the tables that say how long a climb takes
//! and how far its root track has carried the player, and the move that runs
//! every frame of it. Finding a ledge and starting the climb belong to
//! `crate::mantle`; the player's pose and events come from the same tables.

use game_math::{angle_normalize_360, angle_subtract};
use playerstate_t6::game::mantle_flags::{
    ACTIVE, BO2, BO2_DURATION_SHIFT, BO2_WEAPON_ANIM, BO2_WEAPON_UP, FAST_MANTLE, OVER, QUICK,
};
use playerstate_t6::game::{PlayerState, pm_flags};
use trace_t6::Trace;

use crate::bridge::{Bo2Feel, add_predictable_event};

/// Content bit of the surfaces a climb can start from.
pub const CONTENTS_MANTLE: u32 = 0x0100_0000;

/// Surface flags of a face that can be climbed onto or over.
pub const SURF_MANTLE_ON_OR_OVER: u32 = 0x0600_0000;

/// The part of those flags that means "over".
pub const SURF_MANTLE_OVER: u32 = 0x0400_0000;

/// The fast-climb perk (specialty_fastmantle).
pub const PERK_FASTMANTLE: u32 = 0x8_0000;

/// Entity flag that stays up while a climb is under way.
const EF_MANTLE: u32 = 0x1_0000;

/// Player event: the climb has no room to stand, so the player crouches.
const EV_MANTLE_CROUCH: i32 = 7;
/// Player event: the climb ends with room to stand, so the player stands up.
const EV_MANTLE_STAND: i32 = 6;

/// Climb flag: crouch through the climb (sends `EV_MANTLE_CROUCH` each frame).
const FORCE_CROUCH: i32 = 1 << 1;
/// Climb flag: stand at the end (sends `EV_MANTLE_STAND` once).
const FORCE_STAND: i32 = 1 << 2;

/// How high above the feet each ledge probe looks, tallest first.
pub const MANTLE_LEDGE_HEIGHTS: [f32; 3] = [60.0, 40.0, 20.0];

/// How far past the wall the ledge probe reaches.
pub const MANTLE_FORWARD_DIST: f32 = 16.0;

/// The ledge top is searched down to this height above the feet.
pub const MANTLE_LEDGE_FLOOR_Z: f32 = 18.0;

/// Height of the box that has to fit at the top of a ledge.
pub const MANTLE_CLEARANCE_MAXS_Z: f32 = 50.0;

/// Half the width of the player's box for every climb sweep.
pub const MANTLE_HALF_WIDTH: f32 = 15.0;

/// How far a vault carries the player past the ledge.
pub const MANTLE_OVER_FORWARD: f32 = 31.0;

/// How many animations the climb tree holds.
pub const MANTLE_XANIM_TREE_SIZE: usize = 11;

/// What a climb found: which way the wall faces, where it began, and the
/// ledge and landing it reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MantleResults {
    pub dir: [f32; 3],
    pub start_pos: [f32; 3],
    pub ledge_pos: [f32; 3],
    pub end_pos: [f32; 3],
    pub flags: i32,
}

/// A box sweep through the world, which is how a climb looks for its ledge.
pub trait MantleCapsuleTrace {
    fn trace(
        &mut self,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        contentmask: u32,
    ) -> Trace;
}

/// Ledge heights the climb animations were made for, one per row.
const TRANS_HEIGHTS: [f32; 7] = [57.0, 51.0, 45.0, 39.0, 33.0, 27.0, 21.0];
/// The climb-up animation of each row.
const TRANS_UP_ANIM: [i32; 7] = [1, 2, 3, 4, 5, 6, 7];
/// The vault animation of each row.
const TRANS_OVER_ANIM: [i32; 7] = [8, 8, 9, 9, 9, 10, 10];

/// The table row whose height is closest to the ledge's rise; on a tie the
/// taller row wins.
pub fn find_transition(cur_height: f32, goal_height: f32) -> i32 {
    let rise = goal_height - cur_height;
    let mut best = 0;
    let mut best_gap = libm::fabsf(TRANS_HEIGHTS[0] - rise);
    for (row, &height) in TRANS_HEIGHTS.iter().enumerate().skip(1) {
        let gap = libm::fabsf(height - rise);
        if best_gap > gap {
            best = row;
            best_gap = gap;
        }
    }
    best as i32
}

/// A row index from the player state; anything off the table reads row 0.
fn trans_row(trans_index: i32) -> usize {
    usize::try_from(trans_index)
        .ok()
        .filter(|&row| row < TRANS_HEIGHTS.len())
        .unwrap_or(0)
}

/// The climb-up animation for a table row.
pub fn trans_up_anim(trans_index: i32) -> i32 {
    TRANS_UP_ANIM[trans_row(trans_index)]
}

/// The vault animation for a table row.
pub fn trans_over_anim(trans_index: i32) -> i32 {
    TRANS_OVER_ANIM[trans_row(trans_index)]
}

/// How long each climb animation runs.
pub trait MantleXAnimLength {
    /// Length in milliseconds of animation `anim_index` (1 to 10), from the
    /// fast-climb tree or the normal one.
    fn length_msec(&self, fast_mantle: bool, anim_index: i32) -> i32;
}

/// Every climb-up lasts `up_msec` and every vault `over_msec`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatMantleAnimLength {
    pub up_msec: i32,
    pub over_msec: i32,
}

impl Default for FlatMantleAnimLength {
    fn default() -> Self {
        Self {
            up_msec: 400,
            over_msec: 200,
        }
    }
}

impl MantleXAnimLength for FlatMantleAnimLength {
    fn length_msec(&self, _fast_mantle: bool, anim_index: i32) -> i32 {
        if (1..=7).contains(&anim_index) {
            self.up_msec
        } else {
            self.over_msec
        }
    }
}

/// Where an animation's root has travelled from its start.
pub trait MantleRootDelta {
    /// The root's offset after `frac` (0 to 1) of animation `anim_index`.
    fn abs_delta(&self, fast_mantle: bool, anim_index: i32, frac: f32) -> [f32; 3];
}

/// A root that never moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct ZeroMantleRootDelta;

impl MantleRootDelta for ZeroMantleRootDelta {
    fn abs_delta(&self, _fast_mantle: bool, _anim_index: i32, _frac: f32) -> [f32; 3] {
        [0.0; 3]
    }
}

/// A root that travels in a straight line to where its animation ends.
#[derive(Clone, Copy, Debug, Default)]
pub struct CreateAnimsMantleRootDelta;

/// Where animation `anim_index` leaves the root: up and forward for a
/// climb-up, forward and down for a vault.
fn end_offset(anim_index: i32) -> [f32; 3] {
    match anim_index {
        1..=7 => [
            MANTLE_FORWARD_DIST,
            0.0,
            TRANS_HEIGHTS[anim_index as usize - 1],
        ],
        8..=10 => [MANTLE_OVER_FORWARD, 0.0, -MANTLE_LEDGE_FLOOR_Z],
        _ => [0.0; 3],
    }
}

impl MantleRootDelta for CreateAnimsMantleRootDelta {
    fn abs_delta(&self, _fast_mantle: bool, anim_index: i32, frac: f32) -> [f32; 3] {
        let end = end_offset(anim_index);
        let t = frac.clamp(0.0, 1.0);
        [end[0] * t, end[1] * t, end[2] * t]
    }
}

/// The climb uses the fast-climb tree.
fn fast_tree(flags: i32) -> bool {
    flags & FAST_MANTLE != 0
}

/// Length of the climb-up part of the climb in progress.
pub fn up_length(ps: &PlayerState, lengths: &impl MantleXAnimLength) -> i32 {
    let anim = trans_up_anim(ps.mantle_state.trans_index);
    lengths.length_msec(fast_tree(ps.mantle_state.flags), anim)
}

/// Length of the vault part; 0 when the climb does not go over.
pub fn over_length(ps: &PlayerState, lengths: &impl MantleXAnimLength) -> i32 {
    if ps.mantle_state.flags & OVER == 0 {
        return 0;
    }
    let anim = trans_over_anim(ps.mantle_state.trans_index);
    lengths.length_msec(fast_tree(ps.mantle_state.flags), anim)
}

/// Length of the whole climb.
pub fn duration(ps: &PlayerState, lengths: &impl MantleXAnimLength) -> i32 {
    up_length(ps, lengths).saturating_add(over_length(ps, lengths))
}

const DEG_TO_RAD: f32 = 0.017_453_292;

/// Turn the horizontal part of `v` by `yaw_deg` degrees.
fn turn_by_yaw(v: &mut [f32; 3], yaw_deg: f32) {
    let yaw = yaw_deg * DEG_TO_RAD;
    let (cos, sin) = (libm::cosf(yaw), libm::sinf(yaw));
    let (x, y) = (v[0], v[1]);
    v[1] = cos * y + x * sin;
    v[0] = cos * x - sin * y;
}

/// How far the root track has carried the player `time_ms` into the climb,
/// turned to face the wall. Past the climb-up, the vault adds to the whole of it.
pub fn sample_root_track(
    ps: &PlayerState,
    time_ms: i32,
    lengths: &impl MantleXAnimLength,
    root: &impl MantleRootDelta,
) -> [f32; 3] {
    let up_len = up_length(ps, lengths).max(1);
    let over_len = over_length(ps, lengths);
    let fast = fast_tree(ps.mantle_state.flags);
    let up_anim = trans_up_anim(ps.mantle_state.trans_index);
    let mut delta = if time_ms > up_len && over_len > 0 {
        let climbed = root.abs_delta(fast, up_anim, 1.0);
        let frac = (time_ms - up_len) as f32 / over_len as f32;
        let over_anim = trans_over_anim(ps.mantle_state.trans_index);
        let vaulted = root.abs_delta(fast, over_anim, frac);
        [
            climbed[0] + vaulted[0],
            climbed[1] + vaulted[1],
            climbed[2] + vaulted[2],
        ]
    } else {
        root.abs_delta(fast, up_anim, time_ms as f32 / up_len as f32)
    };
    turn_by_yaw(&mut delta, ps.mantle_state.yaw);
    delta
}

/// True while a climb has the gun out of hand: a marked climb says so with
/// its own bit, an unmarked one only keeps the gun for the last vault animation.
pub fn is_weapon_inactive(ps: &PlayerState, mantle_enable: bool) -> bool {
    if !mantle_enable || ps.pm_flags & pm_flags::MANTLE == 0 {
        return false;
    }
    if ps.mantle_state.flags & BO2 != 0 {
        return ps.mantle_state.flags & BO2_WEAPON_UP == 0;
    }
    trans_over_anim(ps.mantle_state.trans_index) != 10
}

/// Mark a climb that has just started for the gun. A climb whose table
/// height is at most `mantle_weapon_height` keeps the gun up; one taller
/// than `mantle_weapon_anim_height` among those also plays the gun's climb
/// camera. The climb's length rides along in the high bits for the client.
pub fn mark_bo2_weapon(ps: &mut PlayerState, feel: Bo2Feel, lengths: &impl MantleXAnimLength) {
    let height = TRANS_HEIGHTS[trans_row(ps.mantle_state.trans_index)];
    let mut marks = BO2;
    if height <= feel.mantle_weapon_height {
        marks |= BO2_WEAPON_UP;
        if height > feel.mantle_weapon_anim_height {
            marks |= BO2_WEAPON_ANIM;
        }
    }
    let length_ms = duration(ps, lengths).clamp(0, 0xffff);
    let low_bits = (1 << BO2_DURATION_SHIFT) - 1;
    let kept = ps.mantle_state.flags & low_bits & !(BO2 | BO2_WEAPON_UP | BO2_WEAPON_ANIM);
    ps.mantle_state.flags = kept | marks | (length_ms << BO2_DURATION_SHIFT);
}

/// Take down the "a climb is possible here" hint at the start of a frame.
pub fn clear_hint(ps: &mut PlayerState) {
    ps.mantle_state.flags &= !(ACTIVE | QUICK);
}

/// How much the view may turn away from the wall during a climb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MantleCapViewContext {
    pub mantle_enable: bool,
    /// Degrees either side of the wall's direction.
    pub view_yawcap: f32,
}

impl Default for MantleCapViewContext {
    fn default() -> Self {
        Self {
            mantle_enable: true,
            view_yawcap: 60.0,
        }
    }
}

/// Hold the view within `view_yawcap` degrees of the wall during a climb.
pub fn cap_view(ps: &mut PlayerState, context: MantleCapViewContext) {
    debug_assert!(
        ps.pm_flags & pm_flags::MANTLE != 0,
        "capping the view needs a climb"
    );
    if !context.mantle_enable {
        return;
    }
    let cap = context.view_yawcap;
    let low = -cap;
    let mut delta = angle_subtract(ps.mantle_state.yaw, ps.viewangles[1]);
    if !(delta < low || cap < delta) {
        return;
    }
    while low > delta {
        delta += cap;
    }
    while delta > cap {
        delta -= cap;
    }
    let edge = if delta <= 0.0 { cap } else { low };
    ps.delta_angles[1] += delta;
    ps.viewangles[1] = angle_normalize_360(ps.mantle_state.yaw + edge);
}

/// How a climb's move ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MantleMoveContext {
    pub mantle_enable: bool,
    /// The player stops dead at the top of a climb onto a ledge (a vault over
    /// one keeps its speed, which is not capped).
    pub stop_on_ledge: bool,
}

impl Default for MantleMoveContext {
    fn default() -> Self {
        Self {
            mantle_enable: true,
            stop_on_ledge: false,
        }
    }
}

/// Run `msec` of the climb: step the timer, carry the player along the root
/// track by what it travelled, set the speed that matches, and at the end
/// leave the climb.
pub fn advance(
    ps: &mut PlayerState,
    msec: i32,
    context: MantleMoveContext,
    lengths: &impl MantleXAnimLength,
    root: &impl MantleRootDelta,
) {
    debug_assert!(
        ps.pm_flags & pm_flags::MANTLE != 0,
        "moving a climb needs a climb"
    );
    if !context.mantle_enable {
        return;
    }

    ps.mantle_state.flags &= !ACTIVE;
    if ps.mantle_state.flags & FORCE_CROUCH != 0 {
        add_predictable_event(ps, EV_MANTLE_CROUCH, 0);
    }

    let total = duration(ps, lengths).max(0);
    let before = ps.mantle_state.timer;
    ps.mantle_state.timer = before.saturating_add(msec).min(total);

    let from = sample_root_track(ps, before, lengths, root);
    let to = sample_root_track(ps, ps.mantle_state.timer, lengths, root);
    let step = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    for (origin, moved) in ps.origin.iter_mut().zip(step) {
        *origin += moved;
    }
    let elapsed = (ps.mantle_state.timer - before) as f32;
    if elapsed > 0.0 {
        let per_second = 1.0 / (elapsed * 0.001);
        for (velocity, moved) in ps.velocity.iter_mut().zip(step) {
            *velocity = moved * per_second;
        }
    }

    if ps.mantle_state.timer < total {
        return;
    }

    ps.pm_flags &= !pm_flags::MANTLE;
    if ps.mantle_state.flags & FORCE_STAND != 0 {
        add_predictable_event(ps, EV_MANTLE_STAND, 0);
        ps.e_flags &= !EF_MANTLE;
    }
    if context.stop_on_ledge && ps.mantle_state.flags & OVER == 0 {
        ps.velocity = [0.0; 3];
    }
    ps.mantle_state.flags |= QUICK;
}

#[cfg(test)]
mod tests {
    use super::*;
    use playerstate_t6::game::mantle_flags;

    const FEEL: Bo2Feel = Bo2Feel {
        mantle_weapon_height: 39.0,
        mantle_weapon_anim_height: 32.0,
        ..Bo2Feel::BO2_MP
    };

    /// Ten units forward and forty up over a whole animation.
    struct Rising;

    impl MantleRootDelta for Rising {
        fn abs_delta(&self, _fast_mantle: bool, _anim_index: i32, frac: f32) -> [f32; 3] {
            [10.0 * frac, 0.0, 40.0 * frac]
        }
    }

    /// A tree whose fast climb is shorter.
    struct FastIsShort;

    impl MantleXAnimLength for FastIsShort {
        fn length_msec(&self, fast_mantle: bool, _anim_index: i32) -> i32 {
            if fast_mantle { 250 } else { 500 }
        }
    }

    fn climbing(trans_index: i32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags |= pm_flags::MANTLE;
        ps.mantle_state.trans_index = trans_index;
        ps
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn the_nearest_table_height_picks_the_row() {
        assert_eq!(find_transition(0.0, 57.0), 0);
        assert_eq!(find_transition(0.0, 100.0), 0);
        assert_eq!(
            find_transition(10.0, 39.0),
            5,
            "a 29 unit rise is nearest 27"
        );
        assert_eq!(
            find_transition(10.0, 40.0),
            4,
            "a 30 unit rise is as near 33 as 27 and the taller row wins"
        );
        assert_eq!(find_transition(0.0, 21.0), 6);
        assert_eq!(find_transition(0.0, 0.0), 6);
        assert_eq!(
            find_transition(0.0, 54.0),
            0,
            "halfway between two rows: the taller one"
        );
    }

    #[test]
    fn each_row_has_its_up_and_over_animation() {
        let up: [i32; 7] = core::array::from_fn(|row| trans_up_anim(row as i32));
        let over: [i32; 7] = core::array::from_fn(|row| trans_over_anim(row as i32));
        assert_eq!(up, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(over, [8, 8, 9, 9, 9, 10, 10]);
    }

    #[test]
    fn a_row_off_the_table_reads_the_first() {
        for off in [-1, 7, 100] {
            assert_eq!(trans_up_anim(off), 1, "row {off}");
            assert_eq!(trans_over_anim(off), 8, "row {off}");
        }
    }

    #[test]
    fn the_climb_lasts_as_long_as_its_parts() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        assert_eq!(
            (up_length(&ps, &lengths), over_length(&ps, &lengths)),
            (400, 0)
        );
        assert_eq!(duration(&ps, &lengths), 400);
        ps.mantle_state.flags |= OVER;
        assert_eq!(over_length(&ps, &lengths), 200);
        assert_eq!(duration(&ps, &lengths), 600);
    }

    #[test]
    fn the_fast_climb_perk_reads_the_fast_tree() {
        let mut ps = climbing(0);
        assert_eq!(duration(&ps, &FastIsShort), 500);
        ps.mantle_state.flags |= FAST_MANTLE;
        assert_eq!(duration(&ps, &FastIsShort), 250);
    }

    #[test]
    fn the_flat_lengths_split_up_from_over() {
        let lengths = FlatMantleAnimLength {
            up_msec: 300,
            over_msec: 120,
        };
        assert_eq!(lengths.length_msec(false, 1), 300);
        assert_eq!(lengths.length_msec(true, 7), 300);
        assert_eq!(lengths.length_msec(false, 8), 120);
        assert_eq!(lengths.length_msec(false, 10), 120);
    }

    #[test]
    fn the_root_track_grows_along_the_climb_then_the_vault() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        assert!(near(
            sample_root_track(&ps, 200, &lengths, &Rising),
            [5.0, 0.0, 20.0]
        ));
        // No vault: the climb-up keeps counting past its end.
        assert!(near(
            sample_root_track(&ps, 600, &lengths, &Rising),
            [15.0, 0.0, 60.0]
        ));

        ps.mantle_state.flags |= OVER;
        assert!(near(
            sample_root_track(&ps, 400, &lengths, &Rising),
            [10.0, 0.0, 40.0]
        ));
        assert!(near(
            sample_root_track(&ps, 500, &lengths, &Rising),
            [15.0, 0.0, 60.0]
        ));
        assert!(near(
            sample_root_track(&ps, 600, &lengths, &Rising),
            [20.0, 0.0, 80.0]
        ));
    }

    #[test]
    fn the_root_track_turns_to_face_the_wall() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        ps.mantle_state.yaw = 90.0;
        let delta = sample_root_track(&ps, 400, &lengths, &Rising);
        assert!(near(delta, [0.0, 10.0, 40.0]), "{delta:?}");
        ps.mantle_state.yaw = 180.0;
        let delta = sample_root_track(&ps, 400, &lengths, &Rising);
        assert!(near(delta, [-10.0, 0.0, 40.0]), "{delta:?}");
    }

    #[test]
    fn a_straight_line_root_ends_where_its_animation_does() {
        let root = CreateAnimsMantleRootDelta;
        assert_eq!(
            root.abs_delta(false, 1, 1.0),
            [MANTLE_FORWARD_DIST, 0.0, 57.0]
        );
        assert_eq!(root.abs_delta(false, 4, 0.5), [8.0, 0.0, 19.5]);
        assert_eq!(
            root.abs_delta(true, 8, 1.0),
            [MANTLE_OVER_FORWARD, 0.0, -MANTLE_LEDGE_FLOOR_Z]
        );
        assert_eq!(
            root.abs_delta(false, 7, 3.0),
            root.abs_delta(false, 7, 1.0),
            "frac stops at 1"
        );
        assert_eq!(root.abs_delta(false, 7, -1.0), [0.0; 3], "and at 0");
        assert_eq!(root.abs_delta(false, 0, 1.0), [0.0; 3], "no such animation");
        assert_eq!(ZeroMantleRootDelta.abs_delta(false, 3, 1.0), [0.0; 3]);
    }

    #[test]
    fn an_advance_moves_the_player_by_what_the_root_travelled() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        ps.origin = [100.0, 200.0, 300.0];
        ps.mantle_state.flags |= ACTIVE;
        advance(
            &mut ps,
            100,
            MantleMoveContext::default(),
            &lengths,
            &Rising,
        );
        assert_eq!(ps.mantle_state.timer, 100);
        assert!(near(ps.origin, [102.5, 200.0, 310.0]), "{:?}", ps.origin);
        assert!(near(ps.velocity, [25.0, 0.0, 100.0]), "{:?}", ps.velocity);
        assert_ne!(ps.pm_flags & pm_flags::MANTLE, 0, "still climbing");
        assert_eq!(ps.mantle_state.flags & ACTIVE, 0, "the hint is down");
    }

    #[test]
    fn the_last_frame_is_cut_short_and_ends_the_climb() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        ps.mantle_state.timer = 390;
        advance(&mut ps, 50, MantleMoveContext::default(), &lengths, &Rising);
        assert_eq!(ps.mantle_state.timer, 400);
        assert_eq!(ps.pm_flags & pm_flags::MANTLE, 0);
        assert_ne!(ps.mantle_state.flags & QUICK, 0);
        // 10 ms of a 400 ms climb carried the player 0.25 forward and 1 up.
        assert!(near(ps.velocity, [25.0, 0.0, 100.0]), "{:?}", ps.velocity);
    }

    #[test]
    fn a_disabled_climb_does_nothing() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        let before = ps;
        let context = MantleMoveContext {
            mantle_enable: false,
            ..MantleMoveContext::default()
        };
        advance(&mut ps, 50, context, &lengths, &Rising);
        assert_eq!(ps.mantle_state.timer, before.mantle_state.timer);
        assert_eq!(ps.origin, before.origin);
    }

    #[test]
    fn a_crouching_climb_sends_its_event_every_frame_and_a_standing_one_at_the_end() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        ps.e_flags |= EF_MANTLE;
        ps.mantle_state.flags |= FORCE_CROUCH | FORCE_STAND;
        advance(
            &mut ps,
            100,
            MantleMoveContext::default(),
            &lengths,
            &Rising,
        );
        assert_eq!(ps.predictable_event_sequence, 1);
        assert_eq!(ps.predictable_events[0], EV_MANTLE_CROUCH);
        assert_ne!(ps.e_flags & EF_MANTLE, 0);

        advance(
            &mut ps,
            1000,
            MantleMoveContext::default(),
            &lengths,
            &Rising,
        );
        assert_eq!(ps.predictable_event_sequence, 3);
        assert_eq!(ps.predictable_events[1], EV_MANTLE_CROUCH);
        assert_eq!(ps.predictable_events[2], EV_MANTLE_STAND);
        assert_eq!(ps.e_flags & EF_MANTLE, 0);
    }

    #[test]
    fn a_climb_with_no_events_flagged_sends_none() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        advance(
            &mut ps,
            1000,
            MantleMoveContext::default(),
            &lengths,
            &Rising,
        );
        assert_eq!(ps.predictable_event_sequence, 0);
    }

    fn end_velocity(context: MantleMoveContext, over: bool) -> [f32; 3] {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(2);
        if over {
            ps.mantle_state.flags |= OVER;
        }
        ps.mantle_state.timer = duration(&ps, &lengths) - 10;
        advance(&mut ps, 50, context, &lengths, &Rising);
        assert_eq!(ps.pm_flags & pm_flags::MANTLE, 0, "the climb ends");
        ps.velocity
    }

    #[test]
    fn multiplayer_stops_dead_on_a_ledge_but_not_after_a_vault() {
        let mp = MantleMoveContext {
            stop_on_ledge: true,
            ..MantleMoveContext::default()
        };
        assert_eq!(end_velocity(mp, false), [0.0; 3], "onto a ledge: stopped");
        let vault = end_velocity(mp, true);
        assert!(
            vault[0] > 0.0 && vault[2] > 0.0,
            "over a ledge: keeps moving"
        );
        let zombies = end_velocity(MantleMoveContext::default(), false);
        assert!(
            zombies[0] > 0.0 && zombies[2] > 0.0,
            "the other mode keeps the climb's speed"
        );
    }

    #[test]
    fn low_climbs_keep_the_gun_and_high_ones_lower_it() {
        let lengths = FlatMantleAnimLength::default();
        // Table heights 57, 51, 45, 39, 33, 27, 21.
        let expect = [
            (0, false, false),
            (2, false, false),
            (3, true, true),
            (4, true, true),
            (5, true, false),
            (6, true, false),
        ];
        for (row, up, anim) in expect {
            let mut ps = climbing(row);
            mark_bo2_weapon(&mut ps, FEEL, &lengths);
            assert_ne!(ps.mantle_state.flags & BO2, 0, "row {row}");
            assert_eq!(ps.mantle_state.flags & BO2_WEAPON_UP != 0, up, "row {row}");
            assert_eq!(
                ps.mantle_state.flags & BO2_WEAPON_ANIM != 0,
                anim,
                "row {row}"
            );
            assert_eq!(is_weapon_inactive(&ps, true), !up, "row {row}");
            assert_eq!(mantle_flags::bo2_weapon_up(&ps), up, "row {row}");
        }
    }

    #[test]
    fn the_climb_length_rides_along_and_progress_counts_up() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(4);
        ps.mantle_state.flags |= OVER;
        mark_bo2_weapon(&mut ps, FEEL, &lengths);
        assert_ne!(
            ps.mantle_state.flags & OVER,
            0,
            "the vault bit survives the marking"
        );
        assert_eq!(mantle_flags::bo2_duration_ms(ps.mantle_state.flags), 600);
        ps.mantle_state.timer = 150;
        assert_eq!(mantle_flags::bo2_progress(&ps), Some(0.25));
        ps.pm_flags &= !pm_flags::MANTLE;
        assert_eq!(mantle_flags::bo2_progress(&ps), None);
        assert!(!mantle_flags::bo2_weapon_up(&ps));
    }

    #[test]
    fn marking_twice_leaves_one_mark() {
        let lengths = FlatMantleAnimLength::default();
        let mut ps = climbing(3);
        mark_bo2_weapon(&mut ps, FEEL, &lengths);
        let once = ps.mantle_state.flags;
        mark_bo2_weapon(&mut ps, FEEL, &lengths);
        assert_eq!(ps.mantle_state.flags, once);
    }

    #[test]
    fn without_the_mark_only_the_last_vault_animation_keeps_the_gun() {
        for (row, inactive) in [(0, true), (4, true), (5, false), (6, false)] {
            let ps = climbing(row);
            assert_eq!(is_weapon_inactive(&ps, true), inactive, "row {row}");
        }
        let still = PlayerState::ZERO;
        assert!(!is_weapon_inactive(&still, true), "not climbing");
        assert!(
            !is_weapon_inactive(&climbing(0), false),
            "climbing disabled"
        );
    }

    #[test]
    fn clearing_the_hint_drops_the_hint_and_quick_bits_only() {
        let mut ps = climbing(1);
        ps.mantle_state.flags = ACTIVE | QUICK | OVER | FAST_MANTLE;
        clear_hint(&mut ps);
        assert_eq!(ps.mantle_state.flags, OVER | FAST_MANTLE);
    }

    #[test]
    fn the_view_stays_within_the_cap_of_the_wall() {
        let mut ps = climbing(0);
        ps.mantle_state.yaw = 0.0;
        ps.viewangles[1] = 30.0;
        cap_view(&mut ps, MantleCapViewContext::default());
        assert_eq!(ps.viewangles[1], 30.0, "inside the cap: left alone");
        assert_eq!(ps.delta_angles[1], 0.0);

        ps.viewangles[1] = 100.0;
        cap_view(&mut ps, MantleCapViewContext::default());
        assert!(
            (ps.viewangles[1] - 60.0).abs() < 1e-3,
            "{}",
            ps.viewangles[1]
        );
        assert!(
            (ps.delta_angles[1] + 40.0).abs() < 1e-3,
            "{}",
            ps.delta_angles[1]
        );
    }

    #[test]
    fn a_disabled_cap_leaves_the_view() {
        let mut ps = climbing(0);
        ps.viewangles[1] = 170.0;
        let context = MantleCapViewContext {
            mantle_enable: false,
            ..MantleCapViewContext::default()
        };
        cap_view(&mut ps, context);
        assert_eq!(ps.viewangles[1], 170.0);
    }
}
