//! Shared pieces of the ledge climb: lookup tables giving each climb's
//! running time and root-bone travel, plus the per-frame move that plays it.
//! Locating a ledge and beginning a climb live in `crate::mantle`; the pose
//! and the events use these same tables.

use game_math::degree_gap;
use game_math::wrap_degrees;
use playerstate_t6::game::ledge_climb_bits::{
    ACTIVE, BO2, BO2_DURATION_SHIFT, BO2_WEAPON_ANIM, BO2_WEAPON_UP, FAST_MANTLE, OVER, QUICK,
};
use playerstate_t6::game::{AvatarState, entity_flag_bits, movement_state_bits};
use trace_t6::Trace;

use crate::bridge::{Bo2Feel, queue_forecast_happening};

/// Contents bit marking surfaces a climb may begin on.
pub const SOLID_CLIMBABLE: u32 = 0x0100_0000;

/// Surface bits of a face that may be scaled or vaulted.
pub const FLOOR_CLIMB_ONTO_OR_VAULT: u32 = 0x0600_0000;

/// Within those bits, the one that picks a vault.
pub const FLOOR_VAULT_ONLY: u32 = 0x0400_0000;

/// The perk that speeds climbing up (specialty_fastmantle).
pub use playerstate_t6::perks::PERK_FASTMANTLE;

/// Entity bit held for as long as a climb runs.
const CLIMBING_EFLAG: u32 = entity_flag_bits::MANTLE;

/// Event sent every frame of a climb that has to stay crouched.
const EVENT_CLIMB_CROUCHED: i32 = entity_t6::event::STANCE_FORCE_CROUCH as i32;
/// Event sent once when the climb finishes with enough room to stand.
const EVENT_CLIMB_STOOD: i32 = entity_t6::event::STANCE_FORCE_STAND as i32;

/// Climb flag: stay crouched all the way up.
const STAY_CROUCHED: i32 = 1 << 1;
/// Climb flag: rise to standing at the top.
const RISE_AT_TOP: i32 = 1 << 2;

/// Gap between two ledge probes.
const LIP_PROBE_GAP: f32 = 20.0;

/// Probe heights above the feet for finding a ledge, highest first: three
/// probes one gap apart, the lowest one gap up.
pub const CLIMB_LIP_RISES: [f32; 3] = [3.0 * LIP_PROBE_GAP, 2.0 * LIP_PROBE_GAP, LIP_PROBE_GAP];

/// Reach of the ledge probe beyond the wall.
pub const CLIMB_AHEAD_REACH: f32 = 16.0;

/// The ledge surface is looked for down to this height over the feet.
pub const CLIMB_LIP_LOWEST_RISE: f32 = 18.0;

/// Tallest the player's box may be and still fit on top of a ledge.
pub const CLIMB_HEADROOM_TOP: f32 = 50.0;

/// Half-width of the box swept in every climb check.
pub const CLIMB_SIDE_REACH: f32 = 15.0;

/// Distance a vault moves the player beyond the ledge.
pub const VAULT_AHEAD_REACH: f32 = 31.0;

/// Number of animations in the climb tree.
pub const CLIMB_CLIP_SLOTS: usize = 11;

/// Outcome of a climb search: the wall's facing, the start point, and the
/// ledge and landing spots.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClimbOutcomes {
    pub heading: [f32; 3],
    pub begin_at: [f32; 3],
    pub lip_point: [f32; 3],
    pub finish_location: [f32; 3],
    pub flag_word: i32,
}

/// Sweeps a box through the world; the climb search uses it to find a ledge.
pub trait ClimbPillSweep {
    fn trace(
        &mut self,
        from_pt: [f32; 3],
        end: [f32; 3],
        box_lo: [f32; 3],
        box_hi: [f32; 3],
        solid_filter: u32,
    ) -> Trace;
}

