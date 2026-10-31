//! The presentation gaps render_anim can be standing in: things the player
//! should be seeing (a remote body, its gun, the first-person gun) that the
//! producers could not put on screen, and why.

use core::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use bevy::prelude::*;
use diag::gap::{self, Gap as _, LackLedger};
use diag::info;

const GAP_SLOTS: usize = <PresentationShortfall as gap::Gap>::ALL_KINDS.len();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PresentationShortfall {
    BodyAnim,
    BodyHeldGun,
    GunViewLayer,
}

impl gap::Gap for PresentationShortfall {
    const ALL_KINDS: &'static [PresentationShortfall] = &[
        PresentationShortfall::BodyAnim,
        PresentationShortfall::BodyHeldGun,
        PresentationShortfall::GunViewLayer,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::BodyAnim => "remote-body-animation",
            Self::BodyHeldGun => "remote-body-world-gun",
            Self::GunViewLayer => "fpv-viewmodel",
        }
    }

    fn still_open(self) -> bool {
        false
    }
}

/// Why a gap is open. Each cause belongs to exactly one gap.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Cause {
    BodySubmitAbsent,
    BodyLightNoSlot,
    RemoteBodyMaterialMissing {
        name: String,
    },
    BodyHeldGunAbsent {
        weap: u32,
        third_person_mesh: String,
    },
    GunTableAbsent,
    FpvGunXModelUnresolved {
        gun_catalog_no: u32,
    },
    FpvDependencyUnresolved {
        gun_catalog_no: u32,
        part_role: &'static str,
        name: String,
    },
    FpvEyePoseFailed {
        weapon_mesh: String,
    },
    GunNoViewer,
    GunNoLightAtlas,
    GunNoPlayingClips,
}

impl gap::ShortfallOrigin for Cause {
    type Gap = PresentationShortfall;

    fn gap(&self) -> PresentationShortfall {
        match self {
            Self::BodySubmitAbsent
            | Self::BodyLightNoSlot
            | Self::RemoteBodyMaterialMissing { .. } => PresentationShortfall::BodyAnim,
            Self::BodyHeldGunAbsent { .. } => PresentationShortfall::BodyHeldGun,
            Self::GunTableAbsent
            | Self::FpvGunXModelUnresolved { .. }
            | Self::FpvDependencyUnresolved { .. }
            | Self::FpvEyePoseFailed { .. }
            | Self::GunNoViewer
            | Self::GunNoLightAtlas
            | Self::GunNoPlayingClips => PresentationShortfall::GunViewLayer,
        }
    }
}

impl Cause {
    /// The short phrase for the cause, without its detail.
    pub fn title(&self) -> &'static str {
        match self {
            Self::BodySubmitAbsent => "remote body submit missing",
            Self::BodyLightNoSlot => "remote body lighting alloc failed",
            Self::RemoteBodyMaterialMissing { .. } => "remote body material missing",
            Self::BodyHeldGunAbsent { .. } => "remote body world gun missing",
            Self::GunTableAbsent => "FPV catalog missing",
            Self::FpvGunXModelUnresolved { .. } => "gunXModel[0] unresolved",
            Self::FpvDependencyUnresolved { .. } => "FPV dependency unresolved",
            Self::FpvEyePoseFailed { .. } => "FPV eye-pose failed",
            Self::GunNoViewer => "FPV waiting for camera",
            Self::GunNoLightAtlas => "FPV hidden: ModelLightingCache atlas missing",
            Self::GunNoPlayingClips => "FPV no active clips this frame",
        }
    }

    /// What follows the label when the cause carries detail.
    fn detail(&self) -> String {
        match self {
            Self::RemoteBodyMaterialMissing { name } if name.is_empty() => {
                ": authored name not retained at capture".to_owned()
            }
            Self::RemoteBodyMaterialMissing { name } => {
                format!(": `{name}` not in global catalog")
            }
            Self::BodyHeldGunAbsent {
                weap: weapon,
                third_person_mesh: model,
            } => {
                format!(": weapon {weapon} `{model}`")
            }
            Self::FpvGunXModelUnresolved { gun_catalog_no } => format!(" for weapon {gun_catalog_no}"),
            Self::FpvDependencyUnresolved {
                gun_catalog_no,
                part_role: role,
                name,
            } => format!(" for weapon {gun_catalog_no}: {role} `{name}`"),
            Self::FpvEyePoseFailed { weapon_mesh } => {
                format!(" for `{weapon_mesh}` (need viewhands+gun skels with tag_view)")
            }
            _ => String::new(),
        }
    }
}

impl fmt::Display for Cause {
    fn fmt(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(sink, "{}{}", self.title(), self.detail())
    }
}

#[derive(Debug, Default)]
struct GapState {
    ledger: LackLedger<Cause, GAP_SLOTS>,
    /// The signature of the last set of gaps written to the log.
    last_signature: String,
    /// Something changed since the log was last considered.
    unlogged: bool,
}

/// The gaps currently open, shared by the producers that raise and clear them.
#[derive(Resource, Debug, Default)]
pub struct ShortfallLedger {
    state: Mutex<GapState>,
}

