//! What content a session is running on, as a list of facts and a digest of
//! them. A fact is either known or comes with the reason it isn't.

use std::collections::HashSet;

use asset_core::{AssetIdentity, AssetVariety};
use asset_game::WeaponLedger;
use bevy::prelude::Resource;
use weapon_t6::combat_facts::GunBattleProfile;

/// Bumped when the digest's layout changes, so old and new never compare equal.
pub const CONTENT_LEDGER_FORMAT: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveRuleVariant {
    /// Its number (1) is part of the digest.
    T6 = 1,
}

/// A weapon's number in this session (1-based; 0 is "no weapon").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ManifestGunId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestEntryState<T> {
    Known(T),
    Unavailable(ContentShortfall),
}

/// Why a fact isn't known. The numbers are part of the digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ContentShortfall {
    MapSourceNotOpened = 1,
    InvalidMapKey = 2,
    ValidatedWeaponProfileMissing = 3,
    AtomicWeaponPresentationBundleNotInstalled = 4,
    CharacterPackageNotInstalled = 5,
    NamespacedMaterialCorpusNotInstalled = 6,
    ProgramCorpusNotFrozen = 7,
}

/// Which rules the server runs a weapon under. The numbers are part of the digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostArmamentLoadout {
    Combat = 1,
    Equipment = 2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestGunEntry {
    pub id: ManifestGunId,
    pub key: AssetIdentity,
    pub extras: Vec<String>,
    pub alternate: bool,
    pub authority: ManifestEntryState<HostArmamentLoadout>,
    pub presentation: ManifestEntryState<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Resource)]
pub struct GameContentLedger {
    pub layout: u32,
    pub ruleset: LiveRuleVariant,
    pub map: ManifestEntryState<AssetIdentity>,
    pub play_state_checksum: u64,
    pub weapons: Vec<ManifestGunEntry>,
    pub character_package: ManifestEntryState<AssetIdentity>,
    pub material_corpus: ManifestEntryState<u64>,
    pub program_corpus: ManifestEntryState<u64>,
    pub digest: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentManifestFault {
    MissingWeaponKey(ManifestGunId),
    /// The same key, attachments and alternate flag twice.
    DuplicateWeaponKey(AssetIdentity, Vec<String>, bool),
}

impl std::fmt::Display for ContentManifestFault {
    fn fmt(&self, sink: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (key, extras, alternate) = match self {
            Self::DuplicateWeaponKey(key, extras, alternate) => (key, extras, alternate),
            Self::MissingWeaponKey(ManifestGunId(number)) => {
                return write!(sink, "session weapon {number} has no durable key");
            }
        };
        sink.write_str("duplicate durable session weapon key `")?;
        write!(sink, "{key}` [{}] alternate={alternate}", extras.join(" "))
    }
}

impl std::error::Error for ContentManifestFault {}

impl<T> ManifestEntryState<T> {
    /// A fact that is not known, for the given reason.
    const fn gap(why: ContentShortfall) -> Self {
        Self::Unavailable(why)
    }
}

/// The facts that decide which rules a weapon number runs under.
#[derive(Clone, Copy)]
struct ProfileTables<'a> {
    combat: &'a [GunBattleProfile],
    gear: &'a [sim::GadgetLiveStats],
}

impl ProfileTables<'_> {
    /// Which profile the server will run weapon number `slot` under, if it has one.
    fn authority(self, slot: usize) -> ManifestEntryState<HostArmamentLoadout> {
        if self.combat.get(slot).is_some_and(|facts| facts.ready_for_use()) {
            return ManifestEntryState::Known(HostArmamentLoadout::Combat);
        }
        match self.gear.get(slot).map(|facts| facts.is_secondary_gear()) {
            Some(true) => ManifestEntryState::Known(HostArmamentLoadout::Equipment),
            _ => ManifestEntryState::gap(ContentShortfall::ValidatedWeaponProfileMissing),
        }
    }
}

fn map_fact(level: &assets::ReadiedMapData) -> ManifestEntryState<AssetIdentity> {
    level.namespace
        .ok_or(ContentShortfall::MapSourceNotOpened)
        .and_then(|space| {
            AssetIdentity::new(space, AssetVariety::Map, level.zone.clone())
                .map_err(|_| ContentShortfall::InvalidMapKey)
        })
        .map_or_else(ManifestEntryState::gap, ManifestEntryState::Known)
}

