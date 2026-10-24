use std::sync::Arc;

use bevy::prelude::*;
use dobj_t6::{RadiusModel, compute_radius};

pub fn occupy_lod_byte(lod: Option<u8>) -> i8 {
    lod.and_then(|lod| i8::try_from(lod).ok()).unwrap_or(-1)
}

pub fn kit_dobj_radius(radii: impl IntoIterator<Item = f32>) -> Option<f32> {
    radii
        .into_iter()
        .fold(None::<f32>, |acc, r| Some(acc.map_or(r, |a| a.max(r))))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KitSource<'a> {
    Body(&'a str),
    World(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PreparedKitSource {
    Body(String),
    World(usize),
}

pub struct PreparedRemoteKit {
    sources: Vec<PreparedKitSource>,
    pub radius: Option<f32>,
    pub hide_part_bits: [u32; 5],
    pub dobj: Option<std::sync::Arc<xmodel_runtime::DObj>>,
    pub bolt_bones: [Option<u16>; 4],
    pub head_bone: Option<usize>,
}

impl PreparedRemoteKit {
    pub fn models<'a>(
        &'a self,
        bodies: &'a assets::PreparedBodies,
        world: &'a assets::PreparedWorldWeapons,
    ) -> Option<Vec<KitModel<'a>>> {
        self.sources
            .iter()
            .map(|source| match source {
                PreparedKitSource::Body(key) => {
                    let entry = bodies.0.get(key)?;
                    Some(KitModel {
                        name: entry.skel.name.as_str(),
                        skel: &entry.skel,
                        hide_tags: Vec::new(),
                        source: KitSource::Body(key.as_str()),
                    })
                }
                PreparedKitSource::World(index) => {
                    let entry = world.0.get_at(*index)?;
                    Some(KitModel {
                        name: entry.skel.name.as_str(),
                        skel: &entry.skel,
                        hide_tags: Vec::new(),
                        source: KitSource::World(*index),
                    })
                }
            })
            .collect()
    }
}

#[derive(bevy::prelude::Resource, Default)]
pub struct PreparedRemoteKits {
    owner: Option<(
        std::sync::Arc<asset_model::BodyMeshCatalog>,
        std::sync::Arc<asset_game::WeaponRegistry>,
        u64,
    )>,
    kits: std::collections::HashMap<(bool, u32), PreparedRemoteKit>,
}

impl PreparedRemoteKits {
    pub fn owned_by(
        &self,
        bodies: &assets::PreparedBodies,
        weapons: &assets::PreparedWeapons,
        world_weapons: &assets::PreparedWorldWeapons,
    ) -> bool {
        self.owner.as_ref().is_some_and(|(b, w, catalog)| {
            std::sync::Arc::ptr_eq(b, &bodies.0)
                && std::sync::Arc::ptr_eq(w, &weapons.0)
                && *catalog == world_weapons.0.identity()
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
        let mut kits = std::collections::HashMap::new();
        for axis in [false, true] {
            for weapon in 0..=weapons.0.len() as u32 {
                let Some((models, radius)) = occupy_remote_kit_dobj(
                    bodies,
                    Some(weapons),
                    Some(world_weapons),
                    axis,
                    weapon,
                    true,
                ) else {
                    continue;
                };
                let sources = models
                    .iter()
                    .map(|model| match model.source {
                        KitSource::Body(key) => PreparedKitSource::Body(key.to_owned()),
                        KitSource::World(index) => PreparedKitSource::World(index),
                    })
                    .collect();
                let dobj =
                    select_remote_models(bodies, Some(weapons), Some(world_weapons), axis, weapon)
                        .ok()
                        .and_then(|set| xmodel_runtime::DObj::build(&set.dobj_models).ok())
                        .map(std::sync::Arc::new);
                kits.insert(
                    (axis, weapon),
                    PreparedRemoteKit {
                        sources,
                        radius,
                        hide_part_bits: kit_hide_part_bits(&models),
                        bolt_bones: [
                            "tag_flash",
                            "tag_brass",
                            "tag_knife_fx",
                            fx_t6::FX_LASER_TAG,
                        ]
                        .map(|tag| {
                            dobj.as_ref()
                                .and_then(|dobj| dobj.find(tag))
                                .and_then(|bone| u16::try_from(bone).ok())
                        }),
                        head_bone: dobj.as_ref().and_then(|dobj| dobj.find("j_head")),
                        dobj,
                    },
                );
            }
        }
        Self {
            owner: Some((
                std::sync::Arc::clone(&bodies.0),
                std::sync::Arc::clone(&weapons.0),
                world_weapons.0.identity(),
            )),
            kits,
        }
    }
}

pub fn kit_hide_part_bits(models: &[KitModel<'_>]) -> [u32; 5] {
    let mut bits = [0u32; 5];
    let mut base = 0usize;
    for model in models {
        render_scene::hide_part_bits_from_tags(&mut bits, model.skel, base, &model.hide_tags);
        base += model.skel.bones.len();
    }
    bits
}

pub struct KitModel<'a> {
    pub name: &'a str,
    pub skel: &'a asset_model::ModelSkel,
    pub hide_tags: Vec<String>,
    pub source: KitSource<'a>,
}

pub fn occupy_remote_kit_dobj<'a>(
    bodies: &'a assets::PreparedBodies,
    weapons: Option<&'a assets::PreparedWeapons>,
    world_weapons: Option<&'a assets::PreparedWorldWeapons>,
    axis: bool,
    weapon: u32,
    with_hide_tags: bool,
) -> Option<(Vec<KitModel<'a>>, Option<f32>)> {
    let kits = bodies.0.kits();
    let kit = kits.kit(axis)?;
    let body = bodies.0.get(&kit.body)?;
    if body.skel.positions.is_empty() || body.skel.bones.is_empty() {
        return None;
    }
    let _pose_src = body.skel.pose.as_ref()?;
    let mut skels: Vec<KitModel<'a>> = vec![KitModel {
        name: body.skel.name.as_str(),
        skel: &body.skel,
        hide_tags: Vec::new(),
        source: KitSource::Body(kit.body.as_str()),
    }];
    if let Some(name) = kit.head.as_deref() {
        if let Some(entry) = bodies.0.get(name) {
            if let (Some(_head_pose), Some(_tag)) = (
                entry.skel.pose.as_ref(),
                xmodel_runtime::tp_head_attach_tag(&body.skel.bone_names),
            ) {
                skels.push(KitModel {
                    name: entry.skel.name.as_str(),
                    skel: &entry.skel,
                    hide_tags: Vec::new(),
                    source: KitSource::Body(name),
                });
            }
        }
    }
    if weapon != 0 {
        if let (Some(registry), Some(catalog)) = (weapons, world_weapons) {
            let gun_index = registry
                .0
                .world_model_edge_of(weapon)
                .and_then(|edge| edge.bound_index());
            if let (Some(entry), Some(gun_index)) =
                (registry.0.world_model_entry(weapon, &catalog.0), gun_index)
            {
                if let (Some(_gun_pose), Some(_tag)) = (
                    entry.skel.pose.as_ref(),
                    xmodel_runtime::tp_weapon_attach_tag(&body.skel.bone_names),
                ) {
                    skels.push(KitModel {
                        name: entry.skel.name.as_str(),
                        skel: &entry.skel,
                        hide_tags: weapons
                            .filter(|_| with_hide_tags)
                            .map(|registry| asset_game::effective_hide_tags(&registry.0, weapon))
                            .unwrap_or_else(Vec::new),
                        source: KitSource::World(gun_index),
                    });
                    for attachment in world_attachments(&registry.0, &catalog.0, weapon) {
                        skels.push(KitModel {
                            name: attachment.entry.skel.name.as_str(),
                            skel: &attachment.entry.skel,
                            hide_tags: Vec::new(),
                            source: KitSource::World(attachment.index.order()),
                        });
                    }
                }
            }
        }
    }

    let radius = kit_dobj_radius(skels.iter().filter_map(|model| model.skel.radius));
    Some((skels, radius))
}

