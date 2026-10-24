//! The sound catalog: every alias a match can play, found by name.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::variant_pick::pick_weighted_variant_index;
use crate::{
    AssetNamespace, CapturedAlias, CapturedSound, CreateFxLoopSound, CreateFxOneshot,
    LoadedSoundPcm, PickLoadedOutcome, RandomPointSound, ScriptedMapFx, SndDuck, SndMaster,
};

/// One entity sound channel and how it limits the voices on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntChannel {
    pub name: String,
    pub priority: i32,
    pub is_3d: bool,
    pub is_restricted: bool,
    pub is_pausable: bool,
    pub max_voices: i32,
}

/// Aliases by name, with what the mixer needs beside them: the ducks, the
/// master mixes, the raw files a map ships, and the map's scripted sounds.
#[derive(Clone, Debug, Default)]
pub struct SoundCatalog {
    pub sounds: Vec<CapturedSound>,
    pub rawfiles: HashMap<(AssetNamespace, String), Vec<u8>>,
    /// What the map's compiled scripts place: effects, sound loops and the
    /// sounds it plays now and then.
    pub scripted_map_fx: Option<ScriptedMapFx>,
    /// Ducks by id; a later bank's duck replaces an earlier one's.
    pub ducks: HashMap<u32, SndDuck>,
    /// Duck group names, indexed by `CapturedAlias::duck_group`.
    pub duck_groups: Vec<String>,
    pub masters: Vec<SndMaster>,
    exact: HashMap<String, usize>,
    folded: HashMap<String, usize>,
    revision: u64,
}

impl SoundCatalog {
    /// Adds an alias. A name already present is shadowed by the new one.
    pub fn add_sound(&mut self, sound: CapturedSound) {
        if sound.name.is_empty() {
            return;
        }
        let at = self.sounds.len();
        self.exact.insert(sound.name.clone(), at);
        self.folded.insert(sound.name.to_ascii_lowercase(), at);
        self.sounds.push(sound);
    }

    /// Takes from `other` what this catalog does not have yet: aliases whose
    /// names are new, raw files, the scripted fx, ducks, duck groups and
    /// masters (the last three only while this catalog has none).
    pub fn absorb_missing_aliases(&mut self, other: SoundCatalog) {
        for sound in other.sounds {
            if self.index_by_name(&sound.name).is_none() {
                self.add_sound(sound);
            }
        }
        for (key, data) in other.rawfiles {
            self.rawfiles.entry(key).or_insert(data);
        }
        if self.scripted_map_fx.is_none() {
            self.scripted_map_fx = other.scripted_map_fx;
        }
        self.ducks.extend(other.ducks);
        if self.duck_groups.is_empty() {
            self.duck_groups = other.duck_groups;
        }
        if self.masters.is_empty() {
            self.masters = other.masters;
        }
    }

    /// Marks the catalog as changed: readers that cached by revision rebuild.
    pub fn publish(&mut self) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        self.revision = NEXT.fetch_add(1, Ordering::Relaxed);
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The position of an alias: the exact spelling first, then any case.
    pub fn index_in(&self, ns: AssetNamespace, alias: &str) -> Option<usize> {
        if ns != AssetNamespace::T6 {
            return None;
        }
        self.exact
            .get(alias)
            .or_else(|| self.folded.get(&alias.to_ascii_lowercase()))
            .copied()
    }

    pub fn index_by_name(&self, alias: &str) -> Option<usize> {
        self.index_in(AssetNamespace::T6, alias)
    }

    pub fn index_unique(&self, alias: &str) -> Option<usize> {
        self.index_by_name(alias)
    }

    pub fn sound_at(&self, index: usize) -> Option<&CapturedSound> {
        self.sounds.get(index)
    }

    pub fn sound_in(&self, ns: AssetNamespace, alias: &str) -> Option<&CapturedSound> {
        self.sound_at(self.index_in(ns, alias)?)
    }

