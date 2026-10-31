//! Ship tools behind `cargo xtask`: Windows builds, release bundles, the
//! master's certificates, and publishing to the server.

use std::path::{Path, PathBuf};

pub mod certs;
pub mod cmd_run;
pub mod dot_vars;
pub mod host_setup;
pub mod lobby_host;
pub mod publish;
pub mod release;
pub mod windows;
pub mod zip_pack;

/// The workspace root, which is where `.env` and `Cargo.toml` live.
pub fn workspace_top_dir() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    match crate_dir.parent() {
        Some(workspace) => workspace.to_path_buf(),
        None => crate_dir.to_path_buf(),
    }
}
