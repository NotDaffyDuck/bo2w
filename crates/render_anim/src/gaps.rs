//! The presentation gaps render_anim can be standing in: things the player
//! should be seeing (a remote body, its gun, the first-person gun) that the
//! producers could not put on screen, and why.

use core::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use bevy::prelude::*;
use diag::gap::{self as ledger, Gap as _, GapLedger};

const RENDER_GAP_COUNT: usize = <RenderGap as ledger::Gap>::ALL.len();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderGap {
    RemoteBodyAnimation,
    RemoteBodyWorldGun,
    FpvViewmodel,
}

impl ledger::Gap for RenderGap {
    const ALL: &'static [RenderGap] = &[
        RenderGap::RemoteBodyAnimation,
        RenderGap::RemoteBodyWorldGun,
        RenderGap::FpvViewmodel,
    ];

    fn name(self) -> &'static str {
        match self {
            RenderGap::RemoteBodyAnimation => "remote-body-animation",
            RenderGap::RemoteBodyWorldGun => "remote-body-world-gun",
            RenderGap::FpvViewmodel => "fpv-viewmodel",
        }
    }

    fn is_standing(self) -> bool {
        false
    }
}

/// Why a gap is open. Each cause belongs to exactly one gap.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RenderGapCause {
    RemoteBodySubmitMissing,
    RemoteBodyLightingAllocFailed,
    RemoteBodyMaterialMissing {
        name: String,
    },
    RemoteBodyWorldGunMissing {
        weapon: u32,
        world_model: String,
    },
    FpvCatalogMissing,
    FpvGunXModelUnresolved {
        weapon_id: u32,
    },
    FpvDependencyUnresolved {
        weapon_id: u32,
        role: &'static str,
        name: String,
    },
    FpvEyePoseFailed {
        gun_xmodel: String,
    },
    FpvNoCamera,
    FpvNoLightingAtlas,
    FpvNoActiveClips,
}

impl ledger::GapCause for RenderGapCause {
    type Gap = RenderGap;

    fn gap(&self) -> RenderGap {
        use RenderGapCause::*;
        match self {
            RemoteBodySubmitMissing
            | RemoteBodyLightingAllocFailed
            | RemoteBodyMaterialMissing { .. } => RenderGap::RemoteBodyAnimation,
            RemoteBodyWorldGunMissing { .. } => RenderGap::RemoteBodyWorldGun,
            FpvCatalogMissing
            | FpvGunXModelUnresolved { .. }
            | FpvDependencyUnresolved { .. }
            | FpvEyePoseFailed { .. }
            | FpvNoCamera
            | FpvNoLightingAtlas
            | FpvNoActiveClips => RenderGap::FpvViewmodel,
        }
    }
}

impl RenderGapCause {
    pub fn label(&self) -> &'static str {
        use RenderGapCause::*;
        match self {
            RemoteBodySubmitMissing => "remote body submit missing",
            RemoteBodyLightingAllocFailed => "remote body lighting alloc failed",
            RemoteBodyMaterialMissing { .. } => "remote body material missing",
            RemoteBodyWorldGunMissing { .. } => "remote body world gun missing",
            FpvCatalogMissing => "FPV catalog missing",
            FpvGunXModelUnresolved { .. } => "gunXModel[0] unresolved",
            FpvDependencyUnresolved { .. } => "FPV dependency unresolved",
            FpvEyePoseFailed { .. } => "FPV eye-pose failed",
            FpvNoCamera => "FPV waiting for camera",
            FpvNoLightingAtlas => "FPV hidden: ModelLightingCache atlas missing",
            FpvNoActiveClips => "FPV no active clips this frame",
        }
    }
}

impl fmt::Display for RenderGapCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RenderGapCause::*;
        let label = self.label();
        match self {
            RemoteBodyMaterialMissing { name } if name.is_empty() => {
                write!(f, "{label}: authored name not retained at capture")
            }
            RemoteBodyMaterialMissing { name } => {
                write!(f, "{label}: `{name}` not in global catalog")
            }
            RemoteBodyWorldGunMissing {
                weapon,
                world_model,
            } => {
                write!(f, "{label}: weapon {weapon} `{world_model}`")
            }
            FpvGunXModelUnresolved { weapon_id } => write!(f, "{label} for weapon {weapon_id}"),
            FpvDependencyUnresolved {
                weapon_id,
                role,
                name,
            } => {
                write!(f, "{label} for weapon {weapon_id}: {role} `{name}`")
            }
            FpvEyePoseFailed { gun_xmodel } => {
                write!(
                    f,
                    "{label} for `{gun_xmodel}` (need viewhands+gun skels with tag_view)"
                )
            }
            RemoteBodySubmitMissing
            | RemoteBodyLightingAllocFailed
            | FpvCatalogMissing
            | FpvNoCamera
            | FpvNoLightingAtlas
            | FpvNoActiveClips => f.write_str(label),
        }
    }
}