/// One registry weapon as a manifest row, refusing an identity already in `taken`.
fn weapon_row(
    number: u32,
    registry: &WeaponLedger,
    tables: ProfileTables<'_>,
    taken: &mut HashSet<(AssetIdentity, Vec<String>, bool)>,
) -> Result<ManifestGunEntry, ContentManifestFault> {
    let id = ManifestGunId(number);
    let unkeyed = ContentManifestFault::MissingWeaponKey(id);
    let identity: (AssetIdentity, Vec<String>, bool) = (
        registry.lookup_key_for(number).ok_or(unkeyed)?,
        registry.prepared_attachments_of(number).to_vec(),
        registry
            .summary_for(number)
            .is_some_and(|facts| facts.carry_slot_kind == 3),
    );
    if taken.contains(&identity) {
        let (key, extras, alternate) = identity;
        return Err(ContentManifestFault::DuplicateWeaponKey(key, extras, alternate));
    }
    taken.insert(identity.clone());
    let (key, extras, alternate) = identity;
    Ok(ManifestGunEntry {
        authority: tables.authority(number as usize),
        presentation: ManifestEntryState::gap(ContentShortfall::AtomicWeaponPresentationBundleNotInstalled),
        id,
        key,
        extras,
        alternate,
    })
}

impl GameContentLedger {
    /// The manifest for `prepared_map` and every weapon the registry holds.
    pub fn build(
        prepared_map: &assets::ReadiedMapData,
        registry: &WeaponLedger,
        weapon_combat: &[GunBattleProfile],
        equipment: &[sim::GadgetLiveStats],
        gameplay: u64,
    ) -> Result<GameContentLedger, ContentManifestFault> {
        let tables = ProfileTables { combat: weapon_combat, gear: equipment };
        let last = u32::try_from(registry.len()).unwrap_or(u32::MAX);
        let mut taken = HashSet::<(AssetIdentity, Vec<String>, bool)>::new();
        let weapons = (1..=last)
            .map(|number| weapon_row(number, registry, tables, &mut taken))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::uninstalled(map_fact(prepared_map), gameplay, weapons).sealed())
    }

    /// What a client is told it is joining; there is none until the map is known.
    pub fn session_summary(
        &self,
        parts: sim::ContentPartDigests,
    ) -> Option<net::GameSessionSummary> {
        matches!(self.map, ManifestEntryState::Known(_))
            .then(|| net::GameSessionSummary::from_components(parts))
            .flatten()
    }

    /// A manifest whose installable content is all still missing; its digest is
    /// not filled in yet.
    fn uninstalled(
        map: ManifestEntryState<AssetIdentity>,
        gameplay: u64,
        weapons: Vec<ManifestGunEntry>,
    ) -> Self {
        Self {
            ruleset: LiveRuleVariant::T6,
            layout: CONTENT_LEDGER_FORMAT,
            character_package: ManifestEntryState::gap(ContentShortfall::CharacterPackageNotInstalled),
            material_corpus: ManifestEntryState::gap(ContentShortfall::NamespacedMaterialCorpusNotInstalled),
            program_corpus: ManifestEntryState::gap(ContentShortfall::ProgramCorpusNotFrozen),
            play_state_checksum: gameplay,
            map,
            weapons,
            digest: 0,
        }
    }

    /// The same manifest with its digest filled in: FNV-1a over its record.
    fn sealed(self) -> Self {
        let digest = fnv1a(&self.record());
        Self { digest, ..self }
    }

    /// Every field, in a fixed order, as the bytes the digest is taken over.
    fn record(&self) -> Vec<u8> {
        let mut rec = Record::default();
        rec.push_u32(self.layout).push_u8(self.ruleset as u8);
        rec.fact(&self.map, Record::key);
        rec.push_u64(self.play_state_checksum).len(self.weapons.len());
        self.weapons.iter().for_each(|weapon| rec.weapon(weapon));
        rec.fact(&self.character_package, Record::key);
        rec.fact(&self.material_corpus, Record::word);
        rec.fact(&self.program_corpus, Record::word);
        rec.0
    }
}

