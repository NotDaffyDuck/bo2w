use super::*;

pub(super) fn compose_item(
    registry: &asset_game::WeaponRegistry,
    catalog: &asset_model::WorldWeaponCatalog,
    weapon: u32,
) -> Option<ItemComposition> {
    let entry = registry.world_model_entry(weapon, catalog)?;
    let model_index = registry.world_model_edge_of(weapon)?.bound_index()?;
    let base_pose = entry.skel.pose.as_ref()?;

    let mut key = entry.skel.name.clone();
    let mut parts = vec![(base_pose, None)];
    let mut attachments = Vec::new();
    for attached in crate::anim::remote_body::world_attachments(registry, catalog, weapon) {
        let Some(pose) = attached.entry.skel.pose.as_ref() else {
            continue;
        };
        key.push('+');
        key.push_str(&attached.entry.skel.name);
        attachments.push(attached.index);
        parts.push((
            pose,
            Some(xmodel_runtime::Attach {
                parent_model: 0,
                tag: attached.tag.to_owned(),
            }),
        ));
    }
    Some(ItemComposition {
        name: entry.skel.name.clone(),
        model: assets::WorldWeaponIndex::from_order(model_index),
        attachments,
        key,
        dobj: xmodel_runtime::DObj::build(&parts).ok()?,
    })
}

pub(super) fn build_item_compositions(
    weapons: Option<Res<assets::PreparedWeapons>>,
    world_weapons: Option<Res<assets::PreparedWorldWeapons>>,
    mut prepared: ResMut<PreparedItemCompositions>,
) {
    let (Some(weapons), Some(world)) = (weapons, world_weapons) else {
        return;
    };
    if prepared.matches(&weapons, &world) {
        return;
    }
    let started = std::time::Instant::now();
    let mut by_weapon = HashMap::new();
    if !world.0.is_empty() {
        for weapon in 1..=weapons.0.len() as u32 {
            if let Some(composed) = compose_item(&weapons.0, &world.0, weapon) {
                by_weapon.insert(weapon, Arc::new(composed));
            }
        }
    }
    diag::info!(
        World,
        "dropped items: {} weapon compositions prepared in {:.1}ms",
        by_weapon.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    *prepared = PreparedItemCompositions {
        built_for: Some(PreparedItemCompositions::built_for(&weapons, &world)),
        by_weapon,
    };
}
