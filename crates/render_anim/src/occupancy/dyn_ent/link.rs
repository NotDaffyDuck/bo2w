use super::*;

/// Refreshes one piece's cell bits and primary-light links.
pub(super) fn relink_piece(
    membership: &mut DynEntCellBits,
    light_vis: &mut DynEntPrimaryLightVis,
    cells: &DpvsPlanes<'_>,
    atpoint: &DynAtPointLookup,
    catalog: Option<&asset_world::MapXModelSceneCatalog>,
    inst: &WorldDynEntInstance,
    transform: &Transform,
) {
    let id = u32::from(inst.index);
    if inst.dead {
        unfilter_dyn_ent_from_cells(
            &mut membership.bits,
            membership.word_count,
            membership.cell_count,
            id,
        );
        relink_dyn_ent_primary_lights(
            &mut light_vis.words,
            Some(light_vis.closest.as_mut_slice()),
            atpoint,
            id,
            Bounds::from_mid_half([0.0; 3], [0.0; 3]),
            true,
        );
        return;
    }
    let bounds = dyn_ent_world_bounds(
        transform,
        model_local_bounds(catalog, &inst.current_model),
        xmodel_radius(catalog, &inst.current_model),
    );
    filter_dyn_ent_into_cells(
        cells,
        bounds,
        &mut membership.bits,
        membership.word_count,
        id,
    );
    relink_dyn_ent_primary_lights(
        &mut light_vis.words,
        Some(light_vis.closest.as_mut_slice()),
        atpoint,
        id,
        bounds,
        false,
    );
}

pub(super) fn link_dyn_ent_cells(
    cells: Res<WorldDpvsCells>,
    atpoint: Res<DynAtPointLookup>,
    catalog: Option<Res<asset_world::MapXModelSceneCatalog>>,
    mut membership: ResMut<DynEntCellBits>,
    mut light_vis: ResMut<DynEntPrimaryLightVis>,
    moved: Query<(&WorldDynEntInstance, &Transform), (With<DynEntModelEntity>, MovedPieces)>,
    every: Query<(&WorldDynEntInstance, &Transform), With<DynEntModelEntity>>,
) {
    let _slow = SlowGuard::new("link_dyn_ent_cells");
    if !cells.is_ready() {
        *membership = DynEntCellBits::default();
        *light_vis = DynEntPrimaryLightVis::default();
        return;
    }
    let planes = DpvsPlanes {
        planes: &cells.planes,
        nodes: &cells.nodes,
        cell_count: cells.cell_count as u32,
    };
    let catalog = catalog.as_deref();
    let slots = every
        .iter()
        .map(|(inst, _)| u32::from(inst.index) + 1)
        .max()
        .unwrap_or(0);
    let rebuilt = membership.ensure(slots, cells.cell_count);
    light_vis.sun_primary = atpoint.sun_primary();
    light_vis.primary_count = atpoint.primary_count();
    let (sun, primary_count) = (light_vis.sun_primary, light_vis.primary_count);
    ensure_primary_light_words(&mut light_vis.words, slots, sun, primary_count);
    if light_vis.closest.len() != slots as usize {
        light_vis.closest = vec![0; slots as usize];
    }

    let mut relink = |inst: &WorldDynEntInstance, transform: &Transform| {
        relink_piece(
            &mut membership,
            &mut light_vis,
            &planes,
            &atpoint,
            catalog,
            inst,
            transform,
        );
    };
    if rebuilt {
        every
            .iter()
            .for_each(|(inst, transform)| relink(inst, transform));
    } else {
        moved
            .iter()
            .for_each(|(inst, transform)| relink(inst, transform));
    }
}

pub(super) fn cull_dyn_ent_cell_models(
    catalog: Option<Res<asset_world::MapXModelSceneCatalog>>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<FpvLens>>,
    published: Res<PublishedCellVis>,
    membership: Res<DynEntCellBits>,
    mut pieces: Query<(&WorldDynEntInstance, &Transform, &mut Visibility), With<DynEntModelEntity>>,
) {
    let _slow = SlowGuard::new("cull_dyn_ent_cell_models");
    let planes = fpv_frustum_planes(&cameras);
    let admitted = admitted_vis(&published);
    let catalog = catalog.as_deref();
    for (inst, transform, mut visibility) in &mut pieces {
        let hidden = inst.dead
            || cell_admission_hides(u32::from(inst.index), &membership, admitted)
            || sphere_behind_frustum(
                transform.translation.to_array(),
                xmodel_radius(catalog, &inst.current_model),
                &planes,
            );
        *visibility = if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}