/// How many rows the climb tables hold.
const ROWS: usize = 7;
/// The rise each table row was animated for.
const ROW_RISE: [f32; ROWS] = [57.0, 51.0, 45.0, 39.0, 33.0, 27.0, 21.0];
/// Climb-up animation number per row.
const ROW_CLIMB_ANIM: [i32; ROWS] = [1, 2, 3, 4, 5, 6, 7];
/// Vault animation number per row.
const ROW_VAULT_ANIM: [i32; ROWS] = [8, 8, 9, 9, 9, 10, 10];

/// Pick the row whose rise is nearest the real one; equal distances go to
/// the earlier (taller) row.
pub fn find_transition(cur_height: f32, target_height: f32) -> i32 {
    let wanted = target_height - cur_height;
    let mut winner = 0;
    let mut winning_gap = libm::fabsf(ROW_RISE[0] - wanted);
    for (index, &rise) in ROW_RISE.iter().enumerate().skip(1) {
        let gap = libm::fabsf(rise - wanted);
        if winning_gap > gap {
            winner = index;
            winning_gap = gap;
        }
    }
    winner as i32
}

/// Turn the stored row number into a safe index; out-of-range means row 0.
fn row_index(trans_index: i32) -> usize {
    match usize::try_from(trans_index) {
        Ok(row) if row < ROW_RISE.len() => row,
        _ => 0,
    }
}

/// Climb-up animation of a row.
pub fn trans_up_anim(trans_index: i32) -> i32 {
    ROW_CLIMB_ANIM[row_index(trans_index)]
}

/// Vault animation of a row.
pub fn trans_over_anim(trans_index: i32) -> i32 {
    ROW_VAULT_ANIM[row_index(trans_index)]
}

/// Supplies the running time of each climb animation.
pub trait ClimbClipDuration {
    /// Milliseconds animation `clip_ordinal` (1 to 10) takes, from the fast
    /// tree or the ordinary one.
    fn span_in_ms(&self, quick_tree: bool, clip_ordinal: i32) -> i32;
}

/// Fixed times: all climb-ups last `release_stamp_ms`, all vaults `surplus_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedClimbClipDuration {
    pub release_stamp_ms: i32,
    pub surplus_ms: i32,
}

impl Default for FixedClimbClipDuration {
    fn default() -> Self {
        Self {
            release_stamp_ms: 400,
            surplus_ms: 200,
        }
    }
}

impl ClimbClipDuration for FixedClimbClipDuration {
    fn span_in_ms(&self, _fast: bool, anim: i32) -> i32 {
        match anim {
            1..=7 => self.release_stamp_ms,
            _ => self.surplus_ms,
        }
    }
}

/// Supplies the root bone's travel since an animation began.
pub trait ClimbBoneOffsetSource {
    /// Root offset once `played` (0 to 1) of animation `clip_ordinal` has played.
    fn root_travel(&self, quick_tree: bool, clip_ordinal: i32, played: f32) -> [f32; 3];
}

/// A root bone that stays put.
#[derive(Clone, Copy, Debug, Default)]
pub struct StillClimbOffset;

impl ClimbBoneOffsetSource for StillClimbOffset {
    fn root_travel(&self, _fast: bool, _anim: i32, _progress: f32) -> [f32; 3] {
        [0.0; 3]
    }
}

/// A root bone that moves in a straight line to the animation's end point.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClipDrivenClimbOffset;

/// Resting place of the root after animation `clip_ordinal`: up and ahead for
/// a climb-up, ahead and down for a vault.
fn final_offset(clip_ordinal: i32) -> [f32; 3] {
    match clip_ordinal {
        1..=7 => [CLIMB_AHEAD_REACH, 0.0, ROW_RISE[clip_ordinal as usize - 1]],
        8..=10 => [VAULT_AHEAD_REACH, 0.0, -CLIMB_LIP_LOWEST_RISE],
        _ => [0.0; 3],
    }
}

