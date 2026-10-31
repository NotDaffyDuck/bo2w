//! Where a first-person gun's attachments and rocket hang on it.

use asset_core::FpvMeshSlot;
use bevy::math::f32::Quat;

use crate::FirstPersonMeshRecord;
use crate::FirstPersonMeshRoster;

/// `FpvMount::parent_model` of a model that hangs on the arms.
const PARENT_ARMS: usize = 0;
/// ...and of one that hangs on the gun.
const PARENT_GUN: usize = 1;
/// Attachment slot `n` is parent `FIRST_ATTACHMENT_PARENT + n`.
const FIRST_ATTACHMENT_PARENT: usize = 2;

/// Bones a sight can hang on, best known first.
const SIGHT_TAGS: [&str; 8] = [
    "tag_scope",
    "tag_acog",
    "tag_red_dot",
    "tag_reflex",
    "tag_hybrid",
    "tag_thermal",
    "tag_thermal_scope",
    "tag_eotech",
];

/// One model hung on another.
#[derive(Clone, Debug)]
pub struct FpvMount {
    pub model: FpvMeshSlot,
    /// 0 the arms, 1 the gun, or 2 plus the slot of an earlier attachment.
    pub host_mesh: usize,
    pub tag: String,
    /// Where the model's root sits on the tag (rotation x y z w, then
    /// translation); `None` puts it on the tag itself.
    pub root_bind: Option<([f32; 4], [f32; 3])>,
}

/// A Black Ops II attachment view model's placement (`attachViewModelTag`,
/// `attachViewModelOffsets` and `attachViewModelRotations`). An empty tag
/// means the gun's root.
#[derive(Clone, Debug, PartialEq)]
pub struct FpvMountOffset {
    pub tag: String,
    pub offset: [f32; 3],
    /// Pitch, yaw, roll in degrees.
    pub rotation: [f32; 3],
    /// Hung on the arms' `tag` rather than the gun's: a dual-wield weapon's
    /// left gun on `tag_weapon1`.
    pub on_arms: bool,
}

/// Pitch, yaw, roll in degrees as a rotation x y z w: yaw about z, then
/// pitch about y, then roll about x.
pub fn angles_quat(angles: [f32; 3]) -> [f32; 4] {
    let turn = |degrees: f32| degrees.to_radians();
    let about_z = Quat::from_rotation_z(turn(angles[1]));
    let about_y = Quat::from_rotation_y(turn(angles[0]));
    let about_x = Quat::from_rotation_x(turn(angles[2]));
    (about_z * about_y * about_x).to_array()
}

#[derive(Clone, Debug)]
pub struct FpvMountPlan {
    pub gun: FpvMeshSlot,
    pub attachments: Vec<FpvMount>,
    pub rocket: Option<FpvMount>,
}

#[derive(Clone, Debug)]
pub struct FpvMountError {
    pub subject: String,
    pub detail: &'static str,
}

impl core::fmt::Display for FpvMountError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "FPV model `{}`: {}", self.subject, self.detail)
    }
}

/// What a missing or unposed model of one kind is reported as.
struct Role {
    absent: &'static str,
    unposed: &'static str,
}

const GUN: Role = Role {
    absent: "gun missing from FPV catalog",
    unposed: "gun has no pose source",
};
const ATTACHMENT: Role = Role {
    absent: "attachment missing from FPV catalog",
    unposed: "attachment has no pose source",
};
const ROCKET: Role = Role {
    absent: "rocket missing from FPV catalog",
    unposed: "rocket has no pose source",
};

/// The catalog's row for a model, which must have a pose to be mounted.
fn posed<'a>(
    catalog: &'a FirstPersonMeshRoster,
    index: FpvMeshSlot,
    role: &Role,
) -> Result<&'a FirstPersonMeshRecord, FpvMountError> {
    let row = index.order();
    let entry = catalog.at_row(row).ok_or_else(|| FpvMountError {
        subject: format!("#{row}"),
        detail: role.absent,
    })?;
    if entry.bone_rig.pose.is_none() {
        return Err(FpvMountError {
            subject: entry.bone_rig.name.clone(),
            detail: role.unposed,
        });
    }
    Ok(entry)
}

fn bone_named<'a>(bones: &'a [String], wanted: &str) -> Option<&'a str> {
    bones
        .iter()
        .find(|bone| bone.eq_ignore_ascii_case(wanted))
        .map(String::as_str)
}

/// The gun bone a sight hangs on when its own root is not one: a sight tag
/// both models have, else any sight tag the gun has.
fn sight_tag<'a>(gun_bones: &'a [String], sight_bones: &[String]) -> Option<&'a str> {
    let shared = SIGHT_TAGS.iter().find_map(|tag| {
        bone_named(sight_bones, tag)?;
        bone_named(gun_bones, tag)
    });
    shared.or_else(|| {
        gun_bones
            .iter()
            .find(|bone| SIGHT_TAGS.iter().any(|tag| bone.eq_ignore_ascii_case(tag)))
            .map(String::as_str)
    })
}

