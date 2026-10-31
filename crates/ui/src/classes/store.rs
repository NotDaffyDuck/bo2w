//! The player's classes for this session, and their copy on disk.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use frame::{ServerKitRoster, ServerKitSlot};
use sim::player_ledger::PERSONAL_CLASS_SLOTS;

use crate::classes::setup::LoadoutSlotStatus;

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchLoadoutVault {
    pub slots: Vec<LoadoutSlotStatus>,
    /// Slot the host has accepted, once there is one.
    pub equipped: Option<usize>,
    pub selected: usize,
}

impl MatchLoadoutVault {
    /// Builds a starting set from the showcase presets: those the registry
    /// can actually equip, repeated round-robin to fill every personal slot.
    pub fn from_showcase(seed: u64, registry: &asset_game::WeaponLedger) -> Self {
        let combat = session::fight_table::from_registry(registry, None);
        let equipment = session::fight_table::equipment_from_registry(registry);
        let equippable = |slot: &ServerKitSlot| {
            let row = session::ClassRow::from(slot);
            !session::project_loadout(0, &row, registry, &combat, &equipment)
                .def
                .locked
        };

        let mut chosen: Vec<ServerKitSlot> =
            frame::pick_showcase(seed, frame::showcase_classes().len())
                .into_iter()
                .map(ServerKitSlot::from)
                .map(|mut slot| {
                    session::fill_default_perks(&mut slot, registry);
                    slot
                })
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
            slots: chosen.iter().map(LoadoutSlotStatus::from_host_slot).collect(),
            equipped: None,
            selected: 0,
        }
    }

    pub fn armed_loadout_index(&self) -> Option<&LoadoutSlotStatus> {
        self.slots.get(self.equipped?)
    }

    /// Sets each slot's lock reason from the list, in slot order. Slots past
    /// the end of the list keep what they had.
    pub fn enforce_cover_pins(&mut self, causes: impl IntoIterator<Item = Option<String>>) {
        self.slots
            .iter_mut()
            .zip(causes)
            .for_each(|(kit, cause)| kit.frozen_because = cause);
    }
}

impl From<&LoadoutSlotStatus> for ServerKitSlot {
    fn from(kit: &LoadoutSlotStatus) -> Self {
        Self {
            name: kit.name.clone(),
            primary: kit.primary.clone(),
            main_gun_parts: kit.primary_attachments.clone(),
            secondary: kit.secondary.clone(),
            side_gun_parts: kit.secondary_attachments.clone(),
            lethal: kit.lethal.clone(),
            tactical: kit.tactical.clone(),
            perks: [kit.first_specialty.clone(), kit.second_specialty.clone(), kit.third_specialty.clone()],
        }
    }
}

/// Hands the host the current classes whenever the store changes.
pub(crate) fn refresh_hosted_loadout_roster(
    store: Res<MatchLoadoutVault>,
    mut host: ResMut<ServerKitRoster>,
) {
    if store.is_changed() {
        host.slots = store.slots.iter().map(ServerKitSlot::from).collect();
    }
}

/// First line of the class file; also its format version.
const FILE_HEADER: &str = "bo2mp-classes 2";

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

fn encode_slots(kits: &[LoadoutSlotStatus]) -> String {
    let mut out = format!("{FILE_HEADER}\n");
    for kit in kits {
        let columns = [
            flatten(&kit.name),
            flatten(&kit.primary),
            join_attachments(&kit.primary_attachments),
            flatten(&kit.secondary),
            join_attachments(&kit.secondary_attachments),
            flatten(&kit.lethal),
            flatten(&kit.tactical),
            flatten(&kit.first_specialty),
            flatten(&kit.second_specialty),
            flatten(&kit.third_specialty),
        ];
        out.push_str(&columns.join("\t"));
        out.push('\n');
    }
    out
}

