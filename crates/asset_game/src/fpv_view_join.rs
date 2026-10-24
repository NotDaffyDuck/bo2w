//! Joining the arms, the gun and what rides on it into one skeleton.

use bevy::math::{Quat, Vec3};
use asset_model::{FpvMeshCatalog, FpvMountPlan};
use dobj_t6::set_hide_part_bit;
use xmodel_runtime::{Attach, DObj, ModelPoseSrc};

use crate::fpv_view_types::{
    FpvAssembly, FpvAssemblyError, FpvAssemblyPart, FpvAssemblyTags, FpvPartRole,
};
use crate::{FpvMeshIndex, bone_has_hidden_ancestor};

/// A model waiting to be joined: how the view sees it and what it hangs from.
struct Piece {
    model: FpvMeshIndex,
    role: FpvPartRole,
    hangs_from: Option<Attach>,
    root_bind: Option<(Quat, Vec3)>,
}

impl Piece {
    fn on(model: FpvMeshIndex, role: FpvPartRole, parent_model: usize, tag: &str) -> Self {
        Self {
            model,
            role,
            hangs_from: Some(Attach {
                parent_model,
                tag: tag.to_owned(),
            }),
            root_bind: None,
        }
    }
}

/// The part-bit words of every bone a hidden tag covers, or `None` when
/// nothing is hidden or the model's surfaces carry no part bits.
fn hidden_part_words(
    catalog: &FpvMeshCatalog,
    model: FpvMeshIndex,
    hide_tags: &[String],
) -> Option<[u32; 5]> {
    let skel = &catalog.get_at(model.order())?.skel;
    if hide_tags.is_empty() || skel.surface_part_bits.len() != skel.surface_vertex_ranges.len() {
        return None;
    }
    let mut words = [0u32; 5];
    for bone in 0..skel.bone_names.len() {
        if bone_has_hidden_ancestor(&skel.bone_names, |b| skel.parent_of(b), bone, hide_tags) {
            set_hide_part_bit(&mut words, bone);
        }
    }
    Some(words)
}

/// The order the models are joined in: arms, gun, attachments, then the
/// rocket and the knife when they are wanted.
fn pieces_for(
    catalog: &FpvMeshCatalog,
    hands: FpvMeshIndex,
    mounts: &FpvMountPlan,
    rocket: bool,
    knife: Option<FpvMeshIndex>,
) -> Result<Vec<Piece>, FpvAssemblyError> {
    // A melee weapon's own model is rooted at `tag_knife`, and rides on the
    // arms' knife bone, the one its swing animates; any other gun rides on
    // `tag_weapon`.
    let gun_is_knife = catalog
        .get_at(mounts.gun.order())
        .and_then(|entry| entry.skel.bone_names.first())
        .is_some_and(|root| root.eq_ignore_ascii_case("tag_knife"));
    let gun_tag = if gun_is_knife { "tag_knife_attach" } else { "tag_weapon" };

    let mut pieces = vec![
        Piece {
            model: hands,
            role: FpvPartRole::Hands,
            hangs_from: None,
            root_bind: None,
        },
        Piece::on(mounts.gun, FpvPartRole::Gun, 0, gun_tag),
    ];
    for mount in &mounts.attachments {
        // Something hanging straight on the arms is a second gun, not a sight.
        let role = if mount.parent_model == 0 {
            FpvPartRole::Gun
        } else {
            FpvPartRole::Attachment
        };
        let mut piece = Piece::on(mount.model, role, mount.parent_model, &mount.tag);
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
        pieces.push(Piece::on(mount.model, FpvPartRole::Rocket, mount.parent_model, &mount.tag));
    }
    if let Some(model) = knife {
        pieces.push(Piece::on(model, FpvPartRole::Knife, 0, "tag_knife_attach"));
    }
    Ok(pieces)
}

fn effect_tags(dobj: &DObj) -> FpvAssemblyTags {
    let bone = |name: &str| dobj.find(name).and_then(|index| u16::try_from(index).ok());
    FpvAssemblyTags {
        flash: bone("tag_flash"),
        flash_silenced: bone("tag_flash_silenced"),
        brass: bone("tag_brass"),
        knife: bone("tag_knife_fx"),
        laser: bone(fx_t6::FX_LASER_TAG),
        tracker_screen: ["tag_screen_tl", "tag_screen_bl", "tag_screen_br"].map(bone),
        tracker_light: bone("tag_motion_tracker_fx"),
        flash_left: bone("tag_flash1"),
        brass_left: bone("tag_brass1"),
        target: bone("tag_target"),
        hud: bone("tag_hud"),
    }
}

impl FpvAssembly {
    pub fn build(
        catalog: &FpvMeshCatalog,
        hands: FpvMeshIndex,
        mounts: &FpvMountPlan,
        rocket: bool,
        knife: Option<FpvMeshIndex>,
        hide_tags: &[String],
    ) -> Result<Self, FpvAssemblyError> {
        let pieces = pieces_for(catalog, hands, mounts, rocket, knife)?;

        let mut poses: Vec<&ModelPoseSrc> = Vec::with_capacity(pieces.len());
        for piece in &pieces {
            let pose = catalog
                .get_at(piece.model.order())
                .and_then(|entry| entry.skel.pose.as_ref())
                .ok_or(FpvAssemblyError::Catalog("model pose"))?;
            poses.push(pose);
        }
        let specs: Vec<_> = poses
            .iter()
            .zip(&pieces)
            .map(|(pose, piece)| (*pose, piece.hangs_from.clone()))
            .collect();
        let root_binds: Vec<_> = pieces.iter().map(|piece| piece.root_bind).collect();
        let dobj = DObj::build_with_root_binds(&specs, &root_binds)
            .map_err(FpvAssemblyError::Skeleton)?;

        let view_bone = dobj.find("tag_view").ok_or(FpvAssemblyError::NoTagView)?;
        let parts = pieces
            .iter()
            .zip(&dobj.models)
            .map(|(piece, slot)| FpvAssemblyPart {
                model: piece.model,
                role: piece.role,
                bone_base: slot.base,
                hide: (piece.role == FpvPartRole::Gun)
                    .then(|| hidden_part_words(catalog, piece.model, hide_tags))
                    .flatten(),
            })
            .collect();

        Ok(Self {
            camera_bone: dobj.find("tag_camera"),
            paired_bones: poses[0].num_bones + poses[1].num_bones,
            tags: effect_tags(&dobj),
            arms_gun: mounts.attachments.iter().any(|mount| mount.parent_model == 0),
            view_bone,
            parts,
            dobj,
        })
    }
}
