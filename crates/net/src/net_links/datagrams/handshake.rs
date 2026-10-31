//! What two peers compare before they play together: the content they were built with, the
//! limits they enforce, and the verdict when the two disagree.

use bevy::prelude::Resource;

use crate::WIRE_REVISION_NUMBER;
use crate::host_tick::peer_mail::HOST_TICK_FREQUENCY;

/// Whether the datagram transport is available in this build.
pub const DATAGRAM_TRANSPORT_READY: bool = true;

/// The most bytes one datagram may carry, before and after decompression.
pub const DATAGRAM_BYTE_CAP: u32 = 256 * 1024;

/// A compressed snapshot may unpack past one packet: a full Zombies map
/// (Die Rise's first snapshot is over 256 KB, 12 KB packed).
pub const MAX_SNAPSHOT_DECODED_BYTES: u32 = 16 * 1024 * 1024;

/// Digests of everything that has to match for two peers to simulate the same match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetDigestSet {
    pub rules_digest: u64,
    pub map: u64,
    pub model_digest: u64,
    pub weapon_digest: u64,
    pub class_digest: u64,
}

impl AssetDigestSet {
    pub fn from_world(sim_world: &sim::SimWorld) -> Self {
        let sim::ContentPartDigests { map, meshes: models, guns: weapons, kits: classes, .. } = sim_world.content_part_fingerprints();
        Self { rules_digest: sim_world.content_fingerprint(), map, model_digest: models, weapon_digest: weapons, class_digest: classes }
    }

    pub fn session_summary(self) -> Option<GameSessionSummary> {
        GameSessionSummary::from_fingerprint(self)
    }
}

/// The part of the content a lobby needs to name a match. A zero map digest means no map is
/// loaded, so there is nothing to describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Resource)]
pub struct GameSessionSummary {
    pub map: u64,
    pub weapons: u64,
    pub classes: u64,
}

impl GameSessionSummary {
    fn named(level: u64, arms: u64, kits: u64) -> Option<Self> {
        (level != 0).then_some(Self {
            map: level,
            weapons: arms,
            classes: kits,
        })
    }

    pub fn from_components(parts: sim::ContentPartDigests) -> Option<Self> {
        Self::named(parts.map, parts.guns, parts.kits)
    }

    pub fn from_fingerprint(content: AssetDigestSet) -> Option<Self> {
        Self::named(content.map, content.weapon_digest, content.class_digest)
    }

    pub fn from_world(sim_world: &sim::SimWorld) -> Option<Self> {
        Self::from_components(sim_world.content_part_fingerprints())
    }
}

/// Bounds both peers agree to enforce on every packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NegotiatedCaps {
    pub host_step_rate: u32,
    pub moves_budget_per_step: u16,
    pub action_budget_per_step: u16,
    pub datagram_size_ceiling: u32,
}

impl NegotiatedCaps {
    pub const fn fallback_bind_address() -> Self {
        Self {
            host_step_rate: HOST_TICK_FREQUENCY as u32,
            moves_budget_per_step: crate::MOVE_RESEND_WINDOW as u16,
            action_budget_per_step: 32,
            datagram_size_ceiling: DATAGRAM_BYTE_CAP,
        }
    }
}

impl Default for NegotiatedCaps {
    fn default() -> Self {
        Self::fallback_bind_address()
    }
}

/// What a peer announces about itself when it connects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinGreeting {
    pub wire_version: u32,
    pub content: AssetDigestSet,
    pub limits: NegotiatedCaps,
}

impl JoinGreeting {
    pub fn current(content: AssetDigestSet) -> Self {
        Self {
            wire_version: WIRE_REVISION_NUMBER,
            content,
            limits: NegotiatedCaps::fallback_bind_address(),
        }
    }

    pub fn from_world(sim_world: &sim::SimWorld) -> Self {
        Self::current(AssetDigestSet::from_world(sim_world))
    }
}

/// Why the authority turned a client away; the first mismatch found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinRefusal {
    VersionClash { host_side: u32, peer_side: u32 },
    LevelAssetHashes { host_side: u64, peer_side: u64 },
    WeaponLedger { host_side: u64, peer_side: u64 },
    LoadoutTableHashes { host_side: u64, peer_side: u64 },
    StepFrequency { host_side: u32, peer_side: u32 },
    Limits,
}

impl core::fmt::Display for JoinRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Content digests print as 16 hex digits, counts as plain numbers.
        let digests = |f: &mut core::fmt::Formatter<'_>, what: &str, host_side: u64, peer_side: u64| {
            write!(f, "{what} ours={host_side:016x} theirs={peer_side:016x}")
        };
        let counts = |f: &mut core::fmt::Formatter<'_>, what: &str, host_side: u32, peer_side: u32| {
            write!(f, "{what} ours={host_side} theirs={peer_side}")
        };
        match *self {
            Self::Limits => f.write_str("protocol limits mismatch"),
            Self::VersionClash { host_side, peer_side } => counts(f, "protocol version", host_side, peer_side),
            Self::StepFrequency { host_side, peer_side } => counts(f, "tick rate", host_side, peer_side),
            Self::LevelAssetHashes { host_side, peer_side } => digests(f, "map content mismatch (spawns/clip)", host_side, peer_side),
            Self::WeaponLedger { host_side, peer_side } => digests(f, "weapon table mismatch (installed catalogs differ)", host_side, peer_side),
            Self::LoadoutTableHashes { host_side, peer_side } => digests(f, "class catalog mismatch", host_side, peer_side),
        }
    }
}

