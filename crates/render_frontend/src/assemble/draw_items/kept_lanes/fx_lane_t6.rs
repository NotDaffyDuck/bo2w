use bevy::prelude::*;
use dpvs_t6::{encode_engine_mesh_sort_key, pack_stain_mesh_facet_draw, encode_particle_cloud_sort_key};
use render_material::sort_band;

use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, HeldDrawKind, catalog_item, draw_order, route_to_products};
use crate::assemble::draw_items::list::{CONTENT_KEY_SEED, blend_content_id};
use crate::assemble::draw_items::mesh_build::fx::{FxCodeDrawPlan, LiveRun};
use crate::assemble::draw_items::mesh_build::stain::{
    VisualStainMeshPlan, DecalSubmeshKey, decal_mesh_surface_samplers,
};
use crate::assemble::draw_items::mesh_build::cloud_lattice::FxCloudDrawPlan;
use crate::assemble::draw_items::MaterialEpoch;
use crate::assemble::draw_items::LiveMaterialLedger;
use crate::assemble::draw_items::FacetSamplerInputs;
use crate::prepare::scene::model_illum_cache::{
    ModelIllumOwner, ResolvedModelIllum, ResolvedModelIllumTable,
};
use crate::prepare::scene::prop_light_slots::LevelPropIllum;
use crate::prepare::scene::world::LevelScriptModelInstance;

/// The effect meshes of the frame (code meshes, particle clouds and decal marks) as items
/// sorted for drawing.
///
/// The items are rebuilt when the plans' layout changes. While it holds, each frame copies the
/// new arguments into the items already there. Glass marks take their lighting from a table
/// that moves every frame, so a lane that holds one is rebuilt every frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct EffectDrawTrack {
    pub colour: Vec<HeldDrawItem>,
    pub emissive: Vec<HeldDrawItem>,
    pub distortion: Vec<HeldDrawItem>,

    pub membership_edition: u64,
    layout: u64,

    pub payload_edition: u64,
    pub engine_sorted: u32,
    pub particle_cloud_ordered: u32,
    pub stain_mesh_sorted: u32,
    pub engine_skipped_no_ordinal: u32,
    pub cloud_skipped_without_ordinal: u32,
    pub stain_mesh_skipped_no_ordinal: u32,
    pub stain_mesh_skipped_no_illum: u32,
}

impl EffectDrawTrack {
    fn products(&mut self) -> [&mut Vec<HeldDrawItem>; 3] {
        [&mut self.colour, &mut self.emissive, &mut self.distortion]
    }

    fn reset(&mut self) {
        for product in self.products() {
            product.clear();
        }
        self.engine_sorted = 0;
        self.particle_cloud_ordered = 0;
        self.stain_mesh_sorted = 0;
        self.engine_skipped_no_ordinal = 0;
        self.cloud_skipped_without_ordinal = 0;
        self.stain_mesh_skipped_no_ordinal = 0;
        self.stain_mesh_skipped_no_illum = 0;
    }

    fn sort(&mut self) {
        for product in self.products() {
            product.sort_unstable_by_key(draw_order);
        }
    }
}

/// The three effect plans of a frame; any of them may be absent.
#[derive(Clone, Copy)]
struct Plans<'a> {
    marks: Option<&'a VisualStainMeshPlan>,
    code: Option<&'a FxCodeDrawPlan>,
    clouds: Option<&'a FxCloudDrawPlan>,
}

