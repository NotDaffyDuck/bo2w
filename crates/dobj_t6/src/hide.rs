//! Hidden-part words and the surface tests that read them. BO2's part sets
//! (surface masks, hide bits, no-scale bits) are 5 words (facts 9), part 0
//! the top bit of word 0.

const BIT_CHUNK_COUNT: usize = 5;
const WORD_BITS: usize = 32;
const PART_CAPACITY: usize = BIT_CHUNK_COUNT * WORD_BITS;

const FIRST_PART: u32 = 1 << (WORD_BITS - 1);

/// The word a part lives in and its bit there; `None` past the last part.
fn locate(part: usize) -> Option<(usize, u32)> {
    if part >= PART_CAPACITY {
        return None;
    }
    Some((part / WORD_BITS, FIRST_PART >> (part % WORD_BITS)))
}

pub fn hide_part(chunks: &mut [u32; 5], part: usize) {
    if let Some((chunk, mask)) = locate(part) {
        chunks[chunk] |= mask;
    }
}

#[must_use]
pub fn part_is_hidden(chunks: &[u32; 5], part: usize) -> bool {
    locate(part).is_some_and(|(chunk, mask)| chunks[chunk] & mask == mask)
}

/// Adds `src` into `dst` with every part moved `joint_start_offset` places later (a
/// right shift of the whole 160-bit set, part 0 being its top bit).
pub fn merge_moved_parts(dst: &mut [u32; 5], src: [u32; 5], joint_start_offset: u32) {
    let skipped_words = joint_start_offset as usize / WORD_BITS;
    let shift = joint_start_offset % WORD_BITS as u32;
    for (at, out) in dst.iter_mut().enumerate() {
        let Some(from) = at.checked_sub(skipped_words) else {
            continue;
        };
        let carried_in = match (shift, from.checked_sub(1)) {
            (0, _) | (_, None) => 0,
            (_, Some(before)) => src[before] << (WORD_BITS as u32 - shift),
        };
        *out |= (src[from] >> shift) | carried_in;
    }
}

/// The part bits of a model whose bones start at `joint_start_offset`, as the object sees them.
fn lifted(surface_bits: &[u32; 5], joint_start_offset: u32) -> [u32; BIT_CHUNK_COUNT] {
    let mut moved = [0; BIT_CHUNK_COUNT];
    merge_moved_parts(&mut moved, *surface_bits, joint_start_offset);
    moved
}

/// MW2's rule: any hidden bone drops the whole surface.
#[must_use]
pub fn any_part_hidden(part_bits: &[u32; 5], hidden_set: &[u32; 5], joint_start_offset: u32) -> bool {
    let moved = lifted(part_bits, joint_start_offset);
    moved
        .iter()
        .zip(hidden_set)
        .map(|(part, hidden)| part & hidden)
        .any(|both| both != 0)
}

/// Black Ops II's rule: a surface is dropped only when every bone it is
/// skinned to is hidden (its part bits less the hidden ones are empty).
/// A partly hidden rigid surface keeps its other bones' vertex lists.
#[must_use]
pub fn all_parts_hidden(part_bits: &[u32; 5], hidden_set: &[u32; 5], joint_start_offset: u32) -> bool {
    let moved = lifted(part_bits, joint_start_offset);
    let has_bones = moved != [0; BIT_CHUNK_COUNT];
    has_bones
        && moved
            .iter()
            .zip(hidden_set)
            .all(|(part, hidden)| part & !hidden == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_read_are_msb_first() {
        let mut chunks = [0u32; 5];
        hide_part(&mut chunks, 33);
        hide_part(&mut chunks, PART_CAPACITY);
        assert_eq!(chunks, [0, 0x4000_0000, 0, 0, 0]);
        assert!(part_is_hidden(&chunks, 33));
        assert!(!part_is_hidden(&chunks, 32));
        assert!(!part_is_hidden(&chunks, PART_CAPACITY));
    }

    #[test]
    fn shift_carries_across_words() {
        let mut dst = [0u32; 5];
        merge_moved_parts(&mut dst, [1, 0x8000_0000, 0, 0, 0], 33);
        assert_eq!(dst[..3], [0, 0, 0xC000_0000]);
        let mut past_the_end = [0u32; 5];
        merge_moved_parts(&mut past_the_end, [u32::MAX; 5], 160);
        assert_eq!(past_the_end, [0; 5]);
    }

    #[test]
    fn any_versus_whole_rule() {
        let part_bits = [0xC000_0000, 0, 0, 0, 0];
        let mut hide = [0u32; 5];
        hide_part(&mut hide, 0);
        assert!(any_part_hidden(&part_bits, &hide, 0));
        assert!(!all_parts_hidden(&part_bits, &hide, 0));
        hide_part(&mut hide, 1);
        assert!(all_parts_hidden(&part_bits, &hide, 0));
        assert!(!all_parts_hidden(&[0; 5], &hide, 0));
    }
}