/// Each component of `v` times `factor`.
fn scale3(v: [f32; 3], factor: f32) -> [f32; 3] {
    core::array::from_fn(|i| v[i] * factor)
}

/// Component-wise `a + b`.
fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| a[i] + b[i])
}

/// Component-wise `a - b`.
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| a[i] - b[i])
}

impl ClimbBoneOffsetSource for ClipDrivenClimbOffset {
    fn root_travel(&self, _fast: bool, anim: i32, progress: f32) -> [f32; 3] {
        scale3(final_offset(anim), progress.clamp(0.0, 1.0))
    }
}

/// Whether the climb runs on the fast tree.
fn on_fast_tree(flags: i32) -> bool {
    flags & FAST_MANTLE != 0
}

/// Running time of the climb-up half of the current climb.
pub fn up_length(avatar: &AvatarState, clip_lengths: &impl ClimbClipDuration) -> i32 {
    let state = &avatar.mantle_state;
    clip_lengths.span_in_ms(on_fast_tree(state.flags), trans_up_anim(state.trans_index))
}

/// Running time of the vault half; 0 when the climb is not a vault.
pub fn over_length(avatar: &AvatarState, clip_lengths: &impl ClimbClipDuration) -> i32 {
    let state = &avatar.mantle_state;
    if state.flags & OVER == 0 {
        return 0;
    }
    clip_lengths.span_in_ms(
        on_fast_tree(state.flags),
        trans_over_anim(state.trans_index),
    )
}

/// Running time of the full climb.
pub fn duration(avatar: &AvatarState, clip_lengths: &impl ClimbClipDuration) -> i32 {
    up_length(avatar, clip_lengths).saturating_add(over_length(avatar, clip_lengths))
}

const DEGREES_TO_RADIANS: f32 = 0.017_453_292;

/// Rotate the x/y part of `vec` by `degrees` about the vertical axis.
fn rotate_flat(vec: &mut [f32; 3], degrees: f32) {
    let radians = degrees * DEGREES_TO_RADIANS;
    let (c, s) = (libm::cosf(radians), libm::sinf(radians));
    let (old_x, old_y) = (vec[0], vec[1]);
    vec[1] = c * old_y + old_x * s;
    vec[0] = c * old_x - s * old_y;
}

/// Root-bone travel `clock_stamp_ms` into the climb, rotated to the wall's heading.
/// Once the climb-up is done, the vault's travel is added to all of it.
pub fn sample_root_track(
    avatar: &AvatarState,
    clock_stamp_ms: i32,
    clip_lengths: &impl ClimbClipDuration,
    root: &impl ClimbBoneOffsetSource,
) -> [f32; 3] {
    let state = &avatar.mantle_state;
    let climb_ms = up_length(avatar, clip_lengths).max(1);
    let vault_ms = over_length(avatar, clip_lengths);
    let quick = on_fast_tree(state.flags);
    let climb_anim = trans_up_anim(state.trans_index);
    let mut travel = if clock_stamp_ms > climb_ms && vault_ms > 0 {
        let whole_climb = root.root_travel(quick, climb_anim, 1.0);
        let progress = (clock_stamp_ms - climb_ms) as f32 / vault_ms as f32;
        let vault_anim = trans_over_anim(state.trans_index);
        add3(whole_climb, root.root_travel(quick, vault_anim, progress))
    } else {
        root.root_travel(quick, climb_anim, clock_stamp_ms as f32 / climb_ms as f32)
    };
    rotate_flat(&mut travel, state.yaw);
    travel
}

/// Whether a climb has taken the gun away. A marked climb carries an explicit
/// bit; an unmarked one only holds the gun back until the last vault animation.
pub fn is_weapon_inactive(avatar: &AvatarState, climb_allowed: bool) -> bool {
    if !climb_allowed || avatar.movement_state_bits & movement_state_bits::MANTLE == 0 {
        return false;
    }
    let state = &avatar.mantle_state;
    if state.flags & BO2 != 0 {
        state.flags & BO2_WEAPON_UP == 0
    } else {
        trans_over_anim(state.trans_index) != 10
    }
}

