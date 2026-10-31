//! The release CA and the server identity it signs, made with `openssl`.
//!
//! Players pin the CA, so nothing here ever replaces one. A half-made or
//! mismatched identity is reported, never papered over with a new key.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use master_protocol::Channel;

use crate::dot_vars::DotVars;
use crate::cmd_run::{grab, demand_host_programs, launch, set_mode};

pub const CA_DAYS: u32 = 3650;
pub const SERVER_DAYS: u32 = 825;
const RSA_BITS: &str = "rsa_keygen_bits:3072";
const CA_SUBJECT: &str = "/CN=BO2MP Release CA";

pub fn is_ipv4(host_name: &str) -> bool {
    std::net::Ipv4Addr::from_str(host_name).is_ok()
}

/// What the server certificate must answer to. The channel labels are always
/// there: the client checks them at the handshake instead of a domain name.
pub enum CertAltName<'a> {
    Labels,
    WithHost(&'a str),
}

impl CertAltName<'_> {
    pub fn entries(&self) -> Vec<String> {
        let host_entry = match self {
            CertAltName::Labels => None,
            CertAltName::WithHost(host_name) => {
                let kind = if is_ipv4(host_name) { "IP" } else { "DNS" };
                Some(format!("{kind}:{host_name}"))
            }
        };
        let labels = Channel::ALL.map(|svc| ["DNS:", svc.tls_name()].concat());
        labels.into_iter().chain(host_entry).collect()
    }
}

/// `openssl x509 -ext subjectAltName` prints `IP Address:` where the config
/// said `IP:`.
fn printed_as(entry: &str) -> String {
    entry
        .strip_prefix("IP:")
        .map_or_else(|| entry.to_owned(), |ip| format!("IP Address:{ip}"))
}

/// Every wanted entry is in the printed SAN list (comma or line separated).
pub fn covers(printed: &str, wanted: &[String]) -> bool {
    let is_listed = |entry: &str| {
        let dest = printed_as(entry);
        printed
            .split(['\n', ','])
            .any(|item| item.trim() == dest)
    };
    wanted.iter().all(|entry| is_listed(entry))
}

pub fn extension_file(san: &CertAltName) -> String {
    let fixed = [
        "basicConstraints=critical,CA:FALSE",
        "keyUsage=critical,digitalSignature,keyEncipherment",
        "extendedKeyUsage=serverAuth",
    ];
    let names = format!("subjectAltName={}", san.entries().join(","));
    fixed
        .into_iter()
        .chain([names.as_str()])
        .flat_map(|line| [line, "\n"])
        .collect()
}

/// An `openssl` command with these arguments, words and paths alike.
fn openssl(cli_words: &[&dyn AsRef<OsStr>]) -> Command {
    let mut command = Command::new("openssl");
    command.args(cli_words.iter().map(|arg| arg.as_ref()));
    command
}

/// What a key/certificate pair looks like on disk.
enum Pair {
    Both,
    Neither,
    Half,
}

fn pair_on_disk(knob: &Path, cert: &Path) -> Pair {
    match [knob.is_file(), cert.is_file()] {
        [true, true] => Pair::Both,
        [false, false] => Pair::Neither,
        _ => Pair::Half,
    }
}

const CA_KEY_FILE: &str = "ca-key.pem";
const CA_CERT_FILE: &str = "bo2mp-ca.pem";
const SERVER_KEY_FILE: &str = "server-key.pem";
const SERVER_CERT_FILE: &str = "server-cert.pem";
const DIR_MODE: u32 = 0o700;
const KEY_MODE: u32 = 0o600;

