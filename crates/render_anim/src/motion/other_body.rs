//! What a remote player is made of when seen from outside: the body model,
//! its head, the gun in hand and the gun's attachments. Kits are put together
//! once for each set of catalogs, then fetched every frame.

use std::collections::HashMap;
use std::sync::Arc;

use asset_model::MeshRig;
use bevy::prelude::*;
use xmodel_runtime::MountedModel;

/// A model's submitted LOD as a signed byte; -1 stands for "none chosen" and
/// also for a LOD too big to fit.
pub fn choose_lod_byte(lod: Option<u8>) -> i8 {
    match lod.map(i8::try_from) {
        Some(Ok(byte)) => byte,
        _ => -1,
    }
}

/// Where a kit model's skeleton is held: a body-catalog key or a position in
/// the world-weapon catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KitSource<'a> {
    Body(&'a str),
    World(usize),
}

/// A [`KitSource`] that owns its key, so a finished kit need not keep the
/// borrow it was built under.
#[derive(Clone, Debug, PartialEq, Eq)]
enum HeldSource {
    Body(String),
    World(usize),
}

impl From<KitSource<'_>> for HeldSource {
    fn from(src: KitSource<'_>) -> Self {
        match src {
            KitSource::World(index) => Self::World(index),
            KitSource::Body(key) => Self::Body(String::from(key)),
        }
    }
}

/// One model of a kit. `concealed_bones` are the bone tags whose surfaces the kit
/// hides (only the gun carries any).
pub struct KitPart<'a> {
    pub name: &'a str,
    pub bone_rig: &'a MeshRig,
    pub concealed_bones: Vec<String>,
    pub source: KitSource<'a>,
}

impl<'a> KitPart<'a> {
    /// A model that hides nothing, named after its skeleton.
    fn of(bone_rig: &'a MeshRig, source: KitSource<'a>) -> Self {
        let name = bone_rig.name.as_str();
        KitPart {
            concealed_bones: Vec::new(),
            name,
            bone_rig,
            source,
        }
    }
}

/// Adds up the part-hide bits of a kit's models. Each model's bones are
/// numbered on from the last bone of the model before it.
pub fn kit_hide_part_bits(models: &[KitPart<'_>]) -> [u32; 5] {
    let mut bits = [0; 5];
    let mut offset = 0;
    for KitPart { bone_rig, concealed_bones, .. } in models {
        render_scene::hidden_part_mask_for_tags(&mut bits, bone_rig, offset, concealed_bones);
        offset += bone_rig.bones.len();
    }
    bits
}

/// The models of a kit in DObj order, and how each one is bolted on.
struct AssembledKit<'a> {
    pieces: Vec<KitPart<'a>>,
    mounts: Vec<Option<MountedModel>>,
    bound: Option<f32>,
}

impl<'a> AssembledKit<'a> {
    fn blank() -> Self {
        AssembledKit {
            pieces: Vec::new(),
            mounts: Vec::new(),
            bound: None,
        }
    }

    fn add_part(&mut self, mdl: KitPart<'a>, mount: Option<MountedModel>) {
        self.pieces.push(mdl);
        self.mounts.push(mount);
    }
}

/// Gathers what the `axis` side wears while holding `weapon`: the body; its
/// head, if the body has a place for one; and the gun with its attachments,
/// if the body has a weapon tag. `None` without a usable body.
fn assemble_kit<'a>(
    bodies: &'a assets::ReadiedCharacterBodies,
    weapons: Option<&'a assets::StagedWeapons>,
    level_placed_weapons: Option<&'a assets::ReadiedWorldWeapons>,
    along: bool,
    weap: u32,
    with_hide_tags: bool,
) -> Option<AssembledKit<'a>> {
    let table = bodies.0.kits();
    let kit = table.kit(along)?;
    let body_skel = &bodies.0.by_label(&kit.torso)?.bone_rig;
    let unusable =
        body_skel.pose.is_none() || body_skel.bones.is_empty() || body_skel.positions.is_empty();
    if unusable {
        return None;
    }
    let mut out = AssembledKit::blank();
    out.add_part(KitPart::of(body_skel, KitSource::Body(kit.torso.as_str())), None);

    if let Some(head_name) = kit.skull.as_deref()
        && let Some(head) = bodies.0.by_label(head_name).filter(|entry| entry.bone_rig.pose.is_some())
        && let Some(tag_name) = xmodel_runtime::tp_head_attach_tag(&body_skel.joint_labels)
    {
        let mount = MountedModel {
            host_model: 0,
            mount_tag: tag_name.into(),
        };
        out.add_part(KitPart::of(&head.bone_rig, KitSource::Body(head_name)), Some(mount));
    }

    if let (true, Some(weapons), Some(catalog)) = (weap != 0, weapons, level_placed_weapons) {
        add_held_gun(&mut out, weap, with_hide_tags, body_skel, weapons, catalog);
    }

    out.bound = out.pieces.iter().flat_map(|part| part.bone_rig.radius).reduce(f32::max);
    Some(out)
}

