//! Version numbers, size caps and the two deployment channels of the lobby service.

use core::fmt::{self, Display};
use core::str;
use core::time::Duration;
use core::str::FromStr;

/// Bumped whenever any byte layout in this crate changes; both ends must match.
pub const WIRE_REV: u16 = 10;
/// QUIC application protocol id. It names the same version as [`WIRE_REVISION_NUMBER`].
pub const ALPN: &[u8] = b"bo2mp-master/10";

/// Largest control message, header included.
pub const CONTROL_MESSAGE_SIZE_LIMIT: usize = 16 * 1024;
/// Largest game packet carried in one relay datagram.
pub const OPAQUE_BODY_BYTE_CAP: usize = 1100;
/// Largest join snapshot carried on one relay stream.
pub const TRANSFER_SIZE_LIMIT: usize = 256 * 1024;
/// A relay stream holds the version byte, the route byte, a member id and the snapshot.
pub const TRANSFER_STREAM_CAP: usize = 2 + 16 + TRANSFER_SIZE_LIMIT;
/// One-way QUIC streams a relay connection may hold open at once.
pub const FORWARD_ONEWAY_CHANNEL_CAP: u32 = 8;
/// Join snapshots the service forwards at the same time.
pub const PARALLEL_TRANSFER_LIMIT: u32 = 4;

/// Players in one room, host included.
pub const ROOM_OCCUPANT_LIMIT: u8 = 18;
/// Text caps, in UTF-8 bytes.
pub const LISTING_TITLE_BYTE_CAP: usize = 48;
pub const LEVEL_NAME_BYTE_CAP: usize = 64;
/// Zombies modes carry the start and the lobby kind
/// (`zstandard:processing:private` is 28).
pub const RULESET_NAME_BYTE_CAP: usize = 48;
pub const VERSION_TEXT_BYTE_CAP: usize = 64;
pub const MAX_PLAYER_NAME_BYTES: usize = 64;
/// Rooms returned by one list request.
pub const LISTING_PAGE_LIMIT: usize = 128;

/// A QUIC connection with no traffic for this long is dropped.
pub const LINK_IDLE_LIMIT: Duration = Duration::from_secs(8);
/// Keep-alive interval, well inside [`SESSION_IDLE`].
pub const LINK_PING_EVERY: Duration = Duration::from_secs(1);

/// Which copy of the service a client talks to: the public one or the test one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Prod,
    PreReleaseBuild,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Prod, Channel::PreReleaseBuild];

    /// Everything that differs between the two copies.
    const fn facts(self) -> CopyFacts {
        match self {
            Channel::Prod => CopyFacts {
                tier: "prod",
                udp_port: 4433,
                systemd: "bo2mp-master.service",
                tls_name: "bo2mp-prod",
            },
            Channel::PreReleaseBuild => CopyFacts {
                tier: "dev",
                udp_port: 4434,
                systemd: "bo2mp-master-dev.service",
                tls_name: "bo2mp-dev",
            },
        }
    }

    /// Name used on command lines and in config files.
    pub const fn as_str(self) -> &'static str {
        self.facts().tier
    }

    /// UDP port the service listens on.
    pub const fn port(self) -> u16 {
        self.facts().udp_port
    }

    /// systemd unit that runs this copy on the server.
    pub const fn systemd_unit(self) -> &'static str {
        self.facts().systemd
    }

    /// TLS name in the service certificate.
    pub const fn tls_name(self) -> &'static str {
        self.facts().tls_name
    }
}

impl Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::as_str(*self))
    }
}

impl FromStr for Channel {
    type Err = UnrecognizedLane;

    fn from_str(text: &str) -> Result<Self, UnrecognizedLane> {
        Self::ALL
            .into_iter()
            .find(|channel| channel.as_str() == text)
            .ok_or(UnrecognizedLane)
    }
}

/// The name, port, service unit and TLS name of one copy of the service.
struct CopyFacts {
    tier: &'static str,
    udp_port: u16,
    systemd: &'static str,
    tls_name: &'static str,
}

/// A channel name that is neither `prod` nor `dev`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnrecognizedLane;

impl Display for UnrecognizedLane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("channel must be either prod or dev")
    }
}

impl std::error::Error for UnrecognizedLane {}
