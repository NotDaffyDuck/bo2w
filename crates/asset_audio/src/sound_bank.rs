//! The sound catalog: every alias a match can play, found by name.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use asset_core::AssetCategoryScope;

use crate::variant_pick::weighted_pick;
use crate::{
    SnapshotSoundEntry, SpawnEffectAudioLoop, SpawnSingleShotEffect,
    ResidentRawAudio, ResidentSelectionResult, RandomPointSound, ScriptedMapFx, SndDuck, SndMaster,
};

/// One entity sound channel and how it limits the voices on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorAudioLane {
    pub name: String,
    pub lane_rank: i32,
    pub has_depth_axis: bool,
    pub is_limited: bool,
    pub can_be_halted: bool,
    pub simultaneous_channel_cap: i32,
}

/// Aliases by name, with what the mixer needs beside them: the ducks, the
/// master mixes, the raw files a map ships, and the map's scripted sounds.
#[derive(Clone, Debug, Default)]
pub struct AudioRoster {
    pub sounds: Vec<SnapshotSoundEntry>,
    pub rawfiles: HashMap<(AssetCategoryScope, String), Vec<u8>>,
    /// What the map's compiled scripts place: effects, sound loops and the
    /// sounds it plays now and then.
    pub scripted_map_fx: Option<ScriptedMapFx>,
    /// Ducks by id; a later bank's duck replaces an earlier one's.
    pub ducks: HashMap<u32, SndDuck>,
    /// bo2zm: the banks' room echoes by name (16 values in `SndRadverb`'s
    /// order, from smoothing to returnHighpass).
    pub radverbs: HashMap<String, [f32; 16]>,
    /// Duck group names, indexed by `SnapshotSoundAlias::duck_group`.
    pub duck_groups: Vec<String>,
    pub masters: Vec<SndMaster>,
    exact: HashMap<String, usize>,
    folded: HashMap<String, usize>,
    edition: u64,
}

impl AudioRoster {
    /// Adds an alias. A name already present is shadowed by the new one.
    pub fn add_sound(&mut self, incoming: SnapshotSoundEntry) {
        if incoming.name.is_empty() {
            return;
        }
        self.index_name(&incoming.name, self.sounds.len());
        self.sounds.push(incoming);
    }

    /// Makes `spelled` find the sound at `at`, as given or in any case.
    fn index_name(&mut self, spelled: &str, at: usize) {
        self.exact.insert(spelled.to_owned(), at);
        self.folded.insert(spelled.to_ascii_lowercase(), at);
    }

    /// Takes from `donor` what this catalog does not have yet: aliases whose
    /// names are new, raw files, the scripted fx, ducks, duck groups and
    /// masters (the last three only while this catalog has none).
    pub fn merge_missing(&mut self, donor: AudioRoster) {
        let mut newcomers = donor.sounds.into_iter();
        while let Some(snd) = newcomers.next() {
            if self.slot_named(&snd.name).is_none() {
                self.add_sound(snd);
            }
        }
        let own = std::mem::take(&mut self.rawfiles);
        self.rawfiles = donor.rawfiles.into_iter().chain(own).collect();
        if self.scripted_map_fx.is_none() {
            self.scripted_map_fx = donor.scripted_map_fx;
        }
        self.ducks.extend(donor.ducks);
        for (name, values) in donor.radverbs {
            self.radverbs.entry(name).or_insert(values);
        }
        if self.duck_groups.is_empty() {
            self.duck_groups = donor.duck_groups;
        }
        if self.masters.is_empty() {
            self.masters = donor.masters;
        }
    }

    /// Marks the catalog as changed: readers that cached by generation rebuild.
    pub fn publish(&mut self) {
        self.edition = next_generation();
    }

    pub fn generation(&self) -> u64 {
        self.edition
    }

    /// The position of an alias: the exact spelling first, then any case.
    pub fn slot_in(&self, space: AssetCategoryScope, wanted: &str) -> Option<usize> {
        if !matches!(space, AssetCategoryScope::T6) {
            return None;
        }
        let lowered = wanted.to_ascii_lowercase();
        self.exact.get(wanted).or_else(|| self.folded.get(&lowered)).copied()
    }

    pub fn slot_named(&self, wanted: &str) -> Option<usize> {
        self.slot_in(AssetCategoryScope::T6, wanted)
    }

    pub fn slot_unique(&self, wanted: &str) -> Option<usize> {
        self.slot_named(wanted)
    }

    pub fn sound_at(&self, index: usize) -> Option<&SnapshotSoundEntry> {
        self.sounds.get(index)
    }

    pub fn entry_in(&self, space: AssetCategoryScope, wanted: &str) -> Option<&SnapshotSoundEntry> {
        self.sound_at(self.slot_in(space, wanted)?)
    }

    pub fn entry_named(&self, wanted: &str) -> Option<&SnapshotSoundEntry> {
        self.entry_in(AssetCategoryScope::T6, wanted)
    }

