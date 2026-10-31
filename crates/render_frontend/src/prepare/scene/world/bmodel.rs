//! Where brush models and dynamic entities stand in the world.

use bevy::prelude::*;
use dpvs_t6::{Bounds, box_around_oriented_frame};

use super::model_set::{LevelDebrisSolid, LevelDebrisInstance};

/// A unit rotation from an authored quaternion; a degenerate one means none.
pub(crate) fn unit_rotation(authored: [f32; 4]) -> Quat {
    let raw = Quat::from_array(authored);
    let usable = raw.length_squared() > 1e-12;
    if usable { raw.normalize() } else { Quat::IDENTITY }
}

/// Where a dynamic entity stands: at its origin, turned by its quaternion.
fn mover_transform(mover: &asset_world::MovableObjectDef) -> Transform {
    Transform::from_translation(Vec3::from_array(mover.origin)).with_rotation(unit_rotation(mover.turn_quat))
}

/// The world-from-local matrix of a brush placed at `origin` turned by pitch/yaw/roll
/// `turn` in degrees, in the order the map authors them: the forward, left and up
/// rows as its first three columns, the origin as its last.
fn placed_matrix(origin: [f32; 3], turn: [f32; 3]) -> Mat4 {
    let [(sin_p, cos_p), (sin_y, cos_y), (sin_r, cos_r)] = turn.map(|deg| deg.to_radians().sin_cos());
    let forward = Vec4::new(cos_p * cos_y, cos_p * sin_y, -sin_p, 0.0);
    let left = Vec4::new(sin_r * sin_p * cos_y - sin_y * cos_r, sin_r * sin_p * sin_y + cos_r * cos_y, sin_r * cos_p, 0.0);
    let up = Vec4::new(cos_r * sin_p * cos_y + sin_r * sin_y, cos_r * sin_p * sin_y - sin_r * cos_y, cos_r * cos_p, 0.0);
    Mat4::from_cols(forward, left, up, Vec3::from_array(origin).extend(1.0))
}

/// One world-from-local matrix per brush model. Script-placed brushes win over
/// dynamic-entity ones for the same model; models nobody placed stay at identity.
pub(crate) fn designer_solid_model_level_from_local(
    model_tally: usize,
    script_solids: &[asset_world::ScriptSolidPlacementInfo],
    debris_solid_list: &[asset_world::MovableObjectDef],
) -> Vec<Mat4> {
    let mut placed: Vec<Option<Mat4>> = vec![None; model_tally];
    // Model 0 is the world itself and is never placed; a model is placed once.
    let open = |placed: &[Option<Mat4>], slot: usize| slot > 0 && placed.get(slot).is_some_and(Option::is_none);
    for solid in script_solids {
        let slot = solid.collision_model_ref as usize;
        if open(&placed, slot) {
            placed[slot] = Some(placed_matrix(solid.origin, solid.angles));
        }
    }
    for mover in debris_solid_list {
        let slot = usize::from(mover.solid_model);
        if open(&placed, slot) {
            placed[slot] = Some(mover_transform(mover).to_matrix());
        }
    }
    placed.into_iter().map(Option::unwrap_or_default).collect()
}

/// The model-carrying dynamic entities, and how many brush ones the catalog holds.
pub(crate) fn level_debris_instances(
    catalog: &asset_world::MovableObjectRoster,
) -> (Vec<LevelDebrisInstance>, usize) {
    let mut found = Vec::new();
    for mover in &catalog.models {
        let Some(mesh) = mover.model_name.as_ref().filter(|named| !named.is_empty()) else { continue };
        let dead = false;
        found.push(LevelDebrisInstance {
            active_mesh: asset_world::LevelModelAssetId(mesh.clone()),
            place: mover_transform(mover),
            illum_origin: mover.origin,
            index: mover.index,
            ty: mover.ty,
            hp: mover.hit_points,
            physics_template: mover.physics_template.clone(),
            destroy_effect: mover.destroy_effect.clone(),
            gone: dead,
        });
    }
    let brush_count = catalog.brushes.len();
    (found, brush_count)
}

/// The brush-model entities with their posed bounds (when the model has local
/// bounds) and face counts (0 when it has none).
pub(crate) fn level_debris_solids(
    defs: &[asset_world::MovableObjectDef],
    local_bounds: &[asset_world::SolidModelExtents],
    surfs: &[asset_world::SolidModelFaces],
) -> Vec<LevelDebrisSolid> {
    let solid = |mover: &asset_world::MovableObjectDef| {
        let model = usize::from(mover.solid_model);
        let extents = local_bounds.get(model).copied();
        LevelDebrisSolid {
            bbox: extents.map(|e| posed_solid_bounds(mover.origin, mover.turn_quat, e)),
            facet_tally: surfs.get(model).map_or(0, |faces| faces.facet_tally),
            solid_model: mover.solid_model,
            at_point: mover.origin,
            index: mover.index,
        }
    };
    defs.iter().map(solid).collect()
}

/// A brush model's local bounds moved to where the entity stands.
pub(crate) fn posed_solid_bounds(
    pos: [f32; 3],
    orientation: [f32; 4],
    local: asset_world::SolidModelExtents,
) -> Bounds {
    let frame = Mat3::from_quat(unit_rotation(orientation));
    let axes = [frame.x_axis, frame.y_axis, frame.z_axis].map(|axis| axis.to_array());
    box_around_oriented_frame(pos, axes, Bounds::from_center_extent(local.centre, local.half))
}
