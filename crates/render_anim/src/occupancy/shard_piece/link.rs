use super::*;
use bevy::ecs::system::SystemParam;

/// What a relink needs besides the piece itself.
struct LinkEnv<'a> {
    planes: VisibilityCellPlanes<'a>,
    lookup: &'a LooseAtSpotLookup,
    xmodels: Option<&'a asset_world::LevelModelPlacementRoster>,
}

/// The two tables a relink writes.
struct LinkTables<'a> {
    bits: &'a mut DebrisRoomBits,
    lights: &'a mut PropMainLightVisibility,
}

impl LinkTables<'_> {
    /// Takes a dead piece out of every cell and clears its primary-light links.
    fn unlink_dead(&mut self, env: &LinkEnv, id: u32) {
        let cells = &mut *self.bits;
        unfilter_dynamic_ent_from_cells(&mut cells.bits, cells.dword_tally, cells.slot_cell_count, id);
        let nothing = Bounds::from_center_extent([0.0; 3], [0.0; 3]);
        self.relink_lights(env, id, nothing, true);
    }

    fn relink_lights(&mut self, env: &LinkEnv, id: u32, extent: Bounds, dead: bool) {
        let closest = Some(self.lights.nearest.as_mut_slice());
        relink_debris_key_lights(
            &mut self.lights.tokens,
            closest,
            env.lookup,
            id,
            extent,
            dead,
        );
    }

    /// Refreshes one piece's cell bits and primary-light links.
    fn relink(&mut self, env: &LinkEnv, (piece, at): (&LevelDebrisInstance, &Transform)) {
        let id = u32::from(piece.index);
        if piece.gone {
            return self.unlink_dead(env, id);
        }
        let model = &piece.active_mesh;
        let local = model_local_bounds(env.xmodels, model);
        let extent = dyn_ent_bounds_world(at, local, model_bound_radius(env.xmodels, model));
        let cells = &mut *self.bits;
        filter_debris_into_rooms(&env.planes, extent, &mut cells.bits, cells.dword_tally, id);
        self.relink_lights(env, id, extent, false);
    }
}

/// Everything `bind_dyn_ents_to_cells` reads or writes.
#[derive(SystemParam)]
pub(super) struct LinkInputs<'w, 's> {
    moved: Query<'w, 's, (&'static LevelDebrisInstance, &'static Transform), (With<DynamicPropModelEntity>, Or<(Added<LevelDebrisInstance>, Changed<LevelDebrisInstance>, Changed<Transform>)>)>,
    all_pieces: Query<'w, 's, (&'static LevelDebrisInstance, &'static Transform), With<DynamicPropModelEntity>>,
    xmodels: Option<Res<'w, asset_world::LevelModelPlacementRoster>>,
    dpvs: Res<'w, LevelSightRooms>,
    lookup: Res<'w, LooseAtSpotLookup>,
    bits: ResMut<'w, DebrisRoomBits>,
    lights: ResMut<'w, PropMainLightVisibility>,
}

pub(super) fn bind_dyn_ents_to_cells(inputs: LinkInputs) {
    let _slow = SlowGuard::new("link_dyn_ent_cells");
    let LinkInputs {
        moved,
        all_pieces,
        xmodels,
        dpvs: vis,
        lookup,
        mut bits,
        mut lights,
    } = inputs;
    if !vis.can_walk() {
        (*bits, *lights) = Default::default();
        return;
    }
    let env = LinkEnv {
        lookup: &lookup,
        xmodels: xmodels.as_deref(),
        planes: VisibilityCellPlanes {
            leaf_cells: u32::try_from(vis.cells_in).unwrap_or(u32::MAX),
            nodes: &vis.knots,
            planes: &vis.planes,
        },
    };
    let top_index = all_pieces.iter().map(|(piece, _)| piece.index).max();
    let slots = top_index.map_or(0, |top| u32::from(top) + 1);
    let resized = bits.ensure(slots, vis.cells_in);

    let lights = &mut *lights;
    lights.sunlight_primary_flag_index = lookup.sunlight_primary_flag_index();
    lights.main_light_total = lookup.main_light_total();
    reserve_main_light_bits(
        &mut lights.tokens,
        slots,
        lights.sunlight_primary_flag_index,
        lights.main_light_total,
    );
    if lights.nearest.len() != slots as usize {
        lights.nearest.clear();
        lights.nearest.resize(slots as usize, 0);
    }

    let mut tables = LinkTables {
        bits: &mut bits,
        lights,
    };
    if resized {
        all_pieces
            .iter()
            .for_each(|piece| tables.relink(&env, piece));
    } else {
        moved.iter().for_each(|piece| tables.relink(&env, piece));
    }
}

/// Everything `cull_dyn_ents_by_cell` reads or writes.
#[derive(SystemParam)]
pub(super) struct CullInputs<'w, 's> {
    pieces: Query<
        'w,
        's,
        (
            &'static LevelDebrisInstance,
            &'static Transform,
            &'static mut Visibility,
        ),
        With<DynamicPropModelEntity>,
    >,
    view_set: Query<
        'w,
        's,
        (
            &'static GlobalTransform,
            &'static Projection,
            &'static Camera,
        ),
        With<FirstPersonLens>,
    >,
    published: Res<'w, ExportedCellVisibility>,
    bits: Res<'w, DebrisRoomBits>,
    xmodels: Option<Res<'w, asset_world::LevelModelPlacementRoster>>,
}

pub(super) fn cull_dyn_ents_by_cell(inputs: CullInputs) {
    let _slow = SlowGuard::new("cull_dyn_ent_cell_models");
    let CullInputs {
        mut pieces,
        view_set: views,
        published,
        bits,
        xmodels,
    } = inputs;
    let admitted = admitted_vis(&published);
    let frustum = fpv_cull_planes(&views);
    let xmodels = xmodels.as_deref();
    for (piece, at, mut visibility) in &mut pieces {
        let id = u32::from(piece.index);
        let radius = model_bound_radius(xmodels, &piece.active_mesh);
        let hidden = piece.gone
            || cell_admission_hides(id, &bits, admitted)
            || sphere_outside_frustum(at.translation.to_array(), radius, &frustum);
        *visibility = match hidden {
            true => Visibility::Hidden,
            false => Visibility::Inherited,
        };
    }
}
