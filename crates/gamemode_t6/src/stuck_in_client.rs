/// gm-29: the "g_playerCollisionEjectSpeed" default. TODO(fact): read the BO2 default.
pub const G_PLAYER_COLLISION_EJECT_SPEED_DEFAULT: f32 = 25.0;

/// gm-30.
pub(crate) const STUCK_PM_TIME: i32 = 300;

/// gm-30.
pub(crate) const STUCK_PM_FLAGS: u32 = 0x80;

/// gm-26: a body that can be stuck has exactly one of these contents.
pub(crate) const STUCK_CONTENTS_A: u32 = 0x200_0000;

pub(crate) const STUCK_CONTENTS_B: u32 = 0x400_0000;

/// gm-28.
const CRANDOM_SCALE: f32 = 3.051_757_8e-5;

/// gm-29.
const EJECT_SPEED_EPS: f32 = 0.0001;

/// One client row. `player` is the otherFlags player bit, `playing` the session state.
#[derive(Clone, Copy, Debug)]
pub struct StuckClient {
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    pub speed: i32,
    pub player: bool,
    pub playing: bool,
    pub contents: u32,
    pub health: i32,
    pub pm_time: i32,
    pub pm_flags: u32,

    pub maxs_x: f32,

    pub bounds_mid: [f32; 3],
    pub bounds_half: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StuckEject {
    pub self_idx: usize,
    pub other_idx: usize,
}

/// gm-26 to gm-30: push `self_idx` and the first client it overlaps apart. Velocity is set,
/// not added to.
pub fn stuck_in_client(
    self_idx: usize,
    clients: &mut [StuckClient],
    eject_speed: f32,
    holdrand: &mut u32,
) -> Option<StuckEject> {
    let self_row = clients.get(self_idx)?;
    if !can_be_stuck(self_row) {
        return None;
    }
    let other_idx = (0..clients.len()).find(|&i| i != self_idx && other_eligible(self_idx, i, clients))?;

    let self_origin = clients[self_idx].origin;
    let other_origin = clients[other_idx].origin;
    let mut dx = other_origin[0] - self_origin[0];
    let mut dy = other_origin[1] - self_origin[1];
    dx += random_signed(holdrand);
    dy += random_signed(holdrand);
    vec2_normalize(&mut dx, &mut dy);

    let other_xy = vec2_length(clients[other_idx].velocity);
    let self_xy = vec2_length(clients[self_idx].velocity);
    let mut other_speed = if other_xy > 0.0 { eject_speed } else { 0.0 };
    let mut self_speed = if self_xy > 0.0 { eject_speed } else { 0.0 };
    if other_speed < EJECT_SPEED_EPS && self_speed < EJECT_SPEED_EPS {
        other_speed = clients[other_idx].speed as f32;
        self_speed = clients[self_idx].speed as f32;
    }

    clients[other_idx].velocity[0] = other_speed * dx;
    clients[other_idx].velocity[1] = other_speed * dy;
    clients[other_idx].pm_time = STUCK_PM_TIME;
    clients[other_idx].pm_flags |= STUCK_PM_FLAGS;

    clients[self_idx].velocity[0] = -self_speed * dx;
    clients[self_idx].velocity[1] = -self_speed * dy;
    clients[self_idx].pm_time = STUCK_PM_TIME;
    clients[self_idx].pm_flags |= STUCK_PM_FLAGS;

    Some(StuckEject {
        self_idx,
        other_idx,
    })
}

/// gm-26: the self check has no health test.
fn can_be_stuck(row: &StuckClient) -> bool {
    row.player
        && row.playing
        && (row.contents == STUCK_CONTENTS_A || row.contents == STUCK_CONTENTS_B)
}

/// gm-27.
fn other_eligible(self_idx: usize, other_idx: usize, clients: &[StuckClient]) -> bool {
    let other = &clients[other_idx];
    let self_row = &clients[self_idx];
    if !can_be_stuck(other) || other.health <= 0 {
        return false;
    }
    if !bounds_overlap(
        other.bounds_mid,
        other.bounds_half,
        self_row.bounds_mid,
        self_row.bounds_half,
    ) {
        return false;
    }
    let dx = other.origin[0] - self_row.origin[0];
    let dy = other.origin[1] - self_row.origin[1];
    let radius = other.maxs_x + self_row.maxs_x;
    dx * dx + dy * dy <= radius * radius
}

fn bounds_overlap(a_mid: [f32; 3], a_half: [f32; 3], b_mid: [f32; 3], b_half: [f32; 3]) -> bool {
    (0..3).all(|i| libm::fabsf(a_mid[i] - b_mid[i]) <= a_half[i] + b_half[i])
}

/// gm-28: G_crandom.
pub(crate) fn random_signed(holdrand: &mut u32) -> f32 {
    *holdrand = holdrand.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    let unit = (*holdrand >> 17) as f32 * CRANDOM_SCALE;
    (unit + unit) - 1.0
}

fn vec2_normalize(x: &mut f32, y: &mut f32) {
    let mut len = libm::sqrtf(*x * *x + *y * *y);
    if 0.0 <= -len {
        len = 1.0;
    }
    let inv = 1.0 / len;
    *x *= inv;
    *y *= inv;
}

fn vec2_length(velocity: [f32; 3]) -> f32 {
    libm::sqrtf(velocity[0] * velocity[0] + velocity[1] * velocity[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(x: f32, moving: bool) -> StuckClient {
        StuckClient {
            origin: [x, 0.0, 0.0],
            velocity: if moving { [10.0, 0.0, 0.0] } else { [0.0; 3] },
            speed: 190,
            player: true,
            playing: true,
            contents: STUCK_CONTENTS_A,
            health: 100,
            pm_time: 0,
            pm_flags: 0,
            maxs_x: 15.0,
            bounds_mid: [x, 0.0, 36.0],
            bounds_half: [15.0, 15.0, 36.0],
        }
    }

    #[test]
    fn overlapping_players_are_pushed_apart() {
        let mut rows = [row(0.0, true), row(10.0, false)];
        let mut seed = 1;
        let eject = stuck_in_client(0, &mut rows, 25.0, &mut seed).unwrap();
        assert_eq!(eject.other_idx, 1);
        assert_eq!(rows[1].velocity[0], 0.0);
        assert!(rows[0].velocity[0] < 0.0);
        assert_eq!(rows[0].pm_time, 300);
        assert_eq!(rows[1].pm_flags & 0x80, 0x80);
    }

    #[test]
    fn dead_self_still_counts_but_dead_other_does_not() {
        let mut rows = [row(0.0, true), row(10.0, false)];
        rows[0].health = 0;
        let mut seed = 1;
        assert!(stuck_in_client(0, &mut rows, 25.0, &mut seed).is_some());
        rows[1].health = 0;
        assert!(stuck_in_client(0, &mut rows, 25.0, &mut seed).is_none());
    }

    #[test]
    fn contents_must_match_exactly() {
        let mut rows = [row(0.0, true), row(10.0, false)];
        rows[1].contents = STUCK_CONTENTS_A | 1;
        let mut seed = 1;
        assert!(stuck_in_client(0, &mut rows, 25.0, &mut seed).is_none());
    }
}
