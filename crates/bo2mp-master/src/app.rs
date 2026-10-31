//! `bo2mp-master`: the lobby service (room browser plus packet relay) and the
//! small commands that check on it or install it.

#![forbid(unsafe_code)]

use std::io::Write;

use bo2mp_master::{args, args::Invocation, probe, service, unit, Error};

#[tokio::main]
async fn main() -> Result<(), Error> {
    match args::read(std::env::args().skip(1))? {
        Invocation::Serve { address, cert_file, key_file } => service::serve(address, &cert_file, &key_file).await,
        Invocation::Status(target) => probe::status(&target).await,
        Invocation::List(target) => probe::list(&target).await,
        Invocation::PrintUnit(settings) => {
            std::io::stdout().write_all(unit::render(&settings).as_bytes())?;
            Ok(())
        }
    }
}
