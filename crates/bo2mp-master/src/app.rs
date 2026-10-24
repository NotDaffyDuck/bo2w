//! `bo2mp-master`: the lobby service (room browser plus packet relay) and the
//! small commands that check on it or install it.

#![forbid(unsafe_code)]

mod args;
mod lobby;
mod pem;
mod probe;
mod service;
mod unit;
mod wire;

use std::io::Write;

use args::Invocation;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    match args::read(std::env::args().skip(1))? {
        Invocation::Serve { bind, cert, key } => service::serve(bind, &cert, &key).await,
        Invocation::Status(target) => probe::status(&target).await,
        Invocation::List(target) => probe::list(&target).await,
        Invocation::PrintUnit(settings) => {
            std::io::stdout().write_all(unit::render(&settings).as_bytes())?;
            Ok(())
        }
    }
}
