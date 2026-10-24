//! Bakes the update channel's CA certificate into the binary, so the updater
//! trusts that one authority and nothing else.

use std::env;
use std::fs;
use std::path::PathBuf;

const CA_ENV: &str = "BO2MP_UPDATER_CA_CERT";

/// Written when no certificate was given. It is not PEM, so the updater finds
/// no certificate in it and says how to rebuild.
const NO_CA_MARKER: &str =
    "BO2MP updater built without BO2MP_UPDATER_CA_CERT; use `make launcher windows`\n";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed={CA_ENV}");
    let out_dir = env::var_os("OUT_DIR").expect("cargo sets OUT_DIR for build scripts");
    let embedded = PathBuf::from(out_dir).join("bo2mp-ca.pem");
    match env::var_os(CA_ENV) {
        Some(source) => {
            let source = PathBuf::from(source);
            println!("cargo:rerun-if-changed={}", source.display());
            if let Err(error) = fs::copy(&source, &embedded) {
                panic!(
                    "cannot copy the updater CA from {}: {error}",
                    source.display()
                );
            }
        }
        None => fs::write(&embedded, NO_CA_MARKER).expect("cannot write the missing-CA marker"),
    }
}
