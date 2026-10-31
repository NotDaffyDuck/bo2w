//! Turning what a hit broke into what the world sees: loose debris, effects and sounds at the break, and notices for a
//! Black Ops II map's scripts.

use super::launch::launch_velocity;
use super::{DamageKind, Notice, Outcome, StageBreak, dobj_of_mut, hide, hit_log};
use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;
use crate::{ServerModelState, ServerModelSource, DebrisRecord, HappeningBody, EventRecipients, StepNo};
use entity_t6::EventCode;
use glam::Mat4;
use xmodel_runtime::KeptModelAbility;

/// Who struck and how: what [`publish`] needs besides the outcome itself.
#[derive(Clone, Copy)]
pub(super) struct Cause {
    pub kind: DamageKind,
    /// The way the hit was travelling, if it had a direction.
    pub dir: Option<[f32; 3]>,
    pub attacker: Option<SeatNo>,
    pub gun: u32,
}

/// An effect or sound a break asks the clients to play at `origin`.
struct Cue {
    kind: EventCode,
    name: String,
    spot: [f32; 3],
    heading: [f32; 3],
}

/// What the world gets from one hit, apart from the notices (which go straight onto the prop).
struct Aftermath {
    debris: Vec<DebrisRecord>,
    cues: Vec<Cue>,
}

/// Records a hit's `out` on the prop `owner`: notices for the scripts, the newly hidden bones, and the debris and cues
/// each stage that broke gives off.
pub(super) fn publish(world: &mut StepWorldView, tick: StepNo, owner: ServerModelSource, out: Outcome, cause: Cause) {
    if out.breaks.is_empty() && out.after.is_empty() && !out.died && out.base_loop.is_none() {
        return;
    }
    let Some(dobj) = dobj_of_mut(world, owner) else {
        return;
    };
    let Some(aftermath) = settle(dobj, out, &cause) else {
        return;
    };
    for record in aftermath.debris {
        world.push_debris(record);
    }
    for cue in aftermath.cues {
        play(world, tick, cue);
    }
}

/// Everything that only touches the prop itself.
fn settle(dobj: &mut ServerModelState, out: Outcome, cause: &Cause) -> Option<Aftermath> {
    let Outcome {
        breaks,
        after,
        died,
        base_loop,
    } = out;
    let state = dobj.breakable.as_mut()?;
    let blueprint = state.blueprint.clone();
    let hosted = state.hosted;
    let piece_health = state.piece_health.clone();
    if hosted {
        state
            .notices
            .extend(after.into_iter().map(|(piece, time, damage)| Notice::BreakAfter { part: piece, time, harm: damage }));
    }

    let request = &dobj.pose_query;
    let model_pose = dobj
        .can_do
        .as_ref()
        .and_then(|c| c.pose(request, dobj.placement_matrix).ok());
    // Where a bone is now, if the model has it.
    let bone_matrix = |show: Option<&String>| -> Option<Mat4> {
        let slot = bone_slot(dobj.can_do.as_ref()?, show)?;
        model_pose.as_ref()?.get(slot).copied()
    };

    let mut notices = Vec::new();
    let mut aftermath = Aftermath {
        debris: Vec::new(),
        cues: Vec::new(),
    };
    for StageBreak { piece_index: index, stage_left: slot } in breaks {
        let piece = &blueprint.pieces[index];
        let stage = &piece.stages[slot];

        // The effect and sound come from the piece's first bone, pointing along its x axis.
        let first = bone_matrix(piece.stages[0].reveal_joint.as_ref()).unwrap_or(dobj.placement_matrix);
        let origin = first.w_axis.truncate().to_array();
        let direction = first.x_axis.truncate().normalize().to_array();
        diag::info!(
            Sim,
            "T5 destructible {}: piece {} left stage {} (fx {:?}, health {})",
            blueprint.name,
            index,
            slot,
            stage.shatter_visual,
            piece_health[index]
        );

        // The pieces this stage drops come loose where the stage's own bone was.
        let (_, turn, at) = bone_matrix(stage.reveal_joint.as_ref())
            .unwrap_or(first)
            .to_scale_rotation_translation();
        for (model, preset) in stage.debris_models.iter().zip(&stage.spawn_presets) {
            let (Some(model), Some(preset)) = (model, preset) else {
                continue;
            };
            let velocity = launch_velocity(preset, cause.kind, cause.dir);
            let (bounce, friction) = (preset.bounce, preset.friction);
            if hit_log() {
                diag::info!(
                    Sim,
                    "bo2mp destructible {} drops {model} at {at:?} flying {velocity:?}",
                    blueprint.name
                );
            }
            aftermath.debris.push(DebrisRecord {
                mesh: model.clone(),
                origin: at.to_array(),
                rotation: turn.to_array(),
                velocity,
                mass: preset.mass,
                bounce,
                friction,
                bullet_force_scale: preset.bullet_push,
                explosive_force_scale: preset.blast_push,
            });
        }

        if let Some(notify) = &stage.shatter_signal {
            if hosted {
                notices.push(Notice::Broken {
                    notify: notify.clone(),
                    attacker: cause.attacker,
                    gun: cause.gun,
                });
            } else {
                diag::warn!(Sim, "T5 destructible script notify unhosted: {} {}", blueprint.name, notify);
            }
        }
        let cue_of = |kind: EventCode, label: &String| Cue {
            kind,
            name: label.clone(),
            spot: origin,
            heading: direction,
        };
        if let Some(fx) = &stage.shatter_visual {
            aftermath.cues.push(cue_of(EventCode::PLAY_FX, fx));
        }
        if let Some(sound) = &stage.shatter_audio {
            aftermath.cues.push(cue_of(EventCode::SOUND_ALIAS, sound));
        }
    }

    match base_loop {
        Some(alias) if hosted => notices.push(Notice::Loop(alias)),
        Some(Some(alias)) => diag::warn!(Sim, "T5 destructible loop sound unhosted: {} {}", blueprint.name, alias),
        _ => {}
    }
    if hosted && died {
        notices.push(Notice::BaseDestroyed {
            attacker: cause.attacker,
        });
    }
    if let Some(state) = dobj.breakable.as_mut() {
        state.notices.extend(notices);
    }
    hide::refresh_hidden_parts(dobj);
    Some(aftermath)
}

/// The index of the bone called `show` in the model's skeleton.
fn bone_slot(cap: &KeptModelAbility, show: Option<&String>) -> Option<usize> {
    let wanted = show?;
    cap.pose.joint_labels.iter().position(|name| name == wanted)
}

/// Sends `cue` to every client as an entity event on the world entity.
fn play(world: &mut StepWorldView, tick: StepNo, cue: Cue) {
    let slot = if cue.kind == EventCode::PLAY_FX {
        world.effect_name_index(&cue.name)
    } else {
        world.sound_alias_index(&cue.name)
    };
    let cargo = HappeningBody {
        number: i32::from(trace_t6::WORLD_ENTITY_SLOT),
        event_parm: i32::from(slot),
        origin: cue.spot,
        direction: cue.heading,
        ..Default::default()
    };
    world.post_entity_event(tick, EventRecipients::All, cue.kind, cargo);
}
