//! BO2's elem flags, effect flags, effect status words and culled state.
//!
//! These are BO2's own bits. MW2 read collision at 0x100 and die-on-touch at
//! 0x200; in BO2 those are 0x200 and 0x400, and 0x100 is a run mode.

pub const FX_ELEM_SPAWN_RELATIVE_TO_EFFECT: u32 = 0x2;
pub const CULL_AT_BIRTH_BIT: u32 = 0x4;
pub const ELEM_RUNNER_RANDOM_ROLL: u32 = 0x8;
pub const FX_ELEM_SPAWN_OFFSET_SPHERE: u32 = 0x10;
pub const FX_ELEM_SPAWN_OFFSET_CYLINDER: u32 = 0x20;
pub const BIRTH_DISPLACEMENT_BITS: u32 = 0x30;
pub const RUN_IN_SPAWN_FRAME: u32 = 0x40;
pub const RUN_IN_EFFECT_FRAME: u32 = 0x80;
pub const RUN_IN_OFFSET_FRAME: u32 = 0xc0;
pub const RUN_IN_OFFSET_FRAME_NOW: u32 = 0x100;
pub const RUN_FRAME_MASK: u32 = 0x1c0;
pub const ELEM_HITS_WORLD: u32 = 0x200;
pub const ELEM_DIES_ON_CONTACT: u32 = 0x400;
pub const FX_ELEM_DRAW_PAST_FOG: u32 = 0x800;
pub const FX_ELEM_DRAW_WITH_VIEWMODEL: u32 = 0x1000;
pub const FX_ELEM_BLOCK_SIGHT: u32 = 0x2000;
pub const FX_ELEM_USE_ITEM_CLIP: u32 = 0x4000;
pub const FX_ELEM_USE_OCCLUSION_QUERY: u32 = 0x8000;
pub const FX_ELEM_SUPERFLARE: u32 = 0x1_0000;
pub const FX_ELEM_USE_THERMAL: u32 = 0x2_0000;
pub const FX_ELEM_DISABLE_ALPHA_CULLING: u32 = 0x4_0000;
pub const FX_ELEM_USE_WORLD_UP: u32 = 0x8_0000;
pub const FX_ELEM_ALIGN_VIEWPOINT: u32 = 0x10_0000;
pub const FX_ELEM_USE_BILLBOARD_PIVOT: u32 = 0x20_0000;
pub const FX_ELEM_GAUSSIAN_CLOUD: u32 = 0x40_0000;
pub const FX_ELEM_USE_ROTATION_AXIS: u32 = 0x80_0000;
pub const FX_ELEM_VELOCITY_LOCAL: u32 = 0x100_0000;
pub const FX_ELEM_VELOCITY_WORLD: u32 = 0x200_0000;
pub const FX_ELEM_HAS_GRAVITY: u32 = 0x400_0000;
pub const FX_ELEM_MODEL_PHYSICS: u32 = 0x800_0000;
pub const FX_ELEM_NON_UNIFORM_SCALE: u32 = 0x1000_0000;
pub const FX_ELEM_FLAME_CHUNK: u32 = 0x2000_0000;
pub const FX_ELEM_HAS_REFLECTION: u32 = 0x4000_0000;
pub const FX_ELEM_MATURE_CONTENT: u32 = 0x8000_0000;

pub const FX_EFFECT_NEEDS_LIGHTING: u16 = 0x1;
pub const FX_EFFECT_HAS_SPOT_LIGHT: u16 = 0x2;
pub const FX_EFFECT_USE_BOUNDING_BOX: u16 = 0x4;
pub const FX_EFFECT_LOCAL_BOUNDING_BOX: u16 = 0x8;
pub const FX_EFFECT_HAS_OMNI_LIGHT: u16 = 0x10;
pub const FX_EFFECT_HAS_MODEL: u16 = 0x20;
pub const FX_EFFECT_HAS_SOUND: u16 = 0x40;
pub const FX_EFFECT_HAS_RUNNER: u16 = 0x80;
pub const FX_EFFECT_HAS_TRAIL: u16 = 0x100;
pub const FX_EFFECT_BBOX_SPAWN_CULL: u16 = 0x200;
pub const FX_EFFECT_BBOX_PORTALLED: u16 = 0x400;
pub const FX_EFFECT_IGNORE_TELEPORTING: u16 = 0x800;
pub const FX_EFFECT_FAST_OFFSCREEN_UPDATE: u16 = 0x8000;

