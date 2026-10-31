//! The BG random numbers the player anim script picks with
//! (`facts/xanim-dobj-t6.md` section 6): a linear congruential generator
//! over a 32-bit state the caller holds ("holdrand").

/// `BG_rand`: steps the state and returns its top 15 bits (0..=32767).
pub fn bg_rand(seed_state: &mut u32) -> u32 {
    *seed_state = seed_state.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    *seed_state >> 17
}

/// `BG_irand`: `min + ((r * (max - min)) >> 15)`, so `min..max`.
pub fn bg_irand(seed_state: &mut u32, min: i32, max: i32) -> i32 {
    let r = bg_rand(seed_state) as i32;
    min + ((r * (max - min)) >> 15)
}

/// `BG_random`: `r / 32768`, so `0.0..1.0`.
pub fn bg_random(seed_state: &mut u32) -> f32 {
    bg_rand(seed_state) as f32 * 3.0517578e-05
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcg_steps() {
        let mut s = 0;
        assert_eq!(bg_rand(&mut s), 0x269ec3 >> 17);
        assert_eq!(s, 0x269ec3);
        let mut s = 1;
        let r = bg_rand(&mut s);
        assert_eq!(s, 0x343fd + 0x269ec3);
        assert_eq!(r, s >> 17);
    }

    #[test]
    fn ranges() {
        let mut s = 12345;
        for _ in 0..1000 {
            let i = bg_irand(&mut s, 3, 7);
            assert!((3..7).contains(&i));
            let f = bg_random(&mut s);
            assert!((0.0..1.0).contains(&f));
        }
    }
}