    pub fn sound(&self, alias: &str) -> Option<&CapturedSound> {
        self.sound_in(AssetNamespace::T6, alias)
    }

    pub fn name_at(&self, index: usize) -> Option<&str> {
        self.sound_at(index).map(|sound| sound.name.as_str())
    }

    pub fn namespace_of_alias(&self, _index: usize) -> AssetNamespace {
        AssetNamespace::T6
    }

    /// No clip is held in memory: every Black Ops II clip streams.
    pub fn pcm_at(&self, _index: usize) -> Option<&LoadedSoundPcm> {
        None
    }

    pub fn loaded_index_in(&self, _ns: AssetNamespace, _name: &str) -> Option<usize> {
        None
    }

    /// Black Ops II's entity channel table is not read yet.
    pub fn ent_channel(&self, _channel: u32) -> Option<&EntChannel> {
        None
    }

    /// Where a variant's clip streams from: its namespace, the bank file and
    /// the entry id.
    pub fn streamed_for_variant(
        &self,
        ns: AssetNamespace,
        alias: &str,
        variant: usize,
    ) -> Option<(AssetNamespace, String, String)> {
        let row = self.sound_in(ns, alias)?.aliases.get(variant)?;
        streamed_clip(ns, row)
    }

    pub fn streamed_for_variant_at(
        &self,
        index: usize,
        variant: usize,
    ) -> Option<(AssetNamespace, String, String)> {
        let row = self.sound_at(index)?.aliases.get(variant)?;
        streamed_clip(AssetNamespace::T6, row)
    }

    /// The map's sound loops. The arguments are not needed: the catalog
    /// holds the one map's scripts.
    pub fn createfx_loop_sounds(&self, _ns: AssetNamespace, _map: &str) -> Vec<CreateFxLoopSound> {
        self.scripted_map_fx
            .as_ref()
            .map(|fx| fx.loop_sounds.clone())
            .unwrap_or_default()
    }

    pub fn createfx_oneshots(&self, _ns: AssetNamespace, _map: &str) -> Vec<CreateFxOneshot> {
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

    /// Picks a variant of `alias` by weight, never `avoid` when there is
    /// another. No variant has an in-memory clip, so `picked` stays empty.
    pub fn pick_loaded_outcome<'a>(
        &'a self,
        ns: AssetNamespace,
        alias: &str,
        rng: &mut u32,
        avoid: Option<usize>,
    ) -> Option<PickLoadedOutcome<'a>> {
        let sound = self.sound_in(ns, alias)?;
        Some(outcome_of(variant_choice(sound, rng, avoid)))
    }

    pub fn pick_loaded_outcome_at<'a>(
        &'a self,
        index: usize,
        rng: &mut u32,
        avoid: Option<usize>,
    ) -> Option<PickLoadedOutcome<'a>> {
        let sound = self.sound_at(index)?;
        Some(outcome_of(variant_choice(sound, rng, avoid)))
    }
}

fn outcome_of<'a>(variant_index: usize) -> PickLoadedOutcome<'a> {
    PickLoadedOutcome {
        variant_index,
        picked: None,
    }
}

fn variant_choice(sound: &CapturedSound, rng: &mut u32, avoid: Option<usize>) -> usize {
    if sound.aliases.is_empty() {
        return 0;
    }
    let weights: Vec<f32> = sound
        .aliases
        .iter()
        .map(|row| if row.probability > 0.0 { row.probability } else { 1.0 })
        .collect();
    pick_weighted_variant_index(&weights, rng, avoid)
}

