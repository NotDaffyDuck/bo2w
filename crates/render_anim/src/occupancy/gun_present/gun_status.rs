//! What the first-person view keeps between frames: the viewmodel that is
//! currently equipped, whether the local player has been cleared to spawn one,
//! and the short status the HUD and the gap log read back.

use std::sync::Arc;

use bevy::prelude::*;

use crate::motion::fpv::HeldGunLoadout;
use crate::motion::fpv_prepared::FpvWeaponView;
use crate::motion::fpv_rig::PreparedFpvRig;
use crate::holes::Cause;
use render_material::LiveMaterialLedger;

/// The equipped viewmodel of this session, or nothing while none is spawned.
#[derive(Resource, Default)]
pub struct ActiveGunMeshes(pub Option<GunMeshHandleSet>);

/// Everything the tick needs about the weapon that is currently drawn: which
/// weapon it is, the prepared view it came from, the live animation state and
/// the rig that skins it.
pub struct GunMeshHandleSet {
    /// The weapon whose model is on screen.
    pub gun_catalog_no: u32,
    /// The weapon it was raised from (an attachment-travel_azimuth variant names its
    /// base here).
    pub parent_weapon: u32,
    /// Identity of the mesh catalog the view was built against.
    pub catalog_id: u64,
    /// Whether the Axis variant of the hands was picked.
    pub axes: bool,
    /// The prepared slot, shared with the weapon table.
    pub view: Arc<FpvWeaponView>,
    /// The animation state driving it.
    pub fpv: HeldGunLoadout,
    pub(crate) active_rig: Option<Arc<PreparedFpvRig>>,
    pub(crate) material_catalog: Arc<LiveMaterialLedger>,
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
    owned: &Arc<LiveMaterialLedger>,
    live: Option<&render_scene::StitchMaterials>,
) -> bool {
    live.is_some_and(|live| Arc::ptr_eq(owned, &live.catalog))
}

/// Set once the session has told this module the local player may be given a
/// viewmodel.
#[derive(Resource, Default)]
pub struct OwnSpawnPrimed(pub bool);

/// The last outcome of trying to put the viewmodel on screen.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct GunStatusShortfall(pub Option<ViewmodelStatus>);

/// Why the viewmodel is, or is not, drawn right now.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewmodelStatus {
    /// The player is dead or not yet spawned, so nothing is kept.
    ResetDead,
    /// The player holds no weapon.
    ResetUnarmed,
    /// A new weapon was noticed and its viewmodel is waiting for a spawn.
    Queued,
    /// On screen. `rest_pose_taken` says whether the first pose came from the
    /// weapon's idle animation or fell back to the bind pose.
    Drawn { rest_pose_taken: bool },
    /// Something is missing; the cause says what.
    Blocked(Cause),
}

impl ViewmodelStatus {
    /// One line for logs and the debug overlay.
    pub fn summary(&self) -> &'static str {
        const IDLE: &str = "Equip spawned it; hands and gun posed at the camera from the idle clip; FPV kept + ModelLightingCache";
        const BIND: &str = "Equip spawned it; hands and gun at the camera in their rest layout, no idle clip yet; FPV kept";
        match self {
            Self::ResetDead => "not Alive — FPV cleared",
            Self::ResetUnarmed => "held weapon 0 — FPV cleared",
            Self::Queued => "held weapon changed; FPV queued (linked gunXModel)",
            Self::Drawn { rest_pose_taken } => {
                if *rest_pose_taken {
                    IDLE
                } else {
                    BIND
                }
            }
            Self::Blocked(cause) => cause.title(),
        }
    }

    /// The gap cause when the state is `Blocked`.
    pub fn cause(&self) -> Option<&Cause> {
        if let Self::Blocked(why) = self {
            Some(why)
        } else {
            None
        }
    }
}

impl core::fmt::Display for ViewmodelStatus {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Self::Blocked(cause) = self {
            return write!(out, "{cause}");
        }
        out.write_str(self.summary())
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn labels_tell_the_two_drawn_cases_apart() {
        let sampled = ViewmodelStatus::Drawn { rest_pose_taken: true }.summary();
        let bind = ViewmodelStatus::Drawn { rest_pose_taken: false }.summary();
        assert_ne!(sampled, bind);
        assert!(sampled.contains("from the idle clip"));
        assert!(bind.contains("rest layout"));
    }

    #[test]
    fn only_a_blocked_state_has_a_cause() {
        let cause = Cause::GunNoViewer;
        assert_eq!(ViewmodelStatus::Blocked(cause.clone()).cause(), Some(&cause));
        assert_eq!(ViewmodelStatus::Queued.cause(), None);
        assert_eq!(ViewmodelStatus::ResetDead.cause(), None);
    }

    #[test]
    fn blocked_displays_its_cause_and_others_their_label() {
        let cause = Cause::GunNoViewer;
        assert_eq!(ViewmodelStatus::Blocked(cause.clone()).to_string(), cause.to_string());
        assert_eq!(ViewmodelStatus::Queued.to_string(), ViewmodelStatus::Queued.summary());
    }
}