/// Tag a freshly begun climb for the weapon. A row whose rise is no more
/// than `mantle_weapon_height` leaves the gun raised; if it also exceeds
/// `mantle_weapon_anim_height`, the gun's own climb camera plays. The climb
/// time is stored in the upper bits for the client.
pub fn mark_bo2_weapon(avatar: &mut AvatarState, feel: Bo2Feel, clip_lengths: &impl ClimbClipDuration) {
    let rise = ROW_RISE[row_index(avatar.mantle_state.trans_index)];
    let mut tag = BO2;
    if rise <= feel.mantle_weapon_height {
        tag |= BO2_WEAPON_UP;
        if rise > feel.mantle_weapon_anim_height {
            tag |= BO2_WEAPON_ANIM;
        }
    }
    let total_ms = duration(avatar, clip_lengths).clamp(0, 0xffff);
    let low_mask = (1 << BO2_DURATION_SHIFT) - 1;
    let preserved = avatar.mantle_state.flags & low_mask & !(BO2 | BO2_WEAPON_UP | BO2_WEAPON_ANIM);
    avatar.mantle_state.flags = preserved | tag | (total_ms << BO2_DURATION_SHIFT);
}

/// Drop the "a climb is available" hint when a frame begins.
pub fn clear_hint(avatar: &mut AvatarState) {
    avatar.mantle_state.flags &= !(ACTIVE | QUICK);
}

/// Limit on how far the view may swing away from the wall mid-climb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimbLookLimitArgs {
    pub climb_allowed: bool,
    /// Allowed degrees on each side of the wall's heading.
    pub yaw_cap: f32,
}

impl Default for ClimbLookLimitArgs {
    fn default() -> Self {
        Self {
            climb_allowed: true,
            yaw_cap: 60.0,
        }
    }
}

/// Keep the view within `view_yawcap` degrees of the wall while climbing.
pub fn cap_view(avatar: &mut AvatarState, frame_ctx: ClimbLookLimitArgs) {
    debug_assert!(
        avatar.movement_state_bits & movement_state_bits::MANTLE != 0,
        "capping the view needs a climb"
    );
    if !frame_ctx.climb_allowed {
        return;
    }
    let limit = frame_ctx.yaw_cap;
    let lower = -limit;
    let mut offset = degree_gap(avatar.mantle_state.yaw, avatar.look_angles[1]);
    if !(offset < lower || limit < offset) {
        return;
    }
    while lower > offset {
        offset += limit;
    }
    while offset > limit {
        offset -= limit;
    }
    let bound = if offset <= 0.0 { limit } else { lower };
    avatar.view_offset_angles[1] += offset;
    avatar.look_angles[1] = wrap_degrees(avatar.mantle_state.yaw + bound);
}

/// Settings for how a climb's move finishes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimbStepArgs {
    pub climb_allowed: bool,
    /// Stop completely on arrival at a ledge top (a vault keeps its speed
    /// and is left uncapped).
    pub stop_on_ledge: bool,
}

impl Default for ClimbStepArgs {
    fn default() -> Self {
        Self {
            climb_allowed: true,
            stop_on_ledge: false,
        }
    }
}

/// Shift the origin by `shift` and set the velocity that covers it in
/// `elapsed_ms`; velocity is left alone when no time passed.
fn carry_player(avatar: &mut AvatarState, shift: [f32; 3], elapsed_ms: f32) {
    avatar.origin = add3(avatar.origin, shift);
    if elapsed_ms > 0.0 {
        let rate = 1.0 / (elapsed_ms * 0.001);
        avatar.velocity = scale3(shift, rate);
    }
}