/// Puts the gun `weapon` into a remote player's hands, then its attachments.
/// Does nothing if the gun has no model or no pose, or the body has no tag to
/// hold it by.
fn add_held_gun<'a>(
    kit: &mut AssembledKit<'a>,
    weapon_in: u32,
    with_hide_tags: bool,
    body_skel: &MeshRig,
    weapons: &'a assets::StagedWeapons,
    cat: &'a assets::ReadiedWorldWeapons,
) {
    let registry = &weapons.0;
    let edge = registry.world_mesh_link_for(weapon_in);
    let Some(world_index) = edge.and_then(|bound| bound.extent_position()) else {
        return;
    };
    let Some(gun) = registry.world_model_entry(weapon_in, &cat.0) else {
        return;
    };
    let Some(tag_name) = xmodel_runtime::tp_weapon_attach_tag(&body_skel.joint_labels) else {
        return;
    };
    if gun.bone_rig.pose.is_none() {
        return;
    }
    let concealed_bones = with_hide_tags
        .then(|| asset_game::effective_hide_tags(registry, weapon_in))
        .unwrap_or_default();
    let gun_slot = kit.pieces.len();
    let in_hand = MountedModel {
        host_model: 0,
        mount_tag: tag_name.into(),
    };
    let gun_model = KitPart {
        concealed_bones,
        ..KitPart::of(&gun.bone_rig, KitSource::World(world_index))
    };
    kit.add_part(gun_model, Some(in_hand));
    for attached in world_attachments(registry, &cat.0, weapon_in) {
        let on_gun = MountedModel {
            host_model: gun_slot,
            mount_tag: attached.tag.into(),
        };
        let source = KitSource::World(attached.index.order());
        kit.add_part(KitPart::of(&attached.entry.bone_rig, source), Some(on_gun));
    }
}

/// Assembles the kit for `axis` holding `weapon` right now and returns its
/// models and radius. This is the slow path, taken when no
/// [`PreparedRemoteKits`] is current.
pub fn build_body_kit_dobj<'a>(
    bodies: &'a assets::ReadiedCharacterBodies,
    weapons: Option<&'a assets::StagedWeapons>,
    level_placed_weapons: Option<&'a assets::ReadiedWorldWeapons>,
    along: bool,
    gun_ix: u32,
    with_hide_tags: bool,
) -> Option<(Vec<KitPart<'a>>, Option<f32>)> {
    assemble_kit(bodies, weapons, level_placed_weapons, along, gun_ix, with_hide_tags)
        .map(|kit| (kit.pieces, kit.bound))
}

/// One attachment of a weapon that has a world model: the catalog entry, and
/// the tag on the gun where it is mounted.
pub(crate) struct WorldAttachment<'a> {
    pub(crate) entry: &'a asset_model::LevelWeaponRecord,
    pub(crate) index: assets::WorldWeaponSlot,
    pub(crate) tag: String,
}

/// Lists the attachments of `weapon` that come with a posed world model and a
/// mount tag. Empty if `catalog` is not the catalog the registry was built for.
pub(crate) fn world_attachments<'a>(
    registry: &asset_game::WeaponLedger,
    catalog: &'a asset_model::LevelWeaponRoster,
    weapon_idx: u32,
) -> Vec<WorldAttachment<'a>> {
    if registry.world_catalog_identity() != catalog.identity() {
        return Vec::new();
    }
    let model_edges = registry.attachment_world_model_edges_of(weapon_idx);
    let tags = registry.attachment_world_mounts_of(weapon_idx);
    let mut found = Vec::new();
    for (edge, mounted_on) in model_edges.iter().zip(tags) {
        let Some(order) = edge.extent_position() else {
            continue;
        };
        let Some(entry) = catalog.at_row(order) else {
            continue;
        };
        let (true, Some(tag)) = (entry.bone_rig.pose.is_some(), mounted_on.clone()) else {
            continue;
        };
        let index = assets::WorldWeaponSlot::from_sequence(order);
        found.push(WorldAttachment { entry, index, tag });
    }
    found
}