/// Path accessors for the fixed file names inside the directory.
macro_rules! named_files {
    ($($(#[$doc:meta])* $method:ident => $title:ident;)+) => {
        $($(#[$doc])* pub fn $method(&self) -> PathBuf {
            self.file($title)
        })+
    };
}

/// One directory holding the CA pair and the server pair.
pub struct Authority(PathBuf);

impl Authority {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self(base_dir.into())
    }

    pub fn base_dir(&self) -> &Path {
        &self.0
    }

    fn file(&self, title: &str) -> PathBuf {
        self.0.join(title)
    }

    named_files! {
        knob => CA_KEY_FILE;
        /// The public half players get, as `bo2mp-ca.pem`.
        cert => CA_CERT_FILE;
        server_key => SERVER_KEY_FILE;
        server_cert => SERVER_CERT_FILE;
    }

    /// Make whichever pair is wholly absent, then prove the server pair chains
    /// to the CA and answers to `san`.
    pub fn make_ready(&self, san: &CertAltName) -> Result<(), String> {
        demand_host_programs(&["openssl"])?;
        self.prepare_dir()?;

        match pair_on_disk(&self.knob(), &self.cert()) {
            Pair::Both => {}
            Pair::Neither => self.make_ca()?,
            Pair::Half => return Err(self.half(CA_KEY_FILE, CA_CERT_FILE)),
        }
        match pair_on_disk(&self.server_key(), &self.server_cert()) {
            Pair::Both => self.check_san(san)?,
            Pair::Neither => self.make_server(san)?,
            Pair::Half => return Err(self.half(SERVER_KEY_FILE, SERVER_CERT_FILE)),
        }
        self.verify_chain()?;
        let ready_dir = self.0.display();
        println!("certificates ready: {ready_dir}");
        Ok(())
    }

    fn prepare_dir(&self) -> Result<(), String> {
        let made = std::fs::create_dir_all(&self.0);
        made.map_err(|error| format!("creating {}: {error}", self.0.display()))?;
        set_mode(&self.0, DIR_MODE)
    }

    fn verify_chain(&self) -> Result<(), String> {
        let root_pem = self.cert();
        let leaf_pem = self.server_cert();
        launch(&mut openssl(&[&"verify", &"-CAfile", &root_pem, &leaf_pem]))
    }

    fn half(&self, knob: &str, cert: &str) -> String {
        let shown = self.0.display();
        format!(
            "{shown} has only one of {knob} and {cert}; restore the missing file or move both away by hand"
        )
    }

    fn new_key(file_at: &Path) -> Result<(), String> {
        let genpkey = ["genpkey", "-algorithm", "RSA", "-pkeyopt", RSA_BITS, "-out"];
        let mut command = Command::new("openssl");
        command.args(genpkey).arg(file_at);
        launch(&mut command).and_then(|()| set_mode(file_at, KEY_MODE))
    }

    fn make_ca(&self) -> Result<(), String> {
        let (knob, cert) = (self.knob(), self.cert());
        Self::new_key(&knob)?;
        let days = CA_DAYS.to_string();
        launch(&mut openssl(&[
            &"req", &"-x509", &"-new", &"-sha256", &"-days", &days, &"-key", &knob, &"-out",
            &cert, &"-subj", &CA_SUBJECT,
        ]))
    }

    /// Writes the extensions file, then has the CA sign a request for the
    /// server key. The caller removes the two scratch files.
    fn sign_server(&self, san: &CertAltName, request: &Path, extensions: &Path) -> Result<(), String> {
        let subject = ["/CN=", Channel::Prod.tls_name()].concat();
        let (server_key, ca_key, trust_anchor_cert) = (self.server_key(), self.knob(), self.cert());
        launch(&mut openssl(&[
            &"req", &"-new", &"-key", &server_key, &"-out", &request, &"-subj", &subject,
        ]))?;
        let ext_shown = extensions.display();
        std::fs::write(extensions, extension_file(san)).map_err(|e| format!("writing {ext_shown}: {e}"))?;
        let days = SERVER_DAYS.to_string();
        let serial = self.file("ca.srl");
        let cert_file = self.server_cert();
        launch(&mut openssl(&[
            &"x509", &"-req", &"-sha256", &"-days", &days, &"-in", &request, &"-CA", &trust_anchor_cert,
            &"-CAkey", &ca_key, &"-CAserial", &serial, &"-CAcreateserial", &"-extfile",
            &extensions, &"-out", &cert_file,
        ]))
    }

    fn make_server(&self, san: &CertAltName) -> Result<(), String> {
        Self::new_key(&self.server_key())?;
        let scratch = [self.file("server.csr"), self.file("server.ext")];
        let signed = self.sign_server(san, &scratch[0], &scratch[1]);
        for file_at in &scratch {
            let _ = std::fs::remove_file(file_at);
        }
        signed
    }

    fn check_san(&self, san: &CertAltName) -> Result<(), String> {
        let cert = self.server_cert();
        let wanted = san.entries();
        let printed = grab(&mut openssl(&[
            &"x509",
            &"-noout",
            &"-ext",
            &"subjectAltName",
            &"-in",
            &cert,
        ]))?;
        if !covers(&printed, &wanted) {
            return Err(format!(
                "{} does not cover {}; it was made for another host. Move the server pair away to make a new one (the CA stays)",
                cert.display(),
                wanted.join(",")
            ));
        }
        Ok(())
    }
}

/// `cargo xtask certs <host>`: the release CA and our host's identity, in
/// `BO2MP_RELEASE_KEY`.
pub fn dispatch_subcommand(dot_conf: &DotVars, cli_words: &[String]) -> Result<(), String> {
    let [host_name] = cli_words else {
        return Err(String::from("usage: cargo xtask certs <host>"));
    };
    let ca = Authority::new(dot_conf.must_get("BO2MP_RELEASE_KEY")?);
    ca.make_ready(&CertAltName::WithHost(host_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn san_lists_both_channel_labels_then_the_host() {
        assert_eq!(CertAltName::Labels.entries(), ["DNS:bo2mp-prod", "DNS:bo2mp-dev"]);
        assert_eq!(
            CertAltName::WithHost("198.51.100.4").entries(),
            ["DNS:bo2mp-prod", "DNS:bo2mp-dev", "IP:198.51.100.4"]
        );
        assert_eq!(
            CertAltName::WithHost("relay.example.net").entries().last().unwrap(),
            "DNS:relay.example.net"
        );
    }

    #[test]
    fn extension_file_is_a_leaf_server_cert() {
        let text = extension_file(&CertAltName::WithHost("198.51.100.4"));
        assert!(text.contains("basicConstraints=critical,CA:FALSE\n"));
        assert!(text.contains("extendedKeyUsage=serverAuth\n"));
        assert!(text.ends_with("subjectAltName=DNS:bo2mp-prod,DNS:bo2mp-dev,IP:198.51.100.4\n"));
    }

    #[test]
    fn printed_san_is_matched_entry_by_entry() {
        let printed = "X509v3 Subject Alternative Name: \n    DNS:bo2mp-prod, DNS:bo2mp-dev, IP Address:198.51.100.4\n";
        assert!(covers(printed, &CertAltName::WithHost("198.51.100.4").entries()));
        assert!(covers(printed, &CertAltName::Labels.entries()));
        assert!(!covers(printed, &CertAltName::WithHost("198.51.100.5").entries()));
        assert!(!covers(
            printed,
            &CertAltName::WithHost("relay.example.net").entries()
        ));
        assert!(!covers("DNS:bo2mp-prod\n", &CertAltName::Labels.entries()));
    }

    #[test]
    fn accessors_name_the_four_pem_files_under_the_directory() {
        let ca = Authority::new("/keys");
        assert_eq!(ca.base_dir(), Path::new("/keys"));
        let names: Vec<_> = [ca.knob(), ca.cert(), ca.server_key(), ca.server_cert()]
            .iter()
            .map(|file_at| file_at.strip_prefix("/keys").unwrap().to_owned())
            .collect();
        let hoped = ["ca-key.pem", "bo2mp-ca.pem", "server-key.pem", "server-cert.pem"];
        assert_eq!(names, hoped.map(PathBuf::from));
    }

    #[test]
    fn extension_file_is_exactly_four_lines() {
        let text = extension_file(&CertAltName::Labels);
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|line| line.ends_with('\n')));
        assert!(lines[1].starts_with("keyUsage=critical,"));
        assert_eq!(lines[3], "subjectAltName=DNS:bo2mp-prod,DNS:bo2mp-dev\n");
    }

    #[test]
    fn half_a_pair_is_named_in_the_error() {
        let message = Authority::new("/keys").half("a.pem", "b.pem");
        let lead = "/keys has only one of a.pem and b.pem;";
        std::assert_eq!(message.get(..lead.len()), Some(lead), "error: {message}");
    }

    #[test]
    fn certs_command_wants_exactly_one_host() {
        let dot_conf = DotVars::parse("");
        for cli_words in [vec![], vec!["a".to_string(), "b".to_string()]] {
            assert_eq!(dispatch_subcommand(&dot_conf, &cli_words).unwrap_err(), "usage: cargo xtask certs <host>");
        }
    }

    #[test]
    fn ipv4_only_for_dotted_quads() {
        assert!(is_ipv4("203.0.113.9"));
        assert!(!is_ipv4("203.0.113"));
        assert!(!is_ipv4("relay.example.net"));
        assert!(!is_ipv4("::1"));
    }
}
