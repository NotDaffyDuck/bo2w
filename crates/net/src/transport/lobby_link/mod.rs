//! The link to the lobby service: the server browser, hosting or joining a
//! room, and relaying a match's packets through the service when the players
//! cannot reach each other directly.

mod bridge;
mod browser;
mod content;
mod fault;
mod intent;
mod legacy_content;
mod pipes;
mod quic;
mod room;
mod state;
mod systems;
mod worker;

pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub use bridge::MasterBridge;
pub use browser::MasterBrowser;
pub use content::{ContentCatalog, ContentPack};
pub use fault::TransportFault;
pub use intent::MasterLaunchIntent;
pub use legacy_content::{CONTENT_T5, CONTENT_T6, content_inventory, content_required_by_map};
pub use state::{
    MasterAdvert, MasterBridgeState, MasterBrowserSnapshot, MasterLifecycleFact, MasterMatchOffer,
    MasterMatchStart, MasterMenuAction, PendingMasterMenuAction, SessionIdentity,
};
pub(crate) use systems::register;
