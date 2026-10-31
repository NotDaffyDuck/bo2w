//! Effects the server asks for by name: Black Ops II's `playfx`, `spawnfx`,
//! `playfxontag` and `playviewmodelfx`, and the stop and kill that take a
//! bolted one back off.
//!
//! The event names the effect by an index into the effect list of the
//! snapshot it arrived with, and says how to play it in its simulation flags.

use bevy::prelude::*;
use entity_t6::event;
use fx::{AttachPointFrame, EffectBoltTarget, PlaybackResult, basis_off_contact_facing};
use net::NewestTakenSnapshot;

use super::stage::{FxStage, Pose};
use crate::adapters::motion::gun_present::AttachTagGoals;

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
/// `playfx` that also gave an up carries that in `hint` (the wall buys'
/// chalk), and the frame is built from the two. Without one the effect lies on
/// the surface the direction is the normal of.
fn frame(direction: Vec3, hint: Vec3) -> [[f32; 3]; 3] {
    let forward = if direction == Vec3::ZERO { Vec3::Z } else { direction };
    if hint == Vec3::ZERO {
        return basis_off_contact_facing(forward.to_array());
    }
    let forward = Vec3::from_array(fx_t6::runtime::vec3_normalize(forward.to_array()));
    // The hint with its share along the forward direction taken out.
    let up = Vec3::from_array(fx_t6::runtime::vec3_normalize(
        (hint - forward * hint.dot(forward)).to_array(),
    ));
    Pose::axes_from(forward, up)
}

pub(super) fn play_fx(
    fx_play: On<net::EffectAt>,
    adopted: Option<Res<NewestTakenSnapshot>>,
    own_gun: Res<AttachTagGoals>,
    mut stage: FxStage,
    // bo2zm: the effects held in the world (def name, spot, handle).
    mut held_world: Local<Vec<(String, [f32; 3], u16)>>,
) {
    // The Sensor Grenade's lasers are beams (sensor_laser.rs), not effects.
    if fx_play.event.event.0 == event::ours::SENSOR_LASER {
        return;
    }
    let Some(mut stagehand) = stage.playback() else {
        return;
    };
    let body_data = fx_play.event.payload;
    let named = adopted.as_deref().and_then(|snapshot| {
        let index = u8::try_from(body_data.event_parm).unwrap_or(0);
        snapshot.effect_label(index)
    });
    let Some(effect) = named else {
        return;
    };
    let name = stagehand.defs().map_fx_name(effect);
    let axis = frame(Vec3::from_array(body_data.direction), Vec3::from_array(body_data.origin2));
    let up_given = body_data.origin2 != [0.0; 3];
    let (dobj, bone) = (body_data.number as u32, u16::from(body_data.surf_type));

    let played = match request(body_data.wall_pass_bits, up_given) {
        Request::OnViewModel { take_off } => {
            // Nothing when the gun in his hands has no such tag.
            if let Some(goal) = own_gun.target[0] {
                match (take_off, name.settle(stagehand.defs())) {
                    (false, _) => {
                        stagehand.bolted_by_name(name, goal);
                    }
                    (true, Some(def)) => {
                        stagehand.host().stop_bolted(&def.name, goal.dobj, goal.bone);
                    }
                    (true, None) => {}
                }
            }
            return;
        }
        Request::TakeOffBone { at_once } => {
            if let Some(def) = name.settle(stagehand.defs()) {
                if dobj == fx_t6::runtime::FX_ENTITYNUM_WORLD {
                    // bo2zm: BO2's stopfx on a held world effect (Buried's "?"
                    // chalk when its wall buy is built).
                    let at = body_data.origin;
                    let host = stagehand.host();
                    held_world.retain(|(held, origin, handle)| {
                        let here =
                            *held == def.name && (0..3).all(|i| (origin[i] - at[i]).abs() < 1.0);
                        if here
                            && host
                                .bin_of_ticket(*handle)
                                .is_some_and(|e| e.definition_name == def.name)
                        {
                            host.stop_owned(*handle);
                        }
                        !here
                    });
                } else if at_once {
                    stagehand.host().kill_bolted(&def.name, dobj, bone);
                } else {
                    stagehand.host().stop_bolted(&def.name, dobj, bone);
                }
            }
            return;
        }
        Request::OnBone => stagehand.bolted_by_name(
            name,
            EffectBoltTarget {
                dobj,
                bone,
                client_actor_teleport: false,
                orientation: AttachPointFrame {
                    origin: body_data.origin,
                    axis,
                },
            },
        ),
        Request::InWorld { kept: false } => {
            stagehand.oriented(name, Pose { anchor: body_data.origin, turn: axis })
        }
        Request::InWorld { kept: true } => {
            let now = stagehand.now();
            stagehand.spawned(name, Pose { anchor: body_data.origin, turn: axis }, now)
        }
    };
    // bo2zm: remember a held world effect, so a later stopfx at its spot can end it.
    if let Some(PlaybackResult::Kept { handle }) = played
        && dobj == fx_t6::runtime::FX_ENTITYNUM_WORLD
        && let Some(def) = name.settle(stagehand.defs())
    {
        let host = stagehand.host();
        held_world.retain(|(held, _, h)| host.bin_of_ticket(*h).is_some_and(|e| e.definition_name == *held));
        held_world.push((def.name.clone(), body_data.origin, handle));
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
        assert_eq!(frame(Vec3::ZERO, Vec3::ZERO), basis_off_contact_facing([0.0, 0.0, 1.0]));
    }

    #[test]
    fn with_no_up_given_the_effect_lies_on_the_surface() {
        assert_eq!(frame(Vec3::Y, Vec3::ZERO), basis_off_contact_facing([0.0, 1.0, 0.0]));
    }

    #[test]
    fn a_given_up_is_made_square_with_the_direction() {
        // Forward along x; an up leaning half into x is straightened to z.
        let [forward, left, up] = frame(Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 3.0));
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
        let [f, l, u] = frame(Vec3::new(0.3, -0.8, 0.5), Vec3::new(0.1, 0.2, 0.9));
        let (f, l, u) = (Vec3::from(f), Vec3::from(l), Vec3::from(u));
        for v in [f, l, u] {
            assert!((v.length() - 1.0).abs() < 1.0e-5, "{v:?}");
        }
        assert!(f.dot(l).abs() < 1.0e-5);
        assert!(f.dot(u).abs() < 1.0e-5);
        assert!(l.dot(u).abs() < 1.0e-5);
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
