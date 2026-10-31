//! Two live players standing inside each other get shoved apart: each gets a burst of sideways velocity away from
//! the other, with a little random jitter so a perfect overlap still picks a direction. Facts gm-26 to gm-30.

/// Push speed when nothing overrides it (the server's player-collision eject speed). TODO(fact): BO2's own default.
pub const SHOVE_SPEED_BASE: f32 = 25.0;

/// How long a shoved player's movement stays overridden (ms), and the movement flag that says so.
const SHOVE_MS: i32 = 300;
const SHOVE_FLAG: u32 = 0x80;

/// A body that can be shoved has one of these two content values, exactly.
const BODY_CONTENTS: [u32; 2] = [0x0200_0000, 0x0400_0000];

/// Both pushes below this count as no push at all; each player's own movement speed is used instead.
const NO_PUSH: f32 = 0.0001;

/// What the shove reads about one client, and what it writes back: velocity x and y, `pm_time`, `movement_state_bits`.
#[derive(Clone, Copy, Debug)]
pub struct ShoveBody {
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    /// Movement speed, the push used when neither player is moving.
    pub speed: i32,
    /// The player bit of the entity's other flags.
    pub player: bool,
    /// The session state is "playing".
    pub playing: bool,
    pub contents: u32,
    pub health: i32,
    pub move_lock_ms: i32,
    pub movement_state_bits: u32,
    /// Half the body's width (its maxs x); two of them side by side is touching distance.
    pub half_width: f32,
    /// Centre and half size of the body's box, for the overlap test.
    pub body_mid: [f32; 3],
    pub body_half: [f32; 3],
}

/// The pair that was shoved, as rows of the slice given to [`shove_apart`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShovedPair {
    pub pusher: usize,
    pub pushed: usize,
}

/// Shoves row `mover_idx` and the first other row it is stuck in apart, and says which pair that was. Velocities are
/// replaced, not added to. `rng_seed` is the server's shared random seed; a shove draws from it twice.
pub fn shove_apart(
    mover_idx: usize,
    rows: &mut [ShoveBody],
    eject_speed: f32,
    rng_seed: &mut u32,
) -> Option<ShovedPair> {
    let me = *rows.get(mover_idx)?;
    if !shoveable(&me) {
        return None;
    }
    let pushed = rows
        .iter()
        .enumerate()
        .find_map(|(i, them)| (i != mover_idx && stuck_together(&me, them)).then_some(i))?;
    let them = rows[pushed];

    // Apart along the floor, each axis nudged by one random draw (x first).
    let jitter_x = crandom(rng_seed);
    let jitter_y = crandom(rng_seed);
    let (dir_x, dir_y) = unit_2d(
        them.origin[0] - me.origin[0] + jitter_x,
        them.origin[1] - me.origin[1] + jitter_y,
    );

    // A moving player is pushed at the eject speed, a still one not at all; if that leaves neither pushed, both go
    // at their own movement speed.
    let mut push_them = if moving(&them) { eject_speed } else { 0.0 };
    let mut push_me = if moving(&me) { eject_speed } else { 0.0 };
    if push_them < NO_PUSH && push_me < NO_PUSH {
        push_them = them.speed as f32;
        push_me = me.speed as f32;
    }
    shove(&mut rows[pushed], push_them * dir_x, push_them * dir_y);
    shove(&mut rows[mover_idx], -(push_me * dir_x), -(push_me * dir_y));
    Some(ShovedPair { pusher: mover_idx, pushed })
}

fn shove(row: &mut ShoveBody, vx: f32, vy: f32) {
    row.velocity[0] = vx;
    row.velocity[1] = vy;
    row.move_lock_ms = SHOVE_MS;
    row.movement_state_bits |= SHOVE_FLAG;
}

/// A live player body: the player bit, playing, and body contents. Health is checked only on the other body.
fn shoveable(row: &ShoveBody) -> bool {
    row.player && row.playing && BODY_CONTENTS.contains(&row.contents)
}

/// `them` is a shoveable body with health left, its box meets `me`'s, and across the floor its centre is within
/// touching distance of `me`'s.
fn stuck_together(me: &ShoveBody, them: &ShoveBody) -> bool {
    if !shoveable(them) || them.health <= 0 {
        return false;
    }
    let boxes_meet = (0..3).all(|axis| {
        libm::fabsf(them.body_mid[axis] - me.body_mid[axis]) <= them.body_half[axis] + me.body_half[axis]
    });
    let gap_x = them.origin[0] - me.origin[0];
    let gap_y = them.origin[1] - me.origin[1];
    let touch = them.half_width + me.half_width;
    boxes_meet && gap_x * gap_x + gap_y * gap_y <= touch * touch
}

/// Any sideways speed at all.
fn moving(row: &ShoveBody) -> bool {
    row.velocity[0] * row.velocity[0] + row.velocity[1] * row.velocity[1] > 0.0
}

/// `(x, y)` scaled to length 1. A zero length divides by 1 instead.
fn unit_2d(x: f32, y: f32) -> (f32, f32) {
    let length = libm::sqrtf(x * x + y * y);
    let scale = 1.0 / if length <= 0.0 { 1.0 } else { length };
    (x * scale, y * scale)
}

/// The server's signed random number in [-1, 1): one step of the shared linear congruential seed, its top 15 bits.
fn crandom(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(214_013).wrapping_add(2_531_011);
    let draw = (*seed >> 17) as f32 / 32_768.0;
    draw * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player_at(x: f32, walking: bool) -> ShoveBody {
        let no_flags = 0;
        ShoveBody {
            origin: [x, 0.0, 0.0],
            velocity: [if walking { 10.0 } else { 0.0 }, 0.0, 0.0],
            speed: 190,
            player: true,
            playing: true,
            contents: BODY_CONTENTS[0],
            health: 100,
            move_lock_ms: 0,
            movement_state_bits: no_flags,
            half_width: 15.0,
            body_mid: [x, 0.0, 36.0],
            body_half: [15.0, 15.0, 36.0],
        }
    }

    #[test]
    fn only_the_walking_player_moves_away() {
        let mut pair = [player_at(0.0, true), player_at(10.0, false)];
        let mut seed = 1;
        let shoved = shove_apart(0, &mut pair, 25.0, &mut seed);
        assert_eq!(shoved, Some(ShovedPair { pusher: 0, pushed: 1 }));
        assert!(pair[0].velocity[0] < 0.0);
        assert_eq!(pair[1].velocity[0], 0.0);
        assert_eq!((pair[0].move_lock_ms, pair[1].movement_state_bits & SHOVE_FLAG), (SHOVE_MS, SHOVE_FLAG));
    }

    #[test]
    fn nobody_is_shoved_into_a_dead_body() {
        let mut pair = [player_at(0.0, true), player_at(10.0, false)];
        let mut seed = 1;
        pair[0].health = 0;
        assert!(shove_apart(0, &mut pair, 25.0, &mut seed).is_some());
        pair[1].health = 0;
        assert!(shove_apart(0, &mut pair, 25.0, &mut seed).is_none());
    }

    #[test]
    fn extra_content_bits_rule_a_body_out() {
        let mut pair = [player_at(0.0, true), player_at(10.0, false)];
        pair[1].contents |= 1;
        assert!(shove_apart(0, &mut pair, 25.0, &mut 1).is_none());
    }

    #[test]
    fn crandom_stays_in_range() {
        let mut seed = 7;
        assert!((0..1000).map(|_| crandom(&mut seed)).all(|r| (-1.0..1.0).contains(&r)));
    }
}
