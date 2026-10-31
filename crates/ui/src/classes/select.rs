//! State of the class-select screen: whether it is open, whether a choice is
//! in flight, and the rules for committing, accepting and refusing a choice.

use bevy::prelude::*;
use frame::{ShellScreen, ProcessDuty};
use net::JoinProgressRecord;

use crate::classes::icons::{LoadoutPickerThumbs, MenuMediaAnchor, preload_kit_pictures};
use crate::classes::store::MatchLoadoutVault;

/// Whether the class-select overlay is showing.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LoadoutPickerShown(pub bool);

/// The overlay opens on arrival at the class screen and stays open for as
/// long as the screen does; replays and failed signons never show it.
pub(crate) fn show_overlay_post_scene(
    before: Option<ShellScreen>,
    current: ShellScreen,
    is_replay: bool,
    signon_failed: bool,
    was_open: bool,
) -> bool {
    if is_replay || signon_failed || current != ShellScreen::KitPicker {
        return false;
    }
    was_open || before != Some(ShellScreen::KitPicker)
}

/// Whether the player may change class at this moment.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LoadoutSwapPermit(pub bool);

/// Why a class change is not allowed, when it is not.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct LoadoutSwapDenial(pub Option<String>);

/// Either the menu is free, or exactly one equip request awaits its answer.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub enum LoadoutPickerStage {
    #[default]
    Idle,
    Pending {
        ask_number: u32,
        category_slot: usize,
    },
}

impl LoadoutPickerStage {
    pub fn queued_ask_number(&self) -> Option<u32> {
        match self {
            Self::Pending { ask_number, .. } => Some(*ask_number),
            Self::Idle => None,
        }
    }

    pub fn awaiting_host(&self) -> bool {
        self.queued_ask_number().is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadoutArmAsk {
    pub ask_number: u32,
    pub category_slot: usize,
}

/// A committed request that has not been sent to the host yet.
#[derive(Resource, Debug, Default)]
pub struct QueuedLoadoutArm(pub Option<LoadoutArmAsk>);

/// One line of feedback shown under the class list.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct LoadoutPickerReport(pub Option<String>);

/// The class the list cursor is on.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct LoadoutPickerFocus(pub usize);

/// Why a commit was turned away before reaching the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadoutArmRejection {
    InFlightAlready { ask_number: u32 },
    UnlistedLoadout(String),
    FrozenSlot { preset: usize, why: String },
}

impl std::fmt::Display for LoadoutArmRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InFlightAlready { ask_number } => {
                write!(f, "equip already pending (request_id={ask_number})")
            }
            Self::UnlistedLoadout(name) => write!(f, "unknown_class: `{name}`"),
            Self::FrozenSlot { preset, why } => {
                write!(f, "locked_content: preset {preset} — {why}")
            }
        }
    }
}

/// Finds a class by position (a number) or, failing that, by name ignoring
/// case.
pub fn loadout_slot_named(vault: &MatchLoadoutVault, wanted: &str) -> Option<usize> {
    let by_number = wanted.parse::<usize>().ok().filter(|&n| n < vault.slots.len());
    by_number.or_else(|| {
        let lowered = wanted.to_ascii_lowercase();
        vault
            .slots
            .iter()
            .position(|slot| slot.name.to_ascii_lowercase() == lowered)
    })
}

/// What stops an equip of the class at `index` right now, if anything.
fn equip_blocker(
    phase: &LoadoutPickerStage,
    classes: &MatchLoadoutVault,
    index: usize,
) -> Option<LoadoutArmRejection> {
    if let Some(ask_number) = phase.queued_ask_number() {
        return Some(LoadoutArmRejection::InFlightAlready { ask_number });
    }
    match classes.slots.get(index) {
        None => Some(LoadoutArmRejection::UnlistedLoadout(index.to_string())),
        Some(slot) => slot.frozen_because.clone().map(|why| LoadoutArmRejection::FrozenSlot { preset: index, why }),
    }
}

/// Starts an equip of the class at `index`, returning the new request id.
pub fn finalize_loadout_arm(
    index: usize,
    classes: &mut MatchLoadoutVault,
    cursor: &mut LoadoutPickerFocus,
    menu_phase: &mut LoadoutPickerStage,
    outbox: &mut QueuedLoadoutArm,
    feedback: &mut LoadoutPickerReport,
    ids: &mut net::ActionRequestCounter,
) -> Result<u32, LoadoutArmRejection> {
    if let Some(blocker) = equip_blocker(menu_phase, classes, index) {
        if let LoadoutArmRejection::FrozenSlot { .. } = blocker {
            feedback.0 = Some(blocker.to_string());
        }
        return Err(blocker);
    }

    let LoadoutArmAsk { ask_number, category_slot } = LoadoutArmAsk {
        category_slot: index,
        ask_number: ids.allocate(),
    };
    feedback.0 = None;
    *menu_phase = LoadoutPickerStage::Pending { ask_number, category_slot };
    outbox.0 = Some(LoadoutArmAsk { ask_number, category_slot });
    (classes.selected, cursor.0) = (index, index);
    Ok(ask_number)
}

