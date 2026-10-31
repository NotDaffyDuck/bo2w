//! A bare HTTPS GET. The only authority trusted is the CA built into this
//! binary, so a stranger's certificate never passes, whatever the system trusts.

use std::io::{self, Read as _, Write as _};
use std::sync::Arc;

use rustls::pki_types::CertificateDer;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::channel::Endpoint;
use io::BufReader;
use std::net::TcpStream;

/// The CA the channel's certificate must chain to (see `build.rs`).
const PINNED_CA_PEM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bo2mp-ca.pem"));

/// The largest body a download may have.
pub const MAX_BODY: usize = 512 * 1024 * 1024;
/// What the status line and headers may add on top of the largest body.
const HEADER_ALLOWANCE: usize = 64 * 1024;

/// The body of `url`, which must be an `https://` address.
pub fn get(url: &str) -> Result<Vec<u8>, String> {
    let target = Endpoint::parse(url)?;
    let mut stream = open_tls(&target)?;
    let io_failed = |why: io::Error| format!("GET {url}: {why}");
    let ask = request(&target);
    stream.write_all(ask.as_bytes()).and_then(|()| stream.flush()).map_err(io_failed)?;

    // Read at most one byte past the limit: enough to tell it was crossed.
    let limit = MAX_BODY + HEADER_ALLOWANCE;
    let mut whole = Vec::new();
    stream.take(limit as u64 + 1).read_to_end(&mut whole).map_err(io_failed)?;
    let fits = whole.len() <= limit;
    let answer = if fits { body_of(whole) } else { Err("response exceeds limit".to_owned()) };
    answer.map_err(|why| format!("GET {url}: {why}"))
}

/// A TLS stream to the endpoint that trusts only the pinned CA.
fn open_tls(target: &Endpoint) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
    let roots = trusted_roots(PINNED_CA_PEM)?;
    let config = Arc::new(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth());
    let host = target.host.as_str();
    let Ok(server) = ServerName::try_from(host.to_owned()) else {
        return Err(format!("invalid TLS server name `{host}`"));
    };
    let session = ClientConnection::new(config, server).map_err(|why| format!("TLS init: {why}"))?;
    let port = target.port;
    let socket = TcpStream::connect((host, port)).map_err(|why| format!("connect {host}:{port}: {why}"))?;
    Ok(StreamOwned::new(session, socket))
}

/// The roots to trust: every certificate in `pem`, and nothing else.
fn trusted_roots(pem: &[u8]) -> Result<RootCertStore, String> {
    let mut reader = BufReader::new(pem);
    let mut found: Vec<CertificateDer<'static>> = Vec::new();
    for parsed in rustls_pemfile::certs(&mut reader) {
        found.push(parsed.map_err(|why| format!("embedded CA parse: {why}"))?);
    }
    if found.is_empty() {
        return Err(String::from("embedded CA contains no certificate; rebuild with `make launcher windows`"));
    }
    found.into_iter().try_fold(RootCertStore::empty(), |mut roots, cert| {
        roots.add(cert).map_err(|why| format!("embedded CA: {why}"))?;
        Ok(roots)
    })
}

/// The request text. HTTP/1.1 with `Connection: close`: the body ends where
/// the connection does, and the server may not compress it.
fn request(target: &Endpoint) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let lines = [
        format!("GET {} HTTP/1.1", target.path),
        format!("Host: {}", target.host),
        format!("User-Agent: bo2mp-launcher/{version}"),
        "Accept-Encoding: identity".to_owned(),
        "Connection: close".to_owned(),
        String::new(),
        String::new(),
    ];
    lines.join("\r\n")
}

/// The body of a whole response, if it is a success this client can read.
fn body_of(raw: Vec<u8>) -> Result<Vec<u8>, String> {
    const BLANK_LINE: &[u8] = b"\r\n\r\n";
    let Some(head_end) = raw.windows(BLANK_LINE.len()).position(|four| four == BLANK_LINE) else {
        return Err("malformed HTTP response".into());
    };
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| String::from("non-UTF8 HTTP headers"))?;
    if head.is_empty() {
        return Err("missing HTTP status".into());
    }
    let (first, headers) = head.split_once('\n').unwrap_or((head, ""));
    let status = first.strip_suffix('\r').unwrap_or(first);
    if !is_success(status) {
        return Err(status.to_owned());
    }
    let fields = headers.lines().filter_map(|line| line.split_once(':')).map(|(key, value)| (key, value.trim()));
    let mut promised = None;
    for (key, value) in fields {
        let chunked = key.eq_ignore_ascii_case("transfer-encoding") && value.to_ascii_lowercase().contains("chunked");
        if chunked {
            return Err("chunked responses are unsupported".into());
        }
        if promised.is_none() && key.eq_ignore_ascii_case("content-length") {
            promised = value.parse::<usize>().ok();
        }
    }
    let body = raw[head_end + BLANK_LINE.len()..].to_vec();
    let got = body.len();
    match promised {
        Some(want) if want != got => Err(format!("body length {got} != {want}")),
        _ => Ok(body),
    }
}

/// `HTTP/1.0 2xx` or `HTTP/1.1 2xx`.
fn is_success(summary_banner: &str) -> bool {
    let words: Vec<&str> = summary_banner.split_whitespace().take(2).collect();
    let [version, code] = words[..] else { return false };
    let known = version == "HTTP/1.1" || version == "HTTP/1.0";
    known && code.parse::<u16>().is_ok_and(|number| (200..300).contains(&number))
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
