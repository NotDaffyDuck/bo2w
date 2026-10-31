//! bo2mp: a destructible's broken-off pieces (a mannequin's head). Each one
//! the server sends spawns as loose clutter where its bone was, turned the
//! same way, and leaves at the speed the hit gave it; the client's own
//! physics flies it from there, as Black Ops II's clients do. Black Ops II
//! keeps 50 at most, the oldest going first.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::adapters::motion::debris_phys::{PropPhysicsNudge, DebrisPhysLevel};
use crate::adapters::motion::free_body_wake::{DEBRIS_BULLET_FORCE, DEBRIS_EXPLODE_FORCE};
use crate::prepare::scene::cull::DynamicPropModelEntity;
use crate::prepare::scene::world::{LevelDebrisInstance, unit_rotation};
use net::PendingDebris;

const MAX_DEBRIS: usize = 50;

/// Black Ops II's push on a loose piece per unit of hit
/// (`dynEnt_bulletForce`, `dynEnt_explodeForce`). The shared code that
/// pushes loose things when shot or blasted uses the older games' numbers,
/// so a piece's own scales are cut to land on these.
const BO2_BULLET_FORCE: f32 = 0.8;
const BO2_EXPLODE_FORCE: f32 = 12.0;

/// A piece that came loose; `launch` is its first push, given once the
/// piece exists for the physics.
#[derive(Component)]
pub struct Debris {
    launch: Option<Vec3>,
}

#[derive(Resource, Default)]
struct DebrisPool {
    live: VecDeque<(Entity, u16)>,
}

fn spawn_debris(
    mut commands: Commands,
    mut queued_debris: ResMut<PendingDebris>,
    mut pool: ResMut<DebrisPool>,
    mut phys: ResMut<DebrisPhysLevel>,
    scene_facts: Res<render_scene::LevelPresentFacts>,
    map: Query<&LevelDebrisInstance, (With<DynamicPropModelEntity>, Without<Debris>)>,
    live: Query<(), With<Debris>>,
) {
    pool.live.retain(|(entity, _)| live.contains(*entity));
    if queued_debris.0.is_empty() {
        return;
    }
    if !scene_facts.spawned {
        queued_debris.0.clear();
        return;
    }
    // The map's own loose things keep their numbers; pieces take the ones
    // after them.
    let first = map.iter().map(|i| i.index).max().map_or(0, |i| i.saturating_add(1));
    for record in queued_debris.0.drain(..) {
        let index = if pool.live.len() >= MAX_DEBRIS {
            let (oldest, index) = pool.live.pop_front().expect("pool is full");
            phys.remove_physics_body(oldest);
            commands.entity(oldest).despawn();
            index
        } else {
            (0..MAX_DEBRIS as u16)
                .map(|k| first.saturating_add(k))
                .find(|i| !pool.live.iter().any(|(_, used)| used == i))
                .unwrap_or(first)
        };
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            diag::info!(
                World,
                "bo2mp debris {} spawns as loose piece {index} at {:?} flying {:?}",
                record.mesh,
                record.origin,
                record.velocity
            );
        }
        let transform =
            Transform::from_translation(Vec3::from_array(record.origin)).with_rotation(unit_rotation(record.rotation));
        let dead = false;
        let instance = LevelDebrisInstance {
            index,
            ty: asset_world::MovableObjectKind::Clutter,
            active_mesh: asset_world::LevelModelAssetId(record.mesh.clone()),
            place: transform,
            illum_origin: record.origin,
            physics_template: Some(asset_world::SelfContainedPhysicsPreset {
                name: record.mesh,
                template_category: 0,
                mass: record.mass,
                bounce: record.bounce,
                friction: record.friction,
                shot_push_gain: record.bullet_force_scale * BO2_BULLET_FORCE
                    / DEBRIS_BULLET_FORCE,
                blast_push_gain: record.explosive_force_scale * BO2_EXPLODE_FORCE
                    / DEBRIS_EXPLODE_FORCE,
                audio_entry_stem: String::new(),
                shard_scatter_ratio: 0.0,
                shard_rise_speed: 0.0,
                falls_back_to_tube: false,
                material_footstep_entry: false,
            }),
            hp: 0,
            destroy_effect: None,
            gone: dead,
        };
        let entity = commands
            .spawn((
                transform,
                Visibility::Inherited,
                DynamicPropModelEntity,
                instance,
                Debris {
                    launch: Some(Vec3::from_array(record.velocity) * record.mass),
                },
            ))
            .id();
        pool.live.push_back((entity, index));
    }
}

fn launch_debris(
    mut pieces: Query<(Entity, &mut Debris)>,
    mut impulses: MessageWriter<PropPhysicsNudge>,
) {
    for (ent, mut piece) in &mut pieces {
        if let Some(impulse) = piece.launch.take() {
            // A piece with no push still falls: wake it.
            let impulse = if impulse.length_squared() > 0.0 {
                impulse
            } else {
                Vec3::new(0.0, 0.0, -1e-6)
            };
            impulses.write(PropPhysicsNudge { entity: ent, impulse });
        }
    }
}

pub fn register_debris(app: &mut App) {
    app.init_resource::<DebrisPool>()
        .add_systems(
            Update,
            spawn_debris.in_set(net::ClientStep::Present),
        )
        .add_systems(
            Update,
            launch_debris
                .in_set(frame::JobQueueStage::Physics)
                .before(render_anim::occupancy::debris_phys::step_phys_first),
        );
}