impl ShortfallLedger {
    /// The state behind the mutex; a producer that panicked while holding it does not
    /// lock everyone else out.
    fn guarded(&self) -> MutexGuard<'_, GapState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn open_gap(&self, cause_in: Cause) {
        if matches!(cause_in, Cause::BodyLightNoSlot) {
            perf::illumination_fault_mark()
        }
        let gap = gap::ShortfallOrigin::gap(&cause_in);
        let GapState {
            ledger, unlogged, ..
        } = &mut *self.guarded();
        *unlogged |= ledger.cause(gap) != Some(&cause_in);
        ledger.raise(cause_in);
    }

    pub fn close_gap(&self, gap: PresentationShortfall) {
        let GapState {
            ledger, unlogged, ..
        } = &mut *self.guarded();
        *unlogged |= ledger.is_live(gap);
        ledger.clear(gap);
    }

    pub fn still_counting(&self, gap: PresentationShortfall) -> bool {
        let guard = self.guarded();
        guard.ledger.is_live(gap)
    }

    pub fn reason_of(&self, gap: PresentationShortfall) -> Option<Cause> {
        let guard = self.guarded();
        guard.ledger.cause(gap).cloned()
    }

    pub fn hit_count(&self, gap: PresentationShortfall) -> u64 {
        let guard = self.guarded();
        guard.ledger.hits(gap)
    }

    pub fn live(&self) -> Vec<PresentationShortfall> {
        let guard = self.guarded();
        guard.ledger.live().collect()
    }

    pub fn count(&self) -> usize {
        let guard = self.guarded();
        guard.ledger.count()
    }
}

/// The log lines for the open gaps, or `None` when nothing changed since the last
/// call (or the set printed the same as before).
fn changed_lines(gaps: &ShortfallLedger) -> Option<Vec<String>> {
    let mut state = gaps.guarded();
    if !std::mem::take(&mut state.unlogged) {
        return None;
    }
    let signature = {
        let mut text = String::new();
        state.ledger.print_fingerprint(&mut text).ok()?;
        text
    };
    if state.last_signature == signature {
        return None;
    }
    state.last_signature = signature;
    let describe = |gap: PresentationShortfall| {
        let cause = state.ledger.cause(gap)?;
        Some(format!("render gap {}: {cause}", gap.name()))
    };
    Some(state.ledger.live().filter_map(describe).collect())
}

/// Logs the open gaps, but only when the set (or a cause) changed since the
/// last line written.
pub(crate) fn log_render_gaps(gaps: Res<ShortfallLedger>) {
    let Some(lines) = changed_lines(&gaps) else {
        return;
    };
    let none_live = "render: no presentation gaps live";
    match lines.as_slice() {
        [] => diag::info!(World, "{none_live}"),
        shown => {
            for line in shown {
                info!(World, "{}", line);
            }
        }
    }
}

pub fn add_render_gaps(application: &mut App) {
    application.init_resource::<ShortfallLedger>();
    application.add_systems(Update, log_render_gaps.in_set(net::ClientStep::Diag));
}

mod tests {
    #![cfg(test)]
    use super::*;
    use gap::ShortfallOrigin as _;

    fn body_material(name: &str) -> Cause {
        Cause::RemoteBodyMaterialMissing {
            name: String::from(name),
        }
    }

    fn world_gun(weapon: u32, mdl: &str) -> Cause {
        let third_person_mesh = mdl.to_owned();
        Cause::BodyHeldGunAbsent {
            weap: weapon,
            third_person_mesh,
        }
    }

    fn dependency(gun_catalog_no: u32, role_name: &'static str, name: &str) -> Cause {
        Cause::FpvDependencyUnresolved {
            gun_catalog_no,
            part_role: role_name,
            name: String::from(name),
        }
    }

    fn eye_pose(weapon_mesh: &str) -> Cause {
        Cause::FpvEyePoseFailed {
            weapon_mesh: String::from(weapon_mesh),
        }
    }

    /// Takes the "something changed" flag, leaving it clear.
    fn take_dirty(board: &ShortfallLedger) -> bool {
        std::mem::take(&mut board.guarded().unlogged)
    }

    fn written(app: &App) -> String {
        let board = app.world().resource::<ShortfallLedger>();
        let state = board.guarded();
        state.last_signature.clone()
    }

    #[test]
    fn each_cause_belongs_to_one_gap() {
        use PresentationShortfall::{GunViewLayer, BodyAnim, BodyHeldGun};
        let owners = [
            (Cause::BodySubmitAbsent, BodyAnim),
            (
                Cause::BodyLightNoSlot,
                BodyAnim,
            ),
            (body_material("m"), BodyAnim),
            (world_gun(3, "w"), BodyHeldGun),
            (Cause::GunTableAbsent, GunViewLayer),
            (
                Cause::FpvGunXModelUnresolved { gun_catalog_no: 1 },
                GunViewLayer,
            ),
            (dependency(1, "hands", "n"), GunViewLayer),
            (eye_pose("g"), GunViewLayer),
            (Cause::GunNoViewer, GunViewLayer),
            (Cause::GunNoLightAtlas, GunViewLayer),
            (Cause::GunNoPlayingClips, GunViewLayer),
        ];
        for (cause, owner) in owners {
            assert_eq!(cause.gap(), owner, "{cause}");
        }
    }

