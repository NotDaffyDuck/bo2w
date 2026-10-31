//! The link to the lobby service: the server browser, hosting or joining a
//! room, and relaying a match's packets through the service when the players
//! cannot reach each other directly.

mod bridge;
mod browser;
mod content_bits;
mod fault;
mod intent;
mod pipes;
mod quic;
mod room;
mod room_view;
mod systems;
mod worker;

pub use bridge::DirectoryLink;
pub use browser::DirectoryBrowser;
pub use content_bits::{ContentCatalog, ContentPack};
pub use fault::LinkFailureRecord;
pub use intent::DirectoryLaunchRequest;
pub use room_view::{
    DirectoryListing, DirectoryLinkStatus, DirectoryBrowserView, DirectoryLifecycleNote, DirectoryGameOffer,
    DirectoryMatchBegin, DirectoryMenuChoice, QueuedMenuChoice, LobbySessionLabel,
};
pub(crate) use systems::register;
