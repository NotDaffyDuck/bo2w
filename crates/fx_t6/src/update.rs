//! How an effect is updated and which scene it is drawn into.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FxUpdateType {
    Independent = 0,
    Bolted = 1,
    NonSprite = 2,
    Portalled = 3,
    Invalid = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FxUpdateResult {
    Remove = 0,
    Keep = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FxDrawScene {
    Main = 0,
    ExtraCamera = 1,
}

/// An effect is processed at most once per frame count.
#[inline]
pub fn should_process_this_frame(effect_frame_count: i32, system_frame_count: i32) -> bool {
    effect_frame_count != system_frame_count
}

/// A culled effect keeps a cheap update until the longer of its two lives.
#[inline]
pub fn culled_effect_alive(
    msec_since_begin: i32,
    msec_looping_life: i32,
    msec_non_looping_life: i32,
) -> bool {
    msec_since_begin < msec_looping_life.max(msec_non_looping_life)
}

/// Looping elems spawn until the effect's looping life runs out.
#[inline]
pub fn looping_spawn_active(msec_since_begin: i32, msec_looping_life: i32) -> bool {
    msec_since_begin < msec_looping_life
}

/// Elapsed over life, the elem's time along its curves. The life span is
/// above 0.
#[inline]
pub fn normalized_time(msec_elapsed: i32, life_span_msec: i32) -> f32 {
    msec_elapsed as f32 / life_span_msec as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lives() {
        assert!(culled_effect_alive(999, 1000, 10));
        assert!(!culled_effect_alive(1000, 1000, 10));
        assert!(culled_effect_alive(1500, 0, 2000));
        assert!(looping_spawn_active(10, 11));
        assert!(!looping_spawn_active(11, 11));
        assert!(!should_process_this_frame(4, 4));
        assert_eq!(normalized_time(250, 1000), 0.25);
    }
}
