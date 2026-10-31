//! The lobby service inside a game that hosts on a LAN: no separate master,
//! the hosting PC opens the port itself and the other PCs dial it.
//!
//! Trust works as with a master of our own: the host makes a certificate for
//! the fixed name [`SERVER_NAME`] once and keeps it beside the exe, and each
//! joining PC is given a copy of that `.pem` (`BO2MP_MASTER_CA_CERT`).

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::{service, Error};

/// The name the LAN certificate carries; joiners set `BO2MP_MASTER_SERVER_NAME` to it.
pub const SERVER_NAME: &str = "bo2mp-lan";
/// The UDP port a LAN host opens unless told otherwise (the master's prod port).
pub const PORT: u16 = 4433;
/// The certificate file a joining PC needs a copy of.
pub const CERT_FILE: &str = "bo2mp-lan.pem";
/// The private key, which stays on the host.
pub const KEY_FILE: &str = "bo2mp-lan.key";

/// The LAN certificate and its key, as files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// The identity kept in `dir`, made there first if it is missing, so the copy
/// a joiner was given keeps working from one match to the next.
pub fn identity(dir: &Path) -> Result<Identity, Error> {
    let kept = Identity { cert: dir.join(CERT_FILE), key: dir.join(KEY_FILE) };
    if kept.cert.is_file() && kept.key.is_file() {
        return Ok(kept);
    }
    fs::create_dir_all(dir)?;
    let made = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_owned()])?;
    // Key first: a certificate on disk without its key would be taken as complete.
    fs::write(&kept.key, made.key_pair.serialize_pem())?;
    fs::write(&kept.cert, made.cert.pem())?;
    Ok(kept)
}

/// Starts the service on its own thread, listening on `bind` with the identity
/// in `dir`. Returns the address it listens on once the port is open.
pub fn start(bind: SocketAddr, dir: &Path) -> Result<SocketAddr, Error> {
    let Identity { cert, key } = identity(dir)?;
    let (opened, port) = mpsc::channel::<Result<SocketAddr, String>>();
    std::thread::Builder::new().name("bo2mp-lan-relay".into()).spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => return drop(opened.send(Err(error.to_string()))),
        };
        runtime.block_on(async move {
            let endpoint = match service::listen(bind, &cert, &key).and_then(|ep| Ok((ep.local_addr()?, ep))) {
                Ok((address, endpoint)) => {
                    opened.send(Ok(address)).ok();
                    endpoint
                }
                Err(error) => return drop(opened.send(Err(error.to_string()))),
            };
            if let Err(error) = service::run(endpoint).await {
                eprintln!("bo2mp LAN relay stopped: {error}");
            }
        });
    })?;
    Ok(port.recv().map_err(|_| "the LAN relay thread ended before opening its port")??)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bo2mp-lan-{tag}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn the_identity_is_made_once_and_kept() {
        let dir = scratch("keep");
        let first = identity(&dir).unwrap();
        let pem = fs::read(&first.cert).unwrap();
        assert!(pem.starts_with(b"-----BEGIN CERTIFICATE-----"));
        assert_eq!(identity(&dir).unwrap(), first);
        assert_eq!(fs::read(&first.cert).unwrap(), pem, "a second call must not make a new certificate");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_taken_port_is_reported() {
        let dir = scratch("taken");
        let held = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let error = start(held.local_addr().unwrap(), &dir).unwrap_err();
        assert!(!error.to_string().is_empty());
        fs::remove_dir_all(&dir).ok();
    }
}