    pub fn label_at(&self, at: usize) -> Option<&str> {
        self.sound_at(at).map(|sound| sound.name.as_str())
    }

    pub fn space_of_slot(&self, _slot: usize) -> AssetCategoryScope {
        AssetCategoryScope::T6
    }

    /// No clip is held in memory: every Black Ops II clip streams.
    pub fn resident_clip(&self, _slot: usize) -> Option<&ResidentRawAudio> {
        None
    }

    pub fn resident_slot_in(&self, _space: AssetCategoryScope, _wanted: &str) -> Option<usize> {
        None
    }

    /// Black Ops II's entity channel table is not read yet.
    pub fn actor_lane(&self, _lane: u32) -> Option<&ActorAudioLane> {
        None
    }

    /// Where a variant's clip streams from: its namespace, the bank file and
    /// the entry id.
    pub fn clip_for_variant(
        &self,
        space: AssetCategoryScope,
        wanted: &str,
        which: usize,
    ) -> Option<(AssetCategoryScope, String, String)> {
        clip_of_variant(self.entry_in(space, wanted)?, space, which)
    }

    pub fn streamed_for_variant_at(
        &self,
        index: usize,
        which: usize,
    ) -> Option<(AssetCategoryScope, String, String)> {
        clip_of_variant(self.sound_at(index)?, AssetCategoryScope::T6, which)
    }

    /// The map's sound loops. The arguments are not needed: the catalog
    /// holds the one map's scripts.
    pub fn createfx_loop_sounds(&self, _ns: AssetCategoryScope, _map: &str) -> Vec<SpawnEffectAudioLoop> {
        self.scripted_map_fx
            .as_ref()
            .map(|fx| fx.loop_sounds.clone())
            .unwrap_or_default()
    }

    pub fn createfx_oneshots(&self, _ns: AssetCategoryScope, _map: &str) -> Vec<SpawnSingleShotEffect> {
        self.scripted_map_fx
            .as_ref()
            .map(|fx| fx.oneshots.clone())
            .unwrap_or_default()
    }

    /// The sounds the map plays now and then.
    pub fn random_point_sounds(&self) -> &[RandomPointSound] {
        self.scripted_map_fx
            .as_ref()
            .map_or(&[], |fx| fx.random_sounds.as_slice())
    }

    /// Picks a variant of `wanted` by weight, never `avoid` when there is
    /// another. No variant has an in-memory clip, so `picked` stays empty.
    pub fn choose_read_result<'a>(
        &'a self,
        space: AssetCategoryScope,
        wanted: &str,
        roll: &mut u32,
        skip: Option<usize>,
    ) -> Option<ResidentSelectionResult<'a>> {
        self.entry_in(space, wanted).map(|found| roll_variant(found, roll, skip))
    }

    pub fn pick_loaded_outcome_at<'a>(
        &'a self,
        index: usize,
        roll: &mut u32,
        skip: Option<usize>,
    ) -> Option<ResidentSelectionResult<'a>> {
        self.sound_at(index).map(|found| roll_variant(found, roll, skip))
    }
}

/// Older spellings other crates still call; the rename sweep moves them over.
impl AudioRoster {

    pub fn revision(&self) -> u64 {
        self.generation()
    }

}

/// Process-wide counter behind [`AudioRoster::publish`].
fn next_generation() -> u64 {
    static GENERATIONS: AtomicU64 = AtomicU64::new(1);
    let order = Ordering::Relaxed;
    GENERATIONS.fetch_add(1, order)
}

/// The weighted pick for `entry`; the clip is never held, so `picked` is empty.
fn roll_variant<'a>(entry: &SnapshotSoundEntry, roll: &mut u32, avoid: Option<usize>) -> ResidentSelectionResult<'a> {
    let chosen_row = if entry.aliases.is_empty() {
        0
    } else {
        // A variant with no stated chance counts as one.
        let weights: Vec<f32> = entry
            .aliases
            .iter()
            .map(|row| if row.chance_weight > 0.0 { row.chance_weight } else { 1.0 })
            .collect();
        weighted_pick(&weights, roll, avoid)
    };
    ResidentSelectionResult { chosen_row, in_memory: None }
}

