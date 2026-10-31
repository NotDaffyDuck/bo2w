use super::*;

pub(super) fn occupy_item_scene_ents(
    shown_frame: Option<Res<net::ShownWorldFrame>>,
    own_client_client: Option<Res<net::HostViewClient>>,
    weapons: Option<Res<assets::StagedWeapons>>,
    level_placed_weapons: Option<Res<assets::ReadiedWorldWeapons>>,
    compositions: Res<PreparedItemCompositions>,
    mut occupancy_in: ResMut<ItemRows>,
    mut skel_cache: ResMut<SceneBonePoseSet>,
    mut submissions: MessageWriter<SkeletonSceneHandoff>,
    clock: Option<Res<net::FrameClock>>,
    t6_bodies: Option<Res<sim::body_anim_script::T6PlayerAnimsRes>>,
) {
    let _slow = SlowGuard::new("occupy_item_scene_ents");
    occupancy_in.rows.clear();
    // On a map using the newer body path, that path draws the items instead;
    // this one's materials refuse that map's, so it would only submit scene
    // entities nobody sees.
    if t6_bodies.is_some_and(|t| t.0.is_some()) {
        return;
    }
    let Some(shown_frame) = shown_frame.as_deref() else {
        return;
    };
    let Some(snap) = shown_frame.snapshot() else {
        return;
    };
    let show_scavenger = viewer_sees_scavenger_items(shown_frame, own_client_client.as_deref());
    let now = match clock.as_deref() {
        Some(clock) if clock.started() => clock.time(),
        _ => sim::level_time_ms(snap.tick),
    };
    let debug_log = std::env::var_os("BO2MP_ITEMDRAWLOG").is_some() && first_frame_of_second(now);

    let prepared_ok = match (weapons.as_deref(), level_placed_weapons.as_deref()) {
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
    let cat = level_placed_weapons
        .as_ref()
        .map(|prepared_in| &prepared_in.0)
        .filter(|cat| !cat.is_empty());
    let items: Vec<&EntitySnapshot> = snap
        .meta
        .entities
        .iter()
        .filter(|entity| entity.e_type == et::ITEM)
        .collect::<_>();
    let debug_log = debug_log && !items.is_empty();
    if debug_log {
        diag::info!(
            World,
            "item draw: {} items, catalog {}, compositions {}",
            items.len(),
            cat.map_or(0, |c| c.len()),
            compositions.by_weapon.len()
        );
    }
    let Some(cat) = cat else {
        return;
    };

    for (slot, entity) in items.into_iter().enumerate() {
        let weapon = item_weapon_index(entity.weapon.0);
        if debug_log {
            diag::info!(
                World,
                "item draw: ent {} weapon {:?} flags {:#x} scavenger {} composed {} at {:?}",
                entity.slot_index,
                weapon,
                entity.lerp.e_flags,
                is_scavenger_item(snap, entity.slot_index),
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
        if !show_scavenger && is_scavenger_item(snap, entity.slot_index) {
            continue;
        }
        let Some(composition) = compositions.by_weapon.get(&weapon) else {
            continue;
        };
        let Some(entry) = cat.at_row(composition.mdl.order()) else {
            continue;
        };
        let mut models = vec![skel_cache.shared(&composition.name, &entry.bone_rig, 0)];
        models.extend(composition.attachments.iter().filter_map(|index| {
            let attached = cat.at_row(index.order())?;
            Some(skel_cache.shared(attached.bone_rig.name.as_str(), &attached.bone_rig, 0))
        }));
        let origin = item_origin(entity, now);
        let illum_origin = item_light_probe_point(origin);
        let angles = evaluate_trajectory(&entity.lerp.apos, now);
        submissions.write(SkeletonSceneHandoff {
            render_effect_flags: 0,
            carries_anim_graph: false,
            origin,
            illum_origin,
            radius: entry.bone_rig.radius,
            ent_ix: u32::try_from(entity.slot_index).unwrap_or(0),
            rot_quat: Some(orientation_quat_from_euler(angles)),
            occupy_model_tally: u8::try_from(models.len()).unwrap_or(u8::MAX),
            models,
            hidden_mask: [0; 5],
            keep_skinned_verts: true,
        });
        occupancy_in.rows.push(ItemRow {
            seat: slot,
            entnum: entity.slot_index,
            pos: origin,
            angs: angles,
            composition: Arc::clone(composition),
            illum_origin,
        });
    }
}
