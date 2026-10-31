//! Keeps an effect's class-0 part list in draw order: authored order first,
//! then back to front from the camera.

use fx_t6::runtime::{elem_random_seed, existing_elem_sorts_before_new, sort_dist_to_cam_sq};

use crate::particle_bin::{NO_PARTICLE_TICKET, particle_bin_of_ticket};
use crate::facing::elem_world_origin;
use crate::host::EffectSystemHost;

/// Sorts the parts added since the last pass into the already-sorted tail of
/// the effect's class-0 list.
///
/// New parts sit at the head of the list, in front of
/// `first_sorted_elem_handle`. Each is unlinked and inserted at its place in
/// the sorted run, one at a time.
pub fn order_fresh_particles(
    host: &mut EffectSystemHost,
    fx_index: usize,
    eye_pos: [f32; 3],
) {
    let Some(effect) = host.emitter_in_slot(fx_index) else {
        return;
    };
    let sorted_head = effect.first_part_ticket;
    let mut pending = effect.lead_particle_ticket[0];
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
    if let Some(effect) = host.live_effect_mut(fx_index) {
        effect.lead_particle_ticket[0] = sorted_head;
    }
    if let Some(slot) = particle_bin_of_ticket(sorted_head)
        && let Some(elem) = host.elems.get_mut(slot)
    {
        elem.preceding_particle_ticket = NO_PARTICLE_TICKET;
    }

    while pending != sorted_head {
        let Some(following) = detach(host, pending) else {
            break;
        };
        insert_sorted(host, fx_index, pending, eye_pos);
        pending = following;
    }

    if let Some(effect) = host.live_effect_mut(fx_index) {
        let head = effect.lead_particle_ticket[0];
        effect.first_part_ticket = head;
    }
}

/// Cuts a part out of its links and returns the handle that followed it.
fn detach(host: &mut EffectSystemHost, handle: u16) -> Option<u16> {
    let elem = host.elems.get_mut(particle_bin_of_ticket(handle)?)?;
    let following = std::mem::replace(&mut elem.following_particle_ticket, NO_PARTICLE_TICKET);
    elem.preceding_particle_ticket = NO_PARTICLE_TICKET;
    Some(following)
}

/// Points the part behind `handle` at `prev`/`next` where those are given.
fn relink(host: &mut EffectSystemHost, ticket: u16, prev: Option<u16>, next: Option<u16>) {
    let Some(elem) = particle_bin_of_ticket(ticket).and_then(|slot| host.elems.get_mut(slot))
    else {
        return;
    };
    elem.preceding_particle_ticket = prev.unwrap_or(elem.preceding_particle_ticket);
    elem.following_particle_ticket = next.unwrap_or(elem.following_particle_ticket);
}

/// Links `incoming` into the effect's list before the first part that does
/// not sort ahead of it.
fn insert_sorted(
    host: &mut EffectSystemHost,
    fx_index: usize,
    arriving: u16,
    eye_pos: [f32; 3],
) {
    let Some(new_slot) = particle_bin_of_ticket(arriving) else {
        return;
    };
    let new_order = match host.elems.get(new_slot) {
        Some(e) if e.is_taken => e.paint_rank,
        _ => return,
    };
    let Some(new_dist) = distance_to_camera(host, fx_index, new_slot, eye_pos) else {
        return;
    };
    let Some(effect) = host.emitter_in_slot(fx_index) else {
        return;
    };

    let mut before = NO_PARTICLE_TICKET;
    let mut at = effect.lead_particle_ticket[0];
    while at != NO_PARTICLE_TICKET {
        let Some(at_slot) = particle_bin_of_ticket(at) else {
            break;
        };
        let Some(existing) = host.elems.get(at_slot).filter(|e| e.is_taken) else {
            break;
        };
        let (kind, visuals, order, next) = (
            existing.part_kind,
            existing.look_variant_total,
            existing.paint_rank,
            existing.following_particle_ticket,
        );
        // Distance only decides between parts of the same authored order, so
        // it is measured only for those.
        let dist = if kind <= 3 && visuals != 0 && order == new_order {
            let Some(d) = distance_to_camera(host, fx_index, at_slot, eye_pos) else {
                break;
            };
            d
        } else {
            0.0
        };
        if !existing_elem_sorts_before_new(kind, visuals, order, dist, new_order, new_dist) {
            break;
        }
        before = at;
        at = next;
    }

    relink(host, arriving, Some(before), Some(at));
    if before == NO_PARTICLE_TICKET {
        if let Some(effect) = host.live_effect_mut(fx_index) {
            effect.lead_particle_ticket[0] = arriving;
        }
    } else {
        relink(host, before, None, Some(arriving));
    }
    relink(host, at, Some(arriving), None);
}

/// Squared distance from the camera to a part's world position, cached per
/// pass.
fn distance_to_camera(
    host: &mut EffectSystemHost,
    fx_index: usize,
    elem_slot: usize,
    eye_pos: [f32; 3],
) -> Option<f32> {
    let (epoch, cached) = host.sort_distances[elem_slot];
    if epoch == host.sort_epoch {
        return Some(cached);
    }
    let part = host.elems.get(elem_slot)?;
    let effect = host.emitter_in_slot(fx_index)?;
    let seed = elem_random_seed(effect.dice_start, part.nth, part.window_open_ms);
    let world = elem_world_origin(
        part.at_point,
        part.bits,
        &effect.current_frame(),
        &effect.spawn_frame(),
        Some(part.spawn_facing(seed)),
    );
    let dist = sort_dist_to_cam_sq(eye_pos, world);
    host.sort_distances[elem_slot] = (host.sort_epoch, dist);
    Some(dist)
}
