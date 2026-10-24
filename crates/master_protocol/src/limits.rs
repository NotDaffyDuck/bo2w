//! Version numbers, size caps and the two deployment channels of the lobby service.

use core::{fmt, str::FromStr, time::Duration};

/// Bumped whenever any byte layout in this crate changes; both ends must match.
pub const PROTOCOL_VERSION: u16 = 9;
/// QUIC application protocol id. It names the same version as [`PROTOCOL_VERSION`].
pub const ALPN: &[u8] = b"bo2mp-master/9";

/// Largest control message, header included.
pub const MAX_CONTROL_BYTES: usize = 16 * 1024;
/// Largest game packet carried in one relay datagram.
pub const MAX_OPAQUE_PAYLOAD: usize = 1100;
/// Largest join snapshot carried on one relay stream.
pub const MAX_BOOTSTRAP_PAYLOAD: usize = 256 * 1024;
/// A relay stream holds the version byte, the route byte, a member id and the snapshot.
pub const MAX_BOOTSTRAP_STREAM_BYTES: usize = 2 + 16 + MAX_BOOTSTRAP_PAYLOAD;
/// One-way QUIC streams a relay connection may hold open at once.
pub const MAX_RELAY_UNI_STREAMS: u32 = 8;
/// Join snapshots the service forwards at the same time.
pub const MAX_CONCURRENT_BOOTSTRAP: u32 = 4;

/// Players in one room, host included.
pub const MAX_SESSION_MEMBERS: u8 = 18;
/// Text caps, in UTF-8 bytes.
pub const MAX_ADVERT_NAME_BYTES: usize = 48;
pub const MAX_MAP_BYTES: usize = 64;
pub const MAX_MODE_BYTES: usize = 24;
pub const MAX_BUILD_BYTES: usize = 64;
pub const MAX_PLAYER_NAME_BYTES: usize = 64;
/// Rooms returned by one list request.
pub const MAX_LIST_ADVERTS: usize = 128;

/// A QUIC connection with no traffic for this long is dropped.
pub const SESSION_IDLE: Duration = Duration::from_secs(8);
/// Keep-alive interval, well inside [`SESSION_IDLE`].
pub const SESSION_KEEP_ALIVE: Duration = Duration::from_secs(1);

/// Which copy of the service a client talks to: the public one or the test one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Prod,
    Dev,
}

impl Channel {
    pub const ALL: [Self; 2] = [Self::Prod, Self::Dev];

    /// Name used on command lines and in config files.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prod => "prod",
            Self::Dev => "dev",
        }
    }

    /// UDP port the service listens on.
    pub const fn port(self) -> u16 {
        match self {
            Self::Prod => 4433,
            Self::Dev => 4434,
        }
    }

    /// systemd unit that runs this copy on the server.
    pub const fn unit(self) -> &'static str {
        match self {
            Self::Prod => "bo2mp-master.service",
            Self::Dev => "bo2mp-master-dev.service",
        }
    }

    /// TLS name in the service certificate.
    pub const fn server_name(self) -> &'static str {
        match self {
            Self::Prod => "bo2mp-prod",
            Self::Dev => "bo2mp-dev",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Channel {
    type Err = UnknownChannel;

    fn from_str(text: &str) -> Result<Self, UnknownChannel> {
        Self::ALL
            .into_iter()
            .find(|channel| channel.as_str() == text)
            .ok_or(UnknownChannel)
    }
}

/// A channel name that is neither `prod` nor `dev`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownChannel;

impl fmt::Display for UnknownChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("channel must be prod or dev")
    }
}

impl std::error::Error for UnknownChannel {}
