use asset_model::{FpvMeshCatalog, FpvMeshEntry, ModelSkel};
use asset_world::{MapXModelAssetKey, MapXModelSceneAsset, MapXModelSceneCatalog};
use assets::FpvMeshIndex;
use dobj_t6::{DOBJ_NO_ATTACH, RadiusModel, compute_radius};
use lighting_t6::lighting_query_box_half;

/// The bone the viewmodel's gun attaches to on the hands.
// TODO(viewmodel-attach-tag): BO2's tag is not in the facts (QUESTIONS.md);
// "tag_weapon" is the bone the viewmodel rig already parents the gun to.
pub const VIEWMODEL_GUN_TAG: &str = "tag_weapon";

/// The lighting query box of a DObj: `DObjGetBounds`' cube around BO2's
/// `DObjComputeBounds` radius of `models`, with model 0's base-pose bone
/// translations. No models: no box.
pub fn lighting_box_half(models: &[RadiusModel], model0_base_trans: &[[f32; 3]]) -> Option<[f32; 3]> {
    if models.is_empty() {
        return None;
    }
    Some(lighting_query_box_half(compute_radius(models, model0_base_trans)))
}

/// The lighting query box around a DObj radius (a lone model's DObj radius
/// is its own radius).
pub fn radius_lighting_box_half(radius: f32) -> [f32; 3] {
    lighting_query_box_half(radius)
}

/// The base-pose (model space) translation of each of `skel`'s bones.
pub fn base_trans(skel: &ModelSkel) -> Vec<[f32; 3]> {
    skel.bones.iter().map(|bone| bone.trans).collect()
}

/// The attach byte of a model put on `tag` of model 0 (`skel`): that bone's
/// index, or no attach when model 0 has no such bone.
pub fn attach_bone(skel: &ModelSkel, tag: &str) -> u8 {
    skel.bone_names
        .iter()
        .position(|name| name == tag)
        .and_then(|i| u8::try_from(i).ok())
        .unwrap_or(DOBJ_NO_ATTACH)
}

pub fn script_model_lighting_box_half(
    catalog: &MapXModelSceneCatalog,
    models: &[xmodel_runtime::DObjModelDescriptor],
) -> Option<[f32; 3]> {
    let mut skels = Vec::with_capacity(models.len());
    for model in models {
        let asset = catalog.get(&MapXModelAssetKey(model.model.clone()))?;
        let MapXModelSceneAsset::T5(skel) = asset else {
            return None;
        };
        skels.push((skel, model));
    }
    let (model0, _) = skels.first()?;
    let mut radius_models = Vec::with_capacity(skels.len());
    for (skel, model) in &skels {
        // Only a bone of model 0 adds its offset; a model on a later
        // model's bone counts its own radius.
        let attach = match (model.parent_model, model.attach_tag.as_deref()) {
            (Some(0), Some(tag)) => attach_bone(model0, tag),
            _ => DOBJ_NO_ATTACH,
        };
        radius_models.push(RadiusModel { radius: skel.radius?, attach });
    }
    lighting_box_half(&radius_models, &base_trans(model0))
}

/// The viewmodel DObj's models: the hands, and the gun on the hands'
/// [`VIEWMODEL_GUN_TAG`].
fn viewmodel_radius_models(hands: &FpvMeshEntry, gun_radius: f32) -> Option<[RadiusModel; 2]> {
    Some([
        RadiusModel::unattached(hands.skel.radius?),
        RadiusModel { radius: gun_radius, attach: attach_bone(&hands.skel, VIEWMODEL_GUN_TAG) },
    ])
}

pub fn fpv_dobj_lighting_box_half(
    catalog: &FpvMeshCatalog,
    hands_index: FpvMeshIndex,
    gun_index: FpvMeshIndex,
) -> Option<[f32; 3]> {
    let hands = catalog.get_at(hands_index.order())?;
    let gun = catalog.get_at(gun_index.order())?;
    let models = viewmodel_radius_models(hands, gun.skel.radius?)?;
    lighting_box_half(&models, &base_trans(&hands.skel))
}

/// The viewmodel DObj radius; with only one of hands and gun known, that
/// one's radius.
pub fn fpv_dobj_radius(
    catalog: &FpvMeshCatalog,
    hands_index: FpvMeshIndex,
    gun_index: FpvMeshIndex,
) -> Option<f32> {
    let hands = catalog.get_at(hands_index.order());
    let gun = catalog.get_at(gun_index.order()).and_then(|g| g.skel.radius);
    match (hands, gun) {
        (Some(hands), Some(gun)) => match viewmodel_radius_models(hands, gun) {
            Some(models) => Some(compute_radius(&models, &base_trans(&hands.skel))),
            None => Some(gun),
        },
        (Some(hands), None) => hands.skel.radius,
        (None, gun) => gun,
    }
}

pub fn viewmodel_lighting_origin(
    origin: [f32; 3],
    view_height_current: f32,
    view_yaw: f32,
    leanf: f32,
) -> [f32; 3] {
    lighting_t6::viewmodel_lighting_origin(origin, view_height_current, view_yaw, leanf)
}
