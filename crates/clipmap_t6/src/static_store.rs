//! bo2mp: a map's static models (`clipMap_t::staticModelList`) and the
//! collision of the models they place, owned. A map loader reads them once,
//! then keeps them for as long as the game runs, filed in a world of their
//! own (`CM_LinkAllStaticModels`) for the point traces. Collision triangles
//! and surfaces sit in one table each, and each surface and model names its
//! run of the one before.

use alloc::boxed::Box;
use alloc::vec::Vec;

use trace_t6::Trace;

use crate::{
    ClipMap, LinkedEntities, StaticModel, StoreError, World, XModelColl, XModelCollSurf, ModelHitTriangle,
};

/// `XModelCollSurf_s`, with its triangles as `tri_tally` entries of
/// `StaticModelStore::triangle_list` from `leading_triangle`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StoredCollSurf {
    pub leading_triangle: u32,
    pub tri_tally: u32,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub joint_position: i32,
    pub contents: u32,
    pub face_property_bits: u32,
}

/// An `XModel`'s collision: its `collLod` and its surfaces as `facet_amount`
/// entries of `StaticModelStore::surfs` from `lead_skin`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StoredXModel {
    pub collision_detail_tier: i32,
    pub lead_skin: u32,
    pub facet_amount: u32,
}

/// `cStaticModel_s`, its model an entry of `StaticModelStore::xmodels`
/// (none when the placement has no model).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StoredStaticModel {
    pub xmodel: Option<u32>,
    pub contents: u32,
    pub origin: [f32; 3],
    pub reverse_scaled_basis: [[f32; 3]; 3],
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
}

/// A map's static models and their models' collision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StaticModelStore {
    pub triangle_list: Vec<ModelHitTriangle>,
    pub surfs: Vec<StoredCollSurf>,
    pub xmodels: Vec<StoredXModel>,
    pub models: Vec<StoredStaticModel>,
}

/// The static models filed in a world of their own: what
/// `CM_PointTraceStaticModels` walks.
#[derive(Debug)]
pub struct StaticWorld {
    pub models: &'static [StaticModel<'static>],
    pub world: Box<World>,
}

/// No entities: the static world files static models only.
struct NoEntities;

impl LinkedEntities for NoEntities {
    fn contents(&self, _num: usize) -> u32 {
        0
    }

    fn abs_bounds(&self, _num: usize) -> ([f32; 3], [f32; 3]) {
        ([0.0; 3], [0.0; 3])
    }
}

fn in_run(first: u32, count: u32, len: usize) -> bool {
    (first as usize).checked_add(count as usize).is_some_and(|end| end <= len)
}

impl StaticModelStore {
    /// Every run and model index lands inside its table, and the sectors
    /// can number every model (one more than its number fits a `u16`).
    pub fn check(&self) -> Result<(), StoreError> {
        let bad = |table, index| Err(StoreError { table, index });
        for (i, s) in self.surfs.iter().enumerate() {
            if !in_run(s.leading_triangle, s.tri_tally, self.triangle_list.len()) {
                return bad("static model surfs", i);
            }
        }
        for (i, m) in self.xmodels.iter().enumerate() {
            if !in_run(m.lead_skin, m.facet_amount, self.surfs.len()) {
                return bad("static model xmodels", i);
            }
        }
        if self.models.len() >= usize::from(u16::MAX) {
            return bad("static models", self.models.len());
        }
        for (i, m) in self.models.iter().enumerate() {
            if m.xmodel.is_some_and(|x| x as usize >= self.xmodels.len()) {
                return bad("static models", i);
            }
        }
        Ok(())
    }