pub struct RemoteModelSet<'a> {
    pub body_name: String,
    pub head_name: String,
    pub body: &'a asset_model::BodyMeshEntry,
    pub head: Option<&'a asset_model::BodyMeshEntry>,
    pub gun: Option<&'a asset_model::WorldWeaponEntry>,

    pub world_gun_gap: Option<WorldGunGap>,
    pub dobj_models: Vec<(&'a xmodel_runtime::ModelPoseSrc, Option<xmodel_runtime::Attach>)>,
    pub gun_model_index: usize,
    pub attachments: Vec<(&'a asset_model::WorldWeaponEntry, usize)>,
}

pub(crate) struct WorldAttachment<'a> {
    pub(crate) entry: &'a asset_model::WorldWeaponEntry,
    pub(crate) index: assets::WorldWeaponIndex,
    pub(crate) tag: String,
}

pub(crate) fn world_attachments<'a>(
    registry: &asset_game::WeaponRegistry,
    catalog: &'a asset_model::WorldWeaponCatalog,
    weapon: u32,
) -> Vec<WorldAttachment<'a>> {
    if registry.world_catalog_identity() != catalog.identity() {
        return Vec::new();
    }
    registry
        .attachment_world_model_edges_of(weapon)
        .iter()
        .zip(registry.attachment_world_mounts_of(weapon))
        .filter_map(|(edge, tag)| {
            let index = edge.bound_index()?;
            let entry = catalog.get_at(index)?;
            entry.skel.pose.as_ref()?;
            let tag = tag.as_ref()?;
            Some(WorldAttachment {
                entry,
                index: assets::WorldWeaponIndex::from_order(index),
                tag: tag.clone(),
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldGunGap {
    pub weapon: u32,

    pub world_model: String,
}

pub fn select_remote_models<'a>(
    bodies: &'a assets::PreparedBodies,
    weapons: Option<&assets::PreparedWeapons>,
    world_weapons: Option<&'a assets::PreparedWorldWeapons>,
    axis: bool,
    weapon: u32,
) -> Result<RemoteModelSet<'a>, String> {
    let kits = bodies.0.kits();
    let kit = kits
        .kit(axis)
        .ok_or_else(|| "no third-person kit".to_string())?;
    let body = bodies
        .0
        .get(&kit.body)
        .ok_or_else(|| format!("body `{}` missing from catalog", kit.body))?;
    let pose_src = body
        .skel
        .pose
        .as_ref()
        .ok_or_else(|| format!("body `{}` has no ModelPoseSrc", kit.body))?;

    let mut dobj_models = vec![(pose_src, None)];
    let head = match kit.head.as_deref() {
        None => None,
        Some(name) => {
            let entry = bodies
                .0
                .get(name)
                .ok_or_else(|| format!("head `{name}` missing from catalog"))?;
            let head_pose = entry
                .skel
                .pose
                .as_ref()
                .ok_or_else(|| format!("head `{name}` has no ModelPoseSrc"))?;
            let tag = xmodel_runtime::tp_head_attach_tag(&body.skel.bone_names).ok_or_else(|| {
                format!(
                    "body `{}` has no {}; refusing a headless DObj for `{name}`",
                    kit.body,
                    xmodel_runtime::TP_HEAD_ATTACH_TAG
                )
            })?;
            dobj_models.push((
                head_pose,
                Some(xmodel_runtime::Attach {
                    parent_model: 0,
                    tag: tag.into(),
                }),
            ));
            Some(entry)
        }
    };
    let gun_model_index = dobj_models.len();

    let mut world_gun_gap = None;
    let mut attachments = Vec::new();
    let gun = match weapon {
        0 => None,
        index => {
            let registry = weapons
                .ok_or_else(|| format!("weapon {index} held but PreparedWeapons not installed"))?;
            let catalog = world_weapons.ok_or_else(|| {
                format!(
                    "weapon {index} worldModel requested but PreparedWorldWeapons not installed"
                )
            })?;
            let leftover = registry.0.world_model_of(index).unwrap_or("<none>");
            let mut refuse_gun = |world_model: &str| -> Option<&'a asset_model::WorldWeaponEntry> {
                world_gun_gap = Some(WorldGunGap {
                    weapon: index,
                    world_model: world_model.to_owned(),
                });
                None
            };
            match registry.0.world_model_entry(index, &catalog.0) {
                None => refuse_gun(leftover),
                Some(entry) => {
                    let name = entry.skel.name.as_str();
                    let gun_pose = entry
                        .skel
                        .pose
                        .as_ref()
                        .ok_or_else(|| format!("world gun `{name}` has no ModelPoseSrc"))?;
                    match xmodel_runtime::tp_weapon_attach_tag(&body.skel.bone_names) {
                        None => refuse_gun(name),
                        Some(tag) => {
                            dobj_models.push((
                                gun_pose,
                                Some(xmodel_runtime::Attach {
                                    parent_model: 0,
                                    tag: tag.into(),
                                }),
                            ));
                            for attachment in world_attachments(&registry.0, &catalog.0, index) {
                                let Some(pose) = attachment.entry.skel.pose.as_ref() else {
                                    continue;
                                };
                                attachments.push((attachment.entry, dobj_models.len()));
                                dobj_models.push((
                                    pose,
                                    Some(xmodel_runtime::Attach {
                                        parent_model: gun_model_index,
                                        tag: attachment.tag.into(),
                                    }),
                                ));
                            }
                            Some(entry)
                        }
                    }
                }
            }
        }
    };
    Ok(RemoteModelSet {
        body_name: kit.body.clone(),
        head_name: kit.head.clone().unwrap_or_default(),
        body,
        head,
        gun,
        world_gun_gap,
        dobj_models,
        gun_model_index,
        attachments,
    })
}

/// The DObj radius of a remote body (`DObjComputeBounds`): the body, the
/// head on no bone, the gun on the body's weapon tag. No body radius: none.
// TODO(remote-head-attach): BO2 attaches the head with an empty tag; whether
// that adds a bone offset is not in the facts (QUESTIONS.md), so it adds none.
pub fn dobj_radius(
    body: &asset_model::BodyMeshEntry,
    head: Option<&asset_model::BodyMeshEntry>,
    gun: Option<&asset_model::WorldWeaponEntry>,
) -> Option<f32> {
    let mut models = vec![RadiusModel::unattached(body.skel.radius?)];
    if let Some(radius) = head.and_then(|head| head.skel.radius) {
        models.push(RadiusModel::unattached(radius));
    }
    if let Some(radius) = gun.and_then(|gun| gun.skel.radius) {
        let tag = xmodel_runtime::tp_weapon_attach_tag(&body.skel.bone_names).unwrap_or("tag_weapon_right");
        models.push(RadiusModel { radius, attach: crate::attach_bone(&body.skel, tag) });
    }
    Some(compute_radius(&models, &crate::base_trans(&body.skel)))
}

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

pub struct RemoteBodySkinnedItem {
    pub geometry_revision: u64,
    pub client: u32,
    pub origin: [f32; 3],
    pub world_from_local: Mat4,
    pub geom: Arc<CpuBodyGeom>,
    pub radius: Option<f32>,
}

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

    pub fn take(&mut self) -> Vec<RemoteBodySkinnedItem> {
        std::mem::take(&mut self.items)
    }
}
