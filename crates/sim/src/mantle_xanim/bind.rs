//! The mantle's animation tree, bound to the clips the content loaded.

use super::MANTLE_XANIM_NAMES_T6;
use movement_t6::bridge::{
    CreateAnimsMantleRootDelta, FlatMantleAnimLength, MANTLE_XANIM_TREE_SIZE, MantleRootDelta, MantleXAnimLength, mantle,
};
use xmodel_runtime::AnimClip;

/// The climbs up are the first `UP_ANIMS` transitions of the mantle.
const UP_ANIMS: i32 = 7;

/// One animation of the tree: how long it runs, and the clip it plays, if the content has one.
#[derive(Clone, Debug, Default)]
struct Timing {
    msec: i32,
    clip: Option<AnimClip>,
}

impl Timing {
    fn from_clip(clip: Option<AnimClip>) -> Self {
        let msec = clip.as_ref().map_or(0, |c| c.length_msec().max(1));
        Self { msec, clip }
    }
}

fn is_climb_up(index: usize) -> bool {
    (0..UP_ANIMS).any(|transition| usize::try_from(mantle::trans_up_anim(transition)).is_ok_and(|up| up == index))
}

/// The tree twice: as it plays normally, and as it plays with the fast-mantle perk.
#[derive(Clone, Debug)]
pub struct MantleXAnimBind {
    normal: [Timing; MANTLE_XANIM_TREE_SIZE],
    perk: [Timing; MANTLE_XANIM_TREE_SIZE],
}

impl Default for MantleXAnimBind {
    fn default() -> Self {
        Self {
            normal: core::array::from_fn(|_| Timing::default()),
            perk: core::array::from_fn(|_| Timing::default()),
        }
    }
}

impl MantleXAnimBind {
    /// The index in the tree of the climb up for mantle transition `trans_index`.
    #[must_use]
    pub fn up_anim(trans_index: i32) -> i32 {
        mantle::trans_up_anim(trans_index)
    }

    /// The index in the tree of the vault over for mantle transition `trans_index`.
    #[must_use]
    pub fn over_anim(trans_index: i32) -> i32 {
        mantle::trans_over_anim(trans_index)
    }

    /// The name of the clip at `index` of the tree.
    #[must_use]
    pub fn clip_name_t6(index: usize) -> Option<&'static str> {
        MANTLE_XANIM_NAMES_T6.get(index).copied()
    }

    /// Binds the tree to the clips `get` finds for each index. With the perk the climb up runs `up_reduction` of its
    /// time and the rest of the tree is the same.
    pub fn from_t6_clips(mut get: impl FnMut(usize) -> Option<AnimClip>, up_reduction: f32) -> Self {
        let normal: [Timing; MANTLE_XANIM_TREE_SIZE] = core::array::from_fn(|index| Timing::from_clip(get(index)));
        let perk = core::array::from_fn(|index| {
            let mut timing = normal[index].clone();
            if timing.msec > 0 && is_climb_up(index) {
                timing.msec = ((timing.msec as f32 * up_reduction) as i32).max(1);
            }
            timing
        });
        Self { normal, perk }
    }

    fn timing(&self, fast_mantle: bool, anim: i32) -> Option<&Timing> {
        let tree = if fast_mantle { &self.perk } else { &self.normal };
        tree.get(usize::try_from(anim).ok()?)
    }
}

impl MantleXAnimLength for MantleXAnimBind {
    fn length_msec(&self, fast_mantle: bool, anim_index: i32) -> i32 {
        match self.timing(fast_mantle, anim_index) {
            Some(timing) if timing.msec > 0 => timing.msec,
            _ => FlatMantleAnimLength::default().length_msec(fast_mantle, anim_index),
        }
    }
}

impl MantleRootDelta for MantleXAnimBind {
    fn abs_delta(&self, fast_mantle: bool, anim_index: i32, frac: f32) -> [f32; 3] {
        let clip = self.timing(fast_mantle, anim_index).and_then(|timing| timing.clip.as_ref());
        match clip {
            // A clip without root motion stands still.
            Some(clip) if clip.has_delta() => clip.abs_delta_trans(frac),
            Some(_) => [0.0; 3],
            None => CreateAnimsMantleRootDelta.abs_delta(fast_mantle, anim_index, frac),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{PERK_MANTLE_REDUCTION_BO2MP, PERK_MANTLE_REDUCTION_DEFAULT};
    use super::*;
    use xmodel_runtime::Translation;

    fn clip(frames: u16, end: [f32; 3]) -> AnimClip {
        AnimClip {
            name: String::new(),
            framerate: 30.0,
            numframes: frames,
            looping: false,
            tracks: Vec::new(),
            notifies: Vec::new(),
            delta_translation: Translation::Constant(end),
        }
    }

    /// The 57-unit climb up (26 frames) and the high vault (10 frames), at 30 frames a second.
    fn climb_and_vault(index: usize) -> Option<AnimClip> {
        match index {
            1 => Some(clip(26, [16.0, 0.0, 57.0])),
            8 => Some(clip(10, [31.2, 0.0, -17.9])),
            _ => None,
        }
    }

    #[test]
    fn the_perk_shortens_the_climb_up_but_not_the_vault() {
        let bind = MantleXAnimBind::from_t6_clips(climb_and_vault, PERK_MANTLE_REDUCTION_DEFAULT);
        assert_eq!(bind.length_msec(false, 1), 866);
        assert_eq!(bind.length_msec(true, 1), 346);
        assert_eq!(bind.length_msec(false, 8), 333);
        assert_eq!(bind.length_msec(true, 8), 333);
    }

    #[test]
    fn the_shorter_climb_covers_the_same_ground() {
        let bind = MantleXAnimBind::from_t6_clips(climb_and_vault, PERK_MANTLE_REDUCTION_DEFAULT);
        assert_eq!(bind.abs_delta(true, 1, 1.0), bind.abs_delta(false, 1, 1.0));
    }

    #[test]
    fn bo2mps_perk_runs_the_climb_up_slower_than_bo2s() {
        let ours = MantleXAnimBind::from_t6_clips(climb_and_vault, PERK_MANTLE_REDUCTION_BO2MP);
        assert_eq!(ours.length_msec(true, 1), 692);
        assert_eq!(ours.length_msec(false, 1), 866);
    }

    #[test]
    fn a_missing_clip_falls_back_to_the_flat_timing() {
        let bind = MantleXAnimBind::from_t6_clips(|_| None, PERK_MANTLE_REDUCTION_DEFAULT);
        assert_eq!(
            bind.length_msec(false, 3),
            FlatMantleAnimLength::default().length_msec(false, 3)
        );
        assert_eq!(
            bind.abs_delta(false, 3, 0.5),
            CreateAnimsMantleRootDelta.abs_delta(false, 3, 0.5)
        );
    }

    #[test]
    fn the_low_vault_is_the_players_own_clip() {
        let low = usize::try_from(MantleXAnimBind::over_anim(6)).unwrap();
        assert_eq!(MantleXAnimBind::clip_name_t6(low), Some("player_mantle_over_low"));
        let top = usize::try_from(MantleXAnimBind::up_anim(0)).unwrap();
        assert_eq!(MantleXAnimBind::clip_name_t6(top), Some("mp_mantle_up_57"));
    }
}
