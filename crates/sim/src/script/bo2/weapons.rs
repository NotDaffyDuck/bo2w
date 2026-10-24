//! Grenades and rockets in flight, as far as the match's scripts own them: each
//! one a player fires is tracked until it lands, and the sticky ones get a
//! collision body of their own so other shots can hit them where they stick.

use std::sync::Arc;

use bevy_ecs::prelude::World;
use weapon_t6::facts::weap_type;

use super::level::ENTITY_CAP;
use super::presence::spawn_presence;
use super::runtime::Runtime;
use crate::equipment::WeaponNote;
use crate::frame::FrameWorld;
use crate::{ProjectileId, ProjectileState, ScriptModelId};

/// One tracked missile and the pose the scripts last saw it at.
#[derive(Clone, Debug)]
pub(crate) struct Missile {
    pub(crate) entnum: i32,
    pub(crate) presence: Option<ScriptModelId>,
    pub(crate) origin: [f32; 3],
    pub(crate) angles: [f32; 3],
    pub(crate) model: Option<Arc<str>>,
}

fn now_ms(world: &World) -> i32 {
    crate::level_time_ms(world.resource::<crate::step::StepRequest>().tick)
}

/// Whether the scripts are up and running, so their things can be tracked.
fn running(world: &World) -> bool {
    let runtime = world.resource::<Runtime>();
    runtime.program.is_some() && runtime.fault.is_none() && runtime.started
}

/// Per-tick pass over the weapon events the step produced.
pub(crate) fn sync_engine_events(world: &mut World) {
    if !world
        .resource::<crate::step::StepRequest>()
        .reason
        .advances_authority_world()
    {
        return;
    }
    // Taken even when nothing runs, so notes never pile up.
    let notes = std::mem::take(&mut FrameWorld::from_world(world).weapon_notes);
    if !running(world) {
        return;
    }
    adopt_fired(world);
    settle_projectiles(world, &notes);
}

/// Start tracking a projectile: it takes an entity number, and a sticky one a
/// collision body besides.
fn adopt(world: &mut World, projectile: &ProjectileState, now: i32) -> Result<Missile, String> {
    if world.resource::<Runtime>().entity_count >= ENTITY_CAP {
        return Err("no free entities".into());
    }
    let (model, sticky) = {
        let frame = FrameWorld::from_world(world);
        let model = Some(frame.weapon_projectile_model(projectile.weapon))
            .filter(|model| !model.is_empty() && !model.starts_with('*'))
            .map(Arc::from);
        let sticky = frame
            .equipment_facts_for(projectile.weapon)
            .is_some_and(|facts| !facts.timed_detonation && facts.stickiness != 0);
        (model, sticky)
    };
    let presence = if sticky { Some(spawn_presence(world, projectile.origin)?) } else { None };
    world.resource_mut::<Runtime>().entity_count += 1;
    Ok(Missile {
        entnum: projectile.entnum,
        presence,
        origin: projectile.origin_at(now),
        angles: entity_t6::evaluate_trajectory(&projectile.apos, now),
        model,
    })
}

/// Pick up the grenades and rockets fired since the last pass.
fn adopt_fired(world: &mut World) {
    let now = now_ms(world);
    let seen = std::mem::replace(&mut world.resource_mut::<Runtime>().missiles_seen_ms, now);
    let fresh: Vec<ProjectileState> = crate::frame::collect_projectiles(world)
        .into_iter()
        .filter(|p| p.live && p.spawn_time_ms > seen)
        .filter(|p| !world.resource::<Runtime>().missiles.contains_key(&p.id))
        .collect();
    for projectile in fresh {
        // Only what a scripted player fired.
        if !world.resource::<Runtime>().players.contains_key(&projectile.owner.0) {
            continue;
        }
        let kind = FrameWorld::from_world(world)
            .combat_facts_for(projectile.weapon)
            .map_or(-1, |facts| facts.weap_type);
        if !matches!(u8::try_from(kind), Ok(weap_type::GRENADE | weap_type::PROJECTILE)) {
            continue;
        }
        match adopt(world, &projectile, now) {
            Ok(missile) => {
                world.resource_mut::<Runtime>().missiles.insert(projectile.id, missile);
            }
            Err(message) => diag::warn!(Sim, "gsc: {message}; projectile not tracked"),
        }
    }
}

/// Follow the tracked missiles: a flying one moves its pose along, one that
/// landed or blew up is let go and its collision body retired.
fn settle_projectiles(world: &mut World, notes: &[WeaponNote]) {
    let now = now_ms(world);
    let tracked: Vec<(ProjectileId, i32)> = world
        .resource::<Runtime>()
        .missiles
        .iter()
        .map(|(id, missile)| (*id, missile.entnum))
        .collect();
    for (id, entnum) in tracked {
        let detonated = notes
            .iter()
            .any(|note| matches!(note, WeaponNote::Detonated { id: at, .. } if *at == id));
        let flying = FrameWorld::from_world(world)
            .projectile_by_number(entnum)
            .filter(|p| p.id == id && p.live);
        let mut runtime = world.resource_mut::<Runtime>();
        match flying {
            Some(projectile) if !detonated => {
                if let Some(missile) = runtime.missiles.get_mut(&id) {
                    missile.origin = projectile.origin_at(now);
                    missile.angles = entity_t6::evaluate_trajectory(&projectile.apos, now);
                }
            }
            _ => {
                if let Some(missile) = runtime.missiles.remove(&id) {
                    runtime.retired_presence.extend(missile.presence);
                    runtime.entity_count = runtime.entity_count.saturating_sub(1);
                }
            }
        }
    }
}