impl Plans<'_> {
    /// Fingerprint of everything that decides which items exist and in what order. Draws
    /// with no indices make no item and are left out.
    fn layout(&self) -> u64 {
        let mut id = CONTENT_KEY_SEED;
        let mut mix = |value: u64| blend_content_id(&mut id, value);
        if let Some(plan) = self.code {
            mix(plan.runs.len() as u64);
            for live in plan.live_runs() {
                let bucket = live.slot.map_or(0, |slot| slot.bin);
                let rank = live.slot.and_then(|slot| slot.sorted_rank);
                let words = [
                    live.index as u64,
                    live.run.surface_material_index.into(),
                    live.run.in_viewmodel.into(),
                    bucket.into(),
                    rank.map_or(u64::MAX, u64::from),
                ];
                words.into_iter().for_each(&mut mix);
            }
        }
        if let Some(plan) = self.clouds {
            mix(plan.runs.len() as u64);
            for live in plan.live_runs() {
                mix(live.index as u64);
                mix(live.run.surface_material_index.into());
            }
        }
        if let Some(plan) = self.marks {
            mix(plan.draws.len() as u64);
            for (i, draw_item) in plan.draws.iter().enumerate() {
                if draw_item.index_count == 0 {
                    continue;
                }
                let key = &draw_item.minor_key;
                mix(i as u64);
                mix(u64::from(draw_item.shader));
                mix(u64::from(key.baked_light_slot));
                mix(u64::from(key.ent_id.unwrap_or(u16::MAX)));
                mix(u64::from(key.smodel.unwrap_or(u16::MAX)));
                mix(u64::from(key.glass.unwrap_or(u16::MAX)));
                mix(u64::from(key.dominant_light));
                mix(u64::from(key.probe));
            }
        }
        id
    }

    /// One number for the revisions of all three plans.
    fn revision(&self) -> u64 {
        let mut id = CONTENT_KEY_SEED;
        blend_content_id(&mut id, self.code.map_or(0, |plan| plan.revision));
        blend_content_id(&mut id, self.clouds.map_or(0, |plan| plan.revision));
        blend_content_id(&mut id, self.marks.map_or(0, |plan| plan.revision));
        id
    }

    /// Copies the plans' current arguments into the items. Fails, leaving the caller to
    /// rebuild, when an item has no draw to read or is one whose lighting is not kept in it.
    fn refresh_in_place(&self, lane: &mut EffectDrawTrack) -> bool {
        for draw_entry in lane.products().into_iter().flatten() {
            match &mut draw_entry.kind {
                HeldDrawKind::CodeMesh {
                    draw,
                    n_args: arg_count,
                    arg_rows: args,
                    ..
                } => {
                    let Some(plan) = self.code else {
                        return false;
                    };
                    let Some(source) = plan.runs.get(*draw as usize) else {
                        return false;
                    };
                    (*arg_count, *args) = plan.shader_rows(source);
                }
                HeldDrawKind::SparkleSwarm { draw: draw_item, clouds, .. } => {
                    let Some(source) = self
                        .clouds
                        .and_then(|plan| plan.runs.get(*draw_item as usize))
                    else {
                        return false;
                    };
                    *clouds = source.descriptors;
                }
                HeldDrawKind::MarkMesh {
                    draw,
                    glass,
                    packed,
                    ..
                } => {
                    let Some(source) = self.marks.and_then(|plan| plan.draws.get(*draw as usize))
                    else {
                        return false;
                    };
                    *packed = source.minor_key.packed;
                    *glass = source.minor_key.glass.is_some();
                    if *glass {
                        return false;
                    }
                }
                HeldDrawKind::Glass { .. } => return false,
                _ => {}
            }
        }
        true
    }
}

/// Where a decal mark gets its lighting: the model it sits on, the glass piece, or the static
/// model it was cast onto.
struct MarkLighting<'a> {
    owner_of: &'a dyn Fn(u16) -> Option<Entity>,
    table: &'a ResolvedModelIllumTable,
    fixed_props: Option<&'a LevelPropIllum>,
}

impl MarkLighting<'_> {
    /// The lighting handle for a mark, or `None` when what it sits on has none yet. Marks on
    /// entities and glass also take their light and probe from the owner's seat.
    fn handle(&self, key: &mut DecalSubmeshKey) -> Option<u32> {
        if let Some(number) = key.ent_id {
            let owner = (self.owner_of)(number)?;
            return self.seat(ModelIllumOwner::ScriptModel(owner), key);
        }
        if let Some(piece) = key.glass {
            return self.seat(ModelIllumOwner::Pane(piece), key);
        }
        if let Some(index) = key.smodel {
            let handles = &self.fixed_props?.handle_list;
            return handles.get(usize::from(index)).copied().map(u32::from);
        }
        Some(0)
    }

    fn seat(&self, owner: ModelIllumOwner, key: &mut DecalSubmeshKey) -> Option<u32> {
        let ResolvedModelIllum::Seated {
            handle,
            stage_light_index,
            env_probe_index,
            ..
        } = self.table.answer_for(owner)?
        else {
            return None;
        };
        key.dominant_light = stage_light_index;
        key.probe = env_probe_index;
        Some(handle)
    }
}

/// A live run that has a sort rank.
struct Ranked<'a, R> {
    index: u32,
    run: &'a R,
    bucket: u8,
    rank: u32,
}

/// Ranks the live runs. Runs whose material has no ordinal are tallied in `skipped`.
fn rank_live<'a, R>(
    live: impl Iterator<Item = LiveRun<'a, R>>,
    skipped: &mut u32,
) -> Vec<Ranked<'a, R>> {
    live.filter_map(|LiveRun { index, run, slot }| {
        let slot = slot?;
        let Some(rank) = slot.sorted_rank else {
            *skipped = skipped.saturating_add(1);
            return None;
        };
        let index = index as u32;
        Some(Ranked { index, run, bucket: slot.bin, rank })
    })
    .collect()
}

fn add_code_meshes(plan: &FxCodeDrawPlan, catalog: &LiveMaterialLedger, draw_lane: &mut EffectDrawTrack) {
    let ranked = rank_live(plan.live_runs(), &mut draw_lane.engine_skipped_no_ordinal);
    for Ranked { index, run, bucket, rank } in ranked {
        let (arg_count, args) = plan.shader_rows(run);
        let kind = HeldDrawKind::CodeMesh {
            draw: index,
            gun_view: run.in_viewmodel,
            material: run.surface_material_index,
            n_args: arg_count,
            arg_rows: args,
        };
        let key = encode_engine_mesh_sort_key(bucket, sort_band(rank), index as u16).packed;
        let item = catalog_item(key, rank, kind, FacetSamplerInputs::default(), catalog);
        let warp_pass = Some(&mut draw_lane.distortion);
        route_to_products(&mut draw_lane.colour, &mut draw_lane.emissive, warp_pass, item);
        draw_lane.engine_sorted = draw_lane.engine_sorted.saturating_add(1);
    }
}