/// The host accepted `ask_number`: the class becomes the equipped one and the
/// overlay closes. Answers for any other request are ignored (`false`).
pub fn grant_loadout_arm(
    classes: &mut MatchLoadoutVault,
    menu_phase: &mut LoadoutPickerStage,
    shown: &mut LoadoutPickerShown,
    answered: u32,
) -> bool {
    match *menu_phase {
        LoadoutPickerStage::Pending { ask_number, category_slot } if ask_number == answered => {
            *menu_phase = LoadoutPickerStage::default();
            classes.equipped = Some(category_slot);
            shown.0 = false;
            true
        }
        _ => false,
    }
}

/// The host refused `ask_number`: the menu frees up and shows why. Answers
/// for any other request are ignored (`false`).
pub fn refuse_loadout_arm(
    menu_phase: &mut LoadoutPickerStage,
    feedback: &mut LoadoutPickerReport,
    answered: u32,
    why: impl Into<String>,
) -> bool {
    let awaited = menu_phase.queued_ask_number() == Some(answered);
    if awaited {
        *menu_phase = LoadoutPickerStage::default();
        feedback.0 = Some(why.into());
    }
    awaited
}

pub(crate) struct LoadoutPickerModule;

impl Plugin for LoadoutPickerModule {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadoutSwapPermit>();
        app.init_resource::<LoadoutSwapDenial>();
        app.init_resource::<LoadoutPickerShown>();
        app.init_resource::<MatchLoadoutVault>();
        app.init_resource::<MenuMediaAnchor>();
        app.init_resource::<LoadoutPickerThumbs>();
        app.init_resource::<LoadoutPickerFocus>();
        app.init_resource::<LoadoutPickerReport>();
        app.init_resource::<LoadoutPickerStage>();
        app.init_resource::<QueuedLoadoutArm>();
        crate::every_ui_frame(app, preload_kit_pictures);
        app.add_systems(
            PostUpdate,
            follow_screen_for_class_overlay.before(crate::tiers::OverlayTierPass),
        );
    }
}

fn follow_screen_for_class_overlay(
    on_screen: Res<ShellScreen>,
    role: Option<Res<ProcessDuty>>,
    signon: Option<Res<JoinProgressRecord>>,
    mut shown: ResMut<LoadoutPickerShown>,
    mut last_screen: Local<Option<ShellScreen>>,
) {
    let is_replay = matches!(role.as_deref(), Some(ProcessDuty::Replay));
    let signon_failed = signon.is_some_and(|state| state.phase.has_failed());
    let current = *on_screen;
    let before = last_screen.replace(current);
    let open = show_overlay_post_scene(before, current, is_replay, signon_failed, shown.0);
    if open != shown.0 {
        shown.0 = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_opens_on_arrival_and_stays_until_the_screen_changes() {
        let open = show_overlay_post_scene;
        let screen = ShellScreen::KitPicker;
        assert!(open(None, screen, false, false, false));
        assert!(open(Some(screen), screen, false, false, true));
        assert!(!open(Some(screen), screen, false, false, false));
        assert!(!open(None, screen, true, false, false));
        assert!(!open(None, screen, false, true, true));
    }

    #[test]
    fn only_the_awaited_request_settles() {
        let mut phase = LoadoutPickerStage::Pending {
            ask_number: 4,
            category_slot: 2,
        };
        let mut words = LoadoutPickerReport::default();
        assert!(!refuse_loadout_arm(&mut phase, &mut words, 5, "nope"));
        assert!(phase.awaiting_host());
        assert!(refuse_loadout_arm(&mut phase, &mut words, 4, "full"));
        assert_eq!(words.0.as_deref(), Some("full"));
        assert_eq!(phase.queued_ask_number(), None);
    }

    #[test]
    fn refusals_read_as_one_line_each() {
        let locked = LoadoutArmRejection::FrozenSlot {
            preset: 3,
            why: "needs rank 9".into(),
        };
        assert_eq!(
            locked.to_string(),
            "locked_content: preset 3 — needs rank 9"
        );
        let busy = LoadoutArmRejection::InFlightAlready { ask_number: 7 };
        assert_eq!(busy.to_string(), "equip already pending (request_id=7)");
    }
}
