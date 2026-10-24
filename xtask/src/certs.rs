//! The release CA and the server identity it signs, made with `openssl`.
//!
//! Players pin the CA, so nothing here ever replaces one. A half-made or
//! mismatched identity is reported, never papered over with a new key.

use std::path::{Path, PathBuf};
use std::process::Command;

use master_protocol::Channel;

use crate::dotenv::Env;
use crate::shell::{Res, capture, require_tools, run, set_mode};

pub const CA_DAYS: u32 = 3650;
pub const SERVER_DAYS: u32 = 825;
const RSA_BITS: &str = "rsa_keygen_bits:3072";
const CA_SUBJECT: &str = "/CN=BO2MP Release CA";

pub fn is_ipv4(host: &str) -> bool {
    host.parse::<std::net::Ipv4Addr>().is_ok()
}

/// What the server certificate must answer to. The channel labels are always
/// there: the client checks them at the handshake instead of a domain name.
pub enum San<'a> {
    Labels,
    WithHost(&'a str),
}

impl San<'_> {
    pub fn entries(&self) -> Vec<String> {
        let mut entries: Vec<String> = Channel::ALL
            .iter()
            .map(|channel| format!("DNS:{}", channel.server_name()))
            .collect();
        if let San::WithHost(host) = self {
            entries.push(if is_ipv4(host) {
                format!("IP:{host}")
            } else {
                format!("DNS:{host}")
            });
        }
        entries
    }
}

/// `openssl x509 -ext subjectAltName` prints `IP Address:` where the config
/// said `IP:`.
fn printed_as(entry: &str) -> String {
    match entry.strip_prefix("IP:") {
        Some(ip) => format!("IP Address:{ip}"),
        None => entry.to_string(),
    }
}

/// Every wanted entry is in the printed SAN list (comma or line separated).
pub fn covers(printed: &str, wanted: &[String]) -> bool {
    let listed: Vec<&str> = printed
        .split(['\n', ','])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect();
    wanted
        .iter()
        .all(|entry| listed.contains(&printed_as(entry).as_str()))
}

pub fn extension_file(san: &San) -> String {
    format!(
        "basicConstraints=critical,CA:FALSE\n\
         keyUsage=critical,digitalSignature,keyEncipherment\n\
         extendedKeyUsage=serverAuth\n\
         subjectAltName={}\n",
        san.entries().join(",")
    )
}

/// One directory holding the CA pair and the server pair.
pub struct Ca {
    dir: PathBuf,
}

impl Ca {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn key(&self) -> PathBuf {
        self.dir.join("ca-key.pem")
    }

    /// The public half players get, as `bo2mp-ca.pem`.
    pub fn cert(&self) -> PathBuf {
        self.dir.join("bo2mp-ca.pem")
    }

    pub fn server_key(&self) -> PathBuf {
        self.dir.join("server-key.pem")
    }

    pub fn server_cert(&self) -> PathBuf {
        self.dir.join("server-cert.pem")
    }

    /// Make whichever pair is wholly absent, then prove the server pair chains
    /// to the CA and answers to `san`.
    pub fn ensure(&self, san: &San) -> Res<()> {
        require_tools(&["openssl"])?;
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| format!("creating {}: {error}", self.dir.display()))?;
        set_mode(&self.dir, 0o700)?;