pub const EMITTER_HOLDER_TALLY_BITS: u32 = 0x3fff;
pub const FX_STATUS_STALE_ATTACHED: u32 = 0x4000;
pub const FX_STATUS_BASE_REF_REMOVED: u32 = 0x8000;
pub const FX_STATUS_PENDING_LOOPING: u32 = 0x1_0000;
pub const FX_STATUS_OWNED_SHIFT: u32 = 17;
pub const FX_STATUS_OWNED_MASK: u32 = 0x7fe_0000;
pub const FX_STATUS_LOCK: u32 = 0x2000_0000;
pub const FX_STATUS_LOCK_MASK: u32 = 0x6000_0000;

pub const FX_STATUS2_NO_RESTART_ON_REWIND: u32 = 0x1;
pub const FX_STATUS2_OFFSCREEN_SPAWN: u32 = 0x4;
pub const FX_STATUS2_OFFSCREEN_RETRIGGER: u32 = 0x8;

pub const FX_CULLED_OUTSIDE: u8 = 3;
// BO2 unknown: the facts call the partial-test marker "bit 2"; read here as
// the value 2 (see facts/QUESTIONS.md).
pub const FX_CULLED_TESTED_PARTIAL: u8 = 2;

/// Where a running elem's position is taken relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxRunMode {
    /// No run bits. Not in BO2's table; world is MW2's meaning of 0.
    World,
    Spawn,
    Effect,
    Offset,
    OffsetEffectNow,
}

