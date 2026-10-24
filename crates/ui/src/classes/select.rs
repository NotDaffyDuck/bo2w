//! State of the class-select screen: whether it is open, whether a choice is
//! in flight, and the rules for committing, accepting and refusing a choice.

use bevy::prelude::*;
use frame::{AppScreen, ClientSet, RuntimeRole};
use net::SignonState;

use crate::classes::icons::{ClassSelectIconCache, UiAssetRoot, warm_class_select_icons};
use crate::classes::store::SessionClassStore;

/// Whether the class-select overlay is showing.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClassSelectOverlayOpen(pub bool);

/// The overlay opens on arrival at the class screen and stays open for as
/// long as the screen does; replays and failed signons never show it.
pub(crate) fn overlay_open_after_screen(
    prev: Option<AppScreen>,
    now: AppScreen,
    replay: bool,
    failed: bool,
    currently_open: bool,
) -> bool {
    if replay || failed || now != AppScreen::ClassSelect {
        return false;
    }
    prev != Some(AppScreen::ClassSelect) || currently_open
}

/// Whether the player may change class at this moment.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClassChangeAllowed(pub bool);

/// Why a class change is not allowed, when it is not.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ClassChangeBlockReason(pub Option<String>);

/// Either the menu is free, or exactly one equip request awaits its answer.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub enum ClassSelectPhase {
    #[default]
    Interactive,
    Pending {
        request_id: u32,
        class_index: usize,
    },
}

impl ClassSelectPhase {
    pub fn is_pending(&self) -> bool {
        self.pending_request_id().is_some()
    }

    pub fn pending_request_id(&self) -> Option<u32> {
        match self {
            Self::Pending { request_id, .. } => Some(*request_id),
            Self::Interactive => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassEquipRequest {
    pub request_id: u32,
    pub class_index: usize,
}

/// A committed request that has not been sent to the host yet.
#[derive(Resource, Debug, Default)]
pub struct PendingClassEquip(pub Option<ClassEquipRequest>);

/// One line of feedback shown under the class list.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ClassSelectStatus(pub Option<String>);

/// The class the list cursor is on.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ClassSelectHighlight(pub usize);

/// Why a commit was turned away before reaching the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassEquipRefusal {
    AlreadyPending { request_id: u32 },
    UnknownClass(String),
    LockedContent { index: usize, reason: String },
}

impl std::fmt::Display for ClassEquipRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyPending { request_id } => {
                write!(f, "equip already pending (request_id={request_id})")
            }
            Self::UnknownClass(name) => write!(f, "unknown_class: `{name}`"),
            Self::LockedContent { index, reason } => {
                write!(f, "locked_content: preset {index} — {reason}")
            }
        }
    }
}

/// Finds a class by position (a number) or, failing that, by name ignoring
/// case.
pub fn class_index_by_name(store: &SessionClassStore, name: &str) -> Option<usize> {
    let by_number = name
        .parse::<usize>()
        .ok()
        .filter(|&index| index < store.slots.len());
    by_number.or_else(|| {
        store
            .slots
            .iter()
            .position(|slot| slot.name.eq_ignore_ascii_case(name))
    })
}

/// Starts an equip of the class at `index`, returning the new request id.
pub fn commit_class_equip(
    index: usize,
    store: &mut SessionClassStore,
    highlight: &mut ClassSelectHighlight,
    phase: &mut ClassSelectPhase,
    pending: &mut PendingClassEquip,
    status: &mut ClassSelectStatus,
    seq: &mut net::ActionRequestIds,
) -> Result<u32, ClassEquipRefusal> {
    if let Some(request_id) = phase.pending_request_id() {
        return Err(ClassEquipRefusal::AlreadyPending { request_id });
    }
    let slot = store
        .slots
        .get(index)
        .ok_or_else(|| ClassEquipRefusal::UnknownClass(index.to_string()))?;
    if let Some(reason) = &slot.lock_reason {
        let refusal = ClassEquipRefusal::LockedContent {
            index,
            reason: reason.clone(),
        };
        status.0 = Some(refusal.to_string());
        return Err(refusal);
    }

    let request_id = seq.allocate();
    highlight.0 = index;
    store.selected = index;
    pending.0 = Some(ClassEquipRequest {
        request_id,
        class_index: index,
    });
    *phase = ClassSelectPhase::Pending {
        request_id,
        class_index: index,
    };
    status.0 = None;
    Ok(request_id)
}

/// The host accepted `request_id`: the class becomes the equipped one and the
/// overlay closes. Answers for any other request are ignored (`false`).
pub fn accept_class_equip(
    store: &mut SessionClassStore,
    phase: &mut ClassSelectPhase,
    overlay: &mut ClassSelectOverlayOpen,
    request_id: u32,
) -> bool {
    let ClassSelectPhase::Pending {
        request_id: awaited,
        class_index,
    } = *phase
    else {
        return false;
    };
    if awaited != request_id {
        return false;
    }
    store.equipped = Some(class_index);
    *phase = ClassSelectPhase::Interactive;
    overlay.0 = false;
    true
}

/// The host refused `request_id`: the menu frees up and shows why. Answers
/// for any other request are ignored (`false`).
pub fn reject_class_equip(
    phase: &mut ClassSelectPhase,
    status: &mut ClassSelectStatus,
    request_id: u32,
    reason: impl Into<String>,
) -> bool {
    if phase.pending_request_id() != Some(request_id) {
        return false;
    }
    *phase = ClassSelectPhase::Interactive;
    status.0 = Some(reason.into());
    true
}

pub(crate) struct ClassSelectPlugin;

impl Plugin for ClassSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClassSelectOverlayOpen>()
            .init_resource::<ClassChangeAllowed>()
            .init_resource::<ClassChangeBlockReason>()
            .init_resource::<PendingClassEquip>()
            .init_resource::<ClassSelectPhase>()
            .init_resource::<ClassSelectStatus>()
            .init_resource::<ClassSelectHighlight>()
            .init_resource::<ClassSelectIconCache>()
            .init_resource::<UiAssetRoot>()
            .init_resource::<SessionClassStore>()
            .add_systems(
                PostUpdate,
                sync_class_select_overlay_to_screen.before(crate::layers::ApplyUiLayers),
            )
            .add_systems(Update, warm_class_select_icons.in_set(ClientSet::Ui));
    }
}

fn sync_class_select_overlay_to_screen(
    screen: Res<AppScreen>,
    role: Option<Res<RuntimeRole>>,
    signon: Option<Res<SignonState>>,
    mut overlay: ResMut<ClassSelectOverlayOpen>,
    mut last_screen: Local<Option<AppScreen>>,
) {
    let replay = role.is_some_and(|role| *role == RuntimeRole::Replay);
    let failed = signon.is_some_and(|signon| signon.phase.is_failed());
    let open = overlay_open_after_screen(*last_screen, *screen, replay, failed, overlay.0);
    if overlay.0 != open {
        overlay.0 = open;
    }
    *last_screen = Some(*screen);
}
