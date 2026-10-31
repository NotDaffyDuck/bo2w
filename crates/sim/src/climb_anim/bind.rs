//! The mantle's animation tree, bound to the clips the content loaded.

use super::MANTLE_XANIM_NAMES_T6;
use movement_t6::bridge::{
    ClipDrivenClimbOffset, FixedClimbClipDuration, CLIMB_CLIP_SLOTS, ClimbBoneOffsetSource, ClimbClipDuration, mantle,
};
use xmodel_runtime::SkeletalClip;

/// The climbs up are the first `UP_ANIMS` transitions of the mantle.
const UP_ANIMS: i32 = 7;

/// One animation of the tree: how long it runs, and the clip it plays, if the content has one.
#[derive(Clone, Debug, Default)]
struct Timing {
    msec: i32,
    source: Option<SkeletalClip>,
}

impl Timing {
    fn from_clip(source: Option<SkeletalClip>) -> Self {
        let msec = source.as_ref().map_or(0, |c| c.span_in_ms().max(1));
        Self { msec, source }
    }
}

fn is_climb_up(index: usize) -> bool {
    (0..UP_ANIMS).any(|transition| usize::try_from(mantle::trans_up_anim(transition)).is_ok_and(|up| up == index))
}

/// The tree twice: as it plays normally, and as it plays with the fast-mantle perk.
#[derive(Clone, Debug)]
pub struct LedgeClimbAnimSet {
    normal: [Timing; CLIMB_CLIP_SLOTS],
    perk: [Timing; CLIMB_CLIP_SLOTS],
}

impl Default for LedgeClimbAnimSet {
    fn default() -> Self {
        Self {
            normal: core::array::from_fn(|_| Timing::default()),
            perk: core::array::from_fn(|_| Timing::default()),
        }
    }
}

impl LedgeClimbAnimSet {
    /// The index in the tree of the climb up for mantle transition `transition`.
    #[must_use]
    pub fn up_anim(transition: i32) -> i32 {
        mantle::trans_up_anim(transition)
    }

    /// The index in the tree of the vault over for mantle transition `transition`.
    #[must_use]
    pub fn over_anim(transition: i32) -> i32 {
        mantle::trans_over_anim(transition)
    }

    /// The name of the clip at `index` of the tree.
    #[must_use]
    pub fn clip_name_t6(index: usize) -> Option<&'static str> {
        MANTLE_XANIM_NAMES_T6.get(index).copied()
    }

    /// Binds the tree to the clips `get` finds for each index. With the perk the climb up runs `up_reduction` of its
    /// time and the rest of the tree is the same.
    pub fn from_t6_clips(mut get: impl FnMut(usize) -> Option<SkeletalClip>, up_reduction: f32) -> Self {
        let normal: [Timing; CLIMB_CLIP_SLOTS] = core::array::from_fn(|index| Timing::from_clip(get(index)));
        let perk = core::array::from_fn(|index| {
            let mut timing = normal[index].clone();
            if timing.msec > 0 && is_climb_up(index) {
                timing.msec = ((timing.msec as f32 * up_reduction) as i32).max(1);
            }
            timing
        });
        Self { normal, perk }
    }

    fn timing(&self, clip: i32, sped_up: bool) -> Option<&Timing> {
        let tree = if sped_up { &self.perk } else { &self.normal };
        tree.get(usize::try_from(clip).ok()?)
    }
}

impl ClimbClipDuration for LedgeClimbAnimSet {
    fn span_in_ms(&self, perk: bool, anim: i32) -> i32 {
        match self.timing(anim, perk) {
            Some(timing) if timing.msec > 0 => timing.msec,
            _ => FixedClimbClipDuration::default().span_in_ms(perk, anim),
        }
    }
}

impl ClimbBoneOffsetSource for LedgeClimbAnimSet {
    fn root_travel(&self, perk: bool, anim: i32, blend: f32) -> [f32; 3] {
        let clip = self.timing(anim, perk).and_then(|timing| timing.source.as_ref());
        match clip {
            // A clip without root motion stands still.
            Some(clip) if clip.moves_root() => clip.absolute_root_shift(blend),
            Some(_) => [0.0; 3],
            None => ClipDrivenClimbOffset.root_travel(perk, anim, blend),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{PERK_MANTLE_REDUCTION_BO2MP, PERK_MANTLE_REDUCTION_DEFAULT};
    use super::*;
    use xmodel_runtime::RootShift;

    fn clip(frames: u16, end: [f32; 3]) -> SkeletalClip {
        SkeletalClip {
            name: String::new(),
            fps: 30.0,
            frame_total: frames,
            looping: false,
            tracks: Vec::new(),
            notifies: Vec::new(),
            root_motion_shift: RootShift::Fixed(end),
        }
    }

    /// The 57-unit climb up (26 frames) and the high vault (10 frames), at 30 frames a second.
    fn climb_and_vault(index: usize) -> Option<SkeletalClip> {
        match index {
            1 => Some(clip(26, [16.0, 0.0, 57.0])),
            8 => Some(clip(10, [31.2, 0.0, -17.9])),
            _ => None,
        }
    }

    #[test]
    fn the_perk_shortens_the_climb_up_but_not_the_vault() {
        let bind = LedgeClimbAnimSet::from_t6_clips(climb_and_vault, PERK_MANTLE_REDUCTION_DEFAULT);
        assert_eq!(bind.span_in_ms(false, 1), 866);
        assert_eq!(bind.span_in_ms(true, 1), 346);
        assert_eq!(bind.span_in_ms(false, 8), 333);
        assert_eq!(bind.span_in_ms(true, 8), 333);
    }

    #[test]
    fn the_shorter_climb_covers_the_same_ground() {
        let bind = LedgeClimbAnimSet::from_t6_clips(climb_and_vault, PERK_MANTLE_REDUCTION_DEFAULT);
        assert_eq!(bind.root_travel(true, 1, 1.0), bind.root_travel(false, 1, 1.0));
    }

    #[test]
    fn bo2mps_perk_runs_the_climb_up_slower_than_bo2s() {
        let ours = LedgeClimbAnimSet::from_t6_clips(climb_and_vault, PERK_MANTLE_REDUCTION_BO2MP);
        assert_eq!(ours.span_in_ms(true, 1), 692);
        assert_eq!(ours.span_in_ms(false, 1), 866);
    }

    #[test]
    fn a_missing_clip_falls_back_to_the_flat_timing() {
        let bind = LedgeClimbAnimSet::from_t6_clips(|_| None, PERK_MANTLE_REDUCTION_DEFAULT);
        assert_eq!(
            bind.span_in_ms(false, 3),
            FixedClimbClipDuration::default().span_in_ms(false, 3)
        );
        assert_eq!(
            bind.root_travel(false, 3, 0.5),
            ClipDrivenClimbOffset.root_travel(false, 3, 0.5)
        );
    }

    #[test]
    fn the_low_vault_is_the_players_own_clip() {
        let low = usize::try_from(LedgeClimbAnimSet::over_anim(6)).unwrap();
        assert_eq!(LedgeClimbAnimSet::clip_name_t6(low), Some("player_mantle_over_low"));
        let top = usize::try_from(LedgeClimbAnimSet::up_anim(0)).unwrap();
        assert_eq!(LedgeClimbAnimSet::clip_name_t6(top), Some("mp_mantle_up_57"));
    }
}