/// 64-bit FNV-1a of `bytes`.
fn fnv1a(bytes: &[u8]) -> u64 {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const MULTIPLIER: u64 = 0x0100_0000_01b3;
    bytes
        .iter()
        .fold(BASIS, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(MULTIPLIER))
}

/// The digest's input, little-endian, appended to field by field.
#[derive(Default)]
struct Record(Vec<u8>);

impl Record {
    fn push_u8(&mut self, byte: u8) -> &mut Self {
        self.0.push(byte);
        self
    }

    fn push_u32(&mut self, word: u32) -> &mut Self {
        self.0.extend(word.to_le_bytes());
        self
    }

    fn push_u64(&mut self, word: u64) -> &mut Self {
        self.0.extend(word.to_le_bytes());
        self
    }

    fn len(&mut self, count: usize) -> &mut Self {
        self.push_u64(count as u64)
    }

    /// Text, after its length, so that neighbouring strings can't blur together.
    fn str(&mut self, text: &str) -> &mut Self {
        self.len(text.len());
        self.0.extend_from_slice(text.as_bytes());
        self
    }

    fn key(&mut self, key: &AssetIdentity) {
        self.str(key.namespace.as_str())
            .str(key.kind.as_str())
            .str(&key.name);
    }

    fn word(&mut self, word: &u64) {
        self.push_u64(*word);
    }

    fn profile(&mut self, profile: &HostArmamentLoadout) {
        self.push_u8(*profile as u8);
    }

    /// 1 then the value, or 0 then the gap's number.
    fn fact<T>(&mut self, entry: &ManifestEntryState<T>, value: impl FnOnce(&mut Self, &T)) {
        if let ManifestEntryState::Known(known) = entry {
            value(self.push_u8(1), known);
        } else if let ManifestEntryState::Unavailable(why) = *entry {
            self.push_u8(0).push_u8(why as u8);
        }
    }

    fn weapon(&mut self, weapon: &ManifestGunEntry) {
        self.push_u32(weapon.id.0);
        self.key(&weapon.key);
        self.len(weapon.extras.len());
        weapon.extras.iter().for_each(|name| {
            self.str(name);
        });
        self.push_u8(weapon.alternate.into());
        self.fact(&weapon.authority, Record::profile);
        self.fact(&weapon.presentation, Record::word);
    }
}

#[cfg(test)]
mod tests {
    use asset_core::AssetCategoryScope;

    use super::*;

    fn t6(kind: AssetVariety, name: &str) -> AssetIdentity {
        AssetIdentity::new(AssetCategoryScope::T6, kind, name.to_owned()).unwrap()
    }

    fn empty_manifest() -> GameContentLedger {
        GameContentLedger::uninstalled(
            ManifestEntryState::gap(ContentShortfall::MapSourceNotOpened),
            7,
            Vec::new(),
        )
    }

    fn with_map(name: &str) -> GameContentLedger {
        let mut manifest = empty_manifest();
        manifest.map = ManifestEntryState::Known(t6(AssetVariety::Map, name));
        manifest
    }

    fn with_gun(name: &str) -> GameContentLedger {
        let mut manifest = empty_manifest();
        manifest.weapons.push(ManifestGunEntry {
            id: ManifestGunId(1),
            key: t6(AssetVariety::Weapon, name),
            extras: Vec::new(),
            alternate: false,
            authority: ManifestEntryState::gap(ContentShortfall::ValidatedWeaponProfileMissing),
            presentation: ManifestEntryState::gap(ContentShortfall::AtomicWeaponPresentationBundleNotInstalled),
        });
        manifest
    }

    fn digest_of(manifest: &GameContentLedger) -> u64 {
        fnv1a(&manifest.record())
    }

    fn text_record(parts: &[&str]) -> Vec<u8> {
        let mut rec = Record::default();
        parts.iter().for_each(|part| {
            rec.str(part);
        });
        rec.0
    }

    #[test]
    fn fnv_matches_the_published_test_vectors() {
        let vectors: [(&[u8], u64); 3] = [
            (b"", 0xcbf2_9ce4_8422_2325),
            (b"a", 0xaf63_dc4c_8601_ec8c),
            (b"foobar", 0x8594_4171_f739_67e8),
        ];
        for (input, expected) in vectors {
            assert_eq!(fnv1a(input), expected);
        }
    }

