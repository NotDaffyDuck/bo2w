//! A bare HTTPS GET. The only authority trusted is the CA built into this
//! binary, so a stranger's certificate never passes, whatever the system trusts.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::channel::Endpoint;

/// The CA the channel's certificate must chain to (see `build.rs`).
const PINNED_CA_PEM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bo2mp-ca.pem"));

/// The largest body a download may have.
pub const MAX_BODY: usize = 512 * 1024 * 1024;
/// What the status line and headers may add on top of the largest body.
const HEADER_ALLOWANCE: usize = 64 * 1024;

/// The body of `url`, which must be an `https://` address.
pub fn get(url: &str) -> Result<Vec<u8>, String> {
    let target = Endpoint::parse(url)?;
    let tls = ClientConfig::builder()
        .with_root_certificates(trusted_roots(PINNED_CA_PEM)?)
        .with_no_client_auth();
    let name = ServerName::try_from(target.host.clone())
        .map_err(|_| format!("invalid TLS server name `{}`", target.host))?;
    let connection =
        ClientConnection::new(Arc::new(tls), name).map_err(|error| format!("TLS init: {error}"))?;
    let socket = TcpStream::connect((target.host.as_str(), target.port))
        .map_err(|error| format!("connect {}:{}: {error}", target.host, target.port))?;
    let mut stream = StreamOwned::new(connection, socket);

    let failed = |error: std::io::Error| format!("GET {url}: {error}");
    stream
        .write_all(request(&target).as_bytes())
        .map_err(failed)?;
    stream.flush().map_err(failed)?;

    let mut raw = Vec::new();
    let cap = (MAX_BODY + HEADER_ALLOWANCE + 1) as u64;
    stream.take(cap).read_to_end(&mut raw).map_err(failed)?;
    if raw.len() > MAX_BODY + HEADER_ALLOWANCE {
        return Err(format!("GET {url}: response exceeds limit"));
    }
    body_of(raw).map_err(|why| format!("GET {url}: {why}"))
}

/// The roots to trust: every certificate in `pem`, and nothing else.
fn trusted_roots(pem: &[u8]) -> Result<RootCertStore, String> {
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut BufReader::new(pem))
        .collect::<Result<_, _>>()
        .map_err(|error| format!("embedded CA parse: {error}"))?;
    if certs.is_empty() {
        return Err(
            "embedded CA contains no certificate; rebuild with `make launcher windows`".to_owned(),
        );
    }
    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots
            .add(cert)
            .map_err(|error| format!("embedded CA: {error}"))?;
    }
    Ok(roots)
}

/// The request text. HTTP/1.1 with `Connection: close`: the body ends where
/// the connection does, and the server may not compress it.
fn request(target: &Endpoint) -> String {
    format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: bo2mp-launcher/{}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        target.path,
        target.host,
        env!("CARGO_PKG_VERSION"),
    )
}

/// The body of a whole response, if it is a success this client can read.
fn body_of(mut raw: Vec<u8>) -> Result<Vec<u8>, String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("malformed HTTP response")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "non-UTF8 HTTP headers")?;
    let mut lines = head.lines();
    let status = lines.next().ok_or("missing HTTP status")?;
    if !is_success(status) {
        return Err(status.to_owned());
    }
    let mut length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            return Err("chunked responses are unsupported".to_owned());
        }
        if name.eq_ignore_ascii_case("content-length") {
            length = length.or(value.parse::<usize>().ok());
        }
    }
    raw.drain(..split + 4);
    match length {
        Some(promised) if promised != raw.len() => {
            Err(format!("body length {} != {promised}", raw.len()))
        }
        _ => Ok(raw),
    }
}

