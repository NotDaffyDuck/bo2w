use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use crate::occupancy::dyn_ent::PieceModels;
use asset_world::ClipCollision;
use net::FrameClock;
use render_scene::DynEntModelEntity;
use render_scene::WorldDynEntInstance;

const GRAVITY_Z: f32 = -800.0;

const MSEC_STEP: i32 = 17;

const MAX_ITERS: i32 = 2;

const SLEEP_SPEED: f32 = 4.0;
const GROUND_NZ: f32 = 0.7;
const CONTACT_SLOP: f32 = 0.125;
const SLEEP_MS: i32 = 200;
/// A contact slower than this many steps of gravity is a resting one.
const RESTING_CONTACT_STEPS: f32 = 2.0;
/// What a loose piece collides with: Black Ops II's physics world mask
/// (`standard_query::query` for a body with no vehicle flags).
const MASK_DYN_ENT_PHYS: u32 = 0x280_ec93;
/// How far a piece that starts inside a clip looks for the way out.
const PUSH_OUT_MAX: f32 = 96.0;
const PUSH_OUT_STEP: f32 = 2.0;
/// codmp: the fastest a loose piece leaves a blast. Raid's 0.001-mass glasses
/// and fruit took 4000-5800 units/s from a grenade and left the map through
/// the windows; the pieces that landed right all left at 2600 or less. Not a
/// measured Black Ops II number: its physics caps the speed in code we do
/// not have.
const PIECE_SPEED_MAX: f32 = 2600.0;

/// Test aid (hit log on): where the last loose piece came to rest, for the
/// console's `nearpiece`.
pub static LAST_REST: std::sync::Mutex<Option<[f32; 3]>> = std::sync::Mutex::new(None);

#[derive(Resource, Clone, Default)]
pub struct DynEntPhysClip(pub Option<Arc<ClipCollision>>);

#[derive(Resource, Default)]
pub struct DynEntPhysWorld {
    last_time: Option<i32>,
    bodies: HashMap<Entity, DynEntPhysBody>,
}

impl DynEntPhysWorld {
    pub fn is_awake(&self, entity: Entity) -> bool {
        self.bodies.contains_key(&entity)
    }

    pub fn destroy_body(&mut self, entity: Entity) {
        self.bodies.remove(&entity);
    }
}

#[derive(Clone, Copy, Debug, Message)]
pub struct DynEntPhysImpulse {
    pub entity: Entity,

