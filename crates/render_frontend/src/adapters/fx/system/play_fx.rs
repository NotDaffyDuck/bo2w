//! Effects the server asks for by name: Black Ops II's `playfx`, `spawnfx`,
//! `playfxontag` and `playviewmodelfx`, and the stop and kill that take a
//! bolted one back off.
//!
//! The event names the effect by an index into the effect list of the
//! snapshot it arrived with, and says how to play it in its simulation flags.

use bevy::prelude::*;
use entity_t6::event as ev;
use fx::{FxBoltOrientation, FxBoltTarget, PlayResult, axis_from_hit_normal};
use net::LastAdoptedSnapshot;
use render_fx::{PresentedVehicleFx, PresentedVehicleFxRow};

use super::stage::FxStage;
use crate::adapters::anim::fpv_present::FpvBoltTargets;

/// Played on the viewmodel's `tag_target` of the player's own gun (the Hunter
/// Killer's lock light).
const ON_VIEW_MODEL: u8 = 0x10;
/// Spawned and kept: a looping effect stays up until something stops it.
const KEPT: u8 = 0x20;
/// Riding a bone of entity `number` (`surf_type` names the bone).
const ON_BONE: u8 = 0x40;
/// Take the effect off instead of playing it.
const TAKE_OFF: u8 = 0x80;
/// With [`TAKE_OFF`]: let go at once instead of letting it burn out (the RC-XD's
/// lights as it blows up and hides).
const AT_ONCE: u8 = 0x08;

/// What the flags ask of the effect.
#[derive(Debug, PartialEq)]
enum Request {
    /// Play on, or with [`TAKE_OFF`] stop on, the player's own gun.
    OnViewModel { take_off: bool },
    /// Stop or kill the copy riding a bone.
    TakeOffBone { at_once: bool },
    /// Play riding a bone.
    OnBone,
    /// Play where the event says, turned along its axis.
    InWorld { kept: bool },
}

fn request(flags: u8, up_given: bool) -> Request {
    if flags & ON_VIEW_MODEL != 0 {
        Request::OnViewModel {
            take_off: flags & TAKE_OFF != 0,
        }
    } else if flags & TAKE_OFF != 0 {
        Request::TakeOffBone {
            at_once: flags & AT_ONCE != 0,
        }
    } else if flags & ON_BONE != 0 {
        Request::OnBone
    } else {
        // A `playfx` keeps a looping effect going (the chalk stays on the
        // wall), and so does `spawnfx`; both are spawned and held. A plain one
        // is played and let go.
        Request::InWorld {
            kept: up_given || flags & KEPT != 0,
        }
    }
}

/// The frame an effect is played in. The event carries the way it points; a
/// `playfx` that also gave an up carries that in `up_hint` (the wall buys'
/// chalk), and the frame is built from the two. Without one the effect lies on
/// the surface the direction is the normal of.
fn frame(direction: [f32; 3], up_hint: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = if direction == [0.0; 3] {
        [0.0, 0.0, 1.0]
    } else {
        direction
    };
    if up_hint == [0.0; 3] {
        return axis_from_hit_normal(forward);
    }
    let forward = Vec3::from_array(fx_t6::runtime::vec3_normalize(forward));
    let hint = Vec3::from_array(up_hint);
    // The hint with its share along the forward direction taken out.
    let up = Vec3::from_array(fx_t6::runtime::vec3_normalize(
        (hint - forward * hint.dot(forward)).to_array(),
    ));
    [
        forward.to_array(),
        up.cross(forward).to_array(),
        up.to_array(),
    ]
}

/// How the vehicle table logs the outcome, for the scorestreak code that waits
/// on it.
fn outcome_label(played: Option<PlayResult>) -> &'static str {
    match played {
        Some(PlayResult::PlayedReleased { .. } | PlayResult::Held { .. }) => "spawned",
        Some(PlayResult::Failed(_)) | None => "miss_def",
    }
}

