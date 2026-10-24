//! Certificates and keys read from PEM files.

use std::{fs::File, io::BufReader, path::Path};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::Error;

fn open(path: &Path) -> Result<BufReader<File>, Error> {
    Ok(BufReader::new(File::open(path)?))
}

/// Every certificate in the file, in order; a file with none is an error.
pub fn certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, Error> {
    let chain = rustls_pemfile::certs(&mut open(path)?).collect::<Result<Vec<_>, _>>()?;
    if chain.is_empty() {
        Err(format!("{} contains no certificates", path.display()).into())
    } else {
        Ok(chain)
    }
}

/// The first private key in the file.
pub fn private_key(path: &Path) -> Result<PrivateKeyDer<'static>, Error> {
    match rustls_pemfile::private_key(&mut open(path)?)? {
        Some(key) => Ok(key),
        None => Err(format!("{} contains no private key", path.display()).into()),
    }
}