    pub impulse: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct DynEntPhysBody {
    origin: Vec3,
    vel: Vec3,
    mins: [f32; 3],
    maxs: [f32; 3],
    bounce: f32,
    friction: f32,
    on_ground: bool,
    asleep_ms: i32,
    fast_logged: bool,
    stuck_logged: bool,
}

pub(crate) fn register_dyn_ent_phys(app: &mut App) {
    app.init_resource::<DynEntPhysClip>()
        .init_resource::<DynEntPhysWorld>()
        .add_message::<DynEntPhysImpulse>();
}

pub fn step_phys_world0(
    mut world: ResMut<DynEntPhysWorld>,
    clip: Res<DynEntPhysClip>,
    clock: Option<Res<FrameClock>>,
    catalog: Option<Res<asset_world::MapXModelSceneCatalog>>,
    t6: Option<Res<assets::PreparedFpvMeshes>>,
    mut impulses: MessageReader<DynEntPhysImpulse>,
    mut instances: Query<
        (Entity, &mut WorldDynEntInstance, &mut Transform),
        With<DynEntModelEntity>,
    >,
) {
    let models = PieceModels {
        map: catalog.as_deref(),
        t6: t6.as_deref(),
    };
    for impulse in impulses.read() {
        let Ok((entity, inst, transform)) = instances.get(impulse.entity) else {
            continue;
        };
        if inst.dead {
            continue;
        }
        let Some(props) = asset_world::dyn_ent_props(inst.ty) else {
            continue;
        };
        if !props.use_physics {
            continue;
        }
        let Some(preset) = inst.phys_preset.as_ref() else {
            continue;
        };
        if preset.mass <= 0.0 {
            continue;
        }
        let (mins, maxs) = models.phys_hull(&inst.current_model);
        let add_vel = impulse.impulse / preset.mass;
        if hitlog() && !world.bodies.contains_key(&entity) {
            diag::info!(
                World,
                "dyn ent piece {entity:?} wakes: model {:?} mass {} bounce {} friction {} hull {mins:?}..{maxs:?} at {:?} vel {add_vel:?}",
                inst.current_model,
                preset.mass,
                preset.bounce,
                preset.friction,
                transform.translation
            );
        }
        world
            .bodies
            .entry(entity)
            .and_modify(|body| {
                if hitlog() {
                    diag::info!(
                        World,
                        "dyn ent piece {entity:?} knocked again while awake at {:?} vel {:?} + {add_vel:?}",
                        body.origin,
                        body.vel
                    );
                }
                body.vel = (body.vel + add_vel).clamp_length_max(PIECE_SPEED_MAX);
                body.asleep_ms = 0;
            })
            .or_insert_with(|| DynEntPhysBody {
                origin: transform.translation,
                vel: add_vel.clamp_length_max(PIECE_SPEED_MAX),
                mins,
                maxs,
                bounce: preset.bounce,
                friction: preset.friction,
                on_ground: false,
                asleep_ms: 0,
                fast_logged: false,
                stuck_logged: false,
            });
    }

    let now = clock
        .as_deref()
        .filter(|clock| clock.started())
        .map(FrameClock::time);
    let mut advanced_ms = 0;
    if let Some(now) = now {
        match world.last_time {
            None => world.last_time = Some(now),
            Some(last) if now <= last => {}
            Some(last) => {
                let clip = clip.0.as_deref();
                let mut remain = now.saturating_sub(last);
                let mut left = MAX_ITERS;
                let mut advanced = 0i32;
                while remain > 0 && left > 0 {
                    let step = if MSEC_STEP < remain / left.max(1) {
                        remain / left
                    } else {
                        MSEC_STEP
                    }
                    .clamp(1, remain);
                    let dt = step as f32 / 1000.0;
                    if let Some(clip) = clip {
                        for (entity, body) in world.bodies.iter_mut() {
                            integrate_body(*entity, body, clip, dt);
                        }
                    } else {
                        for body in world.bodies.values_mut() {
                            integrate_free(body, dt);
                        }
                    }
                    remain -= step;
                    left -= 1;
                    advanced += step;
                }
                world.last_time = Some(last + advanced);
                advanced_ms = advanced;
            }
        }
    }

    let mut sleep = Vec::new();
    for (entity, body) in world.bodies.iter_mut() {
        if body.on_ground && body.vel.length() < SLEEP_SPEED {
            body.asleep_ms = body.asleep_ms.saturating_add(advanced_ms);
            if body.asleep_ms >= SLEEP_MS {
                sleep.push(*entity);
            }
        } else {
            body.asleep_ms = 0;
        }
    }

    for (entity, mut inst, mut transform) in &mut instances {
        let Some(body) = world.bodies.get(&entity) else {
            continue;
        };
        transform.translation = body.origin;
        inst.lighting_origin = body.origin.to_array();
        inst.transform.translation = body.origin;
    }

    for entity in sleep {
        if let Some(body) = world.bodies.remove(&entity)
            && hitlog()
        {
            diag::info!(World, "dyn ent piece {entity:?} comes to rest at {:?}", body.origin);
            if let Ok(mut last) = LAST_REST.lock() {
                *last = Some(body.origin.to_array());
            }
        }
    }
}

fn hitlog() -> bool {
    std::env::var_os("BO2MP_T6_HITLOG").is_some()
}

fn integrate_free(body: &mut DynEntPhysBody, dt: f32) {
    body.vel.z += GRAVITY_Z * dt;
    body.origin += body.vel * dt;
    body.on_ground = false;
}

fn integrate_body(entity: Entity, body: &mut DynEntPhysBody, clip: &ClipCollision, dt: f32) {
    body.vel.z += GRAVITY_Z * dt;
    if !body.fast_logged && body.vel.length() > 3000.0 && hitlog() {
        body.fast_logged = true;
        diag::info!(World, "dyn ent piece {entity:?} fast {:?} at {:?}", body.vel, body.origin);
    }
    let lift = if body.mins[2] < 0.0 {
        -body.mins[2]
    } else {
        1.0
    };
    let sweep = |start: Vec3, end: Vec3| {
        clip.sweep_box(
            start.to_array(),
            end.to_array(),
            body.mins,
            body.maxs,
            MASK_DYN_ENT_PHYS,
        )
    };
    let mut start = body.origin;
    let mut end = start + body.vel * dt;
    let mut hit = sweep(start, end);
    // A piece that starts touching the shelf it sat on leaves from just
    // above it; one inside a clip (Raid's bar shelves sit in one) is pushed
    // out the shortest way, as Black Ops II's contact solver does. One
    // still stuck steps up and may not sleep there, or it hangs in the air.
    if hit.startsolid {
        let clear = |at: Vec3| !sweep(at, at).startsolid;
        let mut out = (clear(start + Vec3::Z * lift)).then_some(Vec3::Z * lift);
        let dirs = [Vec3::Z, Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::NEG_Z];
        let mut d = PUSH_OUT_STEP;
        while out.is_none() && d <= PUSH_OUT_MAX {
            out = dirs.iter().map(|dir| *dir * d).find(|off| clear(start + *off));
            d += PUSH_OUT_STEP;
        }
        if let Some(off) = out {
            if off.length() > 8.0 && hitlog() {
                diag::info!(World, "dyn ent piece {entity:?} pushed out {off:?} from {start:?}");
            }
            start += off;
            end += off;
            body.origin = start;
            hit = sweep(start, end);
        }
    }
    if hit.allsolid || hit.startsolid {
        if !body.stuck_logged && hitlog() {
            body.stuck_logged = true;
            diag::info!(World, "dyn ent piece {entity:?} stuck inside clip at {:?}", body.origin);
        }
        body.vel = Vec3::ZERO;
        body.on_ground = false;
        body.origin.z += lift;
        return;
    }
    if hit.fraction < 1.0 {
        let n = Vec3::from_array(hit.normal);
        body.origin = Vec3::from_array(hit.endpos) + n * CONTACT_SLOP;
        let vn = body.vel.dot(n);
        if vn < 0.0 {
            // A piece lying on the floor meets it again every step with the
            // speed one step of gravity gives it. Bouncing that back (the
            // puck's 0.4 sent 5.4 units/s up every step) kept it above the
            // rest speed for good, so it never slept and no push woke it.
            // Slow contacts stop dead; the cut-off is not a measured Black
            // Ops II number.
            let bounce = if -vn < RESTING_CONTACT_STEPS * -GRAVITY_Z * dt {
                0.0
            } else {
                body.bounce.max(0.0)
            };
            let dvn = -(1.0 + bounce) * vn;
            body.vel += dvn * n;
            // Friction takes speed in step with how hard the contact pushes
            // back, so a piece slides the same way at any frame rate. The old
            // cut of the whole friction share every step stopped a shoved
            // puck within a few units.
            let tan = body.vel - body.vel.dot(n) * n;
            let tan_len = tan.length();
            if tan_len > 0.0 {
                let drop = (body.friction.clamp(0.0, 1.0) * dvn).min(tan_len);
                body.vel -= tan * (drop / tan_len);
            }
        }
        body.on_ground = n.z > GROUND_NZ;
    } else {
        body.origin = end;
        body.on_ground = false;
    }
}