/// Where variant `which` of `snd` streams from, unless its entry is unusable.
fn clip_of_variant(
    snd: &SnapshotSoundEntry,
    space: AssetCategoryScope,
    which: usize,
) -> Option<(AssetCategoryScope, String, String)> {
    let (bank, entry) = snd.aliases.get(which)?.streamed.as_ref()?;
    if bank.is_empty() || entry.is_empty() || entry == ",null.wav" {
        return None;
    }
    Some((space, bank.clone(), entry.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SnapshotSoundAlias;

    const T6: AssetCategoryScope = AssetCategoryScope::T6;

    fn sound(name: &str, clips: &[Option<(&str, &str)>]) -> SnapshotSoundEntry {
        SnapshotSoundEntry {
            name: name.to_owned(),
            aliases: clips
                .iter()
                .map(|clip| SnapshotSoundAlias {
                    streamed: clip.map(|(bank, id)| (bank.to_owned(), id.to_owned())),
                    ..SnapshotSoundAlias::default()
                })
                .collect(),
        }
    }

    fn catalog(names: &[&str]) -> AudioRoster {
        let mut catalog = AudioRoster::default();
        names.iter().for_each(|alias| catalog.add_sound(sound(alias, &[None])));
        catalog
    }

    #[test]
    fn names_match_exactly_and_then_in_any_case() {
        let catalog = catalog(&["Door_Open", "door_open_far"]);
        assert_eq!(catalog.slot_in(T6, "Door_Open"), Some(0));
        assert_eq!(catalog.slot_in(T6, "DOOR_OPEN_FAR"), Some(1));
        assert_eq!(catalog.slot_in(AssetCategoryScope::T5, "Door_Open"), None);
        assert_eq!(catalog.slot_unique("missing"), None);
        assert_eq!(catalog.label_at(1), Some("door_open_far"));
        assert!(catalog.entry_named("door_open").is_some());
    }

    #[test]
    fn a_later_alias_of_a_name_shadows_the_earlier() {
        let mut catalog = catalog(&["step"]);
        catalog.add_sound(sound("step", &[None, None]));
        assert_eq!(catalog.slot_named("step"), Some(1));
        assert_eq!(catalog.sounds.len(), 2);
        catalog.add_sound(sound("", &[None]));
        assert_eq!(catalog.sounds.len(), 2);
    }

    #[test]
    fn absorbing_keeps_what_the_catalog_already_has() {
        let mut base = catalog(&["a", "b"]);
        base.duck_groups = vec!["first".to_owned()];
        let mut other = catalog(&["B", "c"]);
        other.duck_groups = vec!["second".to_owned()];
        other.masters = vec![SndMaster::default()];
        other.scripted_map_fx = Some(ScriptedMapFx::default());
        other.rawfiles.insert((T6, "x".to_owned()), vec![1]);
        base.merge_missing(other);
        let names: Vec<_> = base.sounds.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(base.duck_groups, ["first"]);
        assert_eq!(base.masters.len(), 1);
        assert!(base.scripted_map_fx.is_some());
        assert_eq!(base.rawfiles.len(), 1);
    }

    #[test]
    fn a_variant_streams_from_its_bank_entry() {
        let mut catalog = AudioRoster::default();
        catalog.add_sound(sound(
            "boom",
            &[
                Some(("a.sabs", "0000002a")),
                Some(("", "0000002b")),
                Some(("a.sabs", ",null.wav")),
                None,
            ],
        ));
        let want = (T6, "a.sabs".to_owned(), "0000002a".to_owned());
        assert_eq!(catalog.clip_for_variant(T6, "BOOM", 0), Some(want.clone()));
        assert_eq!(catalog.streamed_for_variant_at(0, 0), Some(want));
        for variant in 1..5 {
            assert_eq!(catalog.streamed_for_variant_at(0, variant), None);
        }
    }

    #[test]
    fn a_pick_follows_the_weights_and_avoids_the_last() {
        let mut catalog = AudioRoster::default();
        catalog.add_sound(sound("pair", &[None, None]));
        catalog.add_sound(sound("empty", &[]));
        for seed in [7, 99, 12345, 777_777] {
            let mut rng = seed;
            let outcome = catalog.choose_read_result(T6, "pair", &mut rng, Some(0)).unwrap();
            assert_eq!(outcome.chosen_row, 1);
            assert!(outcome.in_memory.is_none());
        }
        let mut rng = 5;
        let outcome = catalog.pick_loaded_outcome_at(1, &mut rng, None).unwrap();
        assert_eq!(outcome.chosen_row, 0);
        assert_eq!(rng, 5);
        assert!(catalog.choose_read_result(T6, "nothing", &mut rng, None).is_none());
    }

    #[test]
    fn scripted_sounds_come_from_the_map_scripts() {
        let mut catalog = AudioRoster::default();
        assert!(catalog.createfx_loop_sounds(T6, "mp_x").is_empty());
        assert!(catalog.random_point_sounds().is_empty());
        catalog.scripted_map_fx = Some(ScriptedMapFx {
            loop_sounds: vec![SpawnEffectAudioLoop {
                cue_alias: "hum".to_owned(),
                position_in_inches: [1.0, 2.0, 3.0],
                line_end_inches: None,
            }],
            ..ScriptedMapFx::default()
        });
        assert_eq!(catalog.createfx_loop_sounds(T6, "mp_x").len(), 1);
        assert!(catalog.createfx_oneshots(T6, "mp_x").is_empty());
    }

    #[test]
    fn each_publish_is_a_new_generation() {
        let mut catalog = AudioRoster::default();
        catalog.publish();
        let first = catalog.generation();
        catalog.publish();
        assert!(catalog.generation() > first);
    }
}