/// Where an attachment with no authored placement hangs: its root bone's
/// namesake on the gun, else on the latest earlier attachment holding that
/// bone, else a sight tag on the gun.
fn derived_joint<'a>(
    catalog: &'a FirstPersonMeshRoster,
    gun_bones: &'a [String],
    earlier: &'a [FpvMount],
    bones: &[String],
) -> Option<(usize, &'a str)> {
    let root = bones.first();
    if let Some(tag) = root.and_then(|root| bone_named(gun_bones, root)) {
        return Some((PARENT_GUN, tag));
    }
    let on_earlier = root.and_then(|root| {
        earlier.iter().enumerate().rev().find_map(|(slot, mount)| {
            let held = &catalog.at_row(mount.model.order())?.bone_rig.joint_labels;
            Some((FIRST_ATTACHMENT_PARENT + slot, bone_named(held, root)?))
        })
    });
    on_earlier.or_else(|| sight_tag(gun_bones, bones).map(|tag| (PARENT_GUN, tag)))
}

/// Works out where each attachment and the rocket hang. `offsets` runs
/// alongside `attachments`; a missing or `None` entry means the weapon
/// authors no placement for it.
pub fn plan_fpv_mounts(
    catalog: &FirstPersonMeshRoster,
    gun: FpvMeshSlot,
    attachments: &[FpvMeshSlot],
    offsets: &[Option<FpvMountOffset>],
    rocket: Option<FpvMeshSlot>,
) -> Result<FpvMountPlan, FpvMountError> {
    let gun_skel = &posed(catalog, gun, &GUN)?.bone_rig;
    let mut mounts: Vec<FpvMount> = Vec::with_capacity(attachments.len());
    for (slot, &model) in attachments.iter().enumerate() {
        let bone_rig = &posed(catalog, model, &ATTACHMENT)?.bone_rig;
        let mount = match offsets.get(slot).and_then(Option::as_ref) {
            Some(at) if at.on_arms => FpvMount {
                model,
                host_mesh: PARENT_ARMS,
                tag: at.tag.clone(),
                root_bind: None,
            },
            Some(at) => {
                let named = (!at.tag.is_empty())
                    .then(|| bone_named(&gun_skel.joint_labels, &at.tag))
                    .flatten();
                let tag = named
                    .or_else(|| gun_skel.joint_labels.first().map(String::as_str))
                    .ok_or_else(|| FpvMountError {
                        subject: bone_rig.name.clone(),
                        detail: "gun has no bones to mount on",
                    })?;
                FpvMount {
                    model,
                    host_mesh: PARENT_GUN,
                    tag: tag.to_owned(),
                    root_bind: Some((angles_quat(at.rotation), at.offset)),
                }
            }
            None => {
                let (parent_model, tag) =
                    derived_joint(catalog, &gun_skel.joint_labels, &mounts, &bone_rig.joint_labels)
                        .ok_or_else(|| FpvMountError {
                            subject: bone_rig.name.clone(),
                            detail: "no compatible mount on gun or earlier attachment",
                        })?;
                FpvMount {
                    model,
                    host_mesh: parent_model,
                    tag: tag.to_owned(),
                    root_bind: None,
                }
            }
        };
        mounts.push(mount);
    }
    let mut rocket_mounted = None;
    if let Some(model) = rocket {
        rocket_mounted = Some(rocket_mount(catalog, &gun_skel.joint_labels, model)?);
    }
    Ok(FpvMountPlan {
        gun,
        attachments: mounts,
        rocket: rocket_mounted,
    })
}

