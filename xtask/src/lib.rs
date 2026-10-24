//! Ship tools behind `cargo xtask`: Windows builds, release bundles, the
//! master's certificates, and publishing to the server.

use std::path::{Path, PathBuf};

pub mod bundle_zip;
pub mod certs;
pub mod dotenv;
pub mod master;
pub mod provision;
pub mod publish;
pub mod release;
pub mod shell;
pub mod windows;

/// The workspace root, which is where `.env` and `Cargo.toml` live.
pub fn repo_root() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    match crate_dir.parent() {
        Some(workspace) => workspace.to_path_buf(),
        None => crate_dir.to_path_buf(),
    }
}
