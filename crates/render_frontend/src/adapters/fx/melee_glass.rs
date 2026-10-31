//! bo2mp: a melee swing breaks loose glass in front of the swinger (BO2's
//! GlassCl_MeleeEvent on EV_FIRE_MELEE, for anyone's swing). The server
//! breaks whole panes; this breaks the shards a broken pane left hanging.
//! BO2 tries up to five lines from the eye, the reach ahead: the centre,
//! then the four corners player_meleeWidth across and player_meleeHeight up
//! or down, and stops at the first that breaks glass. Reach is
//! player_meleeRange (55); BO2 uses
//! player_bayonetRange (85) for a weapon with a bayonet, which the client
//! does not know. BO2's dynamic objects take the swing too
//! (DynEntCl_MeleeEvent); ours have no physics, so they do not.

use bevy::prelude::*;
use render_fx::{EffectsSystemHandle, ReadyHitFx};

/// BO2's player_meleeRange default.
const MELEE_RANGE: f32 = 55.0;

/// BO2's player_meleeWidth and player_meleeHeight defaults.
const MELEE_WIDTH: f32 = 10.0;
const MELEE_HEIGHT: f32 = 10.0;

/// The glass swing's five lines (across, up), in widths and heights.
const MELEE_GLASS_OFFSETS: [[f32; 2]; 5] = [[0.0, 0.0], [1.0, 1.0], [-1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]];

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn play(
    swing: On<net::EntityFireMelee>,
    impact_effect: Option<Res<ReadyHitFx>>,
    mut host: ResMut<EffectsSystemHandle>,
) {
    // Black Ops II only.
    if !impact_effect
        .as_ref()
        .and_then(|fx| fx.0.as_ref())
        .is_some_and(|t| t.t6)
    {
        return;
    }
    let p = swing.event.payload;
    if p.weapon == 0 {
        return;
    }
    let seed = (u64::from(swing.event.tick.0) << 32) ^ (u64::from(p.number as u32) << 16) ^ 0x6d;
    for offset in MELEE_GLASS_OFFSETS {
        let (start, end) = melee_line(p.origin, p.direction, offset);
        if host.0.glass.hit_segment(start, end, seed) > 0 {
            break;
        }
    }
}

/// One of the swing's lines: from the eye, the reach along the view, nudged
/// across and up by the offset (in melee widths and heights).
fn melee_line(eye: [f32; 3], rot: [f32; 3], spread: [f32; 2]) -> ([f32; 3], [f32; 3]) {
    let (fwd, right, up) = game_math::euler_basis(rot);
    let across = MELEE_WIDTH * spread[0];
    let lift = MELEE_HEIGHT * spread[1];
    (
        eye,
        [
            eye[0] + fwd[0] * MELEE_RANGE + right[0] * across + up[0] * lift,
            eye[1] + fwd[1] * MELEE_RANGE + right[1] * across + up[1] * lift,
            eye[2] + fwd[2] * MELEE_RANGE + right[2] * across + up[2] * lift,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::melee_line;

    #[test]
    fn the_swing_reaches_55_along_the_view() {
        let (start, end) = melee_line([10.0, 20.0, 60.0], [0.0, 90.0, 0.0], [0.0, 0.0]);
        assert_eq!(start, [10.0, 20.0, 60.0]);
        assert!((end[0] - 10.0).abs() < 1e-3 && (end[1] - 75.0).abs() < 1e-3);
        assert!((end[2] - 60.0).abs() < 1e-3);
        let (_, down) = melee_line([0.0; 3], [90.0, 0.0, 0.0], [0.0, 0.0]);
        assert!((down[2] + 55.0).abs() < 1e-3);
    }

    #[test]
    fn a_corner_line_is_a_width_across_and_a_height_up() {
        // Looking along +x: right is -y, up is +z.
        let (_, end) = melee_line([0.0; 3], [0.0, 0.0, 0.0], [1.0, 1.0]);
        assert!((end[0] - 55.0).abs() < 1e-3);
        assert!((end[1] + 10.0).abs() < 1e-3);
        assert!((end[2] - 10.0).abs() < 1e-3);
    }
}
