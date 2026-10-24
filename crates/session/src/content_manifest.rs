//! What content a session is running on, as a list of facts and a digest of
//! them. A fact is either known or comes with the reason it isn't.

use std::collections::HashSet;

use asset_core::{AssetKey, AssetKind};
use asset_game::WeaponRegistry;
use bevy::prelude::Resource;
use weapon_t6::combat_facts::WeaponCombatFacts;

/// Bumped when the digest's layout changes, so old and new never compare equal.
pub const SESSION_MANIFEST_SCHEME: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeRuleset {
    T6,
}

/// A weapon's number in this session (1-based; 0 is "no weapon").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionWeaponId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestFact<T> {
    Known(T),
    Unavailable(ManifestGap),
}

/// Why a fact isn't known. The numbers are part of the digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ManifestGap {
    MapSourceNotOpened = 1,
    InvalidMapKey = 2,
    ValidatedWeaponProfileMissing = 3,
    AtomicWeaponPresentationBundleNotInstalled = 4,
    CharacterPackageNotInstalled = 5,
    NamespacedMaterialCorpusNotInstalled = 6,
    ProgramCorpusNotFrozen = 7,
}

/// Which rules the server runs a weapon under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityWeaponProfile {
    Combat,
    Equipment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionWeaponManifestRow {
    pub id: SessionWeaponId,
    pub key: AssetKey,
    pub attachments: Vec<String>,
    pub alternate: bool,
    pub authority: ManifestFact<AuthorityWeaponProfile>,
    pub presentation: ManifestFact<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Resource)]
pub struct SessionContentManifest {
    pub scheme: u32,
    pub ruleset: RuntimeRuleset,
    pub map: ManifestFact<AssetKey>,
    pub gameplay_digest: u64,
    pub weapons: Vec<SessionWeaponManifestRow>,
    pub character_package: ManifestFact<AssetKey>,
    pub material_corpus: ManifestFact<u64>,
    pub program_corpus: ManifestFact<u64>,
    pub digest: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionManifestError {
    MissingWeaponKey(SessionWeaponId),
    /// The same key, attachments and alternate flag twice.
    DuplicateWeaponKey(AssetKey, Vec<String>, bool),
}

impl core::fmt::Display for SessionManifestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingWeaponKey(id) => write!(f, "session weapon {} has no durable key", id.0),
            Self::DuplicateWeaponKey(key, attachments, alternate) => write!(
                f,
                "duplicate durable session weapon key `{key}` [{}] alternate={alternate}",
                attachments.join(" ")
            ),
        }
    }
}

impl std::error::Error for SessionManifestError {}

/// Which profile the server will run weapon `id` under, if it has one.
fn authority_of(
    id: usize,
    combat: &[WeaponCombatFacts],
    equipment: &[sim::EquipmentRuntimeFacts],
) -> ManifestFact<AuthorityWeaponProfile> {
    if combat
        .get(id)
        .copied()
        .is_some_and(WeaponCombatFacts::is_usable)
    {
        ManifestFact::Known(AuthorityWeaponProfile::Combat)
    } else if equipment
        .get(id)
        .copied()
        .is_some_and(sim::EquipmentRuntimeFacts::is_offhand)
    {
        ManifestFact::Known(AuthorityWeaponProfile::Equipment)
    } else {
        ManifestFact::Unavailable(ManifestGap::ValidatedWeaponProfileMissing)
    }
}

fn map_fact(map: &assets::PreparedMap) -> ManifestFact<AssetKey> {
    let Some(namespace) = map.namespace else {
        return ManifestFact::Unavailable(ManifestGap::MapSourceNotOpened);
    };
    match AssetKey::new(namespace, AssetKind::Map, map.zone.clone()) {
        Ok(key) => ManifestFact::Known(key),
        Err(_) => ManifestFact::Unavailable(ManifestGap::InvalidMapKey),
    }
}

impl SessionContentManifest {
    /// The manifest for `map` and every weapon the registry holds.
    pub fn build(
        map: &assets::PreparedMap,
        registry: &WeaponRegistry,
        combat: &[WeaponCombatFacts],
        equipment: &[sim::EquipmentRuntimeFacts],
        gameplay_digest: u64,
    ) -> Result<Self, SessionManifestError> {
        let count = u32::try_from(registry.len()).unwrap_or(u32::MAX);
        let mut seen = HashSet::new();
        let mut weapons = Vec::new();
        for raw in 1..=count {
            let id = SessionWeaponId(raw);
            let key = registry
                .key_of(raw)
                .ok_or(SessionManifestError::MissingWeaponKey(id))?;
            let attachments = registry.prepared_attachments_of(raw).to_vec();
            let alternate = registry
                .facts_of(raw)
                .is_some_and(|facts| facts.inventory_type == 3);
            if !seen.insert((key.clone(), attachments.clone(), alternate)) {
                return Err(SessionManifestError::DuplicateWeaponKey(
                    key,
                    attachments,
                    alternate,
                ));
            }
            weapons.push(SessionWeaponManifestRow {
                id,
                key,
                attachments,
                alternate,
                authority: authority_of(raw as usize, combat, equipment),
                presentation: ManifestFact::Unavailable(
                    ManifestGap::AtomicWeaponPresentationBundleNotInstalled,
                ),
            });
        }
        let mut manifest = Self {
            scheme: SESSION_MANIFEST_SCHEME,
            ruleset: RuntimeRuleset::T6,
            map: map_fact(map),
            gameplay_digest,
            weapons,
            character_package: ManifestFact::Unavailable(ManifestGap::CharacterPackageNotInstalled),
            material_corpus: ManifestFact::Unavailable(
                ManifestGap::NamespacedMaterialCorpusNotInstalled,
            ),
            program_corpus: ManifestFact::Unavailable(ManifestGap::ProgramCorpusNotFrozen),
            digest: 0,
        };
        manifest.digest = manifest.compute_digest();
        Ok(manifest)
    }

