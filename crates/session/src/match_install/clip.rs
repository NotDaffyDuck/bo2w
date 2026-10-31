//! The map's collision: what the simulation traces against, built from the
//! clip data the asset side prepared.

use std::sync::Arc;

use asset_world::CollisionClipData;

use super::refusal::InstallRefusal;
use super::script_models::ModelInstall;

/// The map's collision in the simulation's own terms, and how much of it
/// there is.
pub(super) struct ClipParts {
    brushes: Vec<sim::SolidBrush>,
    bsp: sim::CollisionTreeData,
    mesh: sim::CollisionTriangleData,
    cmodels: sim::CollisionSubmodels,
    pub counts: ClipCounts,
}

/// The sizes the match log reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ClipCounts {
    pub solids: usize,
    pub bsp_nodes: usize,
    pub leaves: usize,
    pub vertex_points: usize,
    pub triangle_list: usize,
    pub submodels: usize,
    pub prop_set: usize,
}

impl ClipCounts {
    pub fn log_line(&self) -> String {
        format!(
            "{} clip brushes, {} BSP nodes / {} leaves, {} verts / {} tris, {} cmodels, {} smodels",
            self.solids,
            self.bsp_nodes,
            self.leaves,
            self.vertex_points,
            self.triangle_list,
            self.submodels,
            self.prop_set,
        )
    }
}

fn counts_of(clip: &CollisionClipData) -> ClipCounts {
    ClipCounts {
        solids: clip.brushes.len(),
        bsp_nodes: clip.nodes.len(),
        leaves: clip.leaves.len(),
        vertex_points: clip.t6.map_or(0, |map| map.vertex_points.len()),
        triangle_list: clip.t6.map_or(0, |map| map.tri_indices.len()),
        submodels: clip.cmodels.len(),
        prop_set: clip.t6_static.map_or(0, |world| world.models.len()),
    }
}

fn sim_brush(part: &asset_world::CollisionSolid) -> sim::SolidBrush {
    sim::SolidBrush {
        sides: part.planes.clone(),
        solid_bits: part.contents,
        plane_material_flags: part.plane_material_flags.clone(),
        glass_hit_code: part.glass_hit_code,
        vertex_points: part.vertex_points.clone(),
        hit_surface_flags: part.hit_surface_flags,
    }
}

fn sim_triggers(clip: &CollisionClipData) -> Vec<Vec<sim::SimTriggerHull>> {
    let hull = |hull: &asset_world::LevelTriggerVolume| sim::SimTriggerHull {
        mid: hull.mid,
        half: hull.half,
        planes: hull.slabs.clone(),
    };
    clip.trigger_models
        .iter()
        .map(|collision_volumes| collision_volumes.iter().map(hull).collect())
        .collect()
}

/// Take the collision the match is to run on. A match with no solid world, or
/// an empty one, is refused: nothing could stand on it.
pub(super) fn clip_parts(loaded: Option<Arc<CollisionClipData>>) -> Result<ClipParts, InstallRefusal> {
    let Some(clip) = loaded else {
        return Err(InstallRefusal::new(
            "Required collision geometry is missing",
        ));
    };
    let counts = counts_of(&clip);
    if counts.solids == 0 && counts.triangle_list == 0 {
        return Err(InstallRefusal::new("Required collision geometry is empty"));
    }
    let brushes: Vec<sim::SolidBrush> = clip.brushes.iter().map(sim_brush).collect();
    let bsp = sim::CollisionTreeData {
        brush_leaf_indices: clip.brush_leaf_indices.clone(),
        t6: clip.t6,
        t6_glass: sim::CollisionTreeData::glass_of(&brushes),
        t6_model_brushes: clip.t6_model_brushes.clone(),
    };
    let cmodels = sim::CollisionSubmodels {
        models: clip.cmodels.clone(),
        triggers: sim_triggers(&clip),
        model_cmodels: clip.model_cmodels.clone(),
    };
    Ok(ClipParts {
        brushes,
        bsp,
        mesh: sim::CollisionTriangleData {
            t6_static: clip.t6_static,
        },
        cmodels,
        counts,
    })
}