        match (self.key().is_file(), self.cert().is_file()) {
            (true, true) => {}
            (false, false) => self.make_ca()?,
            _ => return Err(self.half("ca-key.pem", "bo2mp-ca.pem")),
        }
        match (self.server_key().is_file(), self.server_cert().is_file()) {
            (true, true) => self.check_san(san)?,
            (false, false) => self.make_server(san)?,
            _ => return Err(self.half("server-key.pem", "server-cert.pem")),
        }
        run(Command::new("openssl")
            .args(["verify", "-CAfile"])
            .arg(self.cert())
            .arg(self.server_cert()))?;
        println!("certificates ready: {}", self.dir.display());
        Ok(())
    }

    fn half(&self, key: &str, cert: &str) -> String {
        format!(
            "{} has only one of {key} and {cert}; restore the missing file or move both away by hand",
            self.dir.display()
        )
    }

    fn new_key(path: &Path) -> Res<()> {
        run(Command::new("openssl")
            .args(["genpkey", "-algorithm", "RSA", "-pkeyopt", RSA_BITS, "-out"])
            .arg(path))?;
        set_mode(path, 0o600)
    }

    fn make_ca(&self) -> Res<()> {
        Self::new_key(&self.key())?;
        run(Command::new("openssl")
            .args(["req", "-x509", "-new", "-sha256", "-days"])
            .arg(CA_DAYS.to_string())
            .arg("-key")
            .arg(self.key())
            .arg("-out")
            .arg(self.cert())
            .args(["-subj", CA_SUBJECT]))
    }

    fn make_server(&self, san: &San) -> Res<()> {
        Self::new_key(&self.server_key())?;
        let request = self.dir.join("server.csr");
        let extensions = self.dir.join("server.ext");
        run(Command::new("openssl")
            .args(["req", "-new", "-key"])
            .arg(self.server_key())
            .arg("-out")
            .arg(&request)
            .arg("-subj")
            .arg(format!("/CN={}", Channel::Prod.server_name())))?;
        std::fs::write(&extensions, extension_file(san))
            .map_err(|error| format!("writing {}: {error}", extensions.display()))?;
        let signed = run(Command::new("openssl")
            .args(["x509", "-req", "-sha256", "-days"])
            .arg(SERVER_DAYS.to_string())
            .arg("-in")
            .arg(&request)
            .arg("-CA")
            .arg(self.cert())
            .arg("-CAkey")
            .arg(self.key())
            .arg("-CAserial")
            .arg(self.dir.join("ca.srl"))
            .arg("-CAcreateserial")
            .arg("-extfile")
            .arg(&extensions)
            .arg("-out")
            .arg(self.server_cert()));
        let _ = std::fs::remove_file(&request);
        let _ = std::fs::remove_file(&extensions);
        signed
    }

    fn check_san(&self, san: &San) -> Res<()> {
        let printed = capture(
            Command::new("openssl")
                .args(["x509", "-noout", "-ext", "subjectAltName", "-in"])
                .arg(self.server_cert()),
        )?;
        let wanted = san.entries();
        if covers(&printed, &wanted) {
            Ok(())
        } else {
            Err(format!(
                "{} does not cover {}; it was made for another host. Move the server pair away to make a new one (the CA stays)",
                self.server_cert().display(),
                wanted.join(",")
            ))
        }
    }
}

/// `cargo xtask certs <host>`: the release CA and our host's identity, in
/// `BO2MP_RELEASE_KEY`.
pub fn run_cli(env: &Env, args: &[String]) -> Res<()> {
    let [host] = args else {
        return Err("usage: cargo xtask certs <host>".to_string());
    };
    let dir = env.require("BO2MP_RELEASE_KEY")?;
    Ca::new(dir).ensure(&San::WithHost(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn san_lists_both_channel_labels_then_the_host() {
        assert_eq!(San::Labels.entries(), ["DNS:bo2mp-prod", "DNS:bo2mp-dev"]);
        assert_eq!(
            San::WithHost("198.51.100.4").entries(),
            ["DNS:bo2mp-prod", "DNS:bo2mp-dev", "IP:198.51.100.4"]
        );
        assert_eq!(
            San::WithHost("relay.example.net").entries().last().unwrap(),
            "DNS:relay.example.net"
        );
    }

    #[test]
    fn extension_file_is_a_leaf_server_cert() {
        let text = extension_file(&San::WithHost("198.51.100.4"));
        assert!(text.contains("basicConstraints=critical,CA:FALSE\n"));
        assert!(text.contains("extendedKeyUsage=serverAuth\n"));
        assert!(text.ends_with("subjectAltName=DNS:bo2mp-prod,DNS:bo2mp-dev,IP:198.51.100.4\n"));
    }

    #[test]
    fn printed_san_is_matched_entry_by_entry() {
        let printed = "X509v3 Subject Alternative Name: \n    DNS:bo2mp-prod, DNS:bo2mp-dev, IP Address:198.51.100.4\n";
        assert!(covers(printed, &San::WithHost("198.51.100.4").entries()));
        assert!(covers(printed, &San::Labels.entries()));
        assert!(!covers(printed, &San::WithHost("198.51.100.5").entries()));
        assert!(!covers(
            printed,
            &San::WithHost("relay.example.net").entries()
        ));
        assert!(!covers("DNS:bo2mp-prod\n", &San::Labels.entries()));
    }

    #[test]
    fn ipv4_only_for_dotted_quads() {
        assert!(is_ipv4("203.0.113.9"));
        assert!(!is_ipv4("203.0.113"));
        assert!(!is_ipv4("relay.example.net"));
        assert!(!is_ipv4("::1"));
    }
}
