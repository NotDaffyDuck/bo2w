//! The third-person kit of a remote player: which models make up the body
//! (body, head, held gun, gun attachments), prepared once per catalog set and
//! looked up per frame.

use std::collections::HashMap;
use std::sync::Arc;

use asset_model::ModelSkel;
use bevy::prelude::*;
use xmodel_runtime::Attach;

/// The LOD a model is submitted at, as a byte: negative when none was chosen
/// (or it does not fit).
pub fn occupy_lod_byte(lod: Option<u8>) -> i8 {
    lod.map_or(-1, |lod| i8::try_from(lod).unwrap_or(-1))
}

/// Where a kit model's skeleton is held: a body-catalog key or a position in
/// the world-weapon catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KitSource<'a> {
    Body(&'a str),
    World(usize),
}

/// [`KitSource`] with the key owned, so a prepared kit can outlive the borrow
/// it was assembled under.
#[derive(Clone, Debug, PartialEq, Eq)]
enum HeldSource {
    Body(String),
    World(usize),
}

impl From<KitSource<'_>> for HeldSource {
    fn from(source: KitSource<'_>) -> Self {
        match source {
            KitSource::Body(key) => Self::Body(key.to_owned()),
            KitSource::World(index) => Self::World(index),
        }
    }
}

/// One model of a kit. `hide_tags` are the bone tags whose surfaces the kit
/// hides (only the gun carries any).
pub struct KitModel<'a> {
    pub name: &'a str,
    pub skel: &'a ModelSkel,
    pub hide_tags: Vec<String>,
    pub source: KitSource<'a>,
}

/// The part-hide bits a kit's models add up to, each model's bones numbered
/// after the previous model's.
pub fn kit_hide_part_bits(models: &[KitModel<'_>]) -> [u32; 5] {
    let mut bits = [0u32; 5];
    let mut first_bone = 0;
    for model in models {
        render_scene::hide_part_bits_from_tags(&mut bits, model.skel, first_bone, &model.hide_tags);
        first_bone += model.skel.bones.len();
    }
    bits
}

/// A kit's models in DObj order, with the attachment each is bolted on by.
struct AssembledKit<'a> {
    models: Vec<KitModel<'a>>,
    attaches: Vec<Option<Attach>>,
    radius: Option<f32>,
}

impl<'a> AssembledKit<'a> {
    fn push(
        &mut self,
        skel: &'a ModelSkel,
        source: KitSource<'a>,
        hide_tags: Vec<String>,
        attach: Option<Attach>,
    ) {
        self.models.push(KitModel {
            name: skel.name.as_str(),
            skel,
            hide_tags,
            source,
        });
        self.attaches.push(attach);
    }
}

/// Collects the models of the kit `axis` wears while holding `weapon`: the
/// body, its head where the body has somewhere to put one, and the held
/// weapon with its attachments where the body has a weapon tag. `None` when
/// there is no usable body.
fn assemble_kit<'a>(
    bodies: &'a assets::PreparedBodies,
    weapons: Option<&'a assets::PreparedWeapons>,
    world_weapons: Option<&'a assets::PreparedWorldWeapons>,
    axis: bool,
    weapon: u32,
    with_hide_tags: bool,
) -> Option<AssembledKit<'a>> {
    let kits = bodies.0.kits();
    let kit = kits.kit(axis)?;
    let body = bodies.0.get(&kit.body)?;
    let body_skel = &body.skel;
    if body_skel.positions.is_empty() || body_skel.bones.is_empty() || body_skel.pose.is_none() {
        return None;
    }
    let mut out = AssembledKit {
        models: Vec::new(),
        attaches: Vec::new(),
        radius: None,
    };
    out.push(
        body_skel,
        KitSource::Body(kit.body.as_str()),
        Vec::new(),
        None,
    );

    if let Some(name) = kit.head.as_deref() {
        let head = bodies.0.get(name).filter(|entry| entry.skel.pose.is_some());
        if let (Some(head), Some(tag)) = (
            head,
            xmodel_runtime::tp_head_attach_tag(&body_skel.bone_names),
        ) {
            let attach = Attach {
                parent_model: 0,
                tag: tag.into(),
            };
            out.push(&head.skel, KitSource::Body(name), Vec::new(), Some(attach));
        }
    }

    if weapon != 0 {
        if let (Some(weapons), Some(catalog)) = (weapons, world_weapons) {
            add_held_gun(&mut out, body_skel, weapons, catalog, weapon, with_hide_tags);
        }
    }

    out.radius = out
        .models
        .iter()
        .filter_map(|model| model.skel.radius)
        .reduce(f32::max);
    Some(out)
}