/// Reads the file back. Anything unexpected (wrong header, a short row, more
/// rows than slots, no rows at all) is refused as a whole.
fn decode_slots(text: &str) -> Option<Vec<LoadoutSlotStatus>> {
    let mut lines = text.lines();
    if lines.next()? != FILE_HEADER {
        return None;
    }
    let mut kits = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let columns: Vec<&str> = line.split('\t').collect();
        let columns: [&str; 10] = columns.try_into().ok()?;
        if kits.len() >= PERSONAL_CLASS_SLOTS {
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
            first_specialty,
            perk2,
            perk3,
        ] = columns.map(str::to_owned);
        kits.push(LoadoutSlotStatus {
            name,
            primary,
            primary_attachments: split_attachments(&primary_attachments),
            secondary,
            secondary_attachments: split_attachments(&secondary_attachments),
            lethal,
            tactical,
            first_specialty,
            second_specialty: perk2,
            third_specialty: perk3,
            frozen_because: None,
        });
    }
    if kits.is_empty() { None } else { Some(kits) }
}

/// What reading the class file turned up.
enum FileState {
    Classes(Vec<LoadoutSlotStatus>),
    Missing,
    /// Present but not usable; it must not be overwritten.
    Unusable(String),
}

fn read_class_file(path: &Path) -> FileState {
    match std::fs::read_to_string(path) {
        Ok(text) => match decode_slots(&text) {
            Some(kits) => FileState::Classes(kits),
            None => FileState::Unusable("is not a class file this build reads".to_owned()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileState::Missing,
        Err(error) => FileState::Unusable(format!("cannot be read: {error}")),
    }
}

/// Loads the saved classes once the profile folder and weapon catalog are
/// known, or starts from a fresh showcase set when there is nothing to load.
pub(crate) fn load_class_store(
    persona: Option<Res<frame::StartupPersona>>,
    mut file: ResMut<ClassStoreFile>,
    mut store: ResMut<MatchLoadoutVault>,
    catalog: Res<crate::LoadoutRoster>,
) {
    if file.loaded {
        return;
    }
    let Some(persona) = persona else {
        return;
    };
    if persona.artifacts.as_os_str().is_empty() {
        return;
    }
    let Some(registry) = catalog.resolver.0.as_deref() else {
        return;
    };

    let place = persona.artifacts.join("profile").join("classes.txt");
    file.loaded = true;
    let fresh_set = || {
        let seed = std::time::UNIX_EPOCH.elapsed().map_or(0, |since| since.as_nanos() as u64);
        MatchLoadoutVault::from_showcase(seed, registry)
    };

    match read_class_file(&place) {
        FileState::Classes(kits) => {
            diag::info!(Ui, "classes: {} read from {}", kits.len(), place.display());
            file.written = Some(encode_slots(&kits));
            store.slots = kits;
            for slot in &mut store.slots {
                slot.frozen_because = catalog.validate_class(slot).err();
            }
            store.selected = store.selected.min(store.slots.len().saturating_sub(1));
        }
        FileState::Missing => *store = fresh_set(),
        FileState::Unusable(why) => {
            *store = fresh_set();
            diag::warn!(
                Ui,
                "classes: {} {why}; file kept as it is",
                place.display()
            );
            return;
        }
    }
    file.path = Some(place);
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
    store: Res<MatchLoadoutVault>,
    catalog: Res<crate::LoadoutRoster>,
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
    let Some(place) = file.path.clone() else {
        return;
    };
    let contents = encode_slots(&store.slots);
    if file.written.as_ref() == Some(&contents) {
        return;
    }

    // A class that no longer validates may only be saved if it is the very
    // class that is already on disk.
    let on_disk = file.written.as_deref().and_then(decode_slots);
    for (index, kit) in store.slots.iter().enumerate() {
        let Err(fault) = catalog.validate_class(kit) else {
            continue;
        };
        let already_saved = on_disk
            .as_ref()
            .and_then(|kits| kits.get(index))
            .is_some_and(|old| ServerKitSlot::from(old) == ServerKitSlot::from(kit));
        if !already_saved {
            file.retry_at = None;
            diag::warn!(Ui, "classes: left unsaved: {fault}");
            return;
        }
    }

    match write_class_file(&place, &contents) {
        Ok(()) => {
            file.written = Some(contents);
            file.retry_at = None;
        }
        Err(error) => {
            diag::warn!(Ui, "classes: could not write {}: {error}", place.display());
            file.retry_at = Some(Instant::now() + WRITE_RETRY);
        }
    }
}