    /// What a client is told it is joining; there is none until the map is known.
    pub fn match_descriptor(
        &self,
        components: sim::ContentComponents,
    ) -> Option<net::MatchDescriptor> {
        match self.map {
            ManifestFact::Known(_) => net::MatchDescriptor::from_components(components),
            ManifestFact::Unavailable(_) => None,
        }
    }

    fn compute_digest(&self) -> u64 {
        let mut hash = Fnv::new();
        hash.u32(self.scheme);
        hash.u8(match self.ruleset {
            RuntimeRuleset::T6 => 1,
        });
        hash.fact(&self.map, Fnv::key);
        hash.u64(self.gameplay_digest);
        hash.u64(self.weapons.len() as u64);
        for row in &self.weapons {
            hash.u32(row.id.0);
            hash.key(&row.key);
            hash.u64(row.attachments.len() as u64);
            for attachment in &row.attachments {
                hash.text(attachment);
            }
            hash.u8(u8::from(row.alternate));
            hash.fact(&row.authority, |hash, profile| {
                hash.u8(match profile {
                    AuthorityWeaponProfile::Combat => 1,
                    AuthorityWeaponProfile::Equipment => 2,
                });
            });
            hash.fact(&row.presentation, |hash, value| hash.u64(*value));
        }
        hash.fact(&self.character_package, Fnv::key);
        hash.fact(&self.material_corpus, |hash, value| hash.u64(*value));
        hash.fact(&self.program_corpus, |hash, value| hash.u64(*value));
        hash.0
    }
}

/// 64-bit FNV-1a, fed field by field.
struct Fnv(u64);

