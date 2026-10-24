//! Where the match's script-owned things stand in the world: the map's
//! authored props and the missiles the scripts track get a mover and a
//! collision row, and this keeps those in step with what the runtime says each
//! one is (pose, model, whether it can be seen or hit).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::prelude::World;

use super::runtime::Runtime;
use crate::ScriptModelId;
use crate::bullet_collision::{AuthorityDObjState, EntityCollisionCapabilities};
use crate::frame::FrameWorld;
use crate::gentity::ScriptMoverGentity;

/// Script models the scripts spawn at run time number up from here; the
/// authored ones (from the map file) number below it.
const SPAWNED_PRESENCE_BASE: u32 = 0x4000_0000;

/// What the engine was last told about one presence.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    origin: [f32; 3],
    angles: [f32; 3],
    model: Option<Arc<str>>,
    /// Posed on the last pass, so the next pass poses it once more to bring its
    /// velocity back to rest.
    moving: bool,
}

/// What a presence should look like this tick.
struct Wanted {
    presence: ScriptModelId,
    /// Missiles keep a collision body only; their own model draws them.
    collision_only: bool,
    origin: [f32; 3],
    angles: [f32; 3],
    model: Option<Arc<str>>,
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)
}

fn near_angles(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter()
        .zip(b)
        .all(|(a, b)| game_math::angle_subtract(*a, b).abs() < 0.01)
}

/// A mover and collision row for something the scripts spawn.
pub(crate) fn spawn_presence(world: &mut World, origin: [f32; 3]) -> Result<ScriptModelId, String> {
    let mut runtime = world.resource_mut::<Runtime>();
    let serial = runtime.next_spawned_presence;
    runtime.next_spawned_presence = serial
        .checked_add(1)
        .filter(|n| *n < SPAWNED_PRESENCE_BASE)
        .ok_or("spawned script model identifiers exhausted")?;
    let id = ScriptModelId::from_wire(SPAWNED_PRESENCE_BASE + serial);
    let mut frame = FrameWorld::from_world(world);
    frame
        .spawn_script_mover(id, origin, [0.0; 3])
        .map_err(|_| "no free entities".to_owned())?;
    frame.insert_collision_owner(EntityCollisionCapabilities::current_tick(
        crate::AuthorityModelOwner::ScriptModel(id),
        None,
        Vec::new(),
    ));
    Ok(id)
}

/// Every presence the runtime owns with the pose it holds now.
fn poses(runtime: &Runtime) -> BTreeMap<ScriptModelId, ([f32; 3], [f32; 3])> {
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
    let mut frame = FrameWorld::from_world(world);
    for row in frame.entity_collision_capabilities_mut() {
        let Some(&(origin, angles)) = row.owner.script_model().and_then(|id| wanted.get(&id))
        else {
            continue;
        };
        row.hidden = false;
        row.solid = true;
        let settled = row
            .followed_pose
            .is_some_and(|(at, facing)| near(at, origin) && near_angles(facing, angles));
        if settled {
            continue;
        }
        row.followed_pose = Some((origin, angles));
        if let Some(dobj) = row.dobj.as_mut() {
            dobj.set_world_pose(origin, angles);
        }
        for brush in &mut row.linked_brushes {
            brush.origin = origin;
            brush.angles = angles;
        }
    }
}

/// The per-tick pass: publish what the scripts decided to the engine.
pub(crate) fn sync_presence(world: &mut World) {
    let request = world.resource::<crate::step::StepRequest>();
    if !request.reason.advances_authority_world() {
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
        origin: p.origin,
        angles: p.angles,
        model: p.model.clone(),
    });
    let missiles = runtime.missiles.values().filter_map(|m| {
        Some(Wanted {
            presence: m.presence?,
            collision_only: true,
            origin: m.origin,
            angles: m.angles,
            model: m.model.clone(),
        })
    });
    props
        .chain(missiles)
        .filter(|want| {
            !runtime.shown.get(&want.presence).is_some_and(|shown| {
                !shown.moving
                    && shown.model == want.model
                    && near(shown.origin, want.origin)
                    && near_angles(shown.angles, want.angles)
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
    let mut frame = FrameWorld::from_world(world);
    let movers: BTreeMap<ScriptModelId, ScriptMoverGentity> =
        crate::frame::collect_script_movers(frame.ecs())
            .into_iter()
            .map(|mover| (mover.id, mover))
            .collect();
    for id in &retired {
        if let Some(mover) = movers.get(id) {
            frame.remove_script_mover_by_number(mover.state.number);
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
            origin: mover.state.lerp.pos.tr_base,
            angles: mover.state.lerp.apos.tr_base,
            model: frame
                .collision_owner_mut(want.presence)
                .and_then(|row| row.dobj.as_ref())
                .map(|dobj| dobj.current_model.as_str().into()),
            moving: false,
        });
        let posed = !near(shown.origin, want.origin) || !near_angles(shown.angles, want.angles);
        if let Some(mover) = frame.script_mover_mut_by_number(mover.state.number) {
            if want.collision_only {
                mover.state.lerp.e_flags |= entity_t6::missile::EF_NODRAW;
            } else {
                mover.state.lerp.e_flags &= !entity_t6::missile::EF_NODRAW;
            }
            mover.nonsolid = false;
            mover.shown_to = 0;
        }
        if posed || shown.moving {
            frame.set_script_mover_pose(mover.state.number, now, want.origin, want.angles);
        }
        if want.model != shown.model {
            present_model(&mut frame, &want);
        }
        updates.push((
            want.presence,
            Shown {
                origin: if posed { want.origin } else { shown.origin },
                angles: if posed { want.angles } else { shown.angles },
                model: want.model,
                moving: posed,
            },
        ));
    }
    let movers = crate::frame::collect_script_movers(frame.ecs());
    crate::presence::follow_movers(frame.entity_collision_capabilities_mut(), &movers, now);
    let mut runtime = world.resource_mut::<Runtime>();
    for id in retired {
        runtime.shown.remove(&id);
    }
    runtime.shown.extend(updates);
}

fn present_model(frame: &mut FrameWorld, want: &Wanted) {
    let capability = want
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
            if dobj.current_model != **model {
                dobj.replace_model(model, capability);
            }
        }
        (Some(model), None) => {
            row.dobj = Some(AuthorityDObjState::at_pose(
                model,
                capability,
                want.origin,
                want.angles,
            ));
            row.followed_pose = Some((want.origin, want.angles));
        }
    }
    if let Some(dobj) = row.dobj.as_mut() {
        dobj.set_attachments(&[]);
    }
}

impl Runtime {
    /// The bodies a missile's own flight must not collide with: the one the
    /// scripts gave it, when it has one.
    pub(crate) fn missile_collision_models(&self, id: crate::ProjectileId) -> Vec<ScriptModelId> {
        self.missiles
            .get(&id)
            .and_then(|missile| missile.presence)
            .into_iter()
            .collect()
    }
}
