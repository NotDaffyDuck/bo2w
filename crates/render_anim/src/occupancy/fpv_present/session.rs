//! What the first-person view keeps between frames: the viewmodel that is
//! currently equipped, whether the local player has been cleared to spawn one,
//! and the short status the HUD and the gap log read back.

use std::sync::Arc;

use bevy::prelude::*;

use crate::anim::fpv::EquippedFpv;
use crate::anim::fpv_prepared::FpvWeaponView;
use crate::anim::fpv_rig::PreparedFpvRig;
use crate::gaps::RenderGapCause;
use render_material::RuntimeMaterialCatalog;

/// The equipped viewmodel of this session, or nothing while none is spawned.
#[derive(Resource, Default)]
pub struct SessionViewmodel(pub Option<SessionFpvMeshesHandles>);

/// Everything the tick needs about the weapon that is currently drawn: which
/// weapon it is, the prepared view it came from, the live animation state and
/// the rig that skins it.
pub struct SessionFpvMeshesHandles {
    /// The weapon whose model is on screen.
    pub weapon_id: u32,
    /// The weapon it was raised from (an attachment-bearing variant names its
    /// base here).
    pub parent_weapon: u32,
    /// Identity of the mesh catalog the view was built against.
    pub catalog_id: u64,
    /// Whether the Axis variant of the hands was picked.
    pub axis: bool,
    /// The prepared slot, shared with the weapon table.
    pub view: Arc<FpvWeaponView>,
    /// The animation state driving it.
    pub fpv: EquippedFpv,
    pub(crate) active_rig: Option<Arc<PreparedFpvRig>>,
    pub(crate) material_catalog: Arc<RuntimeMaterialCatalog>,
}

/// True when both sets of side assemblies are the very same allocations, which
/// means swapping between the two weapons needs no new spawn.
pub(super) fn same_compositions(
    left: &asset_game::FpvSideAssemblies,
    right: &asset_game::FpvSideAssemblies,
) -> bool {
    fn same_optional<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
    Arc::ptr_eq(&left.bare, &right.bare)
        && same_optional(&left.melee, &right.melee)
        && same_optional(&left.ads, &right.ads)
        && same_optional(&left.rocket, &right.rocket)
}

/// Whether `owned` is the material catalog the scene currently renders with.
pub(super) fn same_material_catalog(
    owned: &Arc<RuntimeMaterialCatalog>,
    live: Option<&render_scene::TessMaterials>,
) -> bool {
    live.is_some_and(|live| Arc::ptr_eq(owned, &live.catalog))
}

/// Set once the session has told this module the local player may be given a
/// viewmodel.
#[derive(Resource, Default)]
pub struct LocalSpawnArmed(pub bool);

/// The last outcome of trying to put the viewmodel on screen.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct FpvStatusGap(pub Option<FpvState>);

/// Why the viewmodel is, or is not, drawn right now.
#[derive(Clone, Debug, PartialEq)]
pub enum FpvState {
    /// The player is dead or not yet spawned, so nothing is kept.
    ClearedNotAlive,
    /// The player holds no weapon.
    ClearedNoWeapon,
    /// A new weapon was noticed and its viewmodel is waiting for a spawn.
    Queued,
    /// On screen. `idle_sampled` says whether the first pose came from the
    /// weapon's idle animation or fell back to the bind pose.
    Drawn { idle_sampled: bool },
    /// Something is missing; the cause says what.
    Blocked(RenderGapCause),
}

impl FpvState {
    /// One line for logs and the debug overlay.
    pub fn label(&self) -> &'static str {
        const IDLE: &str = "spawned after Equip; idle-sampled eye-posed hands+gun; retained FPV + ModelLightingCache";
        const BIND: &str = "spawned after Equip; bind-pose eye-posed hands+gun; idle sample gap; retained FPV";
        match self {
            Self::ClearedNotAlive => "not Alive — FPV cleared",
            Self::ClearedNoWeapon => "held weapon 0 — FPV cleared",
            Self::Queued => "held weapon changed; FPV queued (linked gunXModel)",
            Self::Drawn { idle_sampled } => {
                if *idle_sampled {
                    IDLE
                } else {
                    BIND
                }
            }
            Self::Blocked(cause) => cause.label(),
        }
    }

    /// The gap cause when the state is `Blocked`.
    pub fn cause(&self) -> Option<&RenderGapCause> {
        if let Self::Blocked(cause) = self {
            Some(cause)
        } else {
            None
        }
    }
}

impl core::fmt::Display for FpvState {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Self::Blocked(cause) = self {
            return write!(out, "{cause}");
        }
        out.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_tell_the_two_drawn_cases_apart() {
        let sampled = FpvState::Drawn { idle_sampled: true }.label();
        let bind = FpvState::Drawn { idle_sampled: false }.label();
        assert_ne!(sampled, bind);
        assert!(sampled.contains("idle-sampled"));
        assert!(bind.contains("bind-pose"));
    }

    #[test]
    fn only_a_blocked_state_has_a_cause() {
        let cause = RenderGapCause::FpvNoCamera;
        assert_eq!(FpvState::Blocked(cause.clone()).cause(), Some(&cause));
        assert_eq!(FpvState::Queued.cause(), None);
        assert_eq!(FpvState::ClearedNotAlive.cause(), None);
    }

    #[test]
    fn blocked_displays_its_cause_and_others_their_label() {
        let cause = RenderGapCause::FpvNoCamera;
        assert_eq!(FpvState::Blocked(cause.clone()).to_string(), cause.to_string());
        assert_eq!(FpvState::Queued.to_string(), FpvState::Queued.label());
    }
}