    #[test]
    fn gaps_have_stable_names_and_none_is_standing() {
        let names = PresentationShortfall::ALL_KINDS
            .iter()
            .map(|gap| gap.name())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            names,
            "remote-body-animation remote-body-world-gun fpv-viewmodel"
        );
        assert_eq!(
            PresentationShortfall::ALL_KINDS
                .iter()
                .filter(|gap| gap.still_open())
                .count(),
            0
        );
    }

    #[test]
    fn causes_print_their_label_and_any_detail() {
        let printed = [
            (Cause::GunNoViewer, "FPV waiting for camera"),
            (
                Cause::BodySubmitAbsent,
                "remote body submit missing",
            ),
            (
                body_material(""),
                "remote body material missing: authored name not retained at capture",
            ),
            (
                body_material("skin"),
                "remote body material missing: `skin` not in global catalog",
            ),
            (
                world_gun(12, "gun_w"),
                "remote body world gun missing: weapon 12 `gun_w`",
            ),
            (
                Cause::FpvGunXModelUnresolved { gun_catalog_no: 5 },
                "gunXModel[0] unresolved for weapon 5",
            ),
            (
                dependency(5, "hands", "arms"),
                "FPV dependency unresolved for weapon 5: hands `arms`",
            ),
            (
                eye_pose("viewmodel_gun"),
                "FPV eye-pose failed for `viewmodel_gun` (need viewhands+gun skels with tag_view)",
            ),
        ];
        for (cause, text) in printed {
            assert_eq!(format!("{cause}"), text);
        }
    }

    #[test]
    fn raising_and_clearing_track_a_gap() {
        let board = ShortfallLedger::default();
        assert_eq!(board.count(), 0);
        board.open_gap(Cause::GunNoViewer);
        assert!(board.still_counting(PresentationShortfall::GunViewLayer));
        assert!(!board.still_counting(PresentationShortfall::BodyHeldGun));
        let held = board.reason_of(PresentationShortfall::GunViewLayer);
        assert_eq!(held, Some(Cause::GunNoViewer));
        assert_eq!(
            (board.live(), board.count()),
            (vec![PresentationShortfall::GunViewLayer], 1)
        );
        board.close_gap(PresentationShortfall::GunViewLayer);
        assert!(!board.still_counting(PresentationShortfall::GunViewLayer));
        assert!(board.reason_of(PresentationShortfall::GunViewLayer).is_none());
    }

    #[test]
    fn a_newer_cause_replaces_the_older_one_and_hits_accumulate() {
        let board = ShortfallLedger::default();
        for cause in [
            Cause::GunNoViewer,
            Cause::GunNoPlayingClips,
        ] {
            board.open_gap(cause);
        }
        let held = board.reason_of(PresentationShortfall::GunViewLayer);
        assert_eq!(held, Some(Cause::GunNoPlayingClips));
        assert_eq!(board.count(), 1);
        assert_eq!(board.hit_count(PresentationShortfall::GunViewLayer), 2);
    }

    #[test]
    fn the_log_is_only_reconsidered_after_a_change() {
        let board = ShortfallLedger::default();
        assert!(!take_dirty(&board));

        board.open_gap(Cause::GunNoViewer);
        assert!(take_dirty(&board));

        // the same cause again is not news
        board.open_gap(Cause::GunNoViewer);
        assert!(!take_dirty(&board));
        board.open_gap(Cause::GunNoPlayingClips);
        assert!(take_dirty(&board));

        // clearing a closed gap is not news either
        board.close_gap(PresentationShortfall::BodyAnim);
        assert!(!take_dirty(&board));
        board.close_gap(PresentationShortfall::GunViewLayer);
        assert!(take_dirty(&board));
    }

    #[test]
    fn the_report_system_remembers_what_it_wrote() {
        let mut app = App::default();
        add_render_gaps(&mut app);

        app.update();
        assert!(written(&app).is_empty());

        let board = app.world().resource::<ShortfallLedger>();
        board.open_gap(Cause::GunNoViewer);
        app.update();
        let opened = written(&app);
        assert_ne!(opened, "");
        let board = app.world().resource::<ShortfallLedger>();
        assert!(!board.guarded().unlogged);

        board.close_gap(PresentationShortfall::GunViewLayer);
        app.update();
        assert_ne!(written(&app), opened);
    }

    #[test]
    fn a_lock_poisoned_by_a_panicking_producer_is_still_usable() {
        let board = ShortfallLedger::default();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = board.guarded();
            panic!("producer failed while holding the ledger");
        }));
        assert!(outcome.is_err());
        board.open_gap(Cause::GunNoViewer);
        assert!(board.still_counting(PresentationShortfall::GunViewLayer));
    }
}