#[derive(Debug, Default)]
struct GapState {
    ledger: GapLedger<RenderGapCause, RENDER_GAP_COUNT>,
    /// The signature of the last set of gaps written to the log.
    reported: String,
    /// Something changed since the log was last considered.
    report_dirty: bool,
}

/// The gaps currently open, shared by the producers that raise and clear them.
#[derive(Resource, Debug, Default)]
pub struct RenderPresentationGaps {
    state: Mutex<GapState>,
}

impl RenderPresentationGaps {
    fn lock(&self) -> MutexGuard<'_, GapState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn raise(&self, cause: RenderGapCause) {
        if cause == RenderGapCause::RemoteBodyLightingAllocFailed {
            perf::lighting_fail();
        }
        let mut state = self.lock();
        let gap = ledger::GapCause::gap(&cause);
        state.report_dirty |= state.ledger.cause(gap) != Some(&cause);
        state.ledger.raise(cause);
    }

    pub fn clear(&self, gap: RenderGap) {
        let mut state = self.lock();
        state.report_dirty |= state.ledger.is_live(gap);
        state.ledger.clear(gap);
    }

    pub fn is_live(&self, gap: RenderGap) -> bool {
        self.lock().ledger.is_live(gap)
    }

    pub fn cause(&self, gap: RenderGap) -> Option<RenderGapCause> {
        self.lock().ledger.cause(gap).cloned()
    }

    pub fn hits(&self, gap: RenderGap) -> u64 {
        self.lock().ledger.hits(gap)
    }

    pub fn live(&self) -> Vec<RenderGap> {
        self.lock().ledger.live().collect()
    }

    pub fn count(&self) -> usize {
        self.lock().ledger.count()
    }
}

/// Logs the open gaps, but only when the set (or a cause) changed since the
/// last line written.
pub(crate) fn report_render_gaps(gaps: Res<RenderPresentationGaps>) {
    let lines: Vec<String> = {
        let mut state = gaps.lock();
        if !std::mem::take(&mut state.report_dirty) {
            return;
        }
        let mut signature = String::new();
        state
            .ledger
            .write_signature(&mut signature)
            .expect("writing into a String cannot fail");
        if signature == state.reported {
            return;
        }
        state.reported = signature;
        state
            .ledger
            .live()
            .filter_map(|gap| {
                Some(format!(
                    "render gap {}: {}",
                    gap.name(),
                    state.ledger.cause(gap)?
                ))
            })
            .collect()
    };
    if lines.is_empty() {
        diag::info!(World, "render: no presentation gaps live");
    }
    for line in lines {
        diag::info!(World, "{}", line);
    }
}

