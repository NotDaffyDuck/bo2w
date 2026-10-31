//! Certificates and keys read from PEM files.

use std::fs;
use std::io;
use std::path::Path;

use rustls::pki_types;

use crate::Error;

fn open(path: &Path) -> Result<io::BufReader<fs::File>, Error> {
    Ok(io::BufReader::new(fs::File::open(path)?))
}

/// Every certificate in the file, in order; a file with none is an error.
pub fn certificates(path: &Path) -> Result<Vec<pki_types::CertificateDer<'static>>, Error> {
    let chain = rustls_pemfile::certs(&mut open(path)?).collect::<Result<Vec<_>, _>>()?;
    if chain.is_empty() {
        Err(format!("{} contains no certificates", path.to_string_lossy()).into())
    } else {
        Ok(chain)
    }
}

/// The first private key in the file.
pub fn private_key(path: &Path) -> Result<pki_types::PrivateKeyDer<'static>, Error> {
    match rustls_pemfile::private_key(&mut open(path)?)? {
        Some(key) => Ok(key),
        None => Err(format!("{} contains no private key", path.to_string_lossy()).into()),
    }
}