impl Fnv {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn raw(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(Self::PRIME);
        }
    }

    fn u8(&mut self, value: u8) {
        self.raw(&[value]);
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.raw(&value.to_le_bytes());
    }

    /// Text, with its length so that neighbouring strings can't blur together.
    fn text(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.raw(value.as_bytes());
    }

    fn key(&mut self, key: &AssetKey) {
        self.text(key.namespace.as_str());
        self.text(key.kind.as_str());
        self.text(&key.name);
    }

    fn fact<T>(&mut self, fact: &ManifestFact<T>, known: impl FnOnce(&mut Self, &T)) {
        match fact {
            ManifestFact::Known(value) => {
                self.u8(1);
                known(self, value);
            }
            ManifestFact::Unavailable(gap) => {
                self.u8(0);
                self.u8(*gap as u8);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use asset_core::AssetNamespace;

    use super::*;

    fn key(kind: AssetKind, name: &str) -> AssetKey {
        AssetKey::new(AssetNamespace::T6, kind, name.to_owned()).unwrap()
    }

    fn bare() -> SessionContentManifest {
        SessionContentManifest {
            scheme: SESSION_MANIFEST_SCHEME,
            ruleset: RuntimeRuleset::T6,
            map: ManifestFact::Unavailable(ManifestGap::MapSourceNotOpened),
            gameplay_digest: 7,
            weapons: Vec::new(),
            character_package: ManifestFact::Unavailable(ManifestGap::CharacterPackageNotInstalled),
            material_corpus: ManifestFact::Unavailable(
                ManifestGap::NamespacedMaterialCorpusNotInstalled,
            ),
            program_corpus: ManifestFact::Unavailable(ManifestGap::ProgramCorpusNotFrozen),
            digest: 0,
        }
    }

    fn row(id: u32, name: &str) -> SessionWeaponManifestRow {
        SessionWeaponManifestRow {
            id: SessionWeaponId(id),
            key: key(AssetKind::Weapon, name),
            attachments: Vec::new(),
            alternate: false,
            authority: ManifestFact::Unavailable(ManifestGap::ValidatedWeaponProfileMissing),
            presentation: ManifestFact::Unavailable(
                ManifestGap::AtomicWeaponPresentationBundleNotInstalled,
            ),
        }
    }

    #[test]
    fn fnv_matches_the_published_test_vectors() {
        let mut empty = Fnv::new();
        empty.raw(b"");
        assert_eq!(empty.0, 0xcbf2_9ce4_8422_2325);
        let mut a = Fnv::new();
        a.raw(b"a");
        assert_eq!(a.0, 0xaf63_dc4c_8601_ec8c);
        let mut foobar = Fnv::new();
        foobar.raw(b"foobar");
        assert_eq!(foobar.0, 0x8594_4171_f739_67e8);
    }

    #[test]
    fn text_carries_its_length_so_neighbours_cannot_blur() {
        let mut ab_c = Fnv::new();
        ab_c.text("ab");
        ab_c.text("c");
        let mut a_bc = Fnv::new();
        a_bc.text("a");
        a_bc.text("bc");
        assert_ne!(ab_c.0, a_bc.0);
    }

    #[test]
    fn the_same_manifest_digests_the_same() {
        assert_eq!(bare().compute_digest(), bare().compute_digest());
    }

    #[test]
    fn every_input_changes_the_digest() {
        let base = bare().compute_digest();
        let mut gameplay = bare();
        gameplay.gameplay_digest += 1;
        let mut gap = bare();
        gap.program_corpus = ManifestFact::Unavailable(ManifestGap::InvalidMapKey);
        let mut known = bare();
        known.map = ManifestFact::Known(key(AssetKind::Map, "mp_a"));
        let mut weapon = bare();
        weapon.weapons.push(row(1, "gun"));
        for changed in [gameplay, gap, known, weapon] {
            assert_ne!(changed.compute_digest(), base);
        }
    }

    #[test]
    fn two_maps_digest_differently_and_so_do_two_weapons() {
        let mut a = bare();
        a.map = ManifestFact::Known(key(AssetKind::Map, "mp_a"));
        let mut b = bare();
        b.map = ManifestFact::Known(key(AssetKind::Map, "mp_b"));
        assert_ne!(a.compute_digest(), b.compute_digest());
        let mut gun_a = bare();
        gun_a.weapons.push(row(1, "gun_a"));
        let mut gun_b = bare();
        gun_b.weapons.push(row(1, "gun_b"));
        assert_ne!(gun_a.compute_digest(), gun_b.compute_digest());
    }

    #[test]
    fn a_weapons_attachments_and_alternate_flag_count() {
        let mut plain = bare();
        plain.weapons.push(row(1, "gun"));
        let mut attached = bare();
        attached.weapons.push(row(1, "gun"));
        attached.weapons[0].attachments.push("reflex".to_owned());
        let mut alternate = bare();
        alternate.weapons.push(row(1, "gun"));
        alternate.weapons[0].alternate = true;
        let digests = [&plain, &attached, &alternate].map(|m| m.compute_digest());
        assert_ne!(digests[0], digests[1]);
        assert_ne!(digests[0], digests[2]);
        assert_ne!(digests[1], digests[2]);
    }

    #[test]
    fn gap_numbers_are_the_documented_ones() {
        assert_eq!(ManifestGap::MapSourceNotOpened as u8, 1);
        assert_eq!(ManifestGap::ProgramCorpusNotFrozen as u8, 7);
    }

    #[test]
    fn a_map_that_was_never_opened_has_no_descriptor() {
        let manifest = bare();
        assert_eq!(
            manifest.match_descriptor(sim::ContentComponents::default()),
            None
        );
    }

    #[test]
    fn a_prepared_map_without_a_namespace_is_unopened() {
        let map = assets::PreparedMap::default();
        assert_eq!(
            map_fact(&map),
            ManifestFact::Unavailable(ManifestGap::MapSourceNotOpened)
        );
    }

    #[test]
    fn a_prepared_map_with_a_zone_is_known_by_its_key() {
        let map = assets::PreparedMap {
            zone: "mp_a".to_owned(),
            namespace: Some(AssetNamespace::T6),
            ..assets::PreparedMap::default()
        };
        assert_eq!(
            map_fact(&map),
            ManifestFact::Known(key(AssetKind::Map, "mp_a"))
        );
    }

    #[test]
    fn an_empty_registry_builds_a_manifest_with_no_weapons() {
        let manifest = SessionContentManifest::build(
            &assets::PreparedMap::default(),
            &WeaponRegistry::default(),
            &[],
            &[],
            9,
        )
        .unwrap();
        assert!(manifest.weapons.is_empty());
        assert_eq!(manifest.gameplay_digest, 9);
        assert_eq!(manifest.digest, manifest.compute_digest());
    }

    #[test]
    fn a_weapon_with_no_profile_is_unavailable_and_with_one_is_known() {
        assert_eq!(
            authority_of(1, &[], &[]),
            ManifestFact::Unavailable(ManifestGap::ValidatedWeaponProfileMissing)
        );
    }

    #[test]
    fn the_errors_say_which_weapon() {
        assert_eq!(
            SessionManifestError::MissingWeaponKey(SessionWeaponId(4)).to_string(),
            "session weapon 4 has no durable key"
        );
        let text = SessionManifestError::DuplicateWeaponKey(
            key(AssetKind::Weapon, "gun"),
            vec!["a".to_owned(), "b".to_owned()],
            true,
        )
        .to_string();
        assert!(text.contains("[a b]") && text.contains("alternate=true"));
    }
}
