//! Where brush models and dynamic entities stand in the world.

use bevy::prelude::*;
use dpvs_t6::{Bounds, bounds_from_origin_axis};

use super::models::{WorldDynEntBrush, WorldDynEntInstance};

/// A unit rotation from an authored quaternion; a degenerate one means none.
fn unit_rotation(quat: [f32; 4]) -> Quat {
    let q = Quat::from_xyzw(quat[0], quat[1], quat[2], quat[3]);
    if q.length_squared() > 1e-12 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

pub(crate) fn transform_from_gfx_placement(origin: [f32; 3], quat: [f32; 4]) -> Transform {
    Transform {
        translation: Vec3::from_array(origin),
        rotation: unit_rotation(quat),
        scale: Vec3::ONE,
    }
}

/// Rotation rows for pitch/yaw/roll in degrees, in the order the map authors them.
fn axis_from_angles(angles: [f32; 3]) -> [[f32; 3]; 3] {
    let [pitch, yaw, roll] = angles.map(f32::to_radians);
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    [
        [cp * cy, cp * sy, -sp],
        [sr * sp * cy - sy * cr, sr * sp * sy + cr * cy, sr * cp],
        [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
    ]
}

fn matrix_from_origin_axis(origin: [f32; 3], axis: [[f32; 3]; 3]) -> Mat4 {
    let row = |r: [f32; 3]| Vec4::new(r[0], r[1], r[2], 0.0);
    Mat4::from_cols(
        row(axis[0]),
        row(axis[1]),
        row(axis[2]),
        Vec4::new(origin[0], origin[1], origin[2], 1.0),
    )
}

/// One world-from-local matrix per brush model. Script-placed brushes win over
/// dynamic-entity ones for the same model; models nobody placed stay at identity.
pub(crate) fn authored_bmodel_world_from_local(
    model_n: usize,
    script_brushes: &[asset_world::ScriptBrushModelPlacement],
    dynent_brushes: &[asset_world::DynEntDef],
) -> Vec<Mat4> {
    let mut poses: Vec<Option<Mat4>> = vec![None; model_n];
    let free_slot = |poses: &[Option<Mat4>], index: usize| {
        index != 0 && index < model_n && poses[index].is_none()
    };
    for brush in script_brushes {
        let index = brush.cmodel_handle as usize;
        if free_slot(&poses, index) {
            poses[index] = Some(matrix_from_origin_axis(
                brush.origin,
                axis_from_angles(brush.angles),
            ));
        }
    }
    for def in dynent_brushes {
        let index = usize::from(def.brush_model);
        if free_slot(&poses, index) {
            poses[index] =
                Some(transform_from_gfx_placement(def.origin, def.quat).to_matrix());
        }
    }
    poses
        .into_iter()
        .map(|pose| pose.unwrap_or(Mat4::IDENTITY))
        .collect()
}

/// The model-carrying dynamic entities, and how many brush ones the catalog holds.
pub(crate) fn world_dyn_ent_instances(
    catalog: &asset_world::DynEntCatalog,
) -> (Vec<WorldDynEntInstance>, usize) {
    let instances = catalog
        .models
        .iter()
        .filter_map(|def| {
            let name = def.xmodel.as_ref().filter(|name| !name.is_empty())?;
            Some(WorldDynEntInstance {
                index: def.index,
                ty: def.ty,
                current_model: asset_world::MapXModelAssetKey(name.clone()),
                transform: transform_from_gfx_placement(def.origin, def.quat),
                lighting_origin: def.origin,
                phys_preset: def.phys_preset.clone(),
                health: def.health,
                destroy_fx: def.destroy_fx.clone(),
                dead: false,
            })
        })
        .collect();
    (instances, catalog.brushes.len())
}

pub(crate) fn world_dyn_ent_brushes(
    defs: &[asset_world::DynEntDef],
    local_bounds: &[asset_world::GfxBrushModelBounds],
    surfs: &[asset_world::GfxBrushModelSurfs],
) -> Vec<WorldDynEntBrush> {
    defs.iter()
        .map(|def| {
            let model = usize::from(def.brush_model);
            WorldDynEntBrush {
                index: def.index,
                origin: def.origin,
                bounds: local_bounds
                    .get(model)
                    .copied()
                    .map(|local| posed_brush_bounds(def.origin, def.quat, local)),
                brush_model: def.brush_model,
                surface_count: surfs.get(model).map_or(0, |model| model.surface_count),
            }
        })
        .collect()
}

/// A brush model's local bounds moved to where the entity stands.
pub(crate) fn posed_brush_bounds(
    origin: [f32; 3],
    quat: [f32; 4],
    local: asset_world::GfxBrushModelBounds,
) -> Bounds {
    let m = Mat3::from_quat(unit_rotation(quat));
    bounds_from_origin_axis(
        origin,
        [m.x_axis.to_array(), m.y_axis.to_array(), m.z_axis.to_array()],
        Bounds::from_mid_half(local.mid, local.half),
    )
}