/// The gun `weapon` shows in a remote player's hands, then its attachments.
/// Nothing is added when the gun has no model, no pose, or the body no tag
/// to hold it by.
fn add_held_gun<'a>(
    out: &mut AssembledKit<'a>,
    body_skel: &ModelSkel,
    weapons: &'a assets::PreparedWeapons,
    catalog: &'a assets::PreparedWorldWeapons,
    weapon: u32,
    with_hide_tags: bool,
) {
    let registry = &weapons.0;
    let Some(gun_index) = registry
        .world_model_edge_of(weapon)
        .and_then(|edge| edge.bound_index())
    else {
        return;
    };
    let Some(gun) = registry.world_model_entry(weapon, &catalog.0) else {
        return;
    };
    let Some(tag) = xmodel_runtime::tp_weapon_attach_tag(&body_skel.bone_names) else {
        return;
    };
    if gun.skel.pose.is_none() {
        return;
    }
    let hide_tags = if with_hide_tags {
        asset_game::effective_hide_tags(registry, weapon)
    } else {
        Vec::new()
    };
    let gun_model = out.models.len();
    let attach = Attach {
        parent_model: 0,
        tag: tag.into(),
    };
    out.push(&gun.skel, KitSource::World(gun_index), hide_tags, Some(attach));
    for attachment in world_attachments(registry, &catalog.0, weapon) {
        let attach = Attach {
            parent_model: gun_model,
            tag: attachment.tag.into(),
        };
        out.push(
            &attachment.entry.skel,
            KitSource::World(attachment.index.order()),
            Vec::new(),
            Some(attach),
        );
    }
}

/// The kit's models and radius for `axis` holding `weapon`, assembled on the
/// spot (the slow path when no [`PreparedRemoteKits`] is current).
pub fn occupy_remote_kit_dobj<'a>(
    bodies: &'a assets::PreparedBodies,
    weapons: Option<&'a assets::PreparedWeapons>,
    world_weapons: Option<&'a assets::PreparedWorldWeapons>,
    axis: bool,
    weapon: u32,
    with_hide_tags: bool,
) -> Option<(Vec<KitModel<'a>>, Option<f32>)> {
    let kit = assemble_kit(bodies, weapons, world_weapons, axis, weapon, with_hide_tags)?;
    Some((kit.models, kit.radius))
}

/// A world attachment of a weapon: its catalog entry and the tag of the gun
/// it mounts on.
pub(crate) struct WorldAttachment<'a> {
    pub(crate) entry: &'a asset_model::WorldWeaponEntry,
    pub(crate) index: assets::WorldWeaponIndex,
    pub(crate) tag: String,
}

