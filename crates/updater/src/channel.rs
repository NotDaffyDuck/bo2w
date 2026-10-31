//! Where the update channel lives: the base address a player sets, and the
//! pieces of an `https://` address the client needs to open a connection.

/// A parsed `https://` address.
#[derive(Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    /// Starts with `/`; keeps any query string.
    pub path: String,
}

impl Endpoint {
    pub fn parse(url: &str) -> Result<Self, String> {
        let Some(rest) = url.strip_prefix("https://") else {
            return Err(format!("update URL must use https, not `{url}`"));
        };
        let (host_port, path) = match rest.split_once('/') {
            Some((host_port, path)) => (host_port, format!("/{path}")),
            None => (rest, "/".to_owned()),
        };
        let (domain, port) = match host_port.rsplit_once(':') {
            Some((domain, port)) => (domain, port.parse::<u16>().ok()),
            None => (host_port, Some(443)),
        };
        match port {
            Some(port) if port != 0 && !domain.is_empty() => Ok(Self {
                host: domain.to_owned(),
                port,
                path,
            }),
            _ => Err(format!("invalid update URL `{url}`")),
        }
    }
}

/// The channel's base address from `BO2MP_UPDATE_URL`: trimmed, with no
/// trailing slash, so file names can be joined onto it.
pub fn base_from(setting: Option<&str>) -> Result<String, String> {
    let Some(setting) = setting else {
        return Err(
            "BO2MP_UPDATE_URL is unset; put it in the portable .env before the first HTTP request"
                .to_owned(),
        );
    };
    let stripped = setting.trim();
    if stripped.is_empty() {
        return Err("BO2MP_UPDATE_URL is empty".to_owned());
    }
    if stripped.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("BO2MP_UPDATE_URL contains invalid characters".to_owned());
    }
    // A plain-http or hostless channel is refused here, before anyone is asked.
    Endpoint::parse(stripped)?;
    Ok(stripped.trim_end_matches('/').to_owned())
}

/// The address of `name` on the channel at `base`.
pub fn address_of(base: &str, file: &str) -> String {
    format!("{base}/{file}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(domain: &str, port: u16, path: &str) -> Endpoint {
        Endpoint {
            host: domain.to_owned(),
            port,
            path: path.to_owned(),
        }
    }

    #[test]
    fn https_is_the_default_port_443() {
        assert_eq!(
            Endpoint::parse("https://updates.example.net/prod/manifest.json"),
            Ok(endpoint("updates.example.net", 443, "/prod/manifest.json"))
        );
    }

    #[test]
    fn a_port_and_a_missing_path_are_read() {
        assert_eq!(
            Endpoint::parse("https://updates.example.net:8443"),
            Ok(endpoint("updates.example.net", 8443, "/"))
        );
        assert_eq!(
            Endpoint::parse("https://h:1/a?b=c"),
            Ok(endpoint("h", 1, "/a?b=c"))
        );
    }

    #[test]
    fn only_https_addresses_with_a_host_and_a_real_port_count() {
        for url in [
            "http://example.net/",
            "example.net/x",
            "ftp://example.net",
            "https://",
            "https:///path",
            "https://:443/x",
            "https://host:0/x",
            "https://host:70000/x",
            "https://host:port/x",
        ] {
            assert!(Endpoint::parse(url).is_err(), "{url}");
        }
    }

    #[test]
    fn the_refusal_names_the_address() {
        let error = Endpoint::parse("http://example.net/").unwrap_err();
        assert!(error.contains("must use https"), "{error}");
        assert!(error.contains("http://example.net/"), "{error}");
    }

    #[test]
    fn the_base_loses_spaces_around_it_and_slashes_after_it() {
        assert_eq!(
            base_from(Some("  https://e.net/rel/prod//  ")).as_deref(),
            Ok("https://e.net/rel/prod")
        );
        assert_eq!(
            base_from(Some("https://e.net")).as_deref(),
            Ok("https://e.net")
        );
    }

    #[test]
    fn a_missing_empty_or_broken_base_is_refused() {
        let unset = base_from(None).unwrap_err();
        assert!(unset.contains("unset"), "{unset}");
        for blank in ["", "   ", "\n"] {
            let empty = base_from(Some(blank)).unwrap_err();
            assert!(empty.contains("empty"), "{blank:?}: {empty}");
        }
        for broken in [
            "https://e.net/a b",
            "https://e.net/\0",
            "https://e.net/\r\nx",
        ] {
            let error = base_from(Some(broken)).unwrap_err();
            assert!(error.contains("invalid characters"), "{broken:?}: {error}");
        }
    }

    #[test]
    fn a_base_that_is_not_https_is_refused_up_front() {
        for plain in ["http://e.net/prod", "e.net/prod", "ftp://e.net", "https://"] {
            assert!(base_from(Some(plain)).is_err(), "{plain:?}");
        }
        let error = base_from(Some("http://e.net/prod")).unwrap_err();
        assert!(error.contains("must use https"), "{error}");
    }

    #[test]
    fn files_hang_off_the_base() {
        assert_eq!(
            address_of("https://e.net/prod", "manifest.json"),
            "https://e.net/prod/manifest.json"
        );
    }
}
