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
    pub solid_ents: u32,
    /// One more than the first entity filed here (0 for none).
    pub entities: u16,
    /// One more than the first static model filed here (0 for none).
    pub prop_set: u16,
    pub dist: f32,
    pub axis: u16,
    pub parent: u16,
    /// The side past `dist`, then the side short of it.
    pub children: [u16; 2],
}

impl WorldSector {
    /// A sector with nothing filed, no split and no links.
    const fn blank() -> Self {
        Self {
            contents_static_models: 0,
            solid_ents: 0,
            entities: 0,
            prop_set: 0,
            dist: 0.0,
            axis: 0,
            parent: 0,
            children: [0; 2],
        }
    }

    /// Nothing filed here and nothing below.
    fn is_vacant(&self) -> bool {
        self.entities == 0 && self.prop_set == 0 && self.children == [0; 2]
    }
}

/// `svEntity_s`: the sector an entity is filed in (0 for none), the next
/// entity there (one more than its number) and the box and contents it was
/// linked with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvEntity {
    pub world_sector: u16,
    pub following_sector_entity: u16,
    pub link_contents: u32,
    pub link_min: [f32; 3],
    pub link_max: [f32; 3],
}

impl SvEntity {
    const fn blank() -> Self {
        Self {
            world_sector: 0,
            following_sector_entity: 0,
            link_contents: 0,
            link_min: [0.0; 3],
            link_max: [0.0; 3],
        }
    }

    fn set_link(&mut self, solid_bits: u32, mins: [f32; 3], hi_box: [f32; 3]) {
        (self.link_contents, self.link_min, self.link_max) = (solid_bits, mins, hi_box);
    }

    /// Filed in `node` with contents no wider than `contents`, so a relink
    /// there can keep its place.
    fn sits_in(&self, sector_ix: usize, contents: u32) -> bool {
        usize::from(self.world_sector) == sector_ix && self.link_contents & !contents == 0
    }
}

/// `cStaticModel_s`: the next static model in its sector (one more than
/// its number, `cStaticModelWritable`), its collision model, contents,
/// place, inverse scaled axis (`reverse_scaled_basis`) and box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StaticModel<'a> {
    pub next_model_in_world_sector: u16,
    pub xmodel: Option<&'a XModelColl<'a>>,
    pub contents: u32,
    pub origin: [f32; 3],
    pub reverse_scaled_basis: [[f32; 3]; 3],
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
    pub vacant_list_front: u16,
    pub sectors: [WorldSector; WORLD_SECTORS],
    /// `sv.svEntities`, which `clear` leaves as they are.
    pub entities: [SvEntity; LINKED_ENTITIES],
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

/// The lists here hold one more than an item's number, with 0 ending them.
fn link_index(link: u16) -> Option<usize> {
    usize::from(link).checked_sub(1)
}

/// The link that names item `num`.
fn link_to(num: usize) -> u16 {
    num as u16 + 1
}

/// Which child a box goes down: the side past the split (0), the side short
/// of it (1), or neither when it straddles.
fn side(past: bool, short: bool) -> Option<usize> {
    if past {
        Some(0)
    } else if short {
        Some(1)
    } else {
        None
    }
}

/// Where a list that runs in item order takes `num`: the item it goes after
/// (none for the head) and the link that will follow it.
fn ordered_slot(head: u16, num: usize, next_of: impl Fn(usize) -> u16) -> (Option<usize>, u16) {
    let mut after = None;
    let mut following = head;
    while let Some(item) = link_index(following).filter(|&item| item <= num) {
        after = Some(item);
        following = next_of(item);
    }
    (after, following)
}

/// How a walk down from the root ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The box crosses the sector's split.
    Straddles,
    /// The box fits one side, but the sector has no child there.
    NoChild,
}

/// The part of the world's ground plan a sector covers.
#[derive(Clone, Copy)]
struct Area {
    low: [f32; 2],
    high: [f32; 2],
}

/// The sector a walk from the root stopped in, and the area it covers.
struct Descent {
    stop: Stop,
    sector: usize,
    area: Area,
}

/// One `CM_AreaEntities` request in progress.
struct AreaQuery<'q, G> {
    game: &'q G,
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    found: &'q mut [usize],
    count: usize,
}

