//! The map's collision: what the simulation traces against, built from the
//! clip data the asset side prepared.

use std::sync::Arc;

use asset_world::ClipCollision;

use super::refusal::InstallRefusal;
use super::script_models::ModelInstall;

/// The map's collision in the simulation's own terms, and how much of it
/// there is.
pub(super) struct ClipParts {
    brushes: Vec<sim::SimBrush>,
    bsp: sim::SimClipBsp,
    mesh: sim::SimClipMesh,
    cmodels: sim::SimClipCmodels,
    pub counts: ClipCounts,
}

/// The sizes the match log reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ClipCounts {
    pub brushes: usize,
    pub nodes: usize,
    pub leaves: usize,
    pub verts: usize,
    pub tris: usize,
    pub cmodels: usize,
    pub static_models: usize,
}

impl ClipCounts {
    pub fn log_line(&self) -> String {
        format!(
            "{} clip brushes, {} BSP nodes / {} leaves, {} verts / {} tris, {} cmodels, {} smodels",
            self.brushes,
            self.nodes,
            self.leaves,
            self.verts,
            self.tris,
            self.cmodels,
            self.static_models,
        )
    }
}

fn counts_of(clip: &ClipCollision) -> ClipCounts {
    ClipCounts {
        brushes: clip.brushes.len(),
        nodes: clip.nodes.len(),
        leaves: clip.leaves.len(),
        verts: clip.t6.map_or(0, |map| map.verts.len()),
        tris: clip.t6.map_or(0, |map| map.tri_indices.len()),
        cmodels: clip.cmodels.len(),
        static_models: clip.t6_static.map_or(0, |world| world.models.len()),
    }
}

fn sim_brush(brush: &asset_world::ClipBrush) -> sim::SimBrush {
    sim::SimBrush {
        planes: brush.planes.clone(),
        contents: brush.contents,
        plane_surface_flags: brush.plane_surface_flags.clone(),
        glass_encoded: brush.glass_encoded,
        verts: brush.verts.clone(),
        hit_surface_flags: brush.hit_surface_flags,
    }
}

fn sim_triggers(clip: &ClipCollision) -> Vec<Vec<sim::SimTriggerHull>> {
    let hull = |hull: &asset_world::MapTriggerHull| sim::SimTriggerHull {
        mid: hull.mid,
        half: hull.half,
        slabs: hull.slabs.clone(),
    };
    clip.trigger_models
        .iter()
        .map(|hulls| hulls.iter().map(hull).collect())
        .collect()
}

/// Take the collision the match is to run on. A match with no solid world, or
/// an empty one, is refused: nothing could stand on it.
pub(super) fn clip_parts(clip: Option<Arc<ClipCollision>>) -> Result<ClipParts, InstallRefusal> {
    let Some(clip) = clip else {
        return Err(InstallRefusal::new(
            "Required collision geometry is missing",
        ));
    };
    let counts = counts_of(&clip);
    if counts.brushes == 0 && counts.tris == 0 {
        return Err(InstallRefusal::new("Required collision geometry is empty"));
    }
    let brushes: Vec<sim::SimBrush> = clip.brushes.iter().map(sim_brush).collect();
    let bsp = sim::SimClipBsp {
        leafbrushes: clip.leafbrushes.clone(),
        t6: clip.t6,
        t6_glass: sim::SimClipBsp::glass_of(&brushes),
    };
    let cmodels = sim::SimClipCmodels {
        models: clip.cmodels.clone(),
        triggers: sim_triggers(&clip),
        model_cmodels: clip.model_cmodels.clone(),
    };
    Ok(ClipParts {
        brushes,
        bsp,
        mesh: sim::SimClipMesh {
            t6_static: clip.t6_static,
        },
        cmodels,
        counts,
    })
}

/// Give the simulation its collision and the models that stand in it, and
/// pose those once so their collision exists before the first tick.
pub(super) fn install_collision(
    sim: &mut sim::SimWorld,
    mut content: sim::SimContentBuilder,
    clip: ClipParts,
    models: &mut ModelInstall,
) {
    content.set_clip_map(clip.brushes, clip.bsp, clip.mesh, clip.cmodels);
    sim.install_content(content.finish());
    sim.install_model_library(std::mem::take(&mut models.library));
    let owners = models.capabilities.len();
    let linked_brushes: usize = models
        .capabilities
        .iter()
        .map(|owner| owner.linked_brushes.len())
        .sum();
    sim.install_entity_collision_capabilities(std::mem::take(&mut models.capabilities));
    sim::phase_materialize_entity_dobjs(sim);
    let collision_bones: usize = sim
        .entity_collision_capabilities()
        .iter()
        .filter_map(|owner| owner.dobj.as_ref()?.current_collision.as_ref())
        .map(|geometry| geometry.bones.len())
        .sum();
    diag::info!(
        Sim,
        "spawn: {owners} script collision owners, {collision_bones} nonzero XBoneInfo OBBs, {linked_brushes} linked brush cmodels, {} ambiguous brush links, {} standalone brush cmodels",
        models.ambiguous_links,
        models.brush_movers.len(),
    );
    diag::info!(Sim, "spawn: {}", clip.counts.log_line());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush() -> asset_world::ClipBrush {
        asset_world::ClipBrush {
            planes: Vec::new(),
            contents: 0,
            plane_surface_flags: Vec::new(),
            glass_encoded: 0,
            verts: Vec::new(),
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
        let refusal = clip_parts(Some(Arc::new(ClipCollision::default())))
            .err()
            .unwrap();
        assert_eq!(refusal.error, "Required collision geometry is empty");
    }

    #[test]
    fn a_map_with_brushes_is_taken_and_counted() {
        let clip = ClipCollision {
            brushes: vec![brush(), brush()],
            leafbrushes: vec![0, 1],
            ..Default::default()
        };
        let parts = clip_parts(Some(Arc::new(clip))).ok().unwrap();
        assert_eq!(parts.brushes.len(), 2);
        assert_eq!(parts.bsp.leafbrushes, vec![0, 1]);
        assert_eq!(parts.counts.brushes, 2);
        assert_eq!(parts.counts.tris, 0);
    }

    #[test]
    fn the_log_line_names_every_count() {
        let counts = ClipCounts {
            brushes: 1,
            nodes: 2,
            leaves: 3,
            verts: 4,
            tris: 5,
            cmodels: 6,
            static_models: 7,
        };
        assert_eq!(
            counts.log_line(),
            "1 clip brushes, 2 BSP nodes / 3 leaves, 4 verts / 5 tris, 6 cmodels, 7 smodels"
        );
    }

    #[test]
    fn trigger_hulls_keep_their_shape() {
        let hull = asset_world::MapTriggerHull {
            mid: [1.0, 2.0, 3.0],
            half: [4.0, 5.0, 6.0],
            slabs: Vec::new(),
        };
        let clip = ClipCollision {
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
