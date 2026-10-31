//! Joining the arms, the gun and what rides on it into one skeleton.

use bevy::math::{Quat, Vec3};
use asset_model::{FirstPersonMeshRoster, FpvMountPlan};
use dobj_t6::hide_part;
use xmodel_runtime::{MountedModel, PosedSkeleton, PoseDonorModel};

use crate::fpv_view_types::{
    FpvAssembly, FpvAssemblyError, FpvAssemblyPart, FpvAssemblyTags, FpvPartRole,
};
use crate::{FpvMeshSlot, joint_under_concealed_parent};

/// A model waiting to be joined: how the view sees it and what it hangs from.
struct Piece {
    mdl: FpvMeshSlot,
    role: FpvPartRole,
    hangs_from: Option<MountedModel>,
    root_bind: Option<(Quat, Vec3)>,
}

impl Piece {
    fn on(mdl: FpvMeshSlot, role: FpvPartRole, parent_model: usize, tag: &str) -> Self {
        Self {
            mdl,
            role,
            hangs_from: Some(MountedModel {
                host_model: parent_model,
                mount_tag: tag.to_owned(),
            }),
            root_bind: None,
        }
    }
}

/// The part-bit words of every bone a hidden tag covers, or `None` when
/// nothing is hidden or the model's surfaces carry no part bits.
fn hidden_part_words(
    catalog: &FirstPersonMeshRoster,
    mdl: FpvMeshSlot,
    concealed_bones: &[String],
) -> Option<[u32; 5]> {
    let bone_rig = &catalog.at_row(mdl.order())?.bone_rig;
    let bits_match = bone_rig.face_parts.len() == bone_rig.facet_vertex_ranges.len();
    let wanted = concealed_bones.first().is_some() && bits_match;
    let hidden = |bone: &usize| {
        joint_under_concealed_parent(&bone_rig.joint_labels, |b| bone_rig.hanging_from(b), *bone, concealed_bones)
    };
    wanted.then(|| {
        (0..bone_rig.joint_labels.len())
            .filter(hidden)
            .fold([0u32; 5], |mut words, bone| {
                hide_part(&mut words, bone);
                words
            })
    })
}

/// Join order for the meshes: arms first, the gun, any add-ons on it, then the
/// rocket and the knife when they are wanted.
fn pieces_for(
    catalog: &FirstPersonMeshRoster,
    hands: FpvMeshSlot,
    mounts: &FpvMountPlan,
    rocket: bool,
    knife: Option<FpvMeshSlot>,
) -> Result<Vec<Piece>, FpvAssemblyError> {
    // A melee weapon's own model is rooted at `tag_knife`, and rides on the
    // arms' knife bone, the one its swing animates; any other gun rides on
    // `tag_weapon`.
    let gun_is_knife = catalog
        .at_row(mounts.gun.order())
        .and_then(|entry| entry.bone_rig.joint_labels.first())
        .is_some_and(|root| root.eq_ignore_ascii_case("tag_knife"));
    let gun_tag = if gun_is_knife { "tag_knife_attach" } else { "tag_weapon" };

    let mut pieces = vec![
        Piece {
            mdl: hands,
            role: FpvPartRole::Hands,
            hangs_from: None,
            root_bind: None,
        },
        Piece::on(mounts.gun, FpvPartRole::Gun, 0, gun_tag),
    ];
    for mount in &mounts.attachments {
        // Something hanging straight on the arms is a second gun, not a sight.
        let role = if mount.host_mesh == 0 {
            FpvPartRole::Gun
        } else {
            FpvPartRole::AddOn
        };
        let mut piece = Piece::on(mount.model, role, mount.host_mesh, &mount.tag);
        piece.root_bind = mount
            .root_bind
            .map(|(rotation, offset)| (Quat::from_array(rotation).normalize(), Vec3::from_array(offset)));
        pieces.push(piece);
    }
    if rocket {
        let mount = mounts
            .rocket
            .as_ref()
            .ok_or(FpvAssemblyError::Catalog("rocket mount"))?;
        pieces.push(Piece::on(mount.model, FpvPartRole::Rocket, mount.host_mesh, &mount.tag));
    }
    if let Some(model) = knife {
        pieces.push(Piece::on(model, FpvPartRole::Knife, 0, "tag_knife_attach"));
    }
    Ok(pieces)
}

fn effect_tags(dobj: &PosedSkeleton) -> FpvAssemblyTags {
    let bone = |name: &str| dobj.find(name).and_then(|index| u16::try_from(index).ok());
    FpvAssemblyTags {
        muzzle_bone: bone("tag_flash"),
        muzzle_flash_suppressed: bone("tag_flash_silenced"),
        eject_bone: bone("tag_brass"),
        blade_bone: bone("tag_knife_fx"),
        beam_bone: bone(fx_t6::AIM_BEAM_JOINT),
        tracker_screen: ["tag_screen_tl", "tag_screen_bl", "tag_screen_br"].map(bone),
        tracker_light: bone("tag_motion_tracker_fx"),
        flash_left: bone("tag_flash1"),
        brass_left: bone("tag_brass1"),
        target: bone("tag_target"),
        hud: bone("tag_hud"),
        wrist: ["j_wrist_le", "j_wrist_ri"].map(bone),
        weapon: bone("tag_weapon"),
    }
}

impl FpvAssembly {
    pub fn build(
        catalog: &FirstPersonMeshRoster,
        hands: FpvMeshSlot,
        mounts: &FpvMountPlan,
        rocket: bool,
        knife: Option<FpvMeshSlot>,
        concealed_bones: &[String],
    ) -> Result<Self, FpvAssemblyError> {
        let pieces = pieces_for(catalog, hands, mounts, rocket, knife)?;

        let mut poses: Vec<&PoseDonorModel> = Vec::with_capacity(pieces.len());
        for piece in &pieces {
            let pose = catalog
                .at_row(piece.mdl.order())
                .and_then(|entry| entry.bone_rig.pose.as_ref())
                .ok_or(FpvAssemblyError::Catalog("model pose"))?;
            poses.push(pose);
        }
        let specs: Vec<_> = poses
            .iter()
            .zip(&pieces)
            .map(|(pose, piece)| (*pose, piece.hangs_from.clone()))
            .collect();
        let root_binds: Vec<_> = pieces.iter().map(|piece| piece.root_bind).collect();
        let dobj = PosedSkeleton::build_with_root_binds(&specs, &root_binds)
            .map_err(FpvAssemblyError::Skeleton)?;

        let view_bone = dobj.find("tag_view").ok_or(FpvAssemblyError::NoTagView)?;
        let parts = pieces
            .iter()
            .zip(&dobj.models)
            .map(|(mesh_part, slot)| FpvAssemblyPart {
                mdl: mesh_part.mdl,
                role: mesh_part.role,
                joint_start_offset: slot.first_joint,
                hide: (mesh_part.role == FpvPartRole::Gun)
                    .then(|| hidden_part_words(catalog, mesh_part.mdl, concealed_bones))
                    .flatten(),
            })
            .collect();

        Ok(Self {
            camera_bone: dobj.find("tag_camera"),
            paired_bones: poses[0].skeleton_joint_total + poses[1].skeleton_joint_total,
            tags: effect_tags(&dobj),
            arms_gun: mounts.attachments.iter().any(|mount| mount.host_mesh == 0),
            view_bone,
            pieces: parts,
            joint_tree: dobj,
        })
    }
}
