use super::*;

pub(super) fn compose_item(
    registry: &asset_game::WeaponLedger,
    catalog: &asset_model::LevelWeaponRoster,
    weapon_idx: u32,
) -> Option<ItemComposition> {
    let entry = registry.world_model_entry(weapon_idx, catalog)?;
    let mesh_slot_number = registry.world_mesh_link_for(weapon_idx)?.extent_position()?;
    let base_pose = entry.bone_rig.pose.as_ref()?;

    let mut key = entry.bone_rig.name.clone();
    let mut parts = vec![(base_pose, None)];
    let mut attach_list = Vec::new();
    for attached in crate::motion::other_body::world_attachments(registry, catalog, weapon_idx) {
        let Some(pose) = attached.entry.bone_rig.pose.as_ref() else {
            continue;
        };
        key.push('+');
        key.push_str(&attached.entry.bone_rig.name);
        attach_list.push(attached.index);
        let root_model = 0;
        parts.push((
            pose,
            Some(xmodel_runtime::MountedModel {
                host_model: root_model,
                mount_tag: attached.tag.to_owned(),
            }),
        ));
    }
    Some(ItemComposition {
        name: entry.bone_rig.name.clone(),
        mdl: assets::WorldWeaponSlot::from_sequence(mesh_slot_number),
        attachments: attach_list,
        key,
        dobj: xmodel_runtime::PosedSkeleton::build(&parts).ok()?,
    })
}

pub(super) fn build_item_compositions(
    weapons_in: Option<Res<assets::StagedWeapons>>,
    level_placed_weapons: Option<Res<assets::ReadiedWorldWeapons>>,
    mut prepared: ResMut<PreparedItemCompositions>,
) {
    let (Some(weapons_in), Some(world)) = (weapons_in, level_placed_weapons) else {
        return;
    };
    if prepared.matches(&weapons_in, &world) {
        return;
    }
    let t0 = std::time::Instant::now();
    let mut by_weapon = HashMap::new();
    if !world.0.is_empty() {
        for weapon in 1..=weapons_in.0.len() as u32 {
            if let Some(composed) = compose_item(&weapons_in.0, &world.0, weapon) {
                by_weapon.insert(weapon, Arc::new(composed));
            }
        }
    }
    diag::info!(
        World,
        "dropped items: {} weapon compositions prepared in {:.1}ms",
        by_weapon.len(),
        t0.elapsed().as_secs_f64() * 1000.0
    );
    *prepared = PreparedItemCompositions {
        built_for: Some(PreparedItemCompositions::built_for(&weapons_in, &world)),
        by_weapon,
    };
}