fn add_particle_clouds(
    plan: &FxCloudDrawPlan,
    catalog: &LiveMaterialLedger,
    draw_lane: &mut EffectDrawTrack,
) {
    let ranked = rank_live(plan.live_runs(), &mut draw_lane.cloud_skipped_without_ordinal);
    for Ranked { index, run, bucket, rank } in ranked {
        let kind = HeldDrawKind::SparkleSwarm {
            draw: index,
            material: run.surface_material_index,
            clouds: run.descriptors,
        };
        let key = encode_particle_cloud_sort_key(bucket, sort_band(rank), index as u16).packed;
        let item = catalog_item(key, rank, kind, FacetSamplerInputs::default(), catalog);
        route_to_products(&mut draw_lane.colour, &mut draw_lane.emissive, None, item);
        draw_lane.particle_cloud_ordered = draw_lane.particle_cloud_ordered.saturating_add(1);
    }
}

fn add_marks(
    plan: &VisualStainMeshPlan,
    lighting: &MarkLighting,
    catalog: &LiveMaterialLedger,
    draw_lane: &mut EffectDrawTrack,
) {
    for (i, draw_item) in plan.draws.iter().enumerate() {
        if draw_item.index_count == 0 {
            continue;
        }
        let Some(material) = plan.mat_list.get(draw_item.shader as usize) else {
            continue;
        };
        let Some(rank) = material.sorted_rank else {
            draw_lane.stain_mesh_skipped_no_ordinal = draw_lane.stain_mesh_skipped_no_ordinal.saturating_add(1);
            continue;
        };
        let mut minor_key = draw_item.minor_key;
        let Some(illum_handle) = lighting.handle(&mut minor_key) else {
            draw_lane.stain_mesh_skipped_no_illum =
                draw_lane.stain_mesh_skipped_no_illum.saturating_add(1);
            continue;
        };
        let key = pack_stain_mesh_facet_draw(
            material.bin,
            sort_band(rank),
            i as u16,
            minor_key.baked_light_slot,
            minor_key.dominant_light,
            minor_key.probe,
        )
        .packed;
        let draw = i as u32;
        let item = catalog_item(
            key,
            rank,
            HeldDrawKind::MarkMesh {
                glass: minor_key.glass.is_some(),
                draw,
                material: draw_item.shader,
                packed: minor_key.packed,
                illum_handle,
            },
            decal_mesh_surface_samplers(minor_key),
            catalog,
        );
        route_to_products(&mut draw_lane.colour, &mut draw_lane.emissive, None, item);
        draw_lane.stain_mesh_sorted = draw_lane.stain_mesh_sorted.saturating_add(1);
    }
}

pub(crate) fn refill_fx_draw_lane(
    plans: (
        Option<Res<FxCodeDrawPlan>>,
        Option<Res<FxCloudDrawPlan>>,
        Option<Res<VisualStainMeshPlan>>,
    ),
    generation: Res<MaterialEpoch>,
    mut lane: ResMut<EffectDrawTrack>,
    stain_owners: Query<(Entity, &LevelScriptModelInstance)>,
    lighting: Res<ResolvedModelIllumTable>,
    smodel_lighting: Option<Res<LevelPropIllum>>,
) {
    let plans = Plans {
        code: plans.0.as_deref(),
        clouds: plans.1.as_deref(),
        marks: plans.2.as_deref(),
    };
    let draw_lane = &mut *lane;
    let layout = plans.layout();
    let revision = plans.revision();
    if layout == draw_lane.layout && plans.refresh_in_place(draw_lane) {
        draw_lane.sort();
        draw_lane.payload_edition = revision;
        perf::Counter::EffectLayoutTopmost.emit(1.0);
        return;
    }
    perf::Counter::EffectLayoutTopmost.emit(0.0);

    draw_lane.reset();
    let owner_of = |number: u16| {
        stain_owners
            .iter()
            .find(|(_, owner)| owner.game_actor_number == Some(number))
            .map(|(ent, _)| ent)
    };
    let mark_lighting = MarkLighting {
        owner_of: &owner_of,
        table: &lighting,
        fixed_props: smodel_lighting.as_deref(),
    };
    let effect_book = &generation.catalog;
    if let Some(plan) = plans.code {
        add_code_meshes(plan, effect_book, draw_lane);
    }
    if let Some(plan) = plans.clouds {
        add_particle_clouds(plan, effect_book, draw_lane);
    }
    if let Some(plan) = plans.marks {
        add_marks(plan, &mark_lighting, effect_book, draw_lane);
    }
    draw_lane.sort();
    draw_lane.layout = layout;
    draw_lane.membership_edition = draw_lane.membership_edition.wrapping_add(1);
    draw_lane.payload_edition = revision;
}
