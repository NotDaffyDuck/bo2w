//! The player's classes for this session, and their copy on disk.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use frame::{HostClassLoadouts, HostClassSlot};
use sim::match_state::PERSONAL_CLASS_SLOTS;

use crate::classes::setup::ClassSlotState;

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionClassStore {
    pub slots: Vec<ClassSlotState>,
    /// Slot the host has accepted, once there is one.
    pub equipped: Option<usize>,
    pub selected: usize,
}

impl SessionClassStore {
    /// Builds a starting set from the showcase presets: those the registry
    /// can actually equip, repeated round-robin to fill every personal slot.
    pub fn from_showcase(seed: u64, registry: &asset_game::WeaponRegistry) -> Self {
        let combat = session::combat_table::from_registry(registry, None);
        let equipment = session::combat_table::equipment_from_registry(registry);
        let equippable = |slot: &HostClassSlot| {
            let row = session::ClassRow::from(slot);
            !session::project_class(0, &row, registry, &combat, &equipment)
                .def
                .locked
        };

        let mut chosen: Vec<HostClassSlot> =
            frame::pick_showcase(seed, frame::showcase_classes().len())
                .into_iter()
                .map(HostClassSlot::from)
                .filter(equippable)
                .take(PERSONAL_CLASS_SLOTS)
                .collect();
        let distinct = chosen.len();
        if distinct > 0 {
            while chosen.len() < PERSONAL_CLASS_SLOTS {
                let repeat = chosen[chosen.len() % distinct].clone();
                chosen.push(repeat);
            }
        }

        Self {
            slots: chosen.iter().map(ClassSlotState::from_host_slot).collect(),
            equipped: None,
            selected: 0,
        }
    }

    pub fn equipped_slot(&self) -> Option<&ClassSlotState> {
        self.slots.get(self.equipped?)
    }

    /// Sets each slot's lock reason from the list, in slot order. Slots past
    /// the end of the list keep what they had.
    pub fn apply_coverage_locks(&mut self, reasons: impl IntoIterator<Item = Option<String>>) {
        self.slots
            .iter_mut()
            .zip(reasons)
            .for_each(|(slot, reason)| slot.lock_reason = reason);
    }
}

impl From<&ClassSlotState> for HostClassSlot {
    fn from(slot: &ClassSlotState) -> Self {
        Self {
            name: slot.name.clone(),
            primary: slot.primary.clone(),
            primary_attachments: slot.primary_attachments.clone(),
            secondary: slot.secondary.clone(),
            secondary_attachments: slot.secondary_attachments.clone(),
            lethal: slot.lethal.clone(),
            tactical: slot.tactical.clone(),
            perks: [slot.perk1.clone(), slot.perk2.clone(), slot.perk3.clone()],
            deathstreak: slot.deathstreak.clone(),
        }
    }
}

/// Hands the host the current classes whenever the store changes.
pub(crate) fn sync_host_class_loadouts(
    store: Res<SessionClassStore>,
    mut host: ResMut<HostClassLoadouts>,
) {
    if store.is_changed() {
        host.slots = store.slots.iter().map(HostClassSlot::from).collect();
    }
}

/// First line of the class file; also its format version.
const FILE_HEADER: &str = "bo2mp-classes 1";

/// How long a failed write waits before the next attempt.
const WRITE_RETRY: Duration = Duration::from_secs(5);

/// Where the class file is, and what is known about it.
#[derive(Resource, Default)]
pub(crate) struct ClassStoreFile {
    /// Set only once the file is safe to overwrite.
    path: Option<PathBuf>,
    loaded: bool,
    /// Exactly what the file holds now.
    written: Option<String>,
    retry_at: Option<Instant>,
}

