//! BO2's world sectors (`cm_world_t`): a tree of up to 1024 sectors that
//! files the linked entities (`CM_LinkEntity`, `CM_UnlinkEntity`) and the
//! static models (`CM_LinkAllStaticModels`) by where they stand. A sector
//! splits in two across its longer side as things are filed in it
//! (`CM_SortNode`), and `CM_AreaEntities` walks it.

use crate::{ClipMap, XModelColl};

/// Sectors in the tree: sector 0 is none and sector 1 the root.
pub const WORLD_SECTORS: usize = 1024;
/// Entities the server can link (`sv.svEntities`).
pub const LINKED_ENTITIES: usize = 1024;
/// A sector whose longer side is no longer than this is not split.
pub const SECTOR_SPLIT_MIN: f32 = 512.0;

/// `worldSector_s`: what a sector holds and where it splits. A child of 0
/// is none. While the sector is free, `parent` is the next free one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldSector {
    pub contents_static_models: u32,
    pub contents_entities: u32,
    /// One more than the first entity filed here (0 for none).
    pub entities: u16,
    /// One more than the first static model filed here (0 for none).
    pub static_models: u16,
    pub dist: f32,
    pub axis: u16,
    pub parent: u16,
    /// The side past `dist`, then the side short of it.
    pub children: [u16; 2],
}

impl WorldSector {
    const EMPTY: Self = Self {
        contents_static_models: 0,
        contents_entities: 0,
        entities: 0,
        static_models: 0,
        dist: 0.0,
        axis: 0,
        parent: 0,
        children: [0; 2],
    };
}

/// `svEntity_s`: the sector an entity is filed in (0 for none), the next
/// entity there (one more than its number) and the box and contents it was
/// linked with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvEntity {
    pub world_sector: u16,
    pub next_entity_in_world_sector: u16,
    pub link_contents: u32,
    pub link_min: [f32; 3],
    pub link_max: [f32; 3],
}

impl SvEntity {
    const EMPTY: Self = Self {
        world_sector: 0,
        next_entity_in_world_sector: 0,
        link_contents: 0,
        link_min: [0.0; 3],
        link_max: [0.0; 3],
    };

    fn set_link(&mut self, contents: u32, mins: [f32; 3], maxs: [f32; 3]) {
        self.link_contents = contents;
        self.link_min = mins;
        self.link_max = maxs;
    }
}

/// `cStaticModel_s`: the next static model in its sector (one more than
/// its number, `cStaticModelWritable`), its collision model, contents,
/// place, inverse scaled axis (`inv_scaled_axis`) and box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StaticModel<'a> {
    pub next_model_in_world_sector: u16,
    pub xmodel: Option<&'a XModelColl<'a>>,
    pub contents: u32,
    pub origin: [f32; 3],
    pub inv_scaled_axis: [[f32; 3]; 3],
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
}

/// What the sectors read from the game's entities: `gentity_t`'s
/// `r.contents`, `r.absmin` and `r.absmax`.
pub trait LinkedEntities {
    fn contents(&self, num: usize) -> u32;
    fn abs_bounds(&self, num: usize) -> ([f32; 3], [f32; 3]);
}

