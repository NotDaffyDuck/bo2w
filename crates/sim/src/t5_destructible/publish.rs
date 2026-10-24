//! Turning what a hit broke into what the world sees: loose debris, effects and sounds at the break, and notices for a
//! Black Ops II map's scripts.

use super::launch::launch_velocity;
use super::{Break, DamageKind, Notice, Outcome, dobj_of_mut, hide, hit_log};
use crate::frame::FrameWorld;
use crate::world::ClientId;
use crate::{AuthorityDObjState, AuthorityModelOwner, DebrisRecord, EntityEventPayload, EventAudience, Tick};
use entity_t6::EntityEventKind;
use glam::Mat4;
use xmodel_runtime::RetainedModelCapability;

/// Who struck and how: what [`publish`] needs besides the outcome itself.
#[derive(Clone, Copy)]
pub(super) struct Cause {
    pub kind: DamageKind,
    /// The way the hit was travelling, if it had a direction.
    pub dir: Option<[f32; 3]>,
    pub attacker: Option<ClientId>,
    pub weapon: u32,
}

/// An effect or sound a break asks the clients to play at `origin`.
struct Cue {
    kind: EntityEventKind,
    name: String,
    origin: [f32; 3],
    direction: [f32; 3],
}

/// What the world gets from one hit, apart from the notices (which go straight onto the prop).
struct Aftermath {
    debris: Vec<DebrisRecord>,
    cues: Vec<Cue>,
}

/// Records a hit's `out` on the prop `owner`: notices for the scripts, the newly hidden bones, and the debris and cues
/// each stage that broke gives off.
pub(super) fn publish(world: &mut FrameWorld, tick: Tick, owner: AuthorityModelOwner, out: Outcome, cause: Cause) {
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
fn settle(dobj: &mut AuthorityDObjState, out: Outcome, cause: &Cause) -> Option<Aftermath> {
    let Outcome {
        breaks,
        after,
        died,
        base_loop,
    } = out;
    let state = dobj.t5_destructible.as_mut()?;
    let definition = state.definition.clone();
    let hosted = state.hosted;
    let health = state.health.clone();
    if hosted {
        state
            .notices
            .extend(after.into_iter().map(|(piece, time, damage)| Notice::BreakAfter { piece, time, damage }));
    }

    let pose = dobj
        .capability
        .as_ref()
        .and_then(|cap| cap.pose(&dobj.pose_request, dobj.world_from_model).ok());
    // Where a bone is now, if the model has it.
    let bone_matrix = |show: Option<&String>| -> Option<Mat4> {
        let slot = bone_slot(dobj.capability.as_ref()?, show)?;
        pose.as_ref()?.get(slot).copied()
    };

    let mut notices = Vec::new();
    let mut aftermath = Aftermath {
        debris: Vec::new(),
        cues: Vec::new(),
    };
    for Break { piece: index, stage: slot } in breaks {
        let piece = &definition.pieces[index];
        let stage = &piece.stages[slot];

        // The effect and sound come from the piece's first bone, pointing along its x axis.
        let first = bone_matrix(piece.stages[0].show_bone.as_ref()).unwrap_or(dobj.world_from_model);
        let origin = first.w_axis.truncate().to_array();
        let direction = first.x_axis.truncate().normalize().to_array();
        diag::info!(
            Sim,
            "T5 destructible {} piece={} leaving_stage={} fx={:?} health={}",
            definition.name,
            index,
            slot,
            stage.break_effect,
            health[index]
        );

        // The pieces this stage drops come loose where the stage's own bone was.
        let (_, turn, at) = bone_matrix(stage.show_bone.as_ref())
            .unwrap_or(first)
            .to_scale_rotation_translation();
        for (model, preset) in stage.spawn_models.iter().zip(&stage.spawn_presets) {
            let (Some(model), Some(preset)) = (model, preset) else {
                continue;
            };
            let velocity = launch_velocity(preset, cause.kind, cause.dir);
            if hit_log() {
                diag::info!(
                    Sim,
                    "bo2mp destructible {} drops {model} at {at:?} flying {velocity:?}",
                    definition.name
                );
            }
            aftermath.debris.push(DebrisRecord {
                model: model.clone(),
                origin: at.to_array(),
                rotation: turn.to_array(),
                velocity,
                mass: preset.mass,
                bounce: preset.bounce,
                friction: preset.friction,
                bullet_force_scale: preset.bullet_force_scale,
                explosive_force_scale: preset.explosive_force_scale,
            });
        }

        if let Some(notify) = &stage.break_notify {
            if hosted {
                notices.push(Notice::Broken {
                    notify: notify.clone(),
                    attacker: cause.attacker,
                    weapon: cause.weapon,
                });
            } else {
                diag::warn!(Sim, "T5 destructible script notify unhosted: {} {}", definition.name, notify);
            }
        }
        let cue_of = |kind: EntityEventKind, name: &String| Cue {
            kind,
            name: name.clone(),
            origin,
            direction,
        };
        if let Some(fx) = &stage.break_effect {
            aftermath.cues.push(cue_of(EntityEventKind::PLAY_FX, fx));
        }
        if let Some(sound) = &stage.break_sound {
            aftermath.cues.push(cue_of(EntityEventKind::SOUND_ALIAS, sound));
        }
    }

    match base_loop {
        Some(alias) if hosted => notices.push(Notice::Loop(alias)),
        Some(Some(alias)) => diag::warn!(Sim, "T5 destructible loop sound unhosted: {} {}", definition.name, alias),
        _ => {}
    }
    if hosted && died {
        notices.push(Notice::BaseDestroyed {
            attacker: cause.attacker,
        });
    }
    if let Some(state) = dobj.t5_destructible.as_mut() {
        state.notices.extend(notices);
    }
    hide::refresh_hidden_parts(dobj);
    Some(aftermath)
}

/// The index of the bone called `show` in the model's skeleton.
fn bone_slot(cap: &RetainedModelCapability, show: Option<&String>) -> Option<usize> {
    let wanted = show?;
    cap.pose.bone_names.iter().position(|name| name == wanted)
}

/// Sends `cue` to every client as an entity event on the world entity.
fn play(world: &mut FrameWorld, tick: Tick, cue: Cue) {
    let index = if cue.kind == EntityEventKind::PLAY_FX {
        world.effect_name_index(&cue.name)
    } else {
        world.sound_alias_index(&cue.name)
    };
    world.push_entity_event(
        tick,
        EventAudience::All,
        cue.kind,
        EntityEventPayload {
            number: i32::from(trace_t6::ENTITYNUM_WORLD),
            event_parm: i32::from(index),
            origin: cue.origin,
            direction: cue.direction,
            ..Default::default()
        },
    );
}
