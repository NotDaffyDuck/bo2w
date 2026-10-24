//! Keeps an effect's class-0 part list in draw order: authored order first,
//! then back to front from the camera.

use fx_t6::runtime::{elem_random_seed, existing_elem_sorts_before_new, sort_dist_to_cam_sq};

use crate::elem::{FX_ELEM_HANDLE_NONE, elem_slot_for_handle};
use crate::orient::elem_world_origin;
use crate::system::FxSystemHost;

/// Sorts the parts added since the last pass into the already-sorted tail of
/// the effect's class-0 list.
///
/// New parts sit at the head of the list, in front of
/// `first_sorted_elem_handle`. Each is unlinked and inserted at its place in
/// the sorted run, one at a time.
pub fn sort_new_elems_in_effect(
    host: &mut FxSystemHost,
    effect_slot: usize,
    camera_origin: [f32; 3],
) {
    let Some(effect) = host.effect_at(effect_slot) else {
        return;
    };
    let sorted_head = effect.first_sorted_elem_handle;
    let mut pending = effect.first_elem_handle[0];
    if pending == sorted_head {
        return;
    }

    // Cached distances are good only for this pass: nothing they depend on
    // moves while the links change. A new epoch invalidates the old ones.
    host.sort_epoch = host.sort_epoch.wrapping_add(1);
    if host.sort_epoch == 0 {
        host.sort_distances.fill((0, 0.0));
        host.sort_epoch = 1;
    }

    // The list now holds only the sorted run; new parts get inserted back in.
    if let Some(effect) = host.effect_at_mut(effect_slot) {
        effect.first_elem_handle[0] = sorted_head;
    }
    if let Some(slot) = elem_slot_for_handle(sorted_head)
        && let Some(elem) = host.elems.get_mut(slot)
    {
        elem.prev_elem_handle = FX_ELEM_HANDLE_NONE;
    }

    while pending != sorted_head {
        let Some(slot) = elem_slot_for_handle(pending) else {
            break;
        };
        let Some(elem) = host.elems.get_mut(slot) else {
            break;
        };
        let next = elem.next_elem_handle;
        elem.next_elem_handle = FX_ELEM_HANDLE_NONE;
        elem.prev_elem_handle = FX_ELEM_HANDLE_NONE;
        insert_sorted(host, effect_slot, pending, camera_origin);
        pending = next;
    }

    if let Some(effect) = host.effect_at_mut(effect_slot) {
        effect.first_sorted_elem_handle = effect.first_elem_handle[0];
    }
}

/// Links `new_handle` into the effect's list before the first part that does
/// not sort ahead of it.
fn insert_sorted(
    host: &mut FxSystemHost,
    effect_slot: usize,
    new_handle: u16,
    camera_origin: [f32; 3],
) {
    let Some(new_slot) = elem_slot_for_handle(new_handle) else {
        return;
    };
    let new_order = match host.elems.get(new_slot) {
        Some(e) if e.occupied => e.sort_order,
        _ => return,
    };
    let Some(new_dist) = distance_to_camera(host, effect_slot, new_slot, camera_origin) else {
        return;
    };
    let Some(effect) = host.effect_at(effect_slot) else {
        return;
    };

    let mut before = FX_ELEM_HANDLE_NONE;
    let mut at = effect.first_elem_handle[0];
    while at != FX_ELEM_HANDLE_NONE {
        let Some(at_slot) = elem_slot_for_handle(at) else {
            break;
        };
        let Some(existing) = host.elems.get(at_slot).filter(|e| e.occupied) else {
            break;
        };
        let (kind, visuals, order, next) = (
            existing.elem_type,
            existing.visual_count,
            existing.sort_order,
            existing.next_elem_handle,
        );
        // Distance only decides between parts of the same authored order, so
        // it is measured only for those.
        let dist = if kind <= 3 && visuals != 0 && order == new_order {
            match distance_to_camera(host, effect_slot, at_slot, camera_origin) {
                Some(d) => d,
                None => break,
            }
        } else {
            0.0
        };
        if !existing_elem_sorts_before_new(kind, visuals, order, dist, new_order, new_dist) {
            break;
        }
        before = at;
        at = next;
    }

    if let Some(e) = host.elems.get_mut(new_slot) {
        e.prev_elem_handle = before;
        e.next_elem_handle = at;
    }
    match elem_slot_for_handle(before) {
        Some(slot) => {
            if let Some(e) = host.elems.get_mut(slot) {
                e.next_elem_handle = new_handle;
            }
        }
        None => {
            if let Some(effect) = host.effect_at_mut(effect_slot) {
                effect.first_elem_handle[0] = new_handle;
            }
        }
    }
    if let Some(slot) = elem_slot_for_handle(at)
        && let Some(e) = host.elems.get_mut(slot)
    {
        e.prev_elem_handle = new_handle;
    }
}

/// Squared distance from the camera to a part's world position, cached per
/// pass.
fn distance_to_camera(
    host: &mut FxSystemHost,
    effect_slot: usize,
    elem_slot: usize,
    camera_origin: [f32; 3],
) -> Option<f32> {
    let (epoch, cached) = host.sort_distances[elem_slot];
    if epoch == host.sort_epoch {
        return Some(cached);
    }
    let elem = host.elems.get(elem_slot)?;
    let effect = host.effect_at(effect_slot)?;
    let seed = elem_random_seed(effect.random_seed, elem.sequence, elem.msec_begin);
    let world = elem_world_origin(
        elem.origin,
        elem.flags,
        &effect.frame_now(),
        &effect.frame_when_played(),
        Some(elem.orient_spawn_params(seed)),
    );
    let dist = sort_dist_to_cam_sq(camera_origin, world);
    host.sort_distances[elem_slot] = (host.sort_epoch, dist);
    Some(dist)
}