/// `HTTP/1.0 2xx` or `HTTP/1.1 2xx`.
fn is_success(status_line: &str) -> bool {
    let mut parts = status_line.split_whitespace();
    let version = parts.next();
    let code = parts.next().and_then(|code| code.parse::<u16>().ok());
    matches!(version, Some("HTTP/1.1" | "HTTP/1.0")) && code.is_some_and(|code| code / 100 == 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(head: &str, body: &[u8]) -> Vec<u8> {
        let mut raw = format!("{head}\r\n\r\n").into_bytes();
        raw.extend_from_slice(body);
        raw
    }

    #[test]
    fn the_request_names_the_path_and_asks_for_no_compression() {
        let target = Endpoint::parse("https://e.net/prod/manifest.json").unwrap();
        let text = request(&target);
        assert!(text.starts_with("GET /prod/manifest.json HTTP/1.1\r\n"));
        assert!(text.contains("\r\nHost: e.net\r\n"));
        assert!(text.contains("\r\nAccept-Encoding: identity\r\n"));
        assert!(text.contains("\r\nConnection: close\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn a_success_gives_its_body() {
        let raw = response("HTTP/1.1 200 OK\r\nContent-Length: 5", b"hello");
        assert_eq!(body_of(raw).unwrap(), b"hello");
        let old = response("HTTP/1.0 204 No Content", b"");
        assert_eq!(body_of(old).unwrap(), b"");
    }

    #[test]
    fn a_body_may_hold_a_blank_line_and_any_bytes() {
        let body = b"a\r\n\r\nb\0\xff";
        let raw = response("HTTP/1.1 200 OK\r\ncontent-length: 8", body);
        assert_eq!(body_of(raw).unwrap(), body);
    }

    #[test]
    fn without_a_length_the_body_is_whatever_arrived() {
        let raw = response("HTTP/1.1 200 OK\r\nServer: x", b"abc");
        assert_eq!(body_of(raw).unwrap(), b"abc");
    }

    #[test]
    fn only_http_one_successes_are_read() {
        for status in [
            "HTTP/1.1 404 Not Found",
            "HTTP/1.1 301 Moved Permanently",
            "HTTP/1.1 500 Oops",
            "HTTP/2 200",
            "HTTP/1.1 abc",
            "HTTP/1.1",
            "200 OK",
        ] {
            let error = body_of(response(status, b"x")).unwrap_err();
            assert_eq!(error, status);
        }
    }

    #[test]
    fn a_cut_off_body_is_caught_by_its_length() {
        let raw = response("HTTP/1.1 200 OK\r\nContent-Length: 10", b"short");
        assert_eq!(body_of(raw).unwrap_err(), "body length 5 != 10");
    }

    #[test]
    fn chunked_bodies_are_refused_in_any_case() {
        for header in [
            "Transfer-Encoding: chunked",
            "transfer-encoding: Chunked",
            "TRANSFER-ENCODING: gzip, chunked",
        ] {
            let raw = response(
                &format!("HTTP/1.1 200 OK\r\n{header}"),
                b"5\r\nhello\r\n0\r\n",
            );
            let error = body_of(raw).unwrap_err();
            assert!(error.contains("chunked"), "{header}: {error}");
        }
    }

    #[test]
    fn a_response_that_never_finishes_its_headers_is_malformed() {
        assert!(
            body_of(b"HTTP/1.1 200 OK\r\nContent-Le".to_vec())
                .unwrap_err()
                .contains("malformed")
        );
        assert!(body_of(Vec::new()).is_err());
        let not_text = b"HTTP/1.1 200 OK\r\nX: \xff\r\n\r\nbody".to_vec();
        assert!(body_of(not_text).unwrap_err().contains("non-UTF8"));
    }

    #[test]
    fn a_build_without_the_ca_says_how_to_get_one() {
        let error = trusted_roots(b"BO2MP updater built without a CA\n").unwrap_err();
        assert!(error.contains("make launcher windows"), "{error}");
        assert!(trusted_roots(b"").is_err());
    }

    #[test]
    fn a_damaged_certificate_is_not_trusted() {
        let pem = b"-----BEGIN CERTIFICATE-----\n!!!not base64!!!\n-----END CERTIFICATE-----\n";
        assert!(trusted_roots(pem).is_err());
    }

    #[test]
    fn a_non_https_address_never_reaches_the_network() {
        let error = get("http://e.net/x").unwrap_err();
        assert!(error.contains("must use https"), "{error}");
    }
}
