//! The command line: a command word, then `--flag value` pairs in any order.

use std::net;
use std::path::PathBuf;

use crate::Error;

const HELP_BANNER: &str = "usage: bo2mp-master serve|status|list|print-unit ...";

/// A running service to ask, for `status` and `list`.
pub struct Target {
    pub address: net::SocketAddr,
    /// The name its certificate must carry.
    pub server_name: String,
    /// Trust this CA file instead of the system's trust store.
    pub ca_file: Option<PathBuf>,
}

/// What goes into the systemd unit `print-unit` writes.
pub struct UnitSettings {
    pub channel: master_protocol::Channel,
    pub exec: PathBuf,
    pub cert: PathBuf,
    pub key: PathBuf,
    pub user: String,
    pub group: String,
}

pub enum Invocation {
    Serve {
        address: net::SocketAddr,
        cert_file: PathBuf,
        key_file: PathBuf,
    },
    Status(Target),
    List(Target),
    PrintUnit(UnitSettings),
}

/// Every flag seen so far; which ones a command needs is decided afterwards.
#[derive(Default)]
struct Flags {
    bind: Option<net::SocketAddr>,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    connect: Option<net::SocketAddr>,
    server_name: Option<String>,
    trust_anchor_cert: Option<PathBuf>,
    channel: Option<master_protocol::Channel>,
    exec: Option<PathBuf>,
    user: Option<String>,
    group: Option<String>,
}

impl Flags {
    fn record(&mut self, flag: &str, value: String) -> Result<(), Error> {
        match flag {
            "--bind" => self.bind = Some(value.parse()?),
            "--connect" => self.connect = Some(value.parse()?),
            "--channel" => self.channel = Some(value.parse()?),
            "--cert" => self.cert = Some(value.into()),
            "--key" => self.key = Some(value.into()),
            "--ca-cert" => self.trust_anchor_cert = Some(value.into()),
            "--exec" => self.exec = Some(value.into()),
            "--server-name" => self.server_name = Some(value),
            "--user" => self.user = Some(value),
            "--group" => self.group = Some(value),
            unknown => return Err(format!("unknown option {unknown}").into()),
        }
        Ok(())
    }

    fn target(self) -> Result<Target, Error> {
        Ok(Target {
            address: required(self.connect, "command requires --connect HOST:PORT")?,
            server_name: required(self.server_name, "command requires --server-name NAME")?,
            ca_file: self.trust_anchor_cert,
        })
    }
}

fn required<T>(value: Option<T>, complaint: &'static str) -> Result<T, Error> {
    value.ok_or_else(|| complaint.into())
}

/// Reads the arguments after the program name. All flags are read (and checked)
/// before the command decides which of them it needs.
pub fn read(mut words: impl Iterator<Item = String>) -> Result<Invocation, Error> {
    let command = required(words.next(), HELP_BANNER)?;
    let mut flags = Flags::default();
    while let Some(flag) = words.next() {
        let Some(value) = words.next() else {
            return Err(format!("missing value for {flag}").into());
        };
        flags.record(&flag, value)?;
    }
    let invocation = match command.as_str() {
        "serve" => Invocation::Serve {
            address: required(flags.bind, "serve requires --bind HOST:PORT")?,
            cert_file: required(flags.cert, "serve requires --cert PATH")?,
            key_file: required(flags.key, "serve requires --key PATH")?,
        },
        "status" => Invocation::Status(flags.target()?),
        "list" => Invocation::List(flags.target()?),
        "print-unit" => Invocation::PrintUnit(UnitSettings {
            channel: required(flags.channel, "print-unit requires --channel prod|dev")?,
            exec: required(flags.exec, "print-unit requires --exec PATH")?,
            cert: required(flags.cert, "print-unit requires --cert PATH")?,
            key: required(flags.key, "print-unit requires --key PATH")?,
            user: flags.user.unwrap_or_else(|| "bo2mp".to_owned()),
            group: flags.group.unwrap_or_else(|| "bo2mp".to_owned()),
        }),
        other => return Err(format!("unknown command {other}").into()),
    };
    Ok(invocation)
}