/// Last-frame bookkeeping: release the climb, stand up if asked, and stop on
/// a ledge top.
fn finish_climb(avatar: &mut AvatarState, stop_on_ledge: bool) {
    avatar.movement_state_bits &= !movement_state_bits::MANTLE;
    if avatar.mantle_state.flags & RISE_AT_TOP != 0 {
        queue_forecast_happening(avatar, EVENT_CLIMB_STOOD, 0);
        avatar.e_flags &= !CLIMBING_EFLAG;
    }
    if stop_on_ledge && avatar.mantle_state.flags & OVER == 0 {
        avatar.velocity = Default::default();
    }
    avatar.mantle_state.flags |= QUICK;
}

/// Play `msec` of the climb: advance the clock, move the player by the root
/// travel over that span, set the matching velocity, and release the climb
/// at the end.
pub fn run_climb(
    avatar: &mut AvatarState,
    msec: i32,
    frame_ctx: ClimbStepArgs,
    clip_lengths: &impl ClimbClipDuration,
    root: &impl ClimbBoneOffsetSource,
) {
    debug_assert!(
        avatar.movement_state_bits & movement_state_bits::MANTLE != 0,
        "moving a climb needs a climb"
    );
    if !frame_ctx.climb_allowed {
        return;
    }

    avatar.mantle_state.flags &= !ACTIVE;
    if avatar.mantle_state.flags & STAY_CROUCHED != 0 {
        queue_forecast_happening(avatar, EVENT_CLIMB_CROUCHED, 0);
    }

    let limit = duration(avatar, clip_lengths).max(0);
    let started_at = avatar.mantle_state.timer;
    avatar.mantle_state.timer = started_at.saturating_add(msec).min(limit);

    let earlier = sample_root_track(avatar, started_at, clip_lengths, root);
    let later = sample_root_track(avatar, avatar.mantle_state.timer, clip_lengths, root);
    let elapsed = (avatar.mantle_state.timer - started_at) as f32;
    carry_player(avatar, sub3(later, earlier), elapsed);

    if avatar.mantle_state.timer >= limit {
        finish_climb(avatar, frame_ctx.stop_on_ledge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playerstate_t6::game::ledge_climb_bits;

    const FEEL: Bo2Feel = Bo2Feel {
        mantle_weapon_height: 39.0,
        mantle_weapon_anim_height: 32.0,
        ..Bo2Feel::BO2_MP
    };

    /// Ten units forward and forty up over a whole animation.
    struct Rising;

    impl ClimbBoneOffsetSource for Rising {
        fn root_travel(&self, _quick_tree: bool, _clip: i32, played: f32) -> [f32; 3] {
            [10.0 * played, 0.0, 40.0 * played]
        }
    }

    /// A tree whose fast climb is shorter.
    struct FastIsShort;

    impl ClimbClipDuration for FastIsShort {
        fn span_in_ms(&self, quick_tree: bool, _clip: i32) -> i32 {
            if quick_tree { 250 } else { 500 }
        }
    }

    fn climbing(trans_index: i32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits |= movement_state_bits::MANTLE;
        avatar.mantle_state.trans_index = trans_index;
        avatar
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
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        assert_eq!(
            (up_length(&avatar, &clip_lengths), over_length(&avatar, &clip_lengths)),
            (400, 0)
        );
        assert_eq!(duration(&avatar, &clip_lengths), 400);
        avatar.mantle_state.flags |= OVER;
        assert_eq!(over_length(&avatar, &clip_lengths), 200);
        assert_eq!(duration(&avatar, &clip_lengths), 600);
    }

    #[test]
    fn the_fast_climb_perk_reads_the_fast_tree() {
        let mut avatar = climbing(0);
        assert_eq!(duration(&avatar, &FastIsShort), 500);
        avatar.mantle_state.flags |= FAST_MANTLE;
        assert_eq!(duration(&avatar, &FastIsShort), 250);
    }

    #[test]
    fn the_flat_lengths_split_up_from_over() {
        let clip_lengths = FixedClimbClipDuration {
            release_stamp_ms: 300,
            surplus_ms: 120,
        };
        assert_eq!(clip_lengths.span_in_ms(false, 1), 300);
        assert_eq!(clip_lengths.span_in_ms(true, 7), 300);
        assert_eq!(clip_lengths.span_in_ms(false, 8), 120);
        assert_eq!(clip_lengths.span_in_ms(false, 10), 120);
    }

    #[test]
    fn the_root_track_grows_along_the_climb_then_the_vault() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        assert!(near(
            sample_root_track(&avatar, 200, &clip_lengths, &Rising),
            [5.0, 0.0, 20.0]
        ));
        // No vault: the climb-up keeps counting past its end.
        assert!(near(
            sample_root_track(&avatar, 600, &clip_lengths, &Rising),
            [15.0, 0.0, 60.0]
        ));

        avatar.mantle_state.flags |= OVER;
        assert!(near(
            sample_root_track(&avatar, 400, &clip_lengths, &Rising),
            [10.0, 0.0, 40.0]
        ));
        assert!(near(
            sample_root_track(&avatar, 500, &clip_lengths, &Rising),
            [15.0, 0.0, 60.0]
        ));
        assert!(near(
            sample_root_track(&avatar, 600, &clip_lengths, &Rising),
            [20.0, 0.0, 80.0]
        ));
    }

    #[test]
    fn the_root_track_turns_to_face_the_wall() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        avatar.mantle_state.yaw = 90.0;
        let delta = sample_root_track(&avatar, 400, &clip_lengths, &Rising);
        assert!(near(delta, [0.0, 10.0, 40.0]), "{delta:?}");
        avatar.mantle_state.yaw = 180.0;
        let delta = sample_root_track(&avatar, 400, &clip_lengths, &Rising);
        assert!(near(delta, [-10.0, 0.0, 40.0]), "{delta:?}");
    }

    #[test]
    fn a_straight_line_root_ends_where_its_animation_does() {
        let root = ClipDrivenClimbOffset;
        assert_eq!(
            root.root_travel(false, 1, 1.0),
            [CLIMB_AHEAD_REACH, 0.0, 57.0]
        );
        assert_eq!(root.root_travel(false, 4, 0.5), [8.0, 0.0, 19.5]);
        assert_eq!(
            root.root_travel(true, 8, 1.0),
            [VAULT_AHEAD_REACH, 0.0, -CLIMB_LIP_LOWEST_RISE]
        );
        assert_eq!(
            root.root_travel(false, 7, 3.0),
            root.root_travel(false, 7, 1.0),
            "frac stops at 1"
        );
        assert_eq!(root.root_travel(false, 7, -1.0), [0.0; 3], "and at 0");
        assert_eq!(root.root_travel(false, 0, 1.0), [0.0; 3], "no such animation");
        assert_eq!(StillClimbOffset.root_travel(false, 3, 1.0), [0.0; 3]);
    }

    #[test]
    fn an_advance_moves_the_player_by_what_the_root_travelled() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        avatar.origin = [100.0, 200.0, 300.0];
        avatar.mantle_state.flags |= ACTIVE;
        run_climb(
            &mut avatar,
            100,
            ClimbStepArgs::default(),
            &clip_lengths,
            &Rising,
        );
        assert_eq!(avatar.mantle_state.timer, 100);
        assert!(near(avatar.origin, [102.5, 200.0, 310.0]), "{:?}", avatar.origin);
        assert!(near(avatar.velocity, [25.0, 0.0, 100.0]), "{:?}", avatar.velocity);
        assert_ne!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0, "still climbing");
        assert_eq!(avatar.mantle_state.flags & ACTIVE, 0, "the hint is down");
    }

    #[test]
    fn the_last_frame_is_cut_short_and_ends_the_climb() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        avatar.mantle_state.timer = 390;
        run_climb(&mut avatar, 50, ClimbStepArgs::default(), &clip_lengths, &Rising);
        assert_eq!(avatar.mantle_state.timer, 400);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0);
        assert_ne!(avatar.mantle_state.flags & QUICK, 0);
        // 10 ms of a 400 ms climb carried the player 0.25 forward and 1 up.
        assert!(near(avatar.velocity, [25.0, 0.0, 100.0]), "{:?}", avatar.velocity);
    }

    #[test]
    fn a_disabled_climb_does_nothing() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        let before = avatar;
        let frame_ctx = ClimbStepArgs {
            climb_allowed: false,
            ..ClimbStepArgs::default()
        };
        run_climb(&mut avatar, 50, frame_ctx, &clip_lengths, &Rising);
        assert_eq!(avatar.mantle_state.timer, before.mantle_state.timer);
        assert_eq!(avatar.origin, before.origin);
    }

    #[test]
    fn a_crouching_climb_sends_its_event_every_frame_and_a_standing_one_at_the_end() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        avatar.e_flags |= CLIMBING_EFLAG;
        assert_eq!(avatar.e_flags, 0x8000);
        avatar.mantle_state.flags |= STAY_CROUCHED | RISE_AT_TOP;
        run_climb(
            &mut avatar,
            100,
            ClimbStepArgs::default(),
            &clip_lengths,
            &Rising,
        );
        assert_eq!(avatar.predictable_event_sequence, 1);
        assert_eq!(avatar.predictable_events[0], EVENT_CLIMB_CROUCHED);
        assert_ne!(avatar.e_flags & CLIMBING_EFLAG, 0);

        run_climb(
            &mut avatar,
            1000,
            ClimbStepArgs::default(),
            &clip_lengths,
            &Rising,
        );
        assert_eq!(avatar.predictable_event_sequence, 3);
        assert_eq!(avatar.predictable_events[1], EVENT_CLIMB_CROUCHED);
        assert_eq!(avatar.predictable_events[2], EVENT_CLIMB_STOOD);
        assert_eq!(avatar.e_flags & CLIMBING_EFLAG, 0);
    }

    #[test]
    fn a_climb_with_no_events_flagged_sends_none() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        run_climb(
            &mut avatar,
            1000,
            ClimbStepArgs::default(),
            &clip_lengths,
            &Rising,
        );
        assert_eq!(avatar.predictable_event_sequence, 0);
    }

    fn end_velocity(frame_ctx: ClimbStepArgs, over: bool) -> [f32; 3] {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(2);
        if over {
            avatar.mantle_state.flags |= OVER;
        }
        avatar.mantle_state.timer = duration(&avatar, &clip_lengths) - 10;
        run_climb(&mut avatar, 50, frame_ctx, &clip_lengths, &Rising);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0, "the climb ends");
        avatar.velocity
    }

    #[test]
    fn multiplayer_stops_dead_on_a_ledge_but_not_after_a_vault() {
        let mp = ClimbStepArgs {
            stop_on_ledge: true,
            ..ClimbStepArgs::default()
        };
        assert_eq!(end_velocity(mp, false), [0.0; 3], "onto a ledge: stopped");
        let vault = end_velocity(mp, true);
        assert!(
            vault[0] > 0.0 && vault[2] > 0.0,
            "over a ledge: keeps moving"
        );
        let zombies = end_velocity(ClimbStepArgs::default(), false);
        assert!(
            zombies[0] > 0.0 && zombies[2] > 0.0,
            "the other mode keeps the climb's speed"
        );
    }

    #[test]
    fn low_climbs_keep_the_gun_and_high_ones_lower_it() {
        let clip_lengths = FixedClimbClipDuration::default();
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
            let mut avatar = climbing(row);
            mark_bo2_weapon(&mut avatar, FEEL, &clip_lengths);
            assert_ne!(avatar.mantle_state.flags & BO2, 0, "row {row}");
            assert_eq!(avatar.mantle_state.flags & BO2_WEAPON_UP != 0, up, "row {row}");
            assert_eq!(
                avatar.mantle_state.flags & BO2_WEAPON_ANIM != 0,
                anim,
                "row {row}"
            );
            assert_eq!(is_weapon_inactive(&avatar, true), !up, "row {row}");
            assert_eq!(ledge_climb_bits::bo2_weapon_up(&avatar), up, "row {row}");
        }
    }

    #[test]
    fn the_climb_length_rides_along_and_progress_counts_up() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(4);
        avatar.mantle_state.flags |= OVER;
        mark_bo2_weapon(&mut avatar, FEEL, &clip_lengths);
        assert_ne!(
            avatar.mantle_state.flags & OVER,
            0,
            "the vault bit survives the marking"
        );
        assert_eq!(ledge_climb_bits::bo2_duration_ms(avatar.mantle_state.flags), 600);
        avatar.mantle_state.timer = 150;
        assert_eq!(ledge_climb_bits::bo2_progress(&avatar), Some(0.25));
        avatar.movement_state_bits &= !movement_state_bits::MANTLE;
        assert_eq!(ledge_climb_bits::bo2_progress(&avatar), None);
        assert!(!ledge_climb_bits::bo2_weapon_up(&avatar));
    }

    #[test]
    fn marking_twice_leaves_one_mark() {
        let clip_lengths = FixedClimbClipDuration::default();
        let mut avatar = climbing(3);
        mark_bo2_weapon(&mut avatar, FEEL, &clip_lengths);
        let once = avatar.mantle_state.flags;
        mark_bo2_weapon(&mut avatar, FEEL, &clip_lengths);
        assert_eq!(avatar.mantle_state.flags, once);
    }

    #[test]
    fn without_the_mark_only_the_last_vault_animation_keeps_the_gun() {
        for (row, inactive) in [(0, true), (4, true), (5, false), (6, false)] {
            let avatar = climbing(row);
            assert_eq!(is_weapon_inactive(&avatar, true), inactive, "row {row}");
        }
        let still = AvatarState::ZERO;
        assert!(!is_weapon_inactive(&still, true), "not climbing");
        assert!(
            !is_weapon_inactive(&climbing(0), false),
            "climbing disabled"
        );
    }

    #[test]
    fn clearing_the_hint_drops_the_hint_and_quick_bits_only() {
        let mut avatar = climbing(1);
        avatar.mantle_state.flags = ACTIVE | QUICK | OVER | FAST_MANTLE;
        clear_hint(&mut avatar);
        assert_eq!(avatar.mantle_state.flags, OVER | FAST_MANTLE);
    }

    #[test]
    fn the_view_stays_within_the_cap_of_the_wall() {
        let mut avatar = climbing(0);
        avatar.mantle_state.yaw = 0.0;
        avatar.look_angles[1] = 30.0;
        cap_view(&mut avatar, ClimbLookLimitArgs::default());
        assert_eq!(avatar.look_angles[1], 30.0, "inside the cap: left alone");
        assert_eq!(avatar.view_offset_angles[1], 0.0);

        avatar.look_angles[1] = 100.0;
        cap_view(&mut avatar, ClimbLookLimitArgs::default());
        assert!(
            (avatar.look_angles[1] - 60.0).abs() < 1e-3,
            "{}",
            avatar.look_angles[1]
        );
        assert!(
            (avatar.view_offset_angles[1] + 40.0).abs() < 1e-3,
            "{}",
            avatar.view_offset_angles[1]
        );
    }

    #[test]
    fn a_disabled_cap_leaves_the_view() {
        let mut avatar = climbing(0);
        avatar.look_angles[1] = 170.0;
        let frame_ctx = ClimbLookLimitArgs {
            climb_allowed: false,
            ..ClimbLookLimitArgs::default()
        };
        cap_view(&mut avatar, frame_ctx);
        assert_eq!(avatar.look_angles[1], 170.0);
    }
}
