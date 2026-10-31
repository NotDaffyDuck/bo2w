//! Claiming the viewmodel's slots in the scene each frame: one submission per
//! hand, positioned at the lighting origin of the camera.

use assets::ReadiedFpvMeshes;
use bevy::prelude::*;
use frame::EyeSubject;
use net::{HostViewClient, ShownWorldFrame};
use render_scene::{FIRST_PERSON_WEAPON_SLOT, FIRST_PERSON_WEAPON_EFFECT_BITS, FIRST_PERSON_OFFHAND_SLOT};

use super::gun_status::{ActiveGunMeshes, same_material_catalog};
use super::weapon::drawgun_admit;
use crate::motion::fpv_prepared::PreparedFpv;
use crate::motion::scene_feed::SkeletonSceneHandoff;
use crate::occupancy::other_view::shown_in_third_person;
use crate::occupancy::eye_kick::CameraRecoilState;
use crate::{gun_model_reach, held_view_light_point};

/// The submission for one hand's viewmodel. All hands share the lighting
/// origin and radius; only the entity number and model count differ.
fn hand_submission(spot: [f32; 3], bound: Option<f32>, ent_no: u32, model_count: u8) -> SkeletonSceneHandoff {
    SkeletonSceneHandoff {
        ent_ix: ent_no,
        origin: spot,
        illum_origin: spot,
        radius: bound,
        rot_quat: None,
        occupy_model_tally: model_count,
        models: vec![],
        hidden_mask: [0; 5],
        render_effect_flags: FIRST_PERSON_WEAPON_EFFECT_BITS,
        carries_anim_graph: true,
        keep_skinned_verts: false,
    }
}

/// Writes the viewmodel's scene submissions for this frame: the right hand
/// always, the left hand too while dual wielding. Nothing is written when the
/// view is not first person, the gun is hidden behind an overlay, or the
/// spawned viewmodel belongs to a retired material catalog.
pub fn populate_gun_scene(
    mut out: MessageWriter<SkeletonSceneHandoff>,
    view: Res<EyeSubject>,
    local: Res<HostViewClient>,
    presented: Res<ShownWorldFrame>,
    prepared: Res<PreparedFpv>,
    mat_list: Option<Res<render_scene::StitchMaterials>>,
    meshes: Option<Res<ReadiedFpvMeshes>>,
    spawned: Option<Res<ActiveGunMeshes>>,
    kick: Option<Res<CameraRecoilState>>,
) {
    // A guest weapon (bow, stick, gravity gun) draws itself.
    if crate::guest_weapons::hides_viewmodel() {
        return;
    }
    let Some(pstate) = presented.first_person_weapon_holder(local.0) else {
        return;
    };
    if shown_in_third_person(&presented, local.0, view.replaying_final_death()) {
        return;
    }
    let to_ads = kick.is_none_or(|session| session.snap_to_sights);
    let table = prepared.table().map(|table| &**table);
    if !matches!(drawgun_admit(pstate, table, to_ads), Some((true, _))) {
        return;
    }
    let Some(vm) = spawned.as_deref().and_then(|spawned| spawned.0.as_ref()) else {
        return;
    };
    if !same_material_catalog(&vm.material_catalog, mat_list.as_deref()) {
        return;
    }

    let at = held_view_light_point(pstate.origin, pstate.eye_height_now, pstate.look_angles[1], pstate.tilt_sideways);
    let radius = meshes.and_then(|meshes| gun_model_reach(&meshes.0, vm.fpv.hands_index, vm.fpv.gun_index));
    // Hands plus gun is two models; a bare-handed viewmodel is just one.
    let model_count = 1 + u8::from(!vm.fpv.weapon_mesh.is_empty());

    out.write(hand_submission(at, radius, FIRST_PERSON_WEAPON_SLOT, model_count));
    if pstate.previous_gun_hand == 1 {
        out.write(hand_submission(at, radius, FIRST_PERSON_OFFHAND_SLOT, model_count));
    }
}