    #[test]
    fn text_carries_its_length_so_neighbours_cannot_blur() {
        assert_ne!(
            fnv1a(&text_record(&["ab", "c"])),
            fnv1a(&text_record(&["a", "bc"]))
        );
    }

    #[test]
    fn the_same_manifest_digests_the_same() {
        assert_eq!(digest_of(&empty_manifest()), digest_of(&empty_manifest()));
    }

    #[test]
    fn every_input_changes_the_digest() {
        let reference = digest_of(&empty_manifest());
        let mut more_gameplay = empty_manifest();
        more_gameplay.play_state_checksum += 1;
        let mut other_gap = empty_manifest();
        other_gap.program_corpus = ManifestEntryState::gap(ContentShortfall::InvalidMapKey);
        let variants = [more_gameplay, other_gap, with_map("mp_a"), with_gun("gun")];
        assert!(variants.iter().all(|v| digest_of(v) != reference));
    }

    #[test]
    fn two_maps_digest_differently_and_so_do_two_weapons() {
        assert_ne!(digest_of(&with_map("mp_a")), digest_of(&with_map("mp_b")));
        assert_ne!(digest_of(&with_gun("gun_a")), digest_of(&with_gun("gun_b")));
    }

    #[test]
    fn a_weapons_attachments_and_alternate_flag_count() {
        let plain = with_gun("gun");
        let mut attached = with_gun("gun");
        attached.weapons[0].extras.push("reflex".to_owned());
        let mut alternate = with_gun("gun");
        alternate.weapons[0].alternate = true;
        let digests: HashSet<u64> = [plain, attached, alternate]
            .iter()
            .map(digest_of)
            .collect();
        assert_eq!(digests.len(), 3);
    }

    #[test]
    fn gap_numbers_are_the_documented_ones() {
        assert_eq!(ContentShortfall::MapSourceNotOpened as u8, 1);
        assert_eq!(ContentShortfall::ProgramCorpusNotFrozen as u8, 7);
    }

    #[test]
    fn a_map_that_was_never_opened_has_no_descriptor() {
        let descriptor = empty_manifest().session_summary(sim::ContentPartDigests::default());
        assert!(descriptor.is_none());
    }

    #[test]
    fn a_prepared_map_without_a_namespace_is_unopened() {
        let unopened = map_fact(&assets::ReadiedMapData::default());
        assert_eq!(unopened, ManifestEntryState::gap(ContentShortfall::MapSourceNotOpened));
    }

    #[test]
    fn a_prepared_map_with_a_zone_is_known_by_its_key() {
        let mut map = assets::ReadiedMapData::default();
        map.zone = "mp_a".to_owned();
        map.namespace = Some(AssetCategoryScope::T6);
        assert_eq!(map_fact(&map), ManifestEntryState::Known(t6(AssetVariety::Map, "mp_a")));
    }

    #[test]
    fn an_empty_registry_builds_a_manifest_with_no_weapons() {
        let registry = WeaponLedger::default();
        let built =
            GameContentLedger::build(&assets::ReadiedMapData::default(), &registry, &[], &[], 9);
        let built = built.expect("an empty registry has nothing to refuse");
        assert!(built.weapons.is_empty());
        assert_eq!(built.play_state_checksum, 9);
        assert_eq!(built.digest, digest_of(&built));
    }

    #[test]
    fn a_weapon_with_no_profile_is_unavailable_and_with_one_is_known() {
        let profile = ProfileTables { combat: &[], gear: &[] }.authority(1);
        assert_eq!(profile, ManifestEntryState::gap(ContentShortfall::ValidatedWeaponProfileMissing));
    }

    #[test]
    fn the_errors_say_which_weapon() {
        let missing = ContentManifestFault::MissingWeaponKey(ManifestGunId(4));
        assert_eq!(missing.to_string(), "session weapon 4 has no durable key");
        let extras = vec!["a".to_owned(), "b".to_owned()];
        let duplicate =
            ContentManifestFault::DuplicateWeaponKey(t6(AssetVariety::Weapon, "gun"), extras, true);
        let text = duplicate.to_string();
        assert!(text.contains("[a b]"));
        assert!(text.contains("alternate=true"));
    }
}