/// The format has no escapes, so separators inside a value become spaces.
fn flatten(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

fn join_attachments(attachments: &[String]) -> String {
    let flat: Vec<String> = attachments
        .iter()
        .map(|name| flatten(name).replace(',', " "))
        .collect();
    flat.join(",")
}

fn split_attachments(field: &str) -> Vec<String> {
    field
        .split(',')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn encode_slots(slots: &[ClassSlotState]) -> String {
    let mut text = format!("{FILE_HEADER}\n");
    for slot in slots {
        let columns = [
            flatten(&slot.name),
            flatten(&slot.primary),
            join_attachments(&slot.primary_attachments),
            flatten(&slot.secondary),
            join_attachments(&slot.secondary_attachments),
            flatten(&slot.lethal),
            flatten(&slot.tactical),
            flatten(&slot.perk1),
            flatten(&slot.perk2),
            flatten(&slot.perk3),
            flatten(&slot.deathstreak),
        ];
        text.push_str(&columns.join("\t"));
        text.push('\n');
    }
    text
}

/// Reads the file back. Anything unexpected (wrong header, a short row, more
/// rows than slots, no rows at all) is refused as a whole.
fn decode_slots(text: &str) -> Option<Vec<ClassSlotState>> {
    let mut lines = text.lines();
    if lines.next()? != FILE_HEADER {
        return None;
    }
    let mut slots = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let columns: Vec<&str> = line.split('\t').collect();
        let columns: [&str; 11] = columns.try_into().ok()?;
        if slots.len() >= PERSONAL_CLASS_SLOTS {
            return None;
        }
        let [
            name,
            primary,
            primary_attachments,
            secondary,
            secondary_attachments,
            lethal,
            tactical,
            perk1,
            perk2,
            perk3,
            deathstreak,
        ] = columns.map(str::to_owned);
        slots.push(ClassSlotState {
            name,
            primary,
            primary_attachments: split_attachments(&primary_attachments),
            secondary,
            secondary_attachments: split_attachments(&secondary_attachments),
            lethal,
            tactical,
            perk1,
            perk2,
            perk3,
            deathstreak,
            lock_reason: None,
        });
    }
    if slots.is_empty() { None } else { Some(slots) }
}

/// What reading the class file turned up.
enum FileState {
    Classes(Vec<ClassSlotState>),
    Missing,
    /// Present but not usable; it must not be overwritten.
    Unusable(String),
}

fn read_class_file(path: &Path) -> FileState {
    match std::fs::read_to_string(path) {
        Ok(text) => match decode_slots(&text) {
            Some(slots) => FileState::Classes(slots),
            None => FileState::Unusable("is not a class file this build reads".to_owned()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileState::Missing,
        Err(error) => FileState::Unusable(format!("cannot be read: {error}")),
    }
}

/// Loads the saved classes once the profile folder and weapon catalog are
/// known, or starts from a fresh showcase set when there is nothing to load.
pub(crate) fn load_class_store(
    identity: Option<Res<frame::LaunchIdentity>>,
    mut file: ResMut<ClassStoreFile>,
    mut store: ResMut<SessionClassStore>,
    catalog: Res<crate::ClassLoadoutCatalog>,
) {
    if file.loaded {
        return;
    }
    let Some(identity) = identity else {
        return;
    };
    if identity.artifacts.as_os_str().is_empty() {
        return;
    }
    let Some(registry) = catalog.resolver.0.as_deref() else {
        return;
    };

    let path = identity.artifacts.join("profile").join("classes.txt");
    file.loaded = true;
    let fresh_set = || {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos() as u64);
        SessionClassStore::from_showcase(seed, registry)
    };

    match read_class_file(&path) {
        FileState::Classes(slots) => {
            diag::info!(Ui, "classes: {} read from {}", slots.len(), path.display());
            file.written = Some(encode_slots(&slots));
            store.slots = slots;
            for slot in &mut store.slots {
                slot.lock_reason = catalog.validate_class(slot).err();
            }
            store.selected = store.selected.min(store.slots.len().saturating_sub(1));
        }
        FileState::Missing => *store = fresh_set(),
        FileState::Unusable(why) => {
            *store = fresh_set();
            diag::warn!(
                Ui,
                "classes: {} {why}; file preserved",
                path.display()
            );
            return;
        }
    }
    file.path = Some(path);
}

/// Writes through a temporary file and renames it over the target, so a
/// crash never leaves half a file behind.
fn write_class_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    let written = std::fs::File::create(&staging).and_then(|mut out| {
        out.write_all(contents.as_bytes())?;
        out.sync_all()
    });
    let moved = written.and_then(|()| std::fs::rename(&staging, path));
    if moved.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    moved
}

pub(crate) fn save_class_store(
    store: Res<SessionClassStore>,
    catalog: Res<crate::ClassLoadoutCatalog>,
    mut file: ResMut<ClassStoreFile>,
) {
    if !file.loaded || store.slots.is_empty() {
        return;
    }
    if file.retry_at.is_some_and(|at| Instant::now() < at) {
        return;
    }
    if !store.is_changed() && file.retry_at.is_none() && file.written.is_some() {
        return;
    }
    let Some(path) = file.path.clone() else {
        return;
    };
    let contents = encode_slots(&store.slots);
    if file.written.as_ref() == Some(&contents) {
        return;
    }

    // A class that no longer validates may only be saved if it is the very
    // class that is already on disk.
    let on_disk = file.written.as_deref().and_then(decode_slots);
    for (index, slot) in store.slots.iter().enumerate() {
        let Err(reason) = catalog.validate_class(slot) else {
            continue;
        };
        let already_saved = on_disk
            .as_ref()
            .and_then(|slots| slots.get(index))
            .is_some_and(|old| HostClassSlot::from(old) == HostClassSlot::from(slot));
        if !already_saved {
            file.retry_at = None;
            diag::warn!(Ui, "classes: not saved: {reason}");
            return;
        }
    }

    match write_class_file(&path, &contents) {
        Ok(()) => {
            file.written = Some(contents);
            file.retry_at = None;
        }
        Err(error) => {
            diag::warn!(Ui, "classes: cannot write {}: {error}", path.display());
            file.retry_at = Some(Instant::now() + WRITE_RETRY);
        }
    }
}
