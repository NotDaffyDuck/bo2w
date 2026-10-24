use super::*;

pub(super) fn occupy_item_scene_ents(
    presented: Option<Res<net::PresentedSnapshot>>,
    local_client: Option<Res<net::LocalPresentClient>>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    world_weapons: Option<Res<assets::PreparedWorldWeapons>>,
    compositions: Res<PreparedItemCompositions>,
    mut occupancy: ResMut<ItemOccupancy>,
    mut scene_skels: ResMut<AnimDObjSceneSkels>,
    mut submissions: MessageWriter<AnimDObjSceneSubmission>,
    clock: Option<Res<net::FrameClock>>,
    t6_bodies: Option<Res<sim::t6_playeranim::T6PlayerAnimsRes>>,
) {
    let _slow = SlowGuard::new("occupy_item_scene_ents");
    occupancy.rows.clear();
    // On a map using the newer body path, that path draws the items instead;
    // this one's materials refuse that map's, so it would only submit scene
    // entities nobody sees.
    if t6_bodies.is_some_and(|t| t.0.is_some()) {
        return;
    }
    let Some(presented) = presented.as_deref() else {
        return;
    };
    let Some(snapshot) = presented.snapshot() else {
        return;
    };
    let show_scavenger = viewer_sees_scavenger_items(presented, local_client.as_deref());
    let now = match clock.as_deref() {
        Some(clock) if clock.started() => clock.time(),
        _ => sim::level_time_ms(snapshot.tick),
    };
    let debug_log = std::env::var_os("BO2MP_ITEMDRAWLOG").is_some() && first_frame_of_second(now);

    let prepared_ok = match (weapons.as_deref(), world_weapons.as_deref()) {
        (Some(weapons), Some(world)) => compositions.matches(weapons, world),
        _ => false,
    };
    if !prepared_ok {
        if debug_log {
            diag::info!(
                World,
                "item draw: compositions not owned by the prepared weapons"
            );
        }
        return;
    }
    let catalog = world_weapons
        .as_ref()
        .map(|prepared| &prepared.0)
        .filter(|catalog| !catalog.is_empty());
    let items: Vec<&EntityState> = snapshot
        .meta
        .entities
        .iter()
        .filter(|entity| entity.e_type == et::ITEM)
        .collect();
    let debug_log = debug_log && !items.is_empty();
    if debug_log {
        diag::info!(
            World,
            "item draw: {} items, catalog {}, compositions {}",
            items.len(),
            catalog.map_or(0, |c| c.len()),
            compositions.by_weapon.len()
        );
    }
    let Some(catalog) = catalog else {
        return;
    };

    for (slot, entity) in items.into_iter().enumerate() {
        let weapon = item_weapon_index(entity.weapon.0);
        if debug_log {
            diag::info!(
                World,
                "item draw: ent {} weapon {:?} flags {:#x} scavenger {} composed {} at {:?}",
                entity.number,
                weapon,
                entity.lerp.e_flags,
                is_scavenger_item(snapshot, entity.number),
                weapon.is_some_and(|w| compositions.by_weapon.contains_key(&w)),
                item_origin(entity, now)
            );
        }
        let Some(weapon) = weapon else {
            continue;
        };
        if entity.lerp.e_flags & FLAG_ITEM_HIDDEN != 0 {
            continue;
        }
        if !show_scavenger && is_scavenger_item(snapshot, entity.number) {
            continue;
        }
        let Some(composition) = compositions.by_weapon.get(&weapon) else {
            continue;
        };
        let Some(entry) = catalog.get_at(composition.model.order()) else {
            continue;
        };
        let mut models = vec![scene_skels.shared(&composition.name, &entry.skel, 0)];
        models.extend(composition.attachments.iter().filter_map(|index| {
            let attached = catalog.get_at(index.order())?;
            Some(scene_skels.shared(attached.skel.name.as_str(), &attached.skel, 0))
        }));
        let origin = item_origin(entity, now);
        let lighting_origin = item_lighting_origin(origin);
        let angles = evaluate_trajectory(&entity.lerp.apos, now);
        submissions.write(AnimDObjSceneSubmission {
            render_fx_flags: 0,
            has_tree: false,
            origin,
            lighting_origin,
            radius: entry.skel.radius,
            entnum: u32::try_from(entity.number).unwrap_or(0),
            quat: Some(scene_quat_from_angles(angles)),
            occupy_model_n: u8::try_from(models.len()).unwrap_or(u8::MAX),
            models,
            hide_part_bits: [0; 5],
            store_skin: true,
        });
        occupancy.rows.push(OccupiedItem {
            slot,
            entnum: entity.number,
            origin,
            angles,
            composition: Arc::clone(composition),
            lighting_origin,
        });
    }
}
