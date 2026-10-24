//! What two peers compare before they play together: the content they were built with, the
//! limits they enforce, and the verdict when the two disagree.

use bevy::prelude::Resource;

use crate::PROTOCOL_VERSION;
use crate::authority::inbox::AUTHORITY_HZ;

/// Whether the datagram transport is available in this build.
pub const UDP_IMPLEMENTED: bool = true;

/// The most bytes one datagram may carry, before and after decompression.
pub const MAX_PACKET_BYTES: u32 = 256 * 1024;

/// Digests of everything that has to match for two peers to simulate the same match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentFingerprint {
    pub gameplay: u64,
    pub map: u64,
    pub models: u64,
    pub weapons: u64,
    pub classes: u64,
}

impl ContentFingerprint {
    pub fn from_world(world: &sim::SimWorld) -> Self {
        let parts = world.content_components();
        Self {
            gameplay: world.content_digest(),
            map: parts.map,
            models: parts.models,
            weapons: parts.weapons,
            classes: parts.classes,
        }
    }

    pub fn match_descriptor(self) -> Option<MatchDescriptor> {
        MatchDescriptor::from_fingerprint(self)
    }
}

/// The part of the content a lobby needs to name a match. A zero map digest means no map is
/// loaded, so there is nothing to describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Resource)]
pub struct MatchDescriptor {
    pub map: u64,
    pub weapons: u64,
    pub classes: u64,
}

impl MatchDescriptor {
    fn named(map: u64, weapons: u64, classes: u64) -> Option<Self> {
        (map != 0).then_some(Self {
            map,
            weapons,
            classes,
        })
    }

    pub fn from_components(parts: sim::ContentComponents) -> Option<Self> {
        Self::named(parts.map, parts.weapons, parts.classes)
    }

    pub fn from_fingerprint(content: ContentFingerprint) -> Option<Self> {
        Self::named(content.map, content.weapons, content.classes)
    }

    pub fn from_world(world: &sim::SimWorld) -> Option<Self> {
        Self::from_components(world.content_components())
    }
}

/// Bounds both peers agree to enforce on every packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolLimits {
    pub authority_hz: u32,
    pub max_cmds_per_tick: u16,
    pub max_actions_per_tick: u16,
    pub max_packet_bytes: u32,
}

impl ProtocolLimits {
    pub const fn default_listen() -> Self {
        Self {
            authority_hz: AUTHORITY_HZ as u32,
            max_cmds_per_tick: crate::MAX_REDUNDANT_CMDS as u16,
            max_actions_per_tick: 32,
            max_packet_bytes: MAX_PACKET_BYTES,
        }
    }
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self::default_listen()
    }
}

/// What a peer announces about itself when it connects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandshakeHello {
    pub protocol_version: u32,
    pub content: ContentFingerprint,
    pub limits: ProtocolLimits,
}

impl HandshakeHello {
    pub fn current(content: ContentFingerprint) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            content,
            limits: ProtocolLimits::default_listen(),
        }
    }

    pub fn from_world(world: &sim::SimWorld) -> Self {
        Self::current(ContentFingerprint::from_world(world))
    }
}

/// Why the authority turned a client away; the first mismatch found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandshakeReject {
    ProtocolVersion { ours: u32, theirs: u32 },
    MapContent { ours: u64, theirs: u64 },
    WeaponRegistry { ours: u64, theirs: u64 },
    ClassRegistry { ours: u64, theirs: u64 },
    TickRate { ours: u32, theirs: u32 },
    Limits,
}

impl core::fmt::Display for HandshakeReject {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::ProtocolVersion { ours, theirs } => {
                write!(f, "protocol version ours={ours} theirs={theirs}")
            }
            Self::MapContent { ours, theirs } => write!(
                f,
                "map content mismatch (spawns/clip) ours={ours:016x} theirs={theirs:016x}"
            ),
            Self::WeaponRegistry { ours, theirs } => write!(
                f,
                "weapon table mismatch (installed catalogs differ) ours={ours:016x} theirs={theirs:016x}"
            ),
            Self::ClassRegistry { ours, theirs } => write!(
                f,
                "class catalog mismatch ours={ours:016x} theirs={theirs:016x}"
            ),
            Self::TickRate { ours, theirs } => write!(f, "tick rate ours={ours} theirs={theirs}"),
            Self::Limits => write!(f, "protocol limits mismatch"),
        }
    }
}

