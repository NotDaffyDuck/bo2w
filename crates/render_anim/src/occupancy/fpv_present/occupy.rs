//! Claiming the viewmodel's slots in the scene each frame: one submission per
//! hand, positioned at the lighting origin of the camera.

use assets::PreparedFpvMeshes;
use bevy::prelude::*;
use frame::ViewSubject;
use net::{LocalPresentClient, PresentedSnapshot};
use render_scene::{SCENE_VIEWMODEL_ENTNUM, SCENE_VIEWMODEL_FX_FLAGS, SCENE_VIEWMODEL_LEFT_ENTNUM};

use super::session::{SessionViewmodel, same_material_catalog};
use super::weapon::drawgun_admit;
use crate::anim::fpv_prepared::PreparedFpv;
use crate::anim::scene_submission::AnimDObjSceneSubmission;
use crate::occupancy::third_person::presented_is_third_person;
use crate::occupancy::view_kick::SessionViewKick;
use crate::{fpv_dobj_radius, viewmodel_lighting_origin};

/// The submission for one hand's viewmodel. All hands share the lighting
/// origin and radius; only the entity number and model count differ.
fn hand_submission(
    lighting_origin: [f32; 3],
    radius: Option<f32>,
    entnum: u32,
    models_n: u8,
) -> AnimDObjSceneSubmission {
    AnimDObjSceneSubmission {
        render_fx_flags: SCENE_VIEWMODEL_FX_FLAGS,
        has_tree: true,
        origin: lighting_origin,
        lighting_origin,
        radius,
        entnum,
        quat: None,
        occupy_model_n: models_n,
        models: Vec::new(),
        hide_part_bits: [0; 5],
        store_skin: false,
    }
}

/// Writes the viewmodel's scene submissions for this frame: the right hand
/// always, the left hand too while dual wielding. Nothing is written when the
/// view is not first person, the gun is hidden behind an overlay, or the
/// spawned viewmodel belongs to a retired material catalog.
pub fn occupy_fpv_scene(
    mut submissions: MessageWriter<AnimDObjSceneSubmission>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    view: Res<ViewSubject>,
    prepared: Res<PreparedFpv>,
    kick: Option<Res<SessionViewKick>>,
    session_vm: Option<Res<SessionViewmodel>>,
    tess: Option<Res<render_scene::TessMaterials>>,
    fpv_meshes: Option<Res<PreparedFpvMeshes>>,
) {
    // A guest weapon (bow, stick, gravity gun) draws itself.
    if crate::guest_weapons::hides_viewmodel() {
        return;
    }
    let Some(ps) = presented.viewweapon_player(local.0) else {
        return;
    };
    if presented_is_third_person(&presented, local.0, view.in_killcam()) {
        return;
    }
    let heading_into_ads = kick.as_ref().map_or(true, |k| k.b_position_to_ads);
    let drawn = drawgun_admit(ps, prepared.table().map(|t| &**t), heading_into_ads)
        .is_some_and(|(drawn, _)| drawn);
    if !drawn {
        return;
    }
    let Some(session) = session_vm.as_ref().and_then(|s| s.0.as_ref()) else {
        return;
    };
    if !same_material_catalog(&session.material_catalog, tess.as_deref()) {
        return;
    }

    let lighting = viewmodel_lighting_origin(
        ps.origin,
        ps.view_height_current,
        ps.viewangles[1],
        ps.leanf,
    );
    let radius = fpv_meshes
        .as_ref()
        .and_then(|meshes| fpv_dobj_radius(&meshes.0, session.fpv.hands_index, session.fpv.gun_index));
    // Hands plus gun is two models; a bare-handed viewmodel is just one.
    let models_n = if session.fpv.gun_xmodel.is_empty() { 1 } else { 2 };

    submissions.write(hand_submission(lighting, radius, SCENE_VIEWMODEL_ENTNUM, models_n));
    if ps.last_weapon_hand == 1 {
        submissions.write(hand_submission(
            lighting,
            radius,
            SCENE_VIEWMODEL_LEFT_ENTNUM,
            models_n,
        ));
    }
}