// ---------------------------------------------------------------------------
// Prepared kits
// ---------------------------------------------------------------------------

/// A kit assembled ahead of time: where its models are held, the numbers
/// worked out from them, and the DObj they make.
pub struct PreparedRemoteKit {
    held: Vec<HeldSource>,
    pub rad: Option<f32>,
    pub mask_part_bits: [u32; 5],
    pub dobj: Option<Arc<xmodel_runtime::PosedSkeleton>>,
    /// Bones for the muzzle flash, brass, knife and laser bolts.
    pub bolt_bones: [Option<u16>; 4],
    pub head_bone: Option<usize>,
}

impl PreparedRemoteKit {
    fn assemble(
        along: bool,
        gun_no: u32,
        bodies: &assets::ReadiedCharacterBodies,
        weapons: &assets::StagedWeapons,
        level_placed_weapons: &assets::ReadiedWorldWeapons,
    ) -> Option<Self> {
        let kit = assemble_kit(bodies, Some(weapons), Some(level_placed_weapons), along, gun_no, true)?;
        let held = kit.pieces.iter().map(|part| part.source.into()).collect();
        let hide_part_bits = kit_hide_part_bits(&kit.pieces);
        let posed: Vec<_> = kit
            .pieces
            .iter()
            .zip(kit.mounts)
            .filter_map(|(part, mount)| Some((part.bone_rig.pose.as_ref()?, mount)))
            .collect::<_>();
        let dobj = xmodel_runtime::PosedSkeleton::build(&posed).ok().map(Arc::new);
        let bone_of = |tag: &str| {
            let found = dobj.as_ref()?.find(tag)?;
            u16::try_from(found).ok()
        };
        let bolt_tags = ["tag_flash", "tag_brass", "tag_knife_fx", fx_t6::AIM_BEAM_JOINT];
        let bolt_bones = bolt_tags.map(bone_of);
        let head_bone = dobj.as_ref().and_then(|built| built.find("j_head"));
        Some(PreparedRemoteKit {
            rad: kit.bound,
            held,
            mask_part_bits: hide_part_bits,
            bolt_bones,
            head_bone,
            dobj,
        })
    }

    /// Looks the kit's models up in the catalogs again. `None` if any of them
    /// has dropped out.
    pub fn models<'a>(
        &'a self,
        bodies: &'a assets::ReadiedCharacterBodies,
        world: &'a assets::ReadiedWorldWeapons,
    ) -> Option<Vec<KitPart<'a>>> {
        let mut kit = Vec::with_capacity(self.held.len());
        for source in &self.held {
            let (bone_rig, found_at): (&MeshRig, KitSource<'a>) = match source {
                HeldSource::World(index) => (&world.0.at_row(*index)?.bone_rig, KitSource::World(*index)),
                HeldSource::Body(key) => (&bodies.0.by_label(key)?.bone_rig, KitSource::Body(key.as_str())),
            };
            kit.push(KitPart::of(bone_rig, found_at));
        }
        Some(kit)
    }
}

/// Every kit (each axis, each held weapon), prepared for one set of catalogs.
#[derive(Resource, Default)]
pub struct PreparedRemoteKits {
    built_for: Option<(Arc<asset_model::CharacterMeshRoster>, Arc<asset_game::WeaponLedger>, u64)>,
    by_side_and_weapon: HashMap<(bool, u32), PreparedRemoteKit>,
}

impl PreparedRemoteKits {
    /// True when these kits were prepared from exactly these catalogs.
    pub fn owned_by(
        &self,
        bodies: &assets::ReadiedCharacterBodies,
        weapons: &assets::StagedWeapons,
        level_placed_weapons: &assets::ReadiedWorldWeapons,
    ) -> bool {
        let Some((body_catalog, registry, world_id)) = &self.built_for else {
            return false;
        };
        Arc::ptr_eq(body_catalog, &bodies.0)
            && Arc::ptr_eq(registry, &weapons.0)
            && *world_id == level_placed_weapons.0.identity()
    }

    pub fn kit_for(&self, along: bool, weapon: u32) -> Option<&PreparedRemoteKit> {
        self.by_side_and_weapon.get(&(along, weapon))
    }

