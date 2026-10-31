use super::*;

pub fn reserve_main_light_bits(
    word_list: &mut Vec<u32>,
    client_num_count: u32,
    sun: u32,
    main_light_total: u32,
) {
    let wanted = moving_entity_main_light_visibility_words(client_num_count, sun, main_light_total);
    if word_list.len() == wanted {
        return;
    }
    *word_list = vec![0; wanted];
}

/// Re-links one piece's primary-light bits from its box (or drops them when it
/// is dead) and records which light is closest to it.
pub fn relink_debris_key_lights(
    words_in: &mut Vec<u32>,
    closest_in: Option<&mut [u8]>,
    at_spot: &LooseAtSpotLookup,
    id: u32,
    bounds_in: Bounds,
    dead: bool,
) {
    let (count, sun) = (at_spot.main_light_total(), at_spot.sunlight_primary_flag_index());
    let nearest = match dead {
        true => {
            detach_moving_entity_from_main_lights(words_in, id, sun, count);
            0
        }
        false => {
            let (center, reach) = (bounds_in.mid(), bounds_in.half());
            at_spot.bind_moving_entity_main_light(center, reach, |light, set| {
                attach_dynamic_prop_key_light_bit(words_in, id, light, sun, count, set);
            })
        }
    };
    let row_slot = closest_in.and_then(|row| row.get_mut(usize::try_from(id).ok()?));
    if let Some(slot) = row_slot {
        *slot = nearest;
    }
}

// ---------------------------------------------------------------------------
// Sizes and boxes
// ---------------------------------------------------------------------------
