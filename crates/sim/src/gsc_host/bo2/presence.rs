//! Where the match's script-owned things stand in the world: the map's
//! authored props and the missiles the scripts track get a mover and a
//! collision row, and this keeps those in step with what the runtime says each
//! one is (pose, model, whether it can be seen or hit).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::prelude::World;

use super::runtime::Runtime;
use crate::ScriptedModel;
use crate::shot_trace::{ServerModelState, EntityHitAbilities};
use crate::step_view::StepWorldView;
use crate::ent_slots::MoverSlot;

/// Script models the scripts spawn at run time number up from here; the
/// authored ones (from the map file) number below it.
const SPAWNED_PRESENCE_BASE: u32 = 0x4000_0000;

/// What the engine was last told about one presence.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    spot: [f32; 3],
    turn: [f32; 3],
    model: Option<Arc<str>>,
    /// Posed on the last pass, so the next pass poses it once more to bring its
    /// velocity back to rest.
    moving: bool,
}

/// What a presence should look like this tick.
struct Wanted {
    presence: ScriptedModel,
    /// Missiles keep a collision body only; their own model draws them.
    collision_only: bool,
    spot: [f32; 3],
    turn: [f32; 3],
    model: Option<Arc<str>>,
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)
}

fn near_angles(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter()
        .zip(b)
        .all(|(a, b)| game_math::degree_gap(*a, b).abs() < 0.01)
}

/// A mover and collision row for something the scripts spawn.
pub(crate) fn spawn_presence(world: &mut World, spot: [f32; 3]) -> Result<ScriptedModel, String> {
    let mut runtime = world.resource_mut::<Runtime>();
    let serial = runtime.next_spawned_presence;
    runtime.next_spawned_presence = serial
        .checked_add(1)
        .filter(|n| *n < SPAWNED_PRESENCE_BASE)
        .ok_or("spawned script model identifiers exhausted")?;
    let id = ScriptedModel::decode_net(SPAWNED_PRESENCE_BASE + serial);
    let mut frame = StepWorldView::from_world(world);
    frame
        .create_scripted_mover(id, spot, [0.0; 3])
        .map_err(|_| "no free entities".to_owned())?;
    frame.insert_collision_owner(EntityHitAbilities::now_tick(
        crate::ServerModelSource::ScriptModel(id),
        None,
        Vec::new(),
    ));
    Ok(id)
}

/// Every presence the runtime owns with the pose it holds now.
fn poses(runtime: &Runtime) -> BTreeMap<ScriptedModel, ([f32; 3], [f32; 3])> {
    let props = runtime.props.iter().map(|p| (p.presence, (p.origin, p.angles)));
    let missiles = runtime
        .missiles
        .values()
        .filter_map(|m| Some((m.presence?, (m.origin, m.angles))));
    props.chain(missiles).collect()
}

/// Collision rows only. Networked mover state is left to `present`: posing a
/// mover mid-tick would break its tick-to-tick velocity.
pub(crate) fn settle_collision(world: &mut World) {
    let wanted = poses(world.resource::<Runtime>());
    let mut frame = StepWorldView::from_world(world);
    for row in frame.entity_collision_capabilities_mut() {
        let Some(&(spot, aim)) = row.owner.scripted_model().and_then(|id| wanted.get(&id))
        else {
            continue;
        };
        row.unseen = false;
        row.solid = true;
        let settled = row
            .followed_pose
            .is_some_and(|(at, facing)| near(at, spot) && near_angles(facing, aim));
        if settled {
            continue;
        }
        row.followed_pose = Some((spot, aim));
        if let Some(dobj) = row.dobj.as_mut() {
            dobj.set_world_pose(spot, aim);
        }
        for solid in &mut row.attached_solids {
            solid.origin = spot;
            solid.turn = aim;
        }
    }
}

/// The per-tick pass: publish what the scripts decided to the engine.
pub(crate) fn sync_presence(world: &mut World) {
    let request = world.resource::<crate::frame_step::FrameTicket>();
    if !request.reason.moves_server_world_forward() {
        return;
    }
    let now = crate::level_time_ms(request.tick);
    settle_collision(world);
    super::controls::publish_objectives(world);
    super::controls::sync_script_locks(world);
    present(world, now);
    settle_collision(world);
}

