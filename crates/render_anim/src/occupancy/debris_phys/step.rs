use super::*;

pub fn step_phys_first(
    mut world: ResMut<DebrisPhysLevel>,
    solids: Res<PropPhysicsShape>,
    clock: Option<Res<FrameClock>>,
    catalog: Option<Res<asset_world::LevelModelPlacementRoster>>,
    t6: Option<Res<assets::ReadiedFpvMeshes>>,
    mut impulses: MessageReader<PropPhysicsNudge>,
    mut pieces: Query<(&mut Transform, Entity, &mut LevelDebrisInstance), With<DynamicPropModelEntity>>,
) {
    let (map, t6) = (catalog.as_deref(), t6.as_deref());
    let models = PieceModels { t6, map };
    impulses
        .read()
        .filter_map(|shove| Some((pieces.get(shove.entity).ok()?, shove.impulse)))
        .for_each(|((xform, entity, piece), impulse)| {
            wake_with(&mut world, &models, entity, &piece, &xform, impulse);
        });

    let ticked = match clock.as_deref() {
        Some(clock) if clock.started() => Some(clock.time()),
        _ => None,
    };
    let steps = ticked.map_or(0, |time| run_steps(&mut world, solids.0.as_deref(), time));
    let fallen_asleep = settle(&mut world, steps);

    for (mut xform, id, mut piece) in &mut pieces {
        if let Some(PhysBody { pos: origin, .. }) = world.awake.get(&id).copied() {
            piece.illum_origin = origin.to_array();
            piece.place.translation = origin;
            xform.translation = origin;
        }
    }

    for entity in fallen_asleep {
        let Some(rested) = world.awake.remove(&entity) else {
            continue;
        };
        if !hit_log_on() {
            continue;
        }
        diag::info!(
            World,
            "dyn ent piece {entity:?} comes to rest at {:?}",
            rested.pos
        );
        if let Ok(mut last) = LAST_REST.lock() {
            last.replace(rested.pos.to_array());
        }
    }
}

/// Starts (or adds to) the motion of a piece given a shove. Dead pieces,
/// pieces whose type has no physics, and massless presets ignore it.
pub(super) fn wake_with(
    world: &mut DebrisPhysLevel,
    models: &PieceModels,
    entity_in: Entity,
    piece: &LevelDebrisInstance,
    xform: &Transform,
    impulse: Vec3,
) {
    let moves = asset_world::dyn_ent_props(piece.ty).is_some_and(|props| props.is_simulated);
    let preset_in = match piece.physics_template.as_ref() {
        Some(preset_in) if moves && !piece.gone && preset_in.mass > 0.0 => preset_in,
        _ => return,
    };
    let (mins, maxs) = models.phys_hull(&piece.active_mesh);
    let gained = impulse / preset_in.mass;
    let log = hit_log_on();
    if let Some(awake) = world.awake.get_mut(&entity_in) {
        if log {
            diag::info!(
                World,
                "dyn ent piece {entity_in:?} knocked again while awake at {:?} vel {:?} + {gained:?}",
                awake.pos,
                awake.motion_vec
            );
        }
        awake.still_ms = 0;
        awake.motion_vec = (awake.motion_vec + gained).clamp_length_max(SPEED_LIMIT);
        return;
    }
    if log {
        diag::info!(
            World,
            "dyn ent piece {entity_in:?} wakes: model {:?} mass {} bounce {} friction {} hull {mins:?}..{maxs:?} at {:?} vel {gained:?}",
            piece.active_mesh,
            preset_in.mass,
            preset_in.bounce,
            preset_in.friction,
            xform.translation
        );
    }
    world.awake.insert(
        entity_in,
        PhysBody {
            motion_vec: gained.clamp_length_max(SPEED_LIMIT),
            pos: xform.translation,
            maxs,
            mins,
            drag: preset_in.friction,
            rebound: preset_in.bounce,
            still_ms: 0,
            grounded: false,
            noted_stuck: false,
            noted_fast: false,
        },
    );
}
