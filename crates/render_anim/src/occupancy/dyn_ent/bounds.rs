use super::*;

pub fn xmodel_radius(
    catalog: Option<&asset_world::MapXModelSceneCatalog>,
    key: &asset_world::MapXModelAssetKey,
) -> f32 {
    PieceModels {
        map: catalog,
        t6: None,
    }
    .radius(key)
}

pub fn xmodel_phys_hull(
    catalog: Option<&asset_world::MapXModelSceneCatalog>,
    key: &asset_world::MapXModelAssetKey,
) -> ([f32; 3], [f32; 3]) {
    PieceModels {
        map: catalog,
        t6: None,
    }
    .phys_hull(key)
}

pub(super) fn usable_half_extents(half: &[f32; 3]) -> bool {
    half.iter().all(|h| h.is_finite() && *h > 0.0)
}

/// (centre, half-extents) bounds turned into (mins, maxs); `None` when the
/// extents are missing, non-finite or flat.
pub(crate) fn hull_from_bounds(
    bounds: Option<([f32; 3], [f32; 3])>,
) -> Option<([f32; 3], [f32; 3])> {
    let (centre, half) = bounds?;
    if !usable_half_extents(&half) {
        return None;
    }
    let lo = std::array::from_fn(|axis| centre[axis] - half[axis]);
    let hi = std::array::from_fn(|axis| centre[axis] + half[axis]);
    Some((lo, hi))
}

pub(super) fn model_local_bounds(
    catalog: Option<&asset_world::MapXModelSceneCatalog>,
    key: &asset_world::MapXModelAssetKey,
) -> Option<([f32; 3], [f32; 3])> {
    match catalog?.get(key)? {
        asset_world::MapXModelSceneAsset::T5(skel) => skel.bounds,
        asset_world::MapXModelSceneAsset::Unavailable { .. } => None,
    }
}

/// World-space box of a piece: its model bounds carried through the transform
/// (an axis-aligned box around the rotated one), or a cube of `radius` around
/// its origin when the model has no usable bounds.
pub(crate) fn dyn_ent_world_bounds(
    transform: &Transform,
    local: Option<([f32; 3], [f32; 3])>,
    radius: f32,
) -> Bounds {
    match local {
        Some((centre, half)) if usable_half_extents(&half) => {
            let rotation = Mat3::from_quat(transform.rotation);
            let world_centre = transform.transform_point(Vec3::from_array(centre));
            let world_half = rotation.x_axis.abs() * half[0]
                + rotation.y_axis.abs() * half[1]
                + rotation.z_axis.abs() * half[2];
            Bounds::from_mid_half(world_centre.to_array(), world_half.to_array())
        }
        _ => Bounds::from_mid_half(transform.translation.to_array(), [radius.max(1.0); 3]),
    }
}

// ---------------------------------------------------------------------------
// View culling
// ---------------------------------------------------------------------------

pub fn sphere_behind_frustum(origin: [f32; 3], radius: f32, planes: &[[f32; 4]]) -> bool {
    XModelCasterBound { origin, radius }.outside(planes)
}

pub(super) fn env_switch_on(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1" | "true" | "yes"))
}

/// Frustum planes of the first-person camera; empty (nothing is culled) when
/// culling is switched off or the camera is missing or not a perspective one.
pub fn fpv_frustum_planes(
    cameras: &Query<(&GlobalTransform, &Projection, &Camera), With<FpvLens>>,
) -> Vec<[f32; 4]> {
    if env_switch_on("BO2MP_NO_FRUSTUM") {
        return Vec::new();
    }
    let Ok((placement, projection, camera)) = cameras.single() else {
        return Vec::new();
    };
    let Projection::Perspective(lens) = projection else {
        return Vec::new();
    };
    let mut aspect = lens.aspect_ratio;
    if aspect < 1e-3 {
        aspect = match camera.logical_viewport_size() {
            Some(size) if size.y > 0.0 => size.x / size.y,
            _ => 0.0,
        };
    }
    if aspect < 1e-3 {
        aspect = 16.0 / 9.0;
    }
    let (_, rotation, _) = placement.to_scale_rotation_translation();
    let mut planes = perspective_frustum_planes(FrustumSpec {
        eye: placement.translation(),
        forward: rotation * Vec3::NEG_Z,
        right: rotation * Vec3::X,
        up: rotation * Vec3::Y,
        fov_y_rad: lens.fov,
        aspect,
        near: lens.near,
        far: lens.far,
    })
    .to_vec();
    if env_switch_on("BO2MP_FRUSTUM_NEARFAR") {
        planes.truncate(2);
    }
    planes
}

/// True when the piece sits only in cells the viewer cannot see. Never hides
/// while either table is unavailable.
pub(crate) fn cell_admission_hides(
    dyn_ent_id: u32,
    membership: &DynEntCellBits,
    vis: AdmittedCellVis<'_>,
) -> bool {
    let nothing_to_judge =
        !membership.is_ready() || vis.vis_all || vis.words.is_empty() || vis.cell_count == 0;
    if nothing_to_judge {
        return false;
    }
    let shared_cells = vis.cell_count.min(membership.cell_count);
    !(0..shared_cells).any(|cell| {
        vis.contains(cell)
            && dyn_ent_in_cell(&membership.bits, membership.word_count, cell, dyn_ent_id)
    })
}

pub(super) fn admitted_vis(published: &PublishedCellVis) -> AdmittedCellVis<'_> {
    AdmittedCellVis {
        words: &published.words,
        cell_count: published.cell_count,
        vis_all: published.vis_all,
    }
}

// ---------------------------------------------------------------------------
// Posed output handed from the pose system to the draw system
// ---------------------------------------------------------------------------