/// The rocket's mount on the gun's `tag_clip`.
fn rocket_mount(catalog: &FirstPersonMeshRoster, gun_bones: &[String], model: FpvMeshSlot) -> Result<FpvMount, FpvMountError> {
    let bone_rig = &posed(catalog, model, &ROCKET)?.bone_rig;
    let tag = bone_named(gun_bones, "tag_clip").ok_or_else(|| FpvMountError {
        subject: bone_rig.name.clone(),
        detail: "gun has no tag_clip rocket mount",
    })?;
    Ok(FpvMount { model, host_mesh: PARENT_GUN, tag: tag.to_owned(), root_bind: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FpvMeshBuild;
    use crate::fpv_mesh_catalog::test_support::bone_rig;
    use asset_core::AssetCategoryScope;

    fn catalog_of(models: &[(&str, &[&str], bool)]) -> FirstPersonMeshRoster {
        let mut build = FpvMeshBuild::default();
        for &(name, bones, posed) in models {
            build.put_into_namespace(AssetCategoryScope::T6, bone_rig(name, bones, posed), None);
        }
        build.publish()
    }

    fn at(order: usize) -> FpvMeshSlot {
        FpvMeshSlot::from_sequence(order)
    }

    #[test]
    fn a_sight_hangs_on_the_tag_its_root_shares_with_the_gun() {
        let catalog = catalog_of(&[
            ("gun", &["tag_origin", "tag_acog", "tag_clip"], true),
            ("sight", &["TAG_ACOG"], true),
        ]);
        let plan = plan_fpv_mounts(&catalog, at(0), &[at(1)], &[], None).unwrap();
        let mount = &plan.attachments[0];
        assert_eq!((mount.host_mesh, mount.tag.as_str()), (PARENT_GUN, "tag_acog"));
        assert!(mount.root_bind.is_none());
    }

    #[test]
    fn a_sight_with_its_own_root_takes_a_sight_tag_the_gun_has() {
        let catalog = catalog_of(&[
            ("gun", &["tag_origin", "tag_reflex"], true),
            ("sight", &["j_sight"], true),
        ]);
        let plan = plan_fpv_mounts(&catalog, at(0), &[at(1)], &[], None).unwrap();
        assert_eq!(plan.attachments[0].tag, "tag_reflex");
    }

    #[test]
    fn an_attachment_can_hang_on_an_earlier_one() {
        let catalog = catalog_of(&[
            ("gun", &["tag_origin"], true),
            ("grip", &["tag_flash_grip", "tag_light"], true),
            ("light", &["tag_light"], true),
        ]);
        let plan = plan_fpv_mounts(&catalog, at(0), &[at(1), at(2)], &[], None);
        let err = plan.unwrap_err();
        assert_eq!(err.subject, "grip");
        assert_eq!(err.detail, "no compatible mount on gun or earlier attachment");

        let catalog = catalog_of(&[
            ("gun", &["tag_origin", "tag_flash_grip"], true),
            ("grip", &["tag_flash_grip", "tag_light"], true),
            ("light", &["tag_light"], true),
        ]);
        let plan = plan_fpv_mounts(&catalog, at(0), &[at(1), at(2)], &[], None).unwrap();
        let light = &plan.attachments[1];
        assert_eq!((light.host_mesh, light.tag.as_str()), (FIRST_ATTACHMENT_PARENT, "tag_light"));
    }

    #[test]
    fn authored_offsets_mount_on_the_named_tag_or_the_root() {
        let catalog = catalog_of(&[
            ("gun", &["tag_origin", "tag_barrel"], true),
            ("muzzle", &["x"], true),
            ("other", &["y"], true),
        ]);
        let offsets = [
            Some(FpvMountOffset {
                tag: "TAG_BARREL".into(),
                offset: [1.0, 2.0, 3.0],
                rotation: [0.0, 0.0, 0.0],
                on_arms: false,
            }),
            Some(FpvMountOffset {
                tag: String::new(),
                offset: [0.0; 3],
                rotation: [0.0; 3],
                on_arms: false,
            }),
        ];
        let plan = plan_fpv_mounts(&catalog, at(0), &[at(1), at(2)], &offsets, None).unwrap();
        assert_eq!(plan.attachments[0].tag, "tag_barrel");
        assert_eq!(plan.attachments[0].root_bind.unwrap().1, [1.0, 2.0, 3.0]);
        assert_eq!(plan.attachments[1].tag, "tag_origin");
    }

    #[test]
    fn an_off_hand_gun_hangs_on_the_arms() {
        let catalog = catalog_of(&[("gun", &["a"], true), ("left", &["b"], true)]);
        let offsets = [Some(FpvMountOffset {
            tag: "tag_weapon1".into(),
            offset: [0.0; 3],
            rotation: [0.0; 3],
            on_arms: true,
        })];
        let plan = plan_fpv_mounts(&catalog, at(0), &[at(1)], &offsets, None).unwrap();
        assert_eq!(plan.attachments[0].host_mesh, PARENT_ARMS);
        assert_eq!(plan.attachments[0].tag, "tag_weapon1");
    }

    #[test]
    fn the_rocket_hangs_on_the_clip_tag() {
        let catalog = catalog_of(&[
            ("gun", &["tag_origin", "tag_clip"], true),
            ("rocket", &["r"], true),
        ]);
        let plan = plan_fpv_mounts(&catalog, at(0), &[], &[], Some(at(1))).unwrap();
        let rocket = plan.rocket.unwrap();
        assert_eq!((rocket.host_mesh, rocket.tag.as_str()), (PARENT_GUN, "tag_clip"));

        let bare = catalog_of(&[("gun", &["tag_origin"], true), ("rocket", &["r"], true)]);
        let err = plan_fpv_mounts(&bare, at(0), &[], &[], Some(at(1))).unwrap_err();
        assert_eq!(err.detail, "gun has no tag_clip rocket mount");
    }

    #[test]
    fn missing_and_unposed_models_are_named_in_the_error() {
        let catalog = catalog_of(&[("gun", &["a"], false)]);
        let err = plan_fpv_mounts(&catalog, at(0), &[], &[], None).unwrap_err();
        assert_eq!(err.to_string(), "FPV model `gun`: gun has no pose source");
        let err = plan_fpv_mounts(&catalog, at(5), &[], &[], None).unwrap_err();
        assert_eq!(err.to_string(), "FPV model `#5`: gun missing from FPV catalog");
    }

    #[test]
    fn angles_apply_yaw_then_pitch_then_roll() {
        let [x, y, z, w] = angles_quat([0.0, 90.0, 0.0]);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!((x.abs() + y.abs()) < 1e-6);
        assert!((z - half).abs() < 1e-6 && (w - half).abs() < 1e-6);
    }
}