    pub fn stage(
        bodies: &assets::ReadiedCharacterBodies,
        weapons: &assets::StagedWeapons,
        level_placed_weapons: &assets::ReadiedWorldWeapons,
    ) -> Self {
        let last_weapon = weapons.0.len() as u32;
        let mut by_side_and_weapon = HashMap::new();
        for axis in [false, true] {
            for weapon in 0..=last_weapon {
                let built = PreparedRemoteKit::assemble(axis, weapon, bodies, weapons, level_placed_weapons);
                if let Some(kit) = built {
                    by_side_and_weapon.insert((axis, weapon), kit);
                }
            }
        }
        let built_for = (
            Arc::clone(&bodies.0),
            Arc::clone(&weapons.0),
            level_placed_weapons.0.identity(),
        );
        PreparedRemoteKits {
            built_for: Some(built_for),
            by_side_and_weapon,
        }
    }
}

// ---------------------------------------------------------------------------
// Skinned submissions
// ---------------------------------------------------------------------------

/// A remote body's CPU-skinned geometry: packed vertices, triangle list and
/// the surfaces cut from it.
#[derive(Clone, Default)]
pub struct BodyMeshCpu {
    pub packed_rows: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    pub index_list: Vec<u32>,
    pub decoded_tally: usize,
    pub surfaces: Vec<SurfInfoCpu>,
}

#[derive(Clone)]
pub struct SurfInfoCpu {
    pub first_triangle_slot: u32,
    pub idx_count: u32,
    pub name: Option<String>,
}

/// One remote body waiting to be drawn.
pub struct BodySkinJob {
    pub mesh_rev: u64,
    pub client_no: u32,
    pub spot: [f32; 3],
    pub scene_from_local: Mat4,
    pub shape_data: Arc<BodyMeshCpu>,
    pub reach: Option<f32>,
}

/// The remote bodies waiting to be drawn this frame.
#[derive(Resource, Default)]
pub struct BodySkinJobs {
    waiting: Vec<BodySkinJob>,
}

impl BodySkinJobs {
    pub fn clear(&mut self) {
        self.waiting.truncate(0);
    }

    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &BodySkinJob> {
        self.waiting.iter()
    }

    /// Gives every queued body to the caller and leaves the queue empty.
    pub fn hand_over(&mut self) -> Vec<BodySkinJob> {
        std::mem::take(&mut self.waiting)
    }
}

mod tests {
    #![cfg(test)]
    use super::*;
    use crate::motion::test_skel::blank_skel;

    fn queued_item() -> BodySkinJob {
        BodySkinJob {
            mesh_rev: 1,
            client_no: 2,
            spot: [0.0; 3],
            scene_from_local: Mat4::IDENTITY,
            shape_data: Arc::new(BodyMeshCpu::default()),
            reach: None,
        }
    }

    #[test]
    fn lod_bytes_past_the_signed_range_read_as_none_chosen() {
        let cases = [(None, -1), (Some(0), 0), (Some(127), 127), (Some(128), -1), (Some(255), -1)];
        for (lod, byte) in cases {
            assert_eq!(choose_lod_byte(lod), byte, "lod {lod:?}");
        }
    }

    #[test]
    fn models_without_hide_tags_leave_every_part_bit_clear() {
        let bone_rig = blank_skel("body");
        let only_body = [KitPart::of(&bone_rig, KitSource::Body("body"))];
        assert_eq!(only_body[0].name, "body");
        assert_eq!(kit_hide_part_bits(&only_body), [0; 5]);
        assert_eq!(kit_hide_part_bits(&[]), [0; 5]);
    }

    #[test]
    fn a_held_source_keeps_the_key_or_index_it_came_from() {
        let from_body: HeldSource = KitSource::Body("mp_body").into();
        assert_eq!(from_body, HeldSource::Body(String::from("mp_body")));
        let from_world: HeldSource = KitSource::World(3).into();
        assert_eq!(from_world, HeldSource::World(3));
    }

    #[test]
    fn the_queue_gives_its_bodies_away_once() {
        let mut queue = BodySkinJobs::default();
        assert!(queue.is_empty());
        queue.waiting.push(queued_item());
        assert_eq!(queue.iter().count(), 1);
        assert_eq!(queue.hand_over().len(), 1);
        assert!(queue.is_empty());
        assert!(queue.hand_over().is_empty());
    }

    #[test]
    fn clearing_the_queue_drops_what_is_waiting() {
        let mut queue = BodySkinJobs::default();
        queue.waiting.extend([queued_item(), queued_item()]);
        queue.clear();
        assert!(queue.is_empty());
    }

    #[test]
    fn fresh_kits_were_built_for_nothing_and_hold_nothing() {
        let kits = PreparedRemoteKits::default();
        assert!(kits.built_for.is_none());
        assert!(kits.kit_for(true, 0).is_none());
        assert!(kits.kit_for(false, 9).is_none());
    }
}
