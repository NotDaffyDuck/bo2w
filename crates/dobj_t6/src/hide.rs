//! Hidden-part words and the surface tests that read them. BO2's part sets
//! (surface masks, hide bits, no-scale bits) are 5 words (facts 9), part 0
//! the top bit of word 0.

const PART_CAPACITY: usize = 5 * 32;

pub fn set_hide_part_bit(words: &mut [u32; 5], bone: usize) {
    if bone >= PART_CAPACITY {
        return;
    }
    words[bone >> 5] |= 0x8000_0000u32 >> (bone & 31);
}

#[must_use]
pub fn hide_part_bit(words: &[u32; 5], bone: usize) -> bool {
    bone < PART_CAPACITY && words[bone >> 5] & (0x8000_0000u32 >> (bone & 31)) != 0
}

pub fn or_shift_part_bits(dst: &mut [u32; 5], src: [u32; 5], bone_base: u32) {
    let word_shift = (bone_base >> 5) as usize;
    let bit_shift = bone_base & 31;
    if bit_shift == 0 {
        for i in 0..5 {
            if i >= word_shift {
                dst[i] |= src[i - word_shift];
            }
        }
        return;
    }
    let inv = 32 - bit_shift;
    for i in 0..5 {
        let low = if i >= word_shift {
            src[i - word_shift] >> bit_shift
        } else {
            0
        };
        let high = if i > word_shift {
            src[i - word_shift - 1] << inv
        } else {
            0
        };
        dst[i] |= low | high;
    }
}

/// MW2's rule: any hidden bone drops the whole surface.
#[must_use]
pub fn surface_hidden(part_bits: &[u32; 5], hide: &[u32; 5], bone_base: u32) -> bool {
    let mut lifted = [0u32; 5];
    or_shift_part_bits(&mut lifted, *part_bits, bone_base);
    lifted.iter().zip(hide.iter()).any(|(a, b)| a & b != 0)
}

/// Black Ops II's rule: a surface is dropped only when every bone it is
/// skinned to is hidden (its part bits less the hidden ones are empty).
/// A partly hidden rigid surface keeps its other bones' vertex lists.
#[must_use]
pub fn surface_hidden_whole(part_bits: &[u32; 5], hide: &[u32; 5], bone_base: u32) -> bool {
    let mut lifted = [0u32; 5];
    or_shift_part_bits(&mut lifted, *part_bits, bone_base);
    lifted.iter().any(|a| *a != 0) && lifted.iter().zip(hide.iter()).all(|(a, b)| a & !b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_read_are_msb_first() {
        let mut words = [0u32; 5];
        set_hide_part_bit(&mut words, 33);
        set_hide_part_bit(&mut words, PART_CAPACITY);
        assert_eq!(words[1], 0x4000_0000);
        assert!(hide_part_bit(&words, 33));
        assert!(!hide_part_bit(&words, 32));
        assert!(!hide_part_bit(&words, PART_CAPACITY));
    }

    #[test]
    fn shift_carries_across_words() {
        let mut dst = [0u32; 5];
        or_shift_part_bits(&mut dst, [1, 0x8000_0000, 0, 0, 0], 33);
        assert_eq!(&dst[..3], &[0, 0, 0x8000_0000 | 0x4000_0000]);
    }

    #[test]
    fn any_versus_whole_rule() {
        let part_bits = [0xC000_0000, 0, 0, 0, 0];
        let mut hide = [0u32; 5];
        set_hide_part_bit(&mut hide, 0);
        assert!(surface_hidden(&part_bits, &hide, 0));
        assert!(!surface_hidden_whole(&part_bits, &hide, 0));
        set_hide_part_bit(&mut hide, 1);
        assert!(surface_hidden_whole(&part_bits, &hide, 0));
        assert!(!surface_hidden_whole(&[0; 5], &hide, 0));
    }
}
