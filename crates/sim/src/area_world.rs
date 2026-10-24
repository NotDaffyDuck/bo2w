//! bo2mp: the sim's area entities, filed in Black Ops II's sectors
//! (`cm_world_t`: `CM_LinkEntity`, `CM_UnlinkEntity`, `CM_AreaEntities`),
//! with the contents and box each entity was linked with (`gentity_t`'s
//! `r.contents`, `r.absmin` and `r.absmax`).

use clipmap_t6::{CModel, ClipMap, LINKED_ENTITIES, LinkedEntities, World};

/// The contents and box an entity was linked with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AreaEntity {
    pub(crate) contents: u32,
    pub(crate) absmin: [f32; 3],
    pub(crate) absmax: [f32; 3],
}

/// A box by its corners, as the area queries take it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AreaBox {
    pub(crate) mins: [f32; 3],
    pub(crate) maxs: [f32; 3],
}

impl AreaBox {
    /// The box `half` out from `mid` each way; `None` when it is not a box.
    pub(crate) fn from_mid_half(mid: [f32; 3], half: [f32; 3]) -> Option<Self> {
        let ok = (0..3).all(|i| mid[i].is_finite() && half[i].is_finite() && half[i] >= 0.0);
        ok.then(|| Self {
            mins: [0, 1, 2].map(|i| mid[i] - half[i]),
            maxs: [0, 1, 2].map(|i| mid[i] + half[i]),
        })
    }

    pub(crate) fn mid(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| (self.mins[i] + self.maxs[i]) * 0.5)
    }

    pub(crate) fn half(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| (self.maxs[i] - self.mins[i]) * 0.5)
    }

    pub(crate) fn is_valid(&self) -> bool {
        (0..3).all(|i| self.mins[i].is_finite() && self.maxs[i].is_finite() && self.mins[i] <= self.maxs[i])
    }
}

struct Entities<'a>(&'a [AreaEntity]);

impl LinkedEntities for Entities<'_> {
    fn contents(&self, num: usize) -> u32 {
        self.0[num].contents
    }

    fn abs_bounds(&self, num: usize) -> ([f32; 3], [f32; 3]) {
        (self.0[num].absmin, self.0[num].absmax)
    }
}

/// BO2's sectors with the linked entities beside them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AreaWorld {
    pub(crate) world: Box<World>,
    pub(crate) entities: Vec<AreaEntity>,
}

impl AreaWorld {
    /// `CM_ClearWorld` over a world model of `bounds`.
    pub(crate) fn new(bounds: AreaBox) -> Self {
        let mut world = Box::new(World::new());
        let models = [CModel {
            mins: bounds.mins,
            maxs: bounds.maxs,
            ..CModel::default()
        }];
        world.clear(&ClipMap {
            cmodels: &models,
            ..ClipMap::default()
        });
        Self {
            world,
            entities: vec![AreaEntity::default(); LINKED_ENTITIES],
        }
    }

    /// `CM_LinkEntity` for entity `num`, a box of `contents`. BO2 unknown:
    /// which clip model its server links a player with; this keeps MW2's
    /// (`SV_ClipHandleForEntity`: the temporary box, `CM_TempBoxModel`).
    pub(crate) fn link(&mut self, num: usize, contents: u32, bounds: AreaBox) {
        self.entities[num] = AreaEntity {
            contents,
            absmin: bounds.mins,
            absmax: bounds.maxs,
        };
        let mut cm = ClipMap::default();
        let model = cm.temp_box_model(bounds.mins, bounds.maxs, contents);
        self.world.link_entity(
            &cm,
            &Entities(&self.entities),
            &mut [],
            num,
            bounds.mins,
            bounds.maxs,
            model,
        );
    }

    /// `CM_UnlinkEntity`; whether `num` was linked.
    pub(crate) fn unlink(&mut self, num: usize) -> bool {
        let linked = self.world.entities[num].world_sector != 0;
        self.world.unlink_entity(&Entities(&self.entities), num);
        self.entities[num] = AreaEntity::default();
        linked
    }

    /// Link `num` again moved by `delta`; whether it was linked.
    pub(crate) fn translate(&mut self, num: usize, delta: [f32; 3]) -> bool {
        let Some(bounds) = self.bounds(num) else {
            return false;
        };
        let moved = AreaBox {
            mins: [0, 1, 2].map(|i| bounds.mins[i] + delta[i]),
            maxs: [0, 1, 2].map(|i| bounds.maxs[i] + delta[i]),
        };
        self.link(num, self.entities[num].contents, moved);
        true
    }

    /// The box `num` is linked with.
    pub(crate) fn bounds(&self, num: usize) -> Option<AreaBox> {
        (self.world.entities.get(num)?.world_sector != 0).then(|| AreaBox {
            mins: self.entities[num].absmin,
            maxs: self.entities[num].absmax,
        })
    }

    /// `CM_AreaEntities`: up to `capacity` entities with `mask` touching
    /// `bounds`.
    // TODO(t6-cm-world-merge): BO2 files the static models and the entities
    // in one `cm_world_t`; here the static models have a world of their own
    // (clipmap_t6::StaticWorld), so this world's sectors split on entities
    // alone and the order found can differ from BO2's. The test
    // `area_entities_come_back_in_todays_order` pins today's order.
    pub(crate) fn query(&self, bounds: AreaBox, mask: u32, capacity: usize) -> Vec<u16> {
        let mut list = vec![0; capacity];
        let n = self
            .world
            .area_entities(&Entities(&self.entities), bounds.mins, bounds.maxs, mask, &mut list);
        list[..n].iter().map(|&num| num as u16).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(at: [f32; 3], half: f32) -> AreaBox {
        AreaBox::from_mid_half(at, [half; 3]).unwrap()
    }

    #[test]
    fn a_linked_box_is_found_moved_and_unlinked() {
        let mut area = AreaWorld::new(cube([0.0; 3], 4096.0));
        area.link(5, 0x0200_0000, cube([100.0, 0.0, 0.0], 16.0));
        area.link(9, 0x0200_0000, cube([-2000.0, 0.0, 0.0], 16.0));
        assert_eq!(area.query(cube([100.0, 0.0, 0.0], 8.0), u32::MAX, 16), vec![5]);
        assert!(area.translate(5, [-2100.0, 0.0, 0.0]));
        let mut near = area.query(cube([-2000.0, 0.0, 0.0], 8.0), u32::MAX, 16);
        near.sort_unstable();
        assert_eq!(near, vec![5, 9]);
        assert!(area.unlink(9));
        assert!(!area.unlink(9));
        assert_eq!(area.query(cube([-2000.0, 0.0, 0.0], 8.0), u32::MAX, 16), vec![5]);
        assert_eq!(area.bounds(9), None);
    }

    /// Pins the order `query` returns today (see TODO(t6-cm-world-merge)):
    /// a change to how the sectors are split or walked shows up here.
    #[test]
    fn area_entities_come_back_in_todays_order() {
        let mut area = AreaWorld::new(cube([0.0; 3], 4096.0));
        for (num, at) in [(5, 100.0), (7, 0.0), (9, -2000.0), (3, 120.0), (11, 3000.0), (2, -100.0)] {
            area.link(num, 0x0200_0000, cube([at, 0.0, 0.0], 16.0));
        }
        assert_eq!(area.query(cube([0.0; 3], 4000.0), u32::MAX, 16), vec![2, 3, 5, 7, 9, 11]);
    }
}