/// The attachments of `weapon` that have a world model with a pose and a
/// mount tag. None when `catalog` is not the one the registry was built for.
pub(crate) fn world_attachments<'a>(
    registry: &asset_game::WeaponRegistry,
    catalog: &'a asset_model::WorldWeaponCatalog,
    weapon: u32,
) -> Vec<WorldAttachment<'a>> {
    if registry.world_catalog_identity() != catalog.identity() {
        return Vec::new();
    }
    let edges = registry.attachment_world_model_edges_of(weapon);
    let mounts = registry.attachment_world_mounts_of(weapon);
    edges
        .iter()
        .zip(mounts)
        .filter_map(|(edge, mount)| {
            let order = edge.bound_index()?;
            let entry = catalog.get_at(order)?;
            entry.skel.pose.as_ref()?;
            Some(WorldAttachment {
                entry,
                index: assets::WorldWeaponIndex::from_order(order),
                tag: mount.clone()?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Prepared kits
// ---------------------------------------------------------------------------

/// A kit assembled ahead of time: where its models are held, the numbers
/// derived from them, and the DObj they make.
pub struct PreparedRemoteKit {
    sources: Vec<HeldSource>,
    pub radius: Option<f32>,
    pub hide_part_bits: [u32; 5],
    pub dobj: Option<Arc<xmodel_runtime::DObj>>,
    /// Bones for the muzzle flash, brass, knife and laser bolts.
    pub bolt_bones: [Option<u16>; 4],
    pub head_bone: Option<usize>,
}

impl PreparedRemoteKit {
    fn assemble(
        bodies: &assets::PreparedBodies,
        weapons: &assets::PreparedWeapons,
        world_weapons: &assets::PreparedWorldWeapons,
        axis: bool,
        weapon: u32,
    ) -> Option<Self> {
        let kit = assemble_kit(bodies, Some(weapons), Some(world_weapons), axis, weapon, true)?;
        let sources = kit.models.iter().map(|m| m.source.into()).collect();
        let hide_part_bits = kit_hide_part_bits(&kit.models);
        let poses: Vec<_> = kit
            .models
            .iter()
            .zip(kit.attaches)
            .filter_map(|(model, attach)| Some((model.skel.pose.as_ref()?, attach)))
            .collect();
        let dobj = xmodel_runtime::DObj::build(&poses).ok().map(Arc::new);
        let bone_of = |tag: &str| {
            dobj.as_ref()
                .and_then(|dobj| dobj.find(tag))
                .and_then(|bone| u16::try_from(bone).ok())
        };
        Some(Self {
            sources,
            radius: kit.radius,
            hide_part_bits,
            bolt_bones: [
                "tag_flash",
                "tag_brass",
                "tag_knife_fx",
                fx_t6::FX_LASER_TAG,
            ]
            .map(bone_of),
            head_bone: dobj.as_ref().and_then(|dobj| dobj.find("j_head")),
            dobj,
        })
    }

    /// The kit's models, looked up again in the catalogs. `None` if one has
    /// gone from them.
    pub fn models<'a>(
        &'a self,
        bodies: &'a assets::PreparedBodies,
        world: &'a assets::PreparedWorldWeapons,
    ) -> Option<Vec<KitModel<'a>>> {
        self.sources
            .iter()
            .map(|held| {
                let (skel, source): (&ModelSkel, KitSource<'a>) = match held {
                    HeldSource::Body(key) => {
                        (&bodies.0.get(key)?.skel, KitSource::Body(key.as_str()))
                    }
                    HeldSource::World(index) => {
                        (&world.0.get_at(*index)?.skel, KitSource::World(*index))
                    }
                };
                Some(KitModel {
                    name: skel.name.as_str(),
                    skel,
                    hide_tags: Vec::new(),
                    source,
                })
            })
            .collect()
    }
}

/// Every kit (both axes, every held weapon), prepared for one set of
/// catalogs.
#[derive(Resource, Default)]
pub struct PreparedRemoteKits {
    owner: Option<(
        Arc<asset_model::BodyMeshCatalog>,
        Arc<asset_game::WeaponRegistry>,
        u64,
    )>,
    kits: HashMap<(bool, u32), PreparedRemoteKit>,
}

impl PreparedRemoteKits {
    /// Whether these kits were prepared from exactly these catalogs.
    pub fn owned_by(
        &self,
        bodies: &assets::PreparedBodies,
        weapons: &assets::PreparedWeapons,
        world_weapons: &assets::PreparedWorldWeapons,
    ) -> bool {
        self.owner.as_ref().is_some_and(|(b, w, world)| {
            Arc::ptr_eq(b, &bodies.0)
                && Arc::ptr_eq(w, &weapons.0)
                && *world == world_weapons.0.identity()
        })
    }

    pub fn get(&self, axis: bool, weapon: u32) -> Option<&PreparedRemoteKit> {
        self.kits.get(&(axis, weapon))
    }

    pub fn prepare(
        bodies: &assets::PreparedBodies,
        weapons: &assets::PreparedWeapons,
        world_weapons: &assets::PreparedWorldWeapons,
    ) -> Self {
        let weapon_count = weapons.0.len() as u32;
        let kits = [false, true]
            .into_iter()
            .flat_map(|axis| (0..=weapon_count).map(move |weapon| (axis, weapon)))
            .filter_map(|(axis, weapon)| {
                let kit =
                    PreparedRemoteKit::assemble(bodies, weapons, world_weapons, axis, weapon)?;
                Some(((axis, weapon), kit))
            })
            .collect();
        Self {
            owner: Some((
                Arc::clone(&bodies.0),
                Arc::clone(&weapons.0),
                world_weapons.0.identity(),
            )),
            kits,
        }
    }
}

// ---------------------------------------------------------------------------
// Skinned submissions
// ---------------------------------------------------------------------------

/// A remote body's CPU-skinned geometry: packed vertices, triangle list and
/// the surfaces cut from it.
#[derive(Clone, Default)]
pub struct CpuBodyGeom {
    pub packed: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    pub indices: Vec<u32>,
    pub decoded_n: usize,
    pub surfaces: Vec<CpuSurfMeta>,
}

#[derive(Clone)]
pub struct CpuSurfMeta {
    pub index_start: u32,
    pub index_count: u32,
    pub name: Option<String>,
}

/// One remote body waiting to be drawn.
pub struct RemoteBodySkinnedItem {
    pub geometry_revision: u64,
    pub client: u32,
    pub origin: [f32; 3],
    pub world_from_local: Mat4,
    pub geom: Arc<CpuBodyGeom>,
    pub radius: Option<f32>,
}

/// The remote bodies to draw this frame.
#[derive(Resource, Default)]
pub struct RemoteBodySkinnedQueue {
    items: Vec<RemoteBodySkinnedItem>,
}

impl RemoteBodySkinnedQueue {
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RemoteBodySkinnedItem> {
        self.items.iter()
    }

    /// Hands every queued body over, leaving the queue empty.
    pub fn take(&mut self) -> Vec<RemoteBodySkinnedItem> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::test_skel::blank_skel;

    #[test]
    fn a_lod_that_does_not_fit_a_signed_byte_is_none_chosen() {
        assert_eq!(occupy_lod_byte(None), -1);
        assert_eq!(occupy_lod_byte(Some(0)), 0);
        assert_eq!(occupy_lod_byte(Some(127)), 127);
        assert_eq!(occupy_lod_byte(Some(128)), -1);
    }

    #[test]
    fn a_kit_with_no_hide_tags_hides_nothing() {
        let skel = blank_skel("body");
        let models = [KitModel {
            name: "body",
            skel: &skel,
            hide_tags: Vec::new(),
            source: KitSource::Body("body"),
        }];
        assert_eq!(kit_hide_part_bits(&models), [0; 5]);
        assert_eq!(kit_hide_part_bits(&[]), [0; 5]);
    }

    #[test]
    fn a_held_source_remembers_where_a_model_came_from() {
        assert_eq!(
            HeldSource::from(KitSource::Body("mp_body")),
            HeldSource::Body("mp_body".to_owned())
        );
        assert_eq!(HeldSource::from(KitSource::World(3)), HeldSource::World(3));
    }

    #[test]
    fn the_queue_hands_its_bodies_over_once() {
        let mut queue = RemoteBodySkinnedQueue::default();
        assert!(queue.is_empty());
        queue.items.push(RemoteBodySkinnedItem {
            geometry_revision: 1,
            client: 2,
            origin: [0.0; 3],
            world_from_local: Mat4::IDENTITY,
            geom: Arc::new(CpuBodyGeom::default()),
            radius: None,
        });
        assert_eq!(queue.iter().count(), 1);
        assert_eq!(queue.take().len(), 1);
        assert!(queue.is_empty());
        queue.clear();
        assert!(queue.take().is_empty());
    }

    #[test]
    fn no_kits_are_owned_before_preparation() {
        let kits = PreparedRemoteKits::default();
        assert!(kits.get(false, 0).is_none());
    }
}
