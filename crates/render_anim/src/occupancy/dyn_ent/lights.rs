use super::*;

pub fn ensure_primary_light_words(
    words: &mut Vec<u32>,
    client_count: u32,
    sun: u32,
    primary_count: u32,
) {
    let wanted = dyn_ent_primary_light_vis_word_count(client_count, sun, primary_count);
    if words.len() != wanted {
        words.clear();
        words.resize(wanted, 0);
    }
}

/// Re-links one piece's primary-light bits from its box (or drops them when it
/// is dead) and records which light is closest to it.
pub fn relink_dyn_ent_primary_lights(
    words: &mut Vec<u32>,
    closest: Option<&mut [u8]>,
    atpoint: &DynAtPointLookup,
    id: u32,
    bounds: Bounds,
    dead: bool,
) {
    let sun = atpoint.sun_primary();
    let count = atpoint.primary_count();
    let nearest = if dead {
        unlink_dyn_ent_from_primary_lights(words, id, sun, count);
        0
    } else {
        atpoint.link_dyn_ent_primary(bounds.mid(), bounds.half(), |light, set| {
            link_dyn_ent_primary_light_bit(words, id, light, sun, count, set);
        })
    };
    if let Some(slot) = closest.and_then(|row| row.get_mut(id as usize)) {
        *slot = nearest;
    }
}

// ---------------------------------------------------------------------------
// Sizes and boxes
// ---------------------------------------------------------------------------