impl<G: LinkedEntities> AreaQuery<'_, G> {
    /// Offers entity `num`; false once the list is full.
    fn offer(&mut self, num: usize) -> bool {
        if self.mask & self.game.contents(num) == 0 {
            return true;
        }
        let (lo, hi) = self.game.abs_bounds(num);
        let touches = (0..3).all(|k| lo[k] <= self.maxs[k] && self.mins[k] <= hi[k]);
        if !touches {
            return true;
        }
        let Some(slot) = self.found.get_mut(self.count) else { return false };
        *slot = num;
        self.count += 1;
        true
    }
}

impl World {
    /// All zero, as BO2's globals start.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mins: [0.0; 3],
            maxs: [0.0; 3],
            vacant_list_front: 0,
            sectors: [WorldSector::blank(); WORLD_SECTORS],
            entities: [SvEntity::blank(); LINKED_ENTITIES],
        }
    }

    /// `CM_ClearWorld`: empty the tree, take the world model's bounds and
    /// split the root across their longer side; sectors 2 on are free.
    pub fn clear(&mut self, cm: &ClipMap<'_>) {
        let bounds = cm.solid_model(0).map(|third_person_mesh| (third_person_mesh.mins, third_person_mesh.maxs));
        (self.mins, self.maxs) = bounds.unwrap_or_default();
        self.sectors = [WorldSector::blank(); WORLD_SECTORS];
        // The free list runs 2, 3, ... and ends at the last sector.
        for slot in 2..WORLD_SECTORS - 1 {
            self.sectors[slot].parent = (slot + 1) as u16;
        }
        self.vacant_list_front = 2;
        let (lo, hi) = (self.mins, self.maxs);
        let axis = usize::from(hi[0] - lo[0] <= hi[1] - lo[1]);
        let root = &mut self.sectors[1];
        root.axis = axis as u16;
        root.dist = (hi[axis] + lo[axis]) * 0.5;
    }

    /// `CM_LinkWorld`: clear the tree and file every static model in it.
    pub fn link_world(&mut self, cm: &ClipMap<'_>, game: &impl LinkedEntities, models: &mut [StaticModel<'_>]) {
        self.clear(cm);
        self.link_static_models(game, models);
    }

    /// Walks down from the root, letting `mark` stamp every sector passed,
    /// and taking the child `pick(axis, dist)` names until the box
    /// straddles a split or the child is missing.
    fn descend(
        &mut self,
        mark: impl Fn(&mut WorldSector),
        pick: impl Fn(usize, f32) -> Option<usize>,
    ) -> Descent {
        let whole = Area { low: [self.mins[0], self.mins[1]], high: [self.maxs[0], self.maxs[1]] };
        let mut walk = Descent { stop: Stop::NoChild, sector: 1, area: whole };
        loop {
            let sector = &mut self.sectors[walk.sector];
            mark(sector);
            let axis = usize::from(sector.axis);
            let Some(toward) = pick(axis, sector.dist) else {
                walk.stop = Stop::Straddles;
                return walk;
            };
            // Going past the split raises the low edge; short of it, the high.
            let edge = if toward == 0 { &mut walk.area.low } else { &mut walk.area.high };
            edge[axis] = sector.dist;
            match sector.children[toward] {
                0 => return walk,
                child => walk.sector = usize::from(child),
            }
        }
    }

    /// `CM_LinkAllStaticModels`: file each static model that has contents
    /// in the deepest sector it fits on one side of.
    pub fn link_static_models(&mut self, game: &impl LinkedEntities, models: &mut [StaticModel<'_>]) {
        for num in 0..models.len() {
            let model = models[num];
            let stamp = model.contents;
            if stamp == 0 {
                continue;
            }
            let pick = |axis: usize, dist: f32| side(!(model.absmin[axis] <= dist), !(dist <= model.absmax[axis]));
            let reached = self.descend(|sector| sector.contents_static_models |= stamp, pick);
            self.add_static_model_to_node(models, reached.sector, num);
            self.order_sector(game, models, reached.sector, reached.area);
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
        let clip = cm.solid_model(model).map(|m| m.leaf.terrain_contents | m.leaf.solid_content_flags);
        let link_contents = clip.unwrap_or(0);
        if link_contents == 0 {
            return self.unlink_entity(game, num);
        }
        let stamp = link_contents | game.contents(num);
        let pick = |axis: usize, dist: f32| side(dist < absmin[axis], !(dist <= absmax[axis]));
        loop {
            let reached = self.descend(|sector| sector.solid_ents |= stamp, &pick);
            let filed = self.entities[num];
            let stays = filed.sits_in(reached.sector, link_contents);
            if reached.stop == Stop::Straddles && stays {
                // Already filed in the sector it straddles: only the box moves.
                self.entities[num].set_link(link_contents, absmin, absmax);
                return;
            }
            if filed.world_sector != 0 && !stays {
                self.unlink_entity(game, num);
                continue;
            }
            if filed.world_sector == 0 {
                self.link_ent_at(reached.sector, num);
            }
            self.entities[num].set_link(link_contents, absmin, absmax);
            self.order_sector(game, models, reached.sector, reached.area);
            return;
        }
    }

    /// The entities filed from link `first` on, in list order.
    fn entity_chain(&self, first: u16) -> impl Iterator<Item = usize> + '_ {
        core::iter::successors(link_index(first), |&num| link_index(self.entities[num].following_sector_entity))
    }

    /// The link that leads to whatever comes after `after` in `node`'s
    /// entity list (the list head when `after` is none).
    fn entity_link_mut(&mut self, sector_ix: usize, after: Option<usize>) -> &mut u16 {
        match after {
            Some(prev) => &mut self.entities[prev].following_sector_entity,
            None => &mut self.sectors[sector_ix].entities,
        }
    }

    /// Frees `node` if nothing is left in or under it, unhooking it from its
    /// parent; the parent, when it did. Its entity contents go to zero
    /// either way once it is vacant.
    fn prune(&mut self, node: usize) -> Option<usize> {
        let sector = self.sectors[node];
        if !sector.is_vacant() {
            return None;
        }
        self.sectors[node].solid_ents = 0;
        // The root is never freed.
        let parent = link_index(sector.parent).map(|_| usize::from(sector.parent))?;
        self.sectors[node].parent = core::mem::replace(&mut self.vacant_list_front, node as u16);
        let slots = &mut self.sectors[parent].children;
        let which = usize::from(usize::from(slots[0]) != node);
        slots[which] = 0;
        Some(parent)
    }

    /// `CM_UnlinkEntity`: take entity `num` out of its sector, free the
    /// sectors left empty, and work the entity contents out again up to
    /// the root.
    pub fn unlink_entity(&mut self, game: &impl LinkedEntities, num: usize) {
        let home = usize::from(self.entities[num].world_sector);
        if home == 0 {
            return;
        }
        let leaving = &mut self.entities[num];
        leaving.world_sector = 0;
        let following = leaving.following_sector_entity;
        let me = link_to(num);
        if self.sectors[home].entities == me {
            self.sectors[home].entities = following;
        } else {
            let head = self.sectors[home].entities;
            let before = self.entity_chain(head).find(|&prev| self.entities[prev].following_sector_entity == me);
            if let Some(prev) = before {
                self.entities[prev].following_sector_entity = following;
            }
        }

        let mut node = home;
        while let Some(up) = self.prune(node) {
            node = up;
        }
        let mut climb = Some(node);
        while let Some(at) = climb {
            let sector = self.sectors[at];
            let below = sector.children.iter().fold(0, |acc, &child| acc | self.sectors[usize::from(child)].solid_ents);
            let filed_here = self.entity_chain(sector.entities);
            let all = filed_here.fold(below, |acc, filed| acc | game.contents(filed) | self.entities[filed].link_contents);
            self.sectors[at].solid_ents = all;
            climb = (sector.parent != 0).then(|| usize::from(sector.parent));
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
        want_bits: u32,
        list: &mut [usize],
    ) -> usize {
        let mut query = AreaQuery { game, mins, maxs, mask: want_bits, found: list, count: 0 };
        self.gather(1, &mut query);
        query.count
    }

    /// Collects from `node` down; false once the list is full.
    fn gather<G: LinkedEntities>(&self, from_pt: usize, query: &mut AreaQuery<'_, G>) -> bool {
        let mut node = from_pt;
        while query.mask & self.sectors[node].solid_ents != 0 {
            let sector = &self.sectors[node];
            if !self.entity_chain(sector.entities).all(|num| query.offer(num)) {
                return false;
            }
            let (axis, dist) = (usize::from(sector.axis), sector.dist);
            let [past, short] = sector.children.map(usize::from);
            let wholly_short = query.maxs[axis] <= dist;
            node = match (wholly_short, dist <= query.mins[axis], query.mins[axis] < dist) {
                (true, true, _) => return true,
                (true, false, _) => short,
                (false, _, true) => {
                    if !self.gather(past, query) {
                        return false;
                    }
                    short
                }
                (false, _, false) => past,
            };
        }
        true
    }

    /// `CM_AllocWorldSector`: a free sector splitting `mins`..`maxs`
    /// across its longer side; none when none is free or the area is too
    /// small to split.
    fn claim_sector(&mut self, area: Area) -> Option<usize> {
        let Area { low, high } = area;
        let fresh = usize::from(self.vacant_list_front);
        let span = [high[0] - low[0], high[1] - low[1]];
        let axis = usize::from(span[0] <= span[1]);
        if fresh == 0 || span[axis] <= SECTOR_SPLIT_MIN {
            return None;
        }
        let sector = &mut self.sectors[fresh];
        self.vacant_list_front = sector.parent;
        sector.axis = axis as u16;
        sector.dist = (low[axis] + high[axis]) * 0.5;
        Some(fresh)
    }

    /// Child `side` of `node`, made from a free sector over `area` if it
    /// has none yet.
    fn child_or_alloc(&mut self, sector_ix: usize, side: usize, area: Area) -> Option<usize> {
        let existing = usize::from(self.sectors[sector_ix].children[side]);
        if existing > 0 {
            return Some(existing);
        }
        let made = self.claim_sector(area)?;
        self.sectors[sector_ix].children[side] = made as u16;
        self.sectors[made].parent = sector_ix as u16;
        Some(made)
    }

    /// `CM_AddEntityToNode`: file entity `num` in `node`'s list, which
    /// runs in entity order.
    fn link_ent_at(&mut self, sector_ix: usize, num: usize) {
        let entities = &self.entities;
        let (after, following) =
            ordered_slot(self.sectors[sector_ix].entities, num, |item| entities[item].following_sector_entity);
        let filed = &mut self.entities[num];
        filed.world_sector = sector_ix as u16;
        filed.following_sector_entity = following;
        *self.entity_link_mut(sector_ix, after) = link_to(num);
    }

    /// `CM_AddStaticModelToNode`: file static model `num` in `node`'s
    /// list, which runs in model order.
    fn add_static_model_to_node(&mut self, models: &mut [StaticModel<'_>], sector_ix: usize, num: usize) {
        let head = self.sectors[sector_ix].prop_set;
        let (after, following) = ordered_slot(head, num, |item| models[item].next_model_in_world_sector);
        models[num].next_model_in_world_sector = following;
        *model_link_mut(&mut self.sectors[sector_ix], models, after) = link_to(num);
    }

    /// `CM_SortNode`: move each entity and static model in `node` that
    /// lies wholly on one side of its split down into that child, making
    /// the child from `area` if there is none.
    fn order_sector(&mut self, game: &impl LinkedEntities, models: &mut [StaticModel<'_>], sector_ix: usize, area: Area) {
        let (axis, dist) = (usize::from(self.sectors[sector_ix].axis), self.sectors[sector_ix].dist);

        // The last entity left behind in `node`, whose link skips the movers.
        let mut kept = None;
        let mut cursor = self.sectors[sector_ix].entities;
        while let Some(num) = link_index(cursor) {
            let ent = self.entities[num];
            cursor = ent.following_sector_entity;
            let toward = side(!(ent.link_min[axis] <= dist), ent.link_max[axis] < dist);
            match toward.and_then(|s| self.child_or_alloc(sector_ix, s, area)) {
                None => kept = Some(num),
                Some(child) => {
                    self.link_ent_at(child, num);
                    self.sectors[child].solid_ents |= game.contents(num) | ent.link_contents;
                    *self.entity_link_mut(sector_ix, kept) = cursor;
                }
            }
        }

        let mut kept = None;
        let mut cursor = self.sectors[sector_ix].prop_set;
        while let Some(num) = link_index(cursor) {
            let model = models[num];
            cursor = model.next_model_in_world_sector;
            let toward = side(dist < model.absmin[axis], model.absmax[axis] < dist);
            match toward.and_then(|s| self.child_or_alloc(sector_ix, s, area)) {
                None => kept = Some(num),
                Some(child) => {
                    self.add_static_model_to_node(models, child, num);
                    self.sectors[child].contents_static_models |= model.contents;
                    *model_link_mut(&mut self.sectors[sector_ix], models, kept) = cursor;
                }
            }
        }
    }
}

/// The static-model counterpart of `World::entity_link_mut`.
fn model_link_mut<'m>(
    sector: &'m mut WorldSector,
    models: &'m mut [StaticModel<'_>],
    after: Option<usize>,
) -> &'m mut u16 {
    match after {
        Some(prev) => &mut models[prev].next_model_in_world_sector,
        None => &mut sector.prop_set,
    }
}