/// Compares the authority's hello with a client's. The checks run in a fixed order, so a client
/// that is wrong in several ways is told about the first.
pub fn judge_greeting(
    server: &JoinGreeting,
    client: &JoinGreeting,
) -> Result<(), JoinRefusal> {
    let (host_side, peer_side) = (server, client);

    if host_side.wire_version != peer_side.wire_version {
        return Err(JoinRefusal::VersionClash {
            host_side: host_side.wire_version,
            peer_side: peer_side.wire_version,
        });
    }

    // A match without a map can never agree with anyone, itself included.
    let no_map = host_side.content.map == 0 || peer_side.content.map == 0;
    if no_map || host_side.content.map != peer_side.content.map {
        return Err(JoinRefusal::LevelAssetHashes {
            host_side: host_side.content.map,
            peer_side: peer_side.content.map,
        });
    }
    if host_side.content.weapon_digest != peer_side.content.weapon_digest {
        return Err(JoinRefusal::WeaponLedger {
            host_side: host_side.content.weapon_digest,
            peer_side: peer_side.content.weapon_digest,
        });
    }
    if host_side.content.class_digest != peer_side.content.class_digest {
        return Err(JoinRefusal::LoadoutTableHashes {
            host_side: host_side.content.class_digest,
            peer_side: peer_side.content.class_digest,
        });
    }
    if host_side.limits.host_step_rate != peer_side.limits.host_step_rate {
        return Err(JoinRefusal::StepFrequency {
            host_side: host_side.limits.host_step_rate,
            peer_side: peer_side.limits.host_step_rate,
        });
    }
    if host_side.limits != peer_side.limits {
        return Err(JoinRefusal::Limits);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_map(map: u64) -> AssetDigestSet {
        AssetDigestSet {
            map,
            weapon_digest: 5,
            class_digest: 6,
            ..AssetDigestSet::default()
        }
    }

    fn hello(map: u64) -> JoinGreeting {
        JoinGreeting::current(with_map(map))
    }

    #[test]
    fn identical_hellos_agree() {
        assert_eq!(judge_greeting(&hello(1), &hello(1)), Ok(()));
    }

    #[test]
    fn another_protocol_version_is_refused_first() {
        let mut client = hello(2);
        client.wire_version = WIRE_REVISION_NUMBER - 1;
        assert_eq!(
            judge_greeting(&hello(1), &client),
            Err(JoinRefusal::VersionClash {
                host_side: WIRE_REVISION_NUMBER,
                peer_side: WIRE_REVISION_NUMBER - 1,
            })
        );
    }

    #[test]
    fn a_different_or_missing_map_is_refused() {
        assert_eq!(
            judge_greeting(&hello(1), &hello(2)),
            Err(JoinRefusal::LevelAssetHashes { host_side: 1, peer_side: 2 })
        );
        assert_eq!(
            judge_greeting(&hello(0), &hello(0)),
            Err(JoinRefusal::LevelAssetHashes { host_side: 0, peer_side: 0 })
        );
        assert_eq!(
            judge_greeting(&hello(1), &hello(0)),
            Err(JoinRefusal::LevelAssetHashes { host_side: 1, peer_side: 0 })
        );
    }

    #[test]
    fn weapon_and_class_tables_must_match_in_that_order() {
        let mut client = hello(1);
        client.content.weapon_digest = 9;
        client.content.class_digest = 9;
        assert_eq!(
            judge_greeting(&hello(1), &client),
            Err(JoinRefusal::WeaponLedger { host_side: 5, peer_side: 9 })
        );
        client.content.weapon_digest = 5;
        assert_eq!(
            judge_greeting(&hello(1), &client),
            Err(JoinRefusal::LoadoutTableHashes { host_side: 6, peer_side: 9 })
        );
    }

    #[test]
    fn tick_rate_is_named_before_the_other_limits() {
        let mut client = hello(1);
        client.limits.host_step_rate += 1;
        client.limits.action_budget_per_step += 1;
        assert_eq!(
            judge_greeting(&hello(1), &client),
            Err(JoinRefusal::StepFrequency {
                host_side: hello(1).limits.host_step_rate,
                peer_side: hello(1).limits.host_step_rate + 1,
            })
        );
        client.limits.host_step_rate = hello(1).limits.host_step_rate;
        assert_eq!(
            judge_greeting(&hello(1), &client),
            Err(JoinRefusal::Limits)
        );
    }

    #[test]
    fn a_match_descriptor_needs_a_map() {
        assert_eq!(with_map(0).session_summary(), None);
        assert_eq!(
            with_map(3).session_summary(),
            Some(GameSessionSummary {
                map: 3,
                weapons: 5,
                classes: 6
            })
        );
    }

    #[test]
    fn reasons_read_the_way_the_logs_expect() {
        assert_eq!(
            JoinRefusal::VersionClash { host_side: 3, peer_side: 2 }.to_string(),
            "protocol version ours=3 theirs=2"
        );
        assert_eq!(
            JoinRefusal::LevelAssetHashes { host_side: 255, peer_side: 1 }.to_string(),
            "map content mismatch (spawns/clip) ours=00000000000000ff theirs=0000000000000001"
        );
        assert_eq!(JoinRefusal::Limits.to_string(), "protocol limits mismatch");
    }
}