pub(super) fn play_fx(
    play: On<net::EntityPlayFx>,
    adopted: Option<Res<LastAdoptedSnapshot>>,
    mut presented: ResMut<PresentedVehicleFx>,
    own_gun: Res<FpvBoltTargets>,
    mut stage: FxStage,
) {
    // The Sensor Grenade's lasers are beams (sensor_laser.rs), not effects.
    if play.event.event.0 == ev::ours::SENSOR_LASER {
        return;
    }
    let Some(mut stagehand) = stage.playback() else {
        return;
    };
    let payload = play.event.payload;
    let owner = (payload.correlation != 0).then_some(payload.correlation);
    let index = u8::try_from(payload.event_parm).unwrap_or(0);
    let Some(effect) = adopted
        .as_deref()
        .and_then(|snapshot| snapshot.effect_name(index))
    else {
        if let Some(owner) = owner {
            presented.by_id.insert(
                owner,
                PresentedVehicleFxRow {
                    fx_spawn: Some("miss_cs"),
                    fx_spawn_def: None,
                },
            );
        }
        return;
    };
    let name = stagehand.defs().map_fx_name(effect);
    let axis = frame(payload.direction, payload.origin2);
    let up_given = payload.origin2 != [0.0; 3];
    let bone = (payload.number as u32, u16::from(payload.surf_type));

    let played = match request(payload.simulation_flags, up_given) {
        Request::OnViewModel { take_off } => {
            // Nothing when the gun in his hands has no such tag.
            let Some(target) = own_gun.target[0] else {
                return;
            };
            if take_off {
                if let Some(def) = name.resolve(stagehand.defs()) {
                    stagehand.host().stop_bolted(&def.name, target.dobj, target.bone);
                }
            } else {
                stagehand.bolted_by_name(name, target);
            }
            return;
        }
        Request::TakeOffBone { at_once } => {
            if let Some(def) = name.resolve(stagehand.defs()) {
                if at_once {
                    stagehand.host().kill_bolted(&def.name, bone.0, bone.1);
                } else {
                    stagehand.host().stop_bolted(&def.name, bone.0, bone.1);
                }
            }
            return;
        }
        Request::OnBone => stagehand.bolted_by_name(
            name,
            FxBoltTarget {
                dobj: bone.0,
                bone: bone.1,
                centity_teleport: false,
                orientation: FxBoltOrientation {
                    origin: payload.origin,
                    axis,
                },
            },
        ),
        Request::InWorld { kept: false } => stagehand.oriented(name, payload.origin, axis),
        Request::InWorld { kept: true } => {
            let now = stagehand.now();
            stagehand.spawned(name, payload.origin, axis, now)
        }
    };
    if let Some(owner) = owner {
        presented.by_id.insert(
            owner,
            PresentedVehicleFxRow {
                fx_spawn: Some(outcome_label(played)),
                fx_spawn_def: Some(effect.to_owned()),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_playfx_is_played_and_let_go() {
        assert_eq!(request(0, false), Request::InWorld { kept: false });
    }

    #[test]
    fn spawnfx_and_an_effect_with_an_up_are_kept() {
        assert_eq!(request(KEPT, false), Request::InWorld { kept: true });
        assert_eq!(request(0, true), Request::InWorld { kept: true });
    }

    #[test]
    fn a_bone_flag_rides_a_bone() {
        assert_eq!(request(ON_BONE, false), Request::OnBone);
        // Kept or not, a bolted effect rides until the bone's owner is gone.
        assert_eq!(request(ON_BONE | KEPT, true), Request::OnBone);
    }

    #[test]
    fn taking_off_beats_riding() {
        assert_eq!(
            request(TAKE_OFF | ON_BONE, false),
            Request::TakeOffBone { at_once: false }
        );
        assert_eq!(
            request(TAKE_OFF | AT_ONCE, false),
            Request::TakeOffBone { at_once: true }
        );
    }

    #[test]
    fn the_view_model_flag_beats_everything_else() {
        assert_eq!(
            request(ON_VIEW_MODEL | ON_BONE | KEPT, true),
            Request::OnViewModel { take_off: false }
        );
        assert_eq!(
            request(ON_VIEW_MODEL | TAKE_OFF | AT_ONCE, false),
            Request::OnViewModel { take_off: true }
        );
    }

    #[test]
    fn at_once_alone_means_nothing() {
        assert_eq!(request(AT_ONCE, false), Request::InWorld { kept: false });
    }

    #[test]
    fn without_a_direction_the_effect_points_straight_up() {
        assert_eq!(frame([0.0; 3], [0.0; 3]), axis_from_hit_normal([0.0, 0.0, 1.0]));
    }

    #[test]
    fn with_no_up_given_the_effect_lies_on_the_surface() {
        let normal = [0.0, 1.0, 0.0];
        assert_eq!(frame(normal, [0.0; 3]), axis_from_hit_normal(normal));
    }

    #[test]
    fn a_given_up_is_made_square_with_the_direction() {
        // Forward along x; an up leaning half into x is straightened to z.
        let [forward, left, up] = frame([2.0, 0.0, 0.0], [1.0, 0.0, 3.0]);
        let near = |got: [f32; 3], want: [f32; 3]| {
            Vec3::from(got).distance(Vec3::from(want)) < 1.0e-6
        };
        assert!(near(forward, [1.0, 0.0, 0.0]), "{forward:?}");
        assert!(near(up, [0.0, 0.0, 1.0]), "{up:?}");
        // up x forward: z x x = y.
        assert!(near(left, [0.0, 1.0, 0.0]), "{left:?}");
    }

    #[test]
    fn the_given_frame_stays_orthonormal() {
        let [f, l, u] = frame([0.3, -0.8, 0.5], [0.1, 0.2, 0.9]);
        let (f, l, u) = (Vec3::from(f), Vec3::from(l), Vec3::from(u));
        for v in [f, l, u] {
            assert!((v.length() - 1.0).abs() < 1.0e-5, "{v:?}");
        }
        assert!(f.dot(l).abs() < 1.0e-5);
        assert!(f.dot(u).abs() < 1.0e-5);
        assert!(l.dot(u).abs() < 1.0e-5);
    }

    #[test]
    fn the_vehicle_table_hears_whether_the_effect_started() {
        assert_eq!(outcome_label(Some(PlayResult::Held { handle: 1 })), "spawned");
        assert_eq!(outcome_label(None), "miss_def");
    }

    #[test]
    fn flag_bits_are_distinct() {
        let all = [ON_VIEW_MODEL, KEPT, ON_BONE, TAKE_OFF, AT_ONCE];
        let mut seen = 0u8;
        for bit in all {
            assert_eq!(bit.count_ones(), 1);
            assert_eq!(seen & bit, 0);
            seen |= bit;
        }
    }
}