    /// Keep these tables for the rest of the run, filed in a world over
    /// `cm`'s world model (`CM_ClearWorld`, then `CM_LinkAllStaticModels`).
    /// Each call keeps one more copy, so a loader calls it once a map.
    #[must_use]
    pub fn leak(self, cm: &ClipMap<'_>) -> &'static StaticWorld {
        let triangle_list: &'static [ModelHitTriangle] = Box::leak(self.triangle_list.into_boxed_slice());
        let surfs: &'static [XModelCollSurf<'static>] = Box::leak(
            self.surfs
                .iter()
                .map(|s| XModelCollSurf {
                    triangle_list: &triangle_list[s.leading_triangle as usize..][..s.tri_tally as usize],
                    mins: s.mins,
                    maxs: s.maxs,
                    joint_position: s.joint_position,
                    contents: s.contents,
                    face_property_bits: s.face_property_bits,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let xmodels: &'static [XModelColl<'static>] = Box::leak(
            self.xmodels
                .iter()
                .map(|m| XModelColl {
                    collision_detail_tier: m.collision_detail_tier,
                    surfs: &surfs[m.lead_skin as usize..][..m.facet_amount as usize],
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let mut models: Vec<StaticModel<'static>> = self
            .models
            .iter()
            .map(|m| StaticModel {
                next_model_in_world_sector: 0,
                xmodel: m.xmodel.map(|x| &xmodels[x as usize]),
                contents: m.contents,
                origin: m.origin,
                reverse_scaled_basis: m.reverse_scaled_basis,
                absmin: m.absmin,
                absmax: m.absmax,
            })
            .collect();
        let mut world = Box::new(World::new());
        world.link_world(cm, &NoEntities, &mut models);
        Box::leak(Box::new(StaticWorld {
            models: Box::leak(models.into_boxed_slice()),
            world,
        }))
    }
}

/// A static model a point trace hit: its number, the fraction along the
/// line, the world normal and the hit surface's flags and contents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticHit {
    pub model: u32,
    pub fraction: f32,
    pub normal: [f32; 3],
    pub sflags: u32,
    pub cflags: u32,
}

impl StaticWorld {
    /// `CM_PointTraceStaticModels` over these models.
    pub fn point_trace(&self, hit: &mut Trace, from_pt: [f32; 3], to_pt: [f32; 3], want_bits: u32) {
        self.world
            .point_trace_static_models(self.models, hit, from_pt, to_pt, want_bits);
    }

    /// The nearest static model the line `start`..`end` meets before
    /// `fraction`, if any.
    #[must_use]
    pub fn nearest_hit(&self, from_pt: [f32; 3], to_pt: [f32; 3], frac_at: f32, want_bits: u32) -> Option<StaticHit> {
        let mut tr_out = Trace::new();
        tr_out.fraction = frac_at;
        self.point_trace(&mut tr_out, from_pt, to_pt, want_bits);
        tr_out.static_model.map(|model| StaticHit {
            model,
            fraction: tr_out.fraction,
            normal: tr_out.normal,
            sflags: tr_out.sflags,
            cflags: tr_out.cflags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CModel, reverse_scaled_basis};

    // A wall triangle at local x = 0 facing -x over y and z 0 to 10.
    const WALL: ModelHitTriangle = ModelHitTriangle {
        plane: [-1.0, 0.0, 0.0, 0.0],
        surface_tangent: [0.0, 0.1, 0.0, 0.0],
        surface_bitangent: [0.0, 0.0, 0.1, 0.0],
    };

    fn placed(xmodel: Option<u32>, x: f32) -> StoredStaticModel {
        StoredStaticModel {
            xmodel,
            contents: 1,
            origin: [x, 0.0, 0.0],
            reverse_scaled_basis: reverse_scaled_basis([0.0; 3], 1.0),
            absmin: [x - 12.0, -12.0, -12.0],
            absmax: [x + 12.0, 12.0, 12.0],
        }
    }

    fn store() -> StaticModelStore {
        StaticModelStore {
            triangle_list: alloc::vec![ModelHitTriangle::default(), WALL],
            surfs: alloc::vec![StoredCollSurf {
                leading_triangle: 1,
                tri_tally: 1,
                mins: [-1.0; 3],
                maxs: [10.0; 3],
                joint_position: 0,
                contents: 1,
                face_property_bits: 0x50,
            }],
            xmodels: alloc::vec![StoredXModel {
                collision_detail_tier: 0,
                lead_skin: 0,
                facet_amount: 1,
            }],
            models: alloc::vec![placed(None, -300.0), placed(Some(0), 200.0), placed(Some(0), -100.0)],
        }
    }

    #[test]
    fn a_kept_store_is_filed_and_traced_to_its_nearest_model() {
        let store = store();
        assert_eq!(store.check(), Ok(()));
        let cmodels = [CModel {
            mins: [-4096.0, -1024.0, -512.0],
            maxs: [4096.0, 1024.0, 512.0],
            ..CModel::default()
        }];
        let cm = ClipMap {
            cmodels: &cmodels,
            ..ClipMap::default()
        };
        let kept = store.leak(&cm);
        assert_eq!(kept.models.len(), 3);
        assert_eq!(kept.world.sectors[1].children, [3, 2]);

        let mut trace = Trace::new();
        kept.point_trace(&mut trace, [-400.0, 2.0, 2.0], [400.0, 2.0, 2.0], 1);
        assert_eq!(trace.static_model, Some(2));
        assert_eq!(trace.sflags, 0x50);
        let mut tr_out = Trace::new();
        kept.point_trace(&mut tr_out, [0.0, 2.0, 2.0], [400.0, 2.0, 2.0], 1);
        assert_eq!(tr_out.static_model, Some(1));
        let hit = kept.nearest_hit([0.0, 2.0, 2.0], [400.0, 2.0, 2.0], 1.0, 1).unwrap();
        assert_eq!((hit.model, hit.sflags, hit.cflags), (1, 0x50, 1));
        assert_eq!(hit.fraction, tr_out.fraction);
        assert_eq!(kept.nearest_hit([0.0, 2.0, 2.0], [400.0, 2.0, 2.0], hit.fraction * 0.5, 1), None);
    }

    #[test]
    fn check_names_the_first_row_out_of_its_table() {
        let mut s = store();
        s.surfs[0].tri_tally = 2;
        assert_eq!(s.check(), Err(StoreError { table: "static model surfs", index: 0 }));
        let mut s = store();
        s.xmodels[0].lead_skin = 1;
        assert_eq!(s.check(), Err(StoreError { table: "static model xmodels", index: 0 }));
        let mut s = store();
        s.models[2].xmodel = Some(1);
        assert_eq!(s.check(), Err(StoreError { table: "static models", index: 2 }));
        let mut s = store();
        s.models = alloc::vec![StoredStaticModel::default(); usize::from(u16::MAX)];
        assert_eq!(s.check(), Err(StoreError { table: "static models", index: 65535 }));
    }
}