/// Compares the authority's hello with a client's. The checks run in a fixed order, so a client
/// that is wrong in several ways is told about the first.
pub fn evaluate_handshake(
    server: &HandshakeHello,
    client: &HandshakeHello,
) -> Result<(), HandshakeReject> {
    let (ours, theirs) = (server, client);

    if ours.protocol_version != theirs.protocol_version {
        return Err(HandshakeReject::ProtocolVersion {
            ours: ours.protocol_version,
            theirs: theirs.protocol_version,
        });
    }

    // A match without a map can never agree with anyone, itself included.
    let no_map = ours.content.map == 0 || theirs.content.map == 0;
    if no_map || ours.content.map != theirs.content.map {
        return Err(HandshakeReject::MapContent {
            ours: ours.content.map,
            theirs: theirs.content.map,
        });
    }
    if ours.content.weapons != theirs.content.weapons {
        return Err(HandshakeReject::WeaponRegistry {
            ours: ours.content.weapons,
            theirs: theirs.content.weapons,
        });
    }
    if ours.content.classes != theirs.content.classes {
        return Err(HandshakeReject::ClassRegistry {
            ours: ours.content.classes,
            theirs: theirs.content.classes,
        });
    }
    if ours.limits.authority_hz != theirs.limits.authority_hz {
        return Err(HandshakeReject::TickRate {
            ours: ours.limits.authority_hz,
            theirs: theirs.limits.authority_hz,
        });
    }
    if ours.limits != theirs.limits {
        return Err(HandshakeReject::Limits);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_map(map: u64) -> ContentFingerprint {
        ContentFingerprint {
            map,
            weapons: 5,
            classes: 6,
            ..ContentFingerprint::default()
        }
    }

    fn hello(map: u64) -> HandshakeHello {
        HandshakeHello::current(with_map(map))
    }

    #[test]
    fn identical_hellos_agree() {
        assert_eq!(evaluate_handshake(&hello(1), &hello(1)), Ok(()));
    }

    #[test]
    fn another_protocol_version_is_refused_first() {
        let mut client = hello(2);
        client.protocol_version = PROTOCOL_VERSION - 1;
        assert_eq!(
            evaluate_handshake(&hello(1), &client),
            Err(HandshakeReject::ProtocolVersion {
                ours: PROTOCOL_VERSION,
                theirs: PROTOCOL_VERSION - 1,
            })
        );
    }

    #[test]
    fn a_different_or_missing_map_is_refused() {
        assert_eq!(
            evaluate_handshake(&hello(1), &hello(2)),
            Err(HandshakeReject::MapContent { ours: 1, theirs: 2 })
        );
        assert_eq!(
            evaluate_handshake(&hello(0), &hello(0)),
            Err(HandshakeReject::MapContent { ours: 0, theirs: 0 })
        );
        assert_eq!(
            evaluate_handshake(&hello(1), &hello(0)),
            Err(HandshakeReject::MapContent { ours: 1, theirs: 0 })
        );
    }

    #[test]
    fn weapon_and_class_tables_must_match_in_that_order() {
        let mut client = hello(1);
        client.content.weapons = 9;
        client.content.classes = 9;
        assert_eq!(
            evaluate_handshake(&hello(1), &client),
            Err(HandshakeReject::WeaponRegistry { ours: 5, theirs: 9 })
        );
        client.content.weapons = 5;
        assert_eq!(
            evaluate_handshake(&hello(1), &client),
            Err(HandshakeReject::ClassRegistry { ours: 6, theirs: 9 })
        );
    }

    #[test]
    fn tick_rate_is_named_before_the_other_limits() {
        let mut client = hello(1);
        client.limits.authority_hz += 1;
        client.limits.max_actions_per_tick += 1;
        assert_eq!(
            evaluate_handshake(&hello(1), &client),
            Err(HandshakeReject::TickRate {
                ours: hello(1).limits.authority_hz,
                theirs: hello(1).limits.authority_hz + 1,
            })
        );
        client.limits.authority_hz = hello(1).limits.authority_hz;
        assert_eq!(
            evaluate_handshake(&hello(1), &client),
            Err(HandshakeReject::Limits)
        );
    }

    #[test]
    fn a_match_descriptor_needs_a_map() {
        assert_eq!(with_map(0).match_descriptor(), None);
        assert_eq!(
            with_map(3).match_descriptor(),
            Some(MatchDescriptor {
                map: 3,
                weapons: 5,
                classes: 6
            })
        );
    }

    #[test]
    fn reasons_read_the_way_the_logs_expect() {
        assert_eq!(
            HandshakeReject::ProtocolVersion { ours: 3, theirs: 2 }.to_string(),
            "protocol version ours=3 theirs=2"
        );
        assert_eq!(
            HandshakeReject::MapContent { ours: 255, theirs: 1 }.to_string(),
            "map content mismatch (spawns/clip) ours=00000000000000ff theirs=0000000000000001"
        );
        assert_eq!(HandshakeReject::Limits.to_string(), "protocol limits mismatch");
    }
}