/// `cm_world_t`, with the server's `svEntities` beside it.
#[derive(Clone, Debug, PartialEq)]
pub struct World {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub free_head: u16,
    pub sectors: [WorldSector; WORLD_SECTORS],
    /// `sv.svEntities`, which `clear` leaves as they are.
    pub entities: [SvEntity; LINKED_ENTITIES],
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// All zero, as BO2's globals start.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mins: [0.0; 3],
            maxs: [0.0; 3],
            free_head: 0,
            sectors: [WorldSector::EMPTY; WORLD_SECTORS],
            entities: [SvEntity::EMPTY; LINKED_ENTITIES],
        }
    }

    /// `CM_ClearWorld`: empty the tree, take the world model's bounds and
    /// split the root across their longer side; sectors 2 on are free.
    pub fn clear(&mut self, cm: &ClipMap<'_>) {
        (self.mins, self.maxs) = cm.brush_model(0).map_or(([0.0; 3], [0.0; 3]), |m| (m.mins, m.maxs));
        self.sectors = [WorldSector::EMPTY; WORLD_SECTORS];
        self.free_head = 2;
        for (i, sector) in self.sectors.iter_mut().enumerate().take(WORLD_SECTORS - 1).skip(2) {
            sector.parent = i as u16 + 1;
        }
        let axis = usize::from(self.maxs[0] - self.mins[0] <= self.maxs[1] - self.mins[1]);
        self.sectors[1].axis = axis as u16;
        self.sectors[1].dist = (self.maxs[axis] + self.mins[axis]) * 0.5;
    }

    /// `CM_LinkWorld`: clear the tree and file every static model in it.
    pub fn link_world(&mut self, cm: &ClipMap<'_>, game: &impl LinkedEntities, models: &mut [StaticModel<'_>]) {
        self.clear(cm);
        self.link_static_models(game, models);
    }

    /// `CM_LinkAllStaticModels`: file each static model that has contents
    /// in the deepest sector it fits on one side of.
    pub fn link_static_models(&mut self, game: &impl LinkedEntities, models: &mut [StaticModel<'_>]) {
        for i in 0..models.len() {
            let StaticModel {
                contents,
                absmin,
                absmax,
                ..
            } = models[i];
            if contents == 0 {
                continue;
            }
            let mut mins2 = [self.mins[0], self.mins[1]];
            let mut maxs2 = [self.maxs[0], self.maxs[1]];
            let mut node = 1;
            loop {
                let s = &mut self.sectors[node];
                s.contents_static_models |= contents;
                let axis = s.axis as usize;
                let child = if !(absmin[axis] <= s.dist) {
                    mins2[axis] = s.dist;
                    s.children[0]
                } else if s.dist <= absmax[axis] {
                    break;
                } else {
                    maxs2[axis] = s.dist;
                    s.children[1]
                };
                if child == 0 {
                    break;
                }
                node = child as usize;
            }
            self.add_static_model_to_node(models, node, i);
            self.sort_node(game, models, node, mins2, maxs2);
        }
    }

    /// `CM_LinkEntity`: file entity `num` by its box `absmin`..`absmax`,
    /// with the contents of clip model `model`; an entity whose model has
    /// none is unlinked.
    #[allow(clippy::too_many_arguments)]
    pub fn link_entity(
        &mut self,
        cm: &ClipMap<'_>,
        game: &impl LinkedEntities,
        models: &mut [StaticModel<'_>],
        num: usize,
        absmin: [f32; 3],
        absmax: [f32; 3],
        model: u32,
    ) {
        let link_contents = cm
            .brush_model(model)
            .map_or(0, |m| m.leaf.terrain_contents | m.leaf.brush_contents);
        if link_contents == 0 {
            self.unlink_entity(game, num);
            return;
        }
        let contents = game.contents(num);
        loop {
            let mut mins2 = [self.mins[0], self.mins[1]];
            let mut maxs2 = [self.maxs[0], self.maxs[1]];
            let mut node = 1;
            loop {
                let s = &mut self.sectors[node];
                s.contents_entities |= link_contents | contents;
                let axis = s.axis as usize;
                let child = if s.dist < absmin[axis] {
                    mins2[axis] = s.dist;
                    s.children[0]
                } else if s.dist <= absmax[axis] {
                    let ent = &mut self.entities[num];
                    if ent.world_sector as usize == node && ent.link_contents & !link_contents == 0 {
                        ent.set_link(link_contents, absmin, absmax);
                        return;
                    }
                    break;
                } else {
                    maxs2[axis] = s.dist;
                    s.children[1]
                };
                if child == 0 {
                    break;
                }
                node = child as usize;
            }
            let ent = self.entities[num];
            if ent.world_sector == 0 {
                self.add_entity_to_node(node, num);
            } else if ent.world_sector as usize != node || ent.link_contents & !link_contents != 0 {
                self.unlink_entity(game, num);
                continue;
            }
            self.entities[num].set_link(link_contents, absmin, absmax);
            self.sort_node(game, models, node, mins2, maxs2);
            return;
        }
    }

    /// `CM_UnlinkEntity`: take entity `num` out of its sector, free the
    /// sectors left empty, and work the entity contents out again up to
    /// the root.
    pub fn unlink_entity(&mut self, game: &impl LinkedEntities, num: usize) {
        let mut node = self.entities[num].world_sector as usize;
        if node == 0 {
            return;
        }
        self.entities[num].world_sector = 0;
        let next = self.entities[num].next_entity_in_world_sector;
        let head = self.sectors[node].entities;
        if head as usize == num + 1 {
            self.sectors[node].entities = next;
        } else {
            let mut scan = head as usize - 1;
            loop {
                let scan_next = self.entities[scan].next_entity_in_world_sector;
                if scan_next == 0 {
                    break;
                }
                if scan_next as usize == num + 1 {
                    self.entities[scan].next_entity_in_world_sector = next;
                    break;
                }
                scan = scan_next as usize - 1;
            }
        }
        loop {
            let s = self.sectors[node];
            if s.entities != 0 || s.static_models != 0 || s.children != [0; 2] {
                break;
            }
            self.sectors[node].contents_entities = 0;
            let parent = s.parent as usize;
            if parent == 0 {
                break;
            }
            self.sectors[node].parent = self.free_head;
            self.free_head = node as u16;
            let p = &mut self.sectors[parent];
            if p.children[0] as usize == node {
                p.children[0] = 0;
            } else {
                p.children[1] = 0;
            }
            node = parent;
        }
        loop {
            let s = self.sectors[node];
            let mut contents = self.sectors[s.children[1] as usize].contents_entities
                | self.sectors[s.children[0] as usize].contents_entities;
            let mut e = s.entities;
            while e != 0 {
                let i = e as usize - 1;
                contents |= game.contents(i) | self.entities[i].link_contents;
                e = self.entities[i].next_entity_in_world_sector;
            }
            self.sectors[node].contents_entities = contents;
            if s.parent == 0 {
                break;
            }
            node = s.parent as usize;
        }
    }

    /// `CM_AreaEntities`: the entities with `contentmask` whose boxes
    /// touch `mins`..`maxs`, into `list`; the count. It stops at a full
    /// list.
    pub fn area_entities(
        &self,
        game: &impl LinkedEntities,
        mins: [f32; 3],
        maxs: [f32; 3],
        contentmask: u32,
        list: &mut [usize],
    ) -> usize {
        let mut count = 0;
        self.area_entities_r(game, 1, mins, maxs, contentmask, list, &mut count);
        count
    }

    #[allow(clippy::too_many_arguments)]
    fn area_entities_r(
        &self,
        game: &impl LinkedEntities,
        mut node: usize,
        mins: [f32; 3],
        maxs: [f32; 3],
        contentmask: u32,
        list: &mut [usize],
        count: &mut usize,
    ) {
        while contentmask & self.sectors[node].contents_entities != 0 {
            let s = &self.sectors[node];
            let mut e = s.entities;
            while e != 0 {
                let i = e as usize - 1;
                if contentmask & game.contents(i) != 0 {
                    let (amin, amax) = game.abs_bounds(i);
                    if (0..3).all(|k| amin[k] <= maxs[k] && mins[k] <= amax[k]) {
                        if *count == list.len() {
                            return;
                        }
                        list[*count] = i;
                        *count += 1;
                    }
                }
                e = self.entities[i].next_entity_in_world_sector;
            }
            let axis = s.axis as usize;
            node = if maxs[axis] <= s.dist {
                if s.dist <= mins[axis] {
                    return;
                }
                s.children[1] as usize
            } else if mins[axis] < s.dist {
                self.area_entities_r(game, s.children[0] as usize, mins, maxs, contentmask, list, count);
                s.children[1] as usize
            } else {
                s.children[0] as usize
            };
        }
    }

    /// `CM_AllocWorldSector`: a free sector splitting `mins`..`maxs`
    /// across its longer side, or 0 when none is free or the area is too
    /// small to split.
    fn alloc_sector(&mut self, mins: [f32; 2], maxs: [f32; 2]) -> usize {
        let node = self.free_head as usize;
        if node == 0 {
            return 0;
        }
        let size = [maxs[0] - mins[0], maxs[1] - mins[1]];
        let axis = usize::from(size[0] <= size[1]);
        if size[axis] <= SECTOR_SPLIT_MIN {
            return 0;
        }
        let s = &mut self.sectors[node];
        self.free_head = s.parent;
        s.axis = axis as u16;
        s.dist = (mins[axis] + maxs[axis]) * 0.5;
        node
    }

    /// Child `side` of `node`, made from a free sector over `mins`..`maxs`
    /// if it has none (0 if none can be).
    fn child_or_alloc(&mut self, node: usize, side: usize, mins: [f32; 2], maxs: [f32; 2]) -> usize {
        let child = self.sectors[node].children[side] as usize;
        if child != 0 {
            return child;
        }
        let child = self.alloc_sector(mins, maxs);
        if child != 0 {
            self.sectors[node].children[side] = child as u16;
            self.sectors[child].parent = node as u16;
        }
        child
    }

    /// `CM_AddEntityToNode`: file entity `num` in `node`'s list, which
    /// runs in entity order.
    fn add_entity_to_node(&mut self, node: usize, num: usize) {
        let mut prev = None;
        let mut cur = self.sectors[node].entities;
        while u32::from(cur).wrapping_sub(1) <= num as u32 {
            prev = Some(cur as usize - 1);
            cur = self.entities[cur as usize - 1].next_entity_in_world_sector;
        }
        self.entities[num].world_sector = node as u16;
        self.entities[num].next_entity_in_world_sector = cur;
        match prev {
            None => self.sectors[node].entities = num as u16 + 1,
            Some(p) => self.entities[p].next_entity_in_world_sector = num as u16 + 1,
        }
    }

    /// `CM_AddStaticModelToNode`: file static model `num` in `node`'s
    /// list, which runs in model order.
    fn add_static_model_to_node(&mut self, models: &mut [StaticModel<'_>], node: usize, num: usize) {
        let mut prev = None;
        let mut cur = self.sectors[node].static_models;
        while u32::from(cur).wrapping_sub(1) <= num as u32 {
            prev = Some(cur as usize - 1);
            cur = models[cur as usize - 1].next_model_in_world_sector;
        }
        models[num].next_model_in_world_sector = cur;
        match prev {
            None => self.sectors[node].static_models = num as u16 + 1,
            Some(p) => models[p].next_model_in_world_sector = num as u16 + 1,
        }
    }

    /// `CM_SortNode`: move each entity and static model in `node` that
    /// lies wholly on one side of its split down into that child, making
    /// the child from `mins`..`maxs` if there is none.
    fn sort_node(
        &mut self,
        game: &impl LinkedEntities,
        models: &mut [StaticModel<'_>],
        node: usize,
        mins: [f32; 2],
        maxs: [f32; 2],
    ) {
        let axis = self.sectors[node].axis as usize;
        let dist = self.sectors[node].dist;
        let mut prev: Option<usize> = None;
        let mut cur = self.sectors[node].entities;
        while cur != 0 {
            let e = cur as usize - 1;
            let ent = self.entities[e];
            cur = ent.next_entity_in_world_sector;
            let side = if !(ent.link_min[axis] <= dist) {
                Some(0)
            } else if ent.link_max[axis] < dist {
                Some(1)
            } else {
                None
            };
            let child = side.map_or(0, |side| self.child_or_alloc(node, side, mins, maxs));
            if child == 0 {
                prev = Some(e);
                continue;
            }
            self.add_entity_to_node(child, e);
            self.sectors[child].contents_entities |= game.contents(e) | ent.link_contents;
            match prev {
                None => self.sectors[node].entities = cur,
                Some(p) => self.entities[p].next_entity_in_world_sector = cur,
            }
        }
        let mut prev: Option<usize> = None;
        let mut cur = self.sectors[node].static_models;
        while cur != 0 {
            let m = cur as usize - 1;
            let model = models[m];
            cur = model.next_model_in_world_sector;
            let side = if dist < model.absmin[axis] {
                Some(0)
            } else if model.absmax[axis] < dist {
                Some(1)
            } else {
                None
            };
            let child = side.map_or(0, |side| self.child_or_alloc(node, side, mins, maxs));
            if child == 0 {
                prev = Some(m);
                continue;
            }
            self.add_static_model_to_node(models, child, m);
            self.sectors[child].contents_static_models |= model.contents;
            match prev {
                None => self.sectors[node].static_models = cur,
                Some(p) => models[p].next_model_in_world_sector = cur,
            }
        }
    }
}