pub fn register_render_gaps(app: &mut App) {
    app.init_resource::<RenderPresentationGaps>()
        .add_systems(Update, report_render_gaps.in_set(net::ClientSet::Diag));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger::GapCause as _;

    fn body_material(name: &str) -> RenderGapCause {
        RenderGapCause::RemoteBodyMaterialMissing {
            name: name.to_owned(),
        }
    }

    fn eye_pose() -> RenderGapCause {
        RenderGapCause::FpvEyePoseFailed {
            gun_xmodel: "viewmodel_gun".to_owned(),
        }
    }

    #[test]
    fn each_cause_belongs_to_one_gap() {
        assert_eq!(
            RenderGapCause::RemoteBodySubmitMissing.gap(),
            RenderGap::RemoteBodyAnimation
        );
        assert_eq!(
            RenderGapCause::RemoteBodyLightingAllocFailed.gap(),
            RenderGap::RemoteBodyAnimation
        );
        assert_eq!(body_material("m").gap(), RenderGap::RemoteBodyAnimation);
        let gun = RenderGapCause::RemoteBodyWorldGunMissing {
            weapon: 3,
            world_model: "w".to_owned(),
        };
        assert_eq!(gun.gap(), RenderGap::RemoteBodyWorldGun);
        for cause in [
            RenderGapCause::FpvCatalogMissing,
            RenderGapCause::FpvGunXModelUnresolved { weapon_id: 1 },
            RenderGapCause::FpvDependencyUnresolved {
                weapon_id: 1,
                role: "hands",
                name: "n".to_owned(),
            },
            eye_pose(),
            RenderGapCause::FpvNoCamera,
            RenderGapCause::FpvNoLightingAtlas,
            RenderGapCause::FpvNoActiveClips,
        ] {
            assert_eq!(cause.gap(), RenderGap::FpvViewmodel, "{cause}");
        }
    }

    #[test]
    fn gaps_have_stable_names_and_none_is_standing() {
        let names: Vec<_> = RenderGap::ALL.iter().map(|gap| gap.name()).collect();
        assert_eq!(
            names,
            [
                "remote-body-animation",
                "remote-body-world-gun",
                "fpv-viewmodel"
            ]
        );
        assert!(RenderGap::ALL.iter().all(|gap| !gap.is_standing()));
    }

    #[test]
    fn causes_with_no_detail_print_their_label() {
        assert_eq!(
            RenderGapCause::FpvNoCamera.to_string(),
            "FPV waiting for camera"
        );
        assert_eq!(
            RenderGapCause::RemoteBodySubmitMissing.to_string(),
            "remote body submit missing"
        );
    }

    #[test]
    fn causes_with_detail_print_it() {
        assert_eq!(
            body_material("").to_string(),
            "remote body material missing: authored name not retained at capture"
        );
        assert_eq!(
            body_material("skin").to_string(),
            "remote body material missing: `skin` not in global catalog"
        );
        let gun = RenderGapCause::RemoteBodyWorldGunMissing {
            weapon: 12,
            world_model: "gun_w".to_owned(),
        };
        assert_eq!(
            gun.to_string(),
            "remote body world gun missing: weapon 12 `gun_w`"
        );
        assert_eq!(
            RenderGapCause::FpvGunXModelUnresolved { weapon_id: 5 }.to_string(),
            "gunXModel[0] unresolved for weapon 5"
        );
        let dependency = RenderGapCause::FpvDependencyUnresolved {
            weapon_id: 5,
            role: "hands",
            name: "arms".to_owned(),
        };
        assert_eq!(
            dependency.to_string(),
            "FPV dependency unresolved for weapon 5: hands `arms`"
        );
        assert_eq!(
            eye_pose().to_string(),
            "FPV eye-pose failed for `viewmodel_gun` (need viewhands+gun skels with tag_view)"
        );
    }

    #[test]
    fn raising_and_clearing_track_a_gap() {
        let gaps = RenderPresentationGaps::default();
        assert_eq!(gaps.count(), 0);
        gaps.raise(RenderGapCause::FpvNoCamera);
        assert!(gaps.is_live(RenderGap::FpvViewmodel));
        assert!(!gaps.is_live(RenderGap::RemoteBodyWorldGun));
        assert_eq!(
            gaps.cause(RenderGap::FpvViewmodel),
            Some(RenderGapCause::FpvNoCamera)
        );
        assert_eq!(gaps.live(), [RenderGap::FpvViewmodel]);
        assert_eq!(gaps.count(), 1);
        gaps.clear(RenderGap::FpvViewmodel);
        assert!(!gaps.is_live(RenderGap::FpvViewmodel));
        assert_eq!(gaps.cause(RenderGap::FpvViewmodel), None);
    }

    #[test]
    fn a_newer_cause_replaces_the_older_one_and_hits_accumulate() {
        let gaps = RenderPresentationGaps::default();
        gaps.raise(RenderGapCause::FpvNoCamera);
        gaps.raise(RenderGapCause::FpvNoActiveClips);
        assert_eq!(
            gaps.cause(RenderGap::FpvViewmodel),
            Some(RenderGapCause::FpvNoActiveClips)
        );
        assert_eq!(gaps.hits(RenderGap::FpvViewmodel), 2);
        assert_eq!(gaps.count(), 1);
    }

    #[test]
    fn the_log_is_only_reconsidered_after_a_change() {
        let gaps = RenderPresentationGaps::default();
        assert!(!gaps.lock().report_dirty);

        gaps.raise(RenderGapCause::FpvNoCamera);
        assert!(gaps.lock().report_dirty);
        gaps.lock().report_dirty = false;

        gaps.raise(RenderGapCause::FpvNoCamera);
        assert!(
            !gaps.lock().report_dirty,
            "the same cause again is not news"
        );
        gaps.raise(RenderGapCause::FpvNoActiveClips);
        assert!(gaps.lock().report_dirty);
        gaps.lock().report_dirty = false;

        gaps.clear(RenderGap::RemoteBodyAnimation);
        assert!(
            !gaps.lock().report_dirty,
            "clearing a closed gap is not news"
        );
        gaps.clear(RenderGap::FpvViewmodel);
        assert!(gaps.lock().report_dirty);
    }

    #[test]
    fn the_report_system_remembers_what_it_wrote() {
        let mut app = App::new();
        app.init_resource::<RenderPresentationGaps>()
            .add_systems(Update, report_render_gaps);
        let signature = |app: &App| {
            app.world()
                .resource::<RenderPresentationGaps>()
                .lock()
                .reported
                .clone()
        };

        app.update();
        assert_eq!(signature(&app), "");

        app.world()
            .resource::<RenderPresentationGaps>()
            .raise(RenderGapCause::FpvNoCamera);
        app.update();
        let opened = signature(&app);
        assert!(!opened.is_empty());
        assert!(
            !app.world()
                .resource::<RenderPresentationGaps>()
                .lock()
                .report_dirty
        );

        app.world()
            .resource::<RenderPresentationGaps>()
            .clear(RenderGap::FpvViewmodel);
        app.update();
        assert_ne!(signature(&app), opened);
    }

    #[test]
    fn a_lock_poisoned_by_a_panicking_producer_is_still_usable() {
        let gaps = RenderPresentationGaps::default();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = gaps.lock();
            panic!("producer failed while holding the ledger");
        }));
        assert!(poisoned.is_err());
        gaps.raise(RenderGapCause::FpvNoCamera);
        assert!(gaps.is_live(RenderGap::FpvViewmodel));
    }
}