fn collect_wanted(runtime: &Runtime) -> Vec<Wanted> {
    let props = runtime.props.iter().map(|p| Wanted {
        presence: p.presence,
        collision_only: false,
        spot: p.origin,
        turn: p.angles,
        model: p.model.clone(),
    });
    let missiles = runtime.missiles.values().filter_map(|m| {
        Some(Wanted {
            presence: m.presence?,
            collision_only: true,
            spot: m.origin,
            turn: m.angles,
            model: m.model.clone(),
        })
    });
    props
        .chain(missiles)
        .filter(|want| {
            !runtime.shown.get(&want.presence).is_some_and(|shown| {
                !shown.moving
                    && shown.model == want.model
                    && near(shown.spot, want.spot)
                    && near_angles(shown.turn, want.turn)
            })
        })
        .collect()
}

fn present(world: &mut World, now: i32) {
    let (retired, wanted) = {
        let mut runtime = world.resource_mut::<Runtime>();
        let retired = std::mem::take(&mut runtime.retired_presence);
        let wanted = collect_wanted(&runtime);
        (retired, wanted)
    };
    if retired.is_empty() && wanted.is_empty() {
        return;
    }
    let mut frame = StepWorldView::from_world(world);
    let movers: BTreeMap<ScriptedModel, MoverSlot> =
        crate::step_view::gather_movers(frame.ecs())
            .into_iter()
            .map(|mover| (mover.id, mover))
            .collect();
    for id in &retired {
        if let Some(mover) = movers.get(id) {
            frame.remove_script_mover_by_number(mover.state.slot_index);
            frame.remove_collision_owner(*id);
        }
    }
    let mut updates = Vec::with_capacity(wanted.len());
    for want in wanted {
        let Some(mover) = movers.get(&want.presence) else {
            continue;
        };
        let known = frame
            .ecs()
            .resource::<Runtime>()
            .shown
            .get(&want.presence)
            .cloned();
        let shown = known.unwrap_or_else(|| Shown {
            spot: mover.state.lerp.pos.base_point,
            turn: mover.state.lerp.apos.base_point,
            model: frame
                .collision_owner_mut(want.presence)
                .and_then(|row| row.dobj.as_ref())
                .map(|dobj| dobj.active_mesh.as_str().into()),
            moving: false,
        });
        let posed = !near(shown.spot, want.spot) || !near_angles(shown.turn, want.turn);
        if let Some(mover) = frame.script_mover_mut_by_number(mover.state.slot_index) {
            if want.collision_only {
                mover.state.lerp.e_flags |= entity_t6::missile::EF_NODRAW;
            } else {
                mover.state.lerp.e_flags &= !entity_t6::missile::EF_NODRAW;
            }
            mover.nonsolid = false;
            mover.shown_to = 0;
        }
        if posed || shown.moving {
            frame.set_script_mover_pose(mover.state.slot_index, now, want.spot, want.turn);
        }
        if want.model != shown.model {
            present_model(&mut frame, &want);
        }
        updates.push((
            want.presence,
            Shown {
                spot: if posed { want.spot } else { shown.spot },
                turn: if posed { want.turn } else { shown.turn },
                model: want.model,
                moving: posed,
            },
        ));
    }
    let movers = crate::step_view::gather_movers(frame.ecs());
    crate::mover_clip::follow_movers(frame.entity_collision_capabilities_mut(), &movers, now);
    let mut runtime = world.resource_mut::<Runtime>();
    for id in retired {
        runtime.shown.remove(&id);
    }
    runtime.shown.extend(updates);
}

fn present_model(frame: &mut StepWorldView, want: &Wanted) {
    let ability = want
        .model
        .as_deref()
        .and_then(|model| frame.model_capability(model))
        .flatten();
    let Some(row) = frame.collision_owner_mut(want.presence) else {
        return;
    };
    match (&want.model, row.dobj.as_mut()) {
        (None, _) => row.dobj = None,
        (Some(model), Some(dobj)) => {
            if dobj.active_mesh != **model {
                dobj.replace_model(model, ability);
            }
        }
        (Some(model), None) => {
            row.dobj = Some(ServerModelState::at_pose(
                model,
                ability,
                want.spot,
                want.turn,
            ));
            row.followed_pose = Some((want.spot, want.turn));
        }
    }
    if let Some(dobj) = row.dobj.as_mut() {
        dobj.set_attachments(&[]);
    }
}

impl Runtime {
    /// The bodies a missile's own flight must not collide with: the one the
    /// scripts gave it, when it has one.
    pub(crate) fn missile_collision_models(&self, id: crate::ThrownId) -> Vec<ScriptedModel> {
        self.missiles
            .get(&id)
            .and_then(|missile| missile.presence)
            .into_iter()
            .collect()
    }
}