/// Give the simulation its collision and the models that stand in it, and
/// pose those once so their collision exists before the first tick.
pub(super) fn install_collision(
    running: &mut sim::SimWorld,
    mut content: sim::SimContentBuilder,
    clip: ClipParts,
    models: &mut ModelInstall,
) {
    content.set_clip_map(clip.brushes, clip.bsp, clip.mesh, clip.cmodels);
    running.install_content(content.finish());
    running.install_model_library(std::mem::take(&mut models.library));
    let owners = models.abilities.len();
    let attached_solids: usize = models
        .abilities
        .iter()
        .map(|owner| owner.attached_solids.len())
        .sum();
    running.install_entity_collision_capabilities(std::mem::take(&mut models.abilities));
    sim::stage_build_entity_models(running);
    let collision_bones: usize = running
        .entity_hit_support()
        .iter()
        .filter_map(|owner| owner.dobj.as_ref()?.live_hitbox.as_ref())
        .map(|shape| shape.bones.len())
        .sum();
    diag::info!(
        Sim,
        "spawn: {owners} script collision owners, {collision_bones} nonzero XBoneInfo OBBs, {attached_solids} linked brush cmodels, {} ambiguous brush links, {} standalone brush cmodels",
        models.ambiguous_links,
        models.brush_movers.len(),
    );
    diag::info!(Sim, "spawn: {}", clip.counts.log_line());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush() -> asset_world::CollisionSolid {
        asset_world::CollisionSolid {
            planes: vec![],
            contents: 0,
            plane_material_flags: Vec::new(),
            glass_hit_code: 0,
            vertex_points: Vec::new(),
            hit_surface_flags: 0,
        }
    }

    #[test]
    fn no_collision_is_refused() {
        let refusal = clip_parts(None).err().unwrap();
        assert_eq!(refusal.error, "Required collision geometry is missing");
    }

    #[test]
    fn empty_collision_is_refused() {
        let refusal = clip_parts(Some(Arc::new(CollisionClipData::default())))
            .err()
            .unwrap();
        assert_eq!(refusal.error, "Required collision geometry is empty");
    }

    #[test]
    fn a_map_with_brushes_is_taken_and_counted() {
        let clip = CollisionClipData {
            brushes: vec![brush(), brush()],
            brush_leaf_indices: vec![0, 1],
            ..Default::default()
        };
        let parts = clip_parts(Some(Arc::new(clip))).ok().unwrap();
        assert_eq!(parts.brushes.len(), 2);
        assert_eq!(parts.bsp.brush_leaf_indices, vec![0, 1]);
        assert_eq!(parts.counts.solids, 2);
        assert_eq!(parts.counts.triangle_list, 0);
    }

    #[test]
    fn the_log_line_names_every_count() {
        let counts = ClipCounts {
            solids: 1,
            bsp_nodes: 2,
            leaves: 3,
            vertex_points: 4,
            triangle_list: 5,
            submodels: 6,
            prop_set: 7,
        };
        assert_eq!(
            counts.log_line(),
            "1 clip brushes, 2 BSP nodes / 3 leaves, 4 verts / 5 tris, 6 cmodels, 7 smodels"
        );
    }

    #[test]
    fn trigger_hulls_keep_their_shape() {
        let hull = asset_world::LevelTriggerVolume {
            mid: [1.0, 2.0, 3.0],
            half: [4.0, 5.0, 6.0],
            slabs: Vec::new(),
        };
        let clip = CollisionClipData {
            trigger_models: vec![vec![hull], vec![]],
            ..Default::default()
        };
        let triggers = sim_triggers(&clip);
        assert_eq!(triggers.len(), 2);
        assert_eq!(triggers[0][0].mid, [1.0, 2.0, 3.0]);
        assert_eq!(triggers[0][0].half, [4.0, 5.0, 6.0]);
        assert!(triggers[1].is_empty());
    }
}
