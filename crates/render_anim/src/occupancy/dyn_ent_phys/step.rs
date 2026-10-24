use super::*;

pub fn step_phys_world0(
    mut world: ResMut<DynEntPhysWorld>,
    clip: Res<DynEntPhysClip>,
    clock: Option<Res<FrameClock>>,
    catalog: Option<Res<asset_world::MapXModelSceneCatalog>>,
    t6: Option<Res<assets::PreparedFpvMeshes>>,
    mut impulses: MessageReader<DynEntPhysImpulse>,
    mut pieces: Query<(Entity, &mut WorldDynEntInstance, &mut Transform), With<DynEntModelEntity>>,
) {
    let models = PieceModels {
        map: catalog.as_deref(),
        t6: t6.as_deref(),
    };
    for shove in impulses.read() {
        let Ok((entity, piece, transform)) = pieces.get(shove.entity) else {
            continue;
        };
        wake_with(
            &mut world,
            &models,
            entity,
            &piece,
            &transform,
            shove.impulse,
        );
    }

    let now = clock
        .as_deref()
        .filter(|clock| clock.started())
        .map(FrameClock::time);
    let advanced = now.map_or(0, |now| run_steps(&mut world, clip.0.as_deref(), now));
    let asleep = settle(&mut world, advanced);

    for (entity, mut piece, mut transform) in &mut pieces {
        let Some(body) = world.awake.get(&entity) else {
            continue;
        };
        transform.translation = body.origin;
        piece.lighting_origin = body.origin.to_array();
        piece.transform.translation = body.origin;
    }

    for entity in asleep {
        let Some(body) = world.awake.remove(&entity) else {
            continue;
        };
        if hit_log_on() {
            diag::info!(
                World,
                "dyn ent piece {entity:?} comes to rest at {:?}",
                body.origin
            );
            if let Ok(mut last) = LAST_REST.lock() {
                *last = Some(body.origin.to_array());
            }
        }
    }
}

/// Starts (or adds to) the motion of a piece given a shove. Dead pieces,
/// pieces whose type has no physics, and massless presets ignore it.
pub(super) fn wake_with(
    world: &mut DynEntPhysWorld,
    models: &PieceModels,
    entity: Entity,
    piece: &WorldDynEntInstance,
    transform: &Transform,
    impulse: Vec3,
) {
    if piece.dead {
        return;
    }
    let Some(props) = asset_world::dyn_ent_props(piece.ty) else {
        return;
    };
    if !props.use_physics {
        return;
    }
    let Some(preset) = piece.phys_preset.as_ref() else {
        return;
    };
    if preset.mass <= 0.0 {
        return;
    }
    let (mins, maxs) = models.phys_hull(&piece.current_model);
    let gained = impulse / preset.mass;
    let log = hit_log_on();
    if let Some(body) = world.awake.get_mut(&entity) {
        if log {
            diag::info!(
                World,
                "dyn ent piece {entity:?} knocked again while awake at {:?} vel {:?} + {gained:?}",
                body.origin,
                body.vel
            );
        }
        body.vel = (body.vel + gained).clamp_length_max(SPEED_LIMIT);
        body.still_ms = 0;
        return;
    }
    if log {
        diag::info!(
            World,
            "dyn ent piece {entity:?} wakes: model {:?} mass {} bounce {} friction {} hull {mins:?}..{maxs:?} at {:?} vel {gained:?}",
            piece.current_model,
            preset.mass,
            preset.bounce,
            preset.friction,
            transform.translation
        );
    }
    world.awake.insert(
        entity,
        PhysBody {
            origin: transform.translation,
            vel: gained.clamp_length_max(SPEED_LIMIT),
            mins,
            maxs,
            bounce: preset.bounce,
            friction: preset.friction,
            grounded: false,
            still_ms: 0,
            noted_fast: false,
            noted_stuck: false,
        },
    );
}