impl FxRunMode {
    /// None for run bits BO2 does not name (0x140, 0x180, 0x1c0).
    #[inline]
    pub fn from_flags(flags: u32) -> Option<Self> {
        match flags & RUN_FRAME_MASK {
            0 => Some(FxRunMode::World),
            RUN_IN_SPAWN_FRAME => Some(FxRunMode::Spawn),
            RUN_IN_EFFECT_FRAME => Some(FxRunMode::Effect),
            RUN_IN_OFFSET_FRAME => Some(FxRunMode::Offset),
            RUN_IN_OFFSET_FRAME_NOW => Some(FxRunMode::OffsetEffectNow),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxSpawnOffset {
    None,
    Sphere,
    Cylinder,
}

impl FxSpawnOffset {
    /// None when both offset bits are set.
    #[inline]
    pub fn from_flags(flags: u32) -> Option<Self> {
        match flags & BIRTH_DISPLACEMENT_BITS {
            0 => Some(FxSpawnOffset::None),
            FX_ELEM_SPAWN_OFFSET_SPHERE => Some(FxSpawnOffset::Sphere),
            FX_ELEM_SPAWN_OFFSET_CYLINDER => Some(FxSpawnOffset::Cylinder),
            _ => None,
        }
    }
}

#[inline]
pub fn status_ref_count(status: u32) -> u32 {
    status & EMITTER_HOLDER_TALLY_BITS
}

#[inline]
pub fn status_owned_count(status: u32) -> u32 {
    (status & FX_STATUS_OWNED_MASK) >> FX_STATUS_OWNED_SHIFT
}

#[inline]
pub fn status_with_owned_count(status: u32, owned: u32) -> u32 {
    (status & !FX_STATUS_OWNED_MASK) | ((owned << FX_STATUS_OWNED_SHIFT) & FX_STATUS_OWNED_MASK)
}

#[inline]
pub fn status_locked(status: u32) -> bool {
    status & FX_STATUS_LOCK_MASK != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::ELEM_DIES_ON_CONTACT;
    use super::ELEM_RUNNER_RANDOM_ROLL;
    use super::CULL_AT_BIRTH_BIT;
    use super::ELEM_HITS_WORLD;

    #[test]
    fn collision_bits_moved_up_one() {
        assert_eq!(ELEM_HITS_WORLD, 0x100 << 1);
        assert_eq!(ELEM_DIES_ON_CONTACT, 0x200 << 1);
        assert_eq!(RUN_FRAME_MASK & ELEM_HITS_WORLD, 0);
    }

    #[test]
    fn elem_flags_do_not_overlap() {
        let all = [
            FX_ELEM_SPAWN_RELATIVE_TO_EFFECT,
            CULL_AT_BIRTH_BIT,
            ELEM_RUNNER_RANDOM_ROLL,
            BIRTH_DISPLACEMENT_BITS,
            RUN_FRAME_MASK,
            ELEM_HITS_WORLD,
            ELEM_DIES_ON_CONTACT,
            FX_ELEM_DRAW_PAST_FOG,
            FX_ELEM_DRAW_WITH_VIEWMODEL,
            FX_ELEM_BLOCK_SIGHT,
            FX_ELEM_USE_ITEM_CLIP,
            FX_ELEM_USE_OCCLUSION_QUERY,
            FX_ELEM_SUPERFLARE,
            FX_ELEM_USE_THERMAL,
            FX_ELEM_DISABLE_ALPHA_CULLING,
            FX_ELEM_USE_WORLD_UP,
            FX_ELEM_ALIGN_VIEWPOINT,
            FX_ELEM_USE_BILLBOARD_PIVOT,
            FX_ELEM_GAUSSIAN_CLOUD,
            FX_ELEM_USE_ROTATION_AXIS,
            FX_ELEM_VELOCITY_LOCAL,
            FX_ELEM_VELOCITY_WORLD,
            FX_ELEM_HAS_GRAVITY,
            FX_ELEM_MODEL_PHYSICS,
            FX_ELEM_NON_UNIFORM_SCALE,
            FX_ELEM_FLAME_CHUNK,
            FX_ELEM_HAS_REFLECTION,
            FX_ELEM_MATURE_CONTENT,
        ];
        let mut seen = 0u32;
        for f in all {
            assert_eq!(seen & f, 0, "{f:#x}");
            seen |= f;
        }
        assert_eq!(seen, !1);
    }

    #[test]
    fn run_modes() {
        assert_eq!(FxRunMode::from_flags(0), Some(FxRunMode::World));
        assert_eq!(FxRunMode::from_flags(0x40), Some(FxRunMode::Spawn));
        assert_eq!(FxRunMode::from_flags(0x80 | 0x200), Some(FxRunMode::Effect));
        assert_eq!(FxRunMode::from_flags(0xc0), Some(FxRunMode::Offset));
        assert_eq!(
            FxRunMode::from_flags(0x100),
            Some(FxRunMode::OffsetEffectNow)
        );
        assert_eq!(FxRunMode::from_flags(0x140), None);
        assert_eq!(FxSpawnOffset::from_flags(0x30), None);
        assert_eq!(
            FxSpawnOffset::from_flags(0x20),
            Some(FxSpawnOffset::Cylinder)
        );
    }

    #[test]
    fn status_words() {
        let s = status_with_owned_count(5 | FX_STATUS_PENDING_LOOPING, 1023);
        assert_eq!(status_owned_count(s), 1023);
        assert_eq!(status_ref_count(s), 5);
        assert_ne!(s & FX_STATUS_PENDING_LOOPING, 0);
        assert!(!status_locked(s));
        assert!(status_locked(s | FX_STATUS_LOCK));
    }
}