fn streamed_clip(ns: AssetNamespace, row: &CapturedAlias) -> Option<(AssetNamespace, String, String)> {
    let (bank, entry) = row.streamed.as_ref()?;
    let usable = !bank.is_empty() && !entry.is_empty() && entry != ",null.wav";
    usable.then(|| (ns, bank.clone(), entry.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T6: AssetNamespace = AssetNamespace::T6;

    fn sound(name: &str, clips: &[Option<(&str, &str)>]) -> CapturedSound {
        CapturedSound {
            name: name.to_owned(),
            aliases: clips
                .iter()
                .map(|clip| CapturedAlias {
                    streamed: clip.map(|(bank, id)| (bank.to_owned(), id.to_owned())),
                    ..CapturedAlias::default()
                })
                .collect(),
        }
    }

    fn catalog(names: &[&str]) -> SoundCatalog {
        let mut catalog = SoundCatalog::default();
        for name in names {
            catalog.add_sound(sound(name, &[None]));
        }
        catalog
    }

    #[test]
    fn names_match_exactly_and_then_in_any_case() {
        let catalog = catalog(&["Door_Open", "door_open_far"]);
        assert_eq!(catalog.index_in(T6, "Door_Open"), Some(0));
        assert_eq!(catalog.index_in(T6, "DOOR_OPEN_FAR"), Some(1));
        assert_eq!(catalog.index_in(AssetNamespace::T5, "Door_Open"), None);
        assert_eq!(catalog.index_unique("missing"), None);
        assert_eq!(catalog.name_at(1), Some("door_open_far"));
        assert!(catalog.sound("door_open").is_some());
    }

    #[test]
    fn a_later_alias_of_a_name_shadows_the_earlier() {
        let mut catalog = catalog(&["step"]);
        catalog.add_sound(sound("step", &[None, None]));
        assert_eq!(catalog.index_by_name("step"), Some(1));
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
        base.absorb_missing_aliases(other);
        let names: Vec<_> = base.sounds.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(base.duck_groups, ["first"]);
        assert_eq!(base.masters.len(), 1);
        assert!(base.scripted_map_fx.is_some());
        assert_eq!(base.rawfiles.len(), 1);
    }

    #[test]
    fn a_variant_streams_from_its_bank_entry() {
        let mut catalog = SoundCatalog::default();
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
        assert_eq!(catalog.streamed_for_variant(T6, "BOOM", 0), Some(want.clone()));
        assert_eq!(catalog.streamed_for_variant_at(0, 0), Some(want));
        for variant in 1..5 {
            assert_eq!(catalog.streamed_for_variant_at(0, variant), None);
        }
    }

    #[test]
    fn a_pick_follows_the_weights_and_avoids_the_last() {
        let mut catalog = SoundCatalog::default();
        catalog.add_sound(sound("pair", &[None, None]));
        catalog.add_sound(sound("empty", &[]));
        for seed in [7, 99, 12345, 777_777] {
            let mut rng = seed;
            let outcome = catalog.pick_loaded_outcome(T6, "pair", &mut rng, Some(0)).unwrap();
            assert_eq!(outcome.variant_index, 1);
            assert!(outcome.picked.is_none());
        }
        let mut rng = 5;
        let outcome = catalog.pick_loaded_outcome_at(1, &mut rng, None).unwrap();
        assert_eq!(outcome.variant_index, 0);
        assert_eq!(rng, 5);
        assert!(catalog.pick_loaded_outcome(T6, "nothing", &mut rng, None).is_none());
    }

    #[test]
    fn scripted_sounds_come_from_the_map_scripts() {
        let mut catalog = SoundCatalog::default();
        assert!(catalog.createfx_loop_sounds(T6, "mp_x").is_empty());
        assert!(catalog.random_point_sounds().is_empty());
        catalog.scripted_map_fx = Some(ScriptedMapFx {
            loop_sounds: vec![CreateFxLoopSound {
                soundalias: "hum".to_owned(),
                origin_inches: [1.0, 2.0, 3.0],
            }],
            ..ScriptedMapFx::default()
        });
        assert_eq!(catalog.createfx_loop_sounds(T6, "mp_x").len(), 1);
        assert!(catalog.createfx_oneshots(T6, "mp_x").is_empty());
    }

    #[test]
    fn each_publish_is_a_new_revision() {
        let mut catalog = SoundCatalog::default();
        catalog.publish();
        let first = catalog.revision();
        catalog.publish();
        assert!(catalog.revision() > first);
    }
}
