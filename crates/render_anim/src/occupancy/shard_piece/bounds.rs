use super::*;

/// The piece models that only the map supplies, with no T6 side.
fn map_only(catalog: Option<&asset_world::LevelModelPlacementRoster>) -> PieceModels<'_> {
    PieceModels {
        t6: None,
        map: catalog,
    }
}

pub fn model_bound_radius(scene: Option<&asset_world::LevelModelPlacementRoster>, mdl: &asset_world::LevelModelAssetId) -> f32 {
    map_only(scene).radius(mdl)
}

pub fn model_collision_corners(scene: Option<&asset_world::LevelModelPlacementRoster>, mdl: &asset_world::LevelModelAssetId) -> ([f32; 3], [f32; 3]) {
    map_only(scene).phys_hull(mdl)
}

pub(super) fn usable_half_extents(half: &[f32; 3]) -> bool {
    half.iter().all(|h| h.is_finite() && *h > 0.0)
}

/// Turns (centre, half-extents) bounds into (mins, maxs). `None` when the
/// extents are absent, not finite or flat.
pub(crate) fn hull_from_bounds(centred: Option<([f32; 3], [f32; 3])>) -> Option<([f32; 3], [f32; 3])> {
    let (mid, half) = centred?;
    if !usable_half_extents(&half) {
        return None;
    }
    let low = std::array::from_fn(|axis| mid[axis] - half[axis]);
    let high = std::array::from_fn(|axis| mid[axis] + half[axis]);
    Some((low, high))
}

pub(super) fn model_local_bounds(catalog: Option<&asset_world::LevelModelPlacementRoster>, key: &asset_world::LevelModelAssetId) -> Option<([f32; 3], [f32; 3])> {
    let asset_world::LevelModelPlacementAsset::T5(bone_rig) = catalog?.get(key)? else {
        return None;
    };
    bone_rig.bounds
}

/// World-space box of a piece: its model bounds carried through the transform
/// (an axis-aligned box around the rotated one), or a cube of `radius` around
/// its origin when the model has no usable bounds.
pub(crate) fn dyn_ent_bounds_world(
    at: &Transform,
    model_box: Option<([f32; 3], [f32; 3])>,
    rad: f32,
) -> Bounds {
    let usable = model_box.filter(|(_, half)| usable_half_extents(half));
    let Some((mid, half)) = usable else {
        let cube = [rad.max(1.0); 3];
        return Bounds::from_center_extent(at.translation.into(), cube);
    };
    let turn = Mat3::from_quat(at.rotation);
    let [x, y, z] = [turn.x_axis, turn.y_axis, turn.z_axis];
    let reach = x.abs() * half[0] + y.abs() * half[1] + z.abs() * half[2];
    let centre = at.transform_point(mid.into());
    Bounds::from_center_extent(centre.into(), reach.into())
}

// ---------------------------------------------------------------------------
// View culling
// ---------------------------------------------------------------------------

pub fn sphere_outside_frustum(centre: [f32; 3], r: f32, planes: &[[f32; 4]]) -> bool {
    XModelCasterBound {
        origin: centre,
        reach: r,
    }
    .outside(planes)
}

pub(super) fn env_switch_on(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1" | "true" | "yes"))
}

/// Frustum planes of the first-person camera; empty (nothing is culled) when
/// culling is switched off or the camera is missing or not a perspective one.
pub fn fpv_cull_planes(lenses: &Query<'_, '_, (&'static GlobalTransform, &'static Projection, &'static Camera), With<FirstPersonLens>>) -> Vec<[f32; 4]> {
    /// Used when neither the lens nor the viewport has a usable aspect ratio.
    const WIDESCREEN: f32 = 16.0 / 9.0;
    const NEAR_AND_FAR: usize = 2;
    let usable = |ratio: f32| ratio >= 1e-3;

    if env_switch_on("BO2MP_NO_FRUSTUM") {
        return Vec::new();
    }
    let Ok((pose, Projection::Perspective(lens), camera)) = lenses.single() else {
        return Vec::new();
    };
    let from_viewport = || {
        camera
            .logical_viewport_size()
            .filter(|size| size.y > 0.0)
            .map(|size| size.x / size.y).unwrap_or(0.0)
    };
    let aspect = [lens.aspect_ratio, from_viewport()]
        .into_iter()
        .find(|&ratio| usable(ratio))
        .unwrap_or(WIDESCREEN);
    let turn = pose.to_scale_rotation_translation().1;
    let spec = FrustumShape {
        vertical_fov_radians: lens.fov,
        far_cut: lens.far,
        near_cut: lens.near,
        wide_by_tall: aspect,
        up_dir: turn * Vec3::Y,
        side_dir: turn * Vec3::X,
        forward: turn * -Vec3::Z,
        eye: pose.translation(),
    };
    let mut planes = perspective_frustum_planes(spec).to_vec();
    if env_switch_on("BO2MP_FRUSTUM_NEARFAR") {
        planes.truncate(NEAR_AND_FAR);
    }
    planes
}

/// True when the piece sits only in cells the viewer cannot see. Never hides
/// while either table is unavailable.
pub(crate) fn cell_admission_hides(id: u32, cells: &DebrisRoomBits, seen: CellVisAdmit<'_>) -> bool {
    let nothing_to_judge_by = seen.sight_all || seen.tokens.is_empty() || seen.slot_count == 0;
    if nothing_to_judge_by || !cells.sized() {
        return false;
    }
    let in_piece = |cell| debris_marked_in_cell(&cells.bits, cells.dword_tally, cell, id);
    let shared = 0..seen.slot_count.min(cells.slot_cell_count);
    !shared
        .into_iter()
        .any(|cell| seen.holds_cell(cell) && in_piece(cell))
}

pub(super) fn admitted_vis(published: &ExportedCellVisibility) -> CellVisAdmit<'_> {
    CellVisAdmit {
        tokens: &published.words,
        slot_count: published.cells_in,
        sight_all: published.sight_all,
    }
}

// ---------------------------------------------------------------------------
// Posed output handed from the pose system to the draw system
// ---------------------------------------------------------------------------
