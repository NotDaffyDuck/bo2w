//! `manifest.json`: what the channel says a working install is made of.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The game, which every manifest must list.
pub const GAME_FILE: &str = "bo2mp.exe";
/// The CA the game checks the master server against, which every manifest must
/// list.
pub const CA_FILE: &str = "bo2mp-ca.pem";

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub channel: String,
    pub protocol: u16,
    pub git: String,
    /// Defaults for the game's environment (the master server's address and
    /// the like). Only names starting `BO2MP_` are allowed.
    pub env: BTreeMap<String, String>,
    pub files: Vec<FileEntry>,
}

/// One file of the install. A file may be shipped under another name and
/// compressed (`download` ends in `.zst`); its own hash and size are those of
/// the installed result.
#[derive(Debug, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub download: Option<String>,
    pub download_sha256: Option<String>,
    pub download_size: Option<u64>,
}

impl FileEntry {
    /// The name to ask the channel for.
    pub fn remote_name(&self) -> &str {
        self.download.as_deref().unwrap_or(&self.path)
    }
}

impl Manifest {
    /// Reads and checks a manifest. Nothing is installed from one that fails.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|error| format!("manifest parse: {error}"))?;
        manifest.check().map(|()| manifest)
    }

    fn check(&self) -> Result<(), String> {
        if self.channel.trim().is_empty() || self.git.trim().is_empty() || self.protocol == 0 {
            return Err("the manifest identity is incomplete".to_owned());
        }
        for name in [GAME_FILE, CA_FILE] {
            if !self.files.iter().any(|entry| entry.path == name) {
                return Err(format!("manifest has no {name}"));
            }
        }
        for file in &self.files {
            plain_name(&file.path)?;
            plain_name(file.remote_name())?;
            hash_digits(&file.sha256)?;
            if let Some(hash) = &file.download_sha256 {
                hash_digits(hash)?;
            }
        }
        for (key, setting) in &self.env {
            if !key.starts_with("BO2MP_") || key.contains(['=', '\0']) {
                return Err(format!("manifest has an invalid env name `{key}`"));
            }
            if setting.contains(['\r', '\n', '\0']) {
                return Err(format!("manifest has an invalid env value for `{key}`"));
            }
        }
        Ok(())
    }
}

/// `name` if it is a bare file name: it is joined onto the install folder and
/// onto the channel's address, so it must not point anywhere else.
pub fn plain_name(name: &str) -> Result<&str, String> {
    let path = std::path::Path::new(name);
    let bare = path.file_name().is_some_and(|file| file == name);
    if bare && !name.contains(['\\', '/', ':']) {
        Ok(name)
    } else {
        Err(format!("manifest name must be a bare file name: `{name}`"))
    }
}

/// A SHA-256 as 64 lowercase hex digits; upper case is accepted.
pub fn hash_digits(text: &str) -> Result<String, String> {
    if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(text.to_ascii_lowercase())
    } else {
        Err(format!("invalid sha256 `{text}`"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serde_json::Value;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A manifest the way the release tool writes it.
    fn sample() -> Value {
        json!({
            "id": "0123456789abcdef",
            "channel": "prod",
            "protocol": 7,
            "git": "abc1234",
            "env": {
                "BO2MP_MASTER_ADDR": "master.example.net:4433",
                "BO2MP_MASTER_SERVER_NAME": "bo2mp-prod",
                "BO2MP_MASTER_CA_CERT": "bo2mp-ca.pem",
            },
            "files": [
                {
                    "path": "bo2mp.exe", "sha256": HASH, "size": 10,
                    "download": "bo2mp-abc.exe.zst", "download_sha256": HASH, "download_size": 4,
                },
                { "path": "bo2mp-ca.pem", "sha256": HASH, "size": 3 },
            ],
        })
    }

    fn parse(value: &Value) -> Result<Manifest, String> {
        Manifest::parse(value.to_string().as_bytes())
    }

    #[test]
    fn what_the_release_tool_writes_is_accepted() {
        let manifest = parse(&sample()).unwrap();
        assert_eq!(manifest.channel, "prod");
        assert_eq!(manifest.protocol, 7);
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.files[0].remote_name(), "bo2mp-abc.exe.zst");
        assert_eq!(manifest.files[1].remote_name(), "bo2mp-ca.pem");
        assert_eq!(manifest.env["BO2MP_MASTER_ADDR"], "master.example.net:4433");
    }

    #[test]
    fn text_that_is_not_a_manifest_is_refused() {
        let error = Manifest::parse(b"<html>").unwrap_err();
        assert!(error.starts_with("manifest parse:"), "{error}");
        let mut without_files = sample();
        without_files.as_object_mut().unwrap().remove("files");
        assert!(parse(&without_files).is_err());
    }

    #[test]
    fn a_manifest_must_say_which_release_it_is() {
        for (key, value) in [
            ("channel", json!(" ")),
            ("git", json!("")),
            ("protocol", json!(0)),
        ] {
            let mut manifest = sample();
            manifest[key] = value;
            let error = parse(&manifest).unwrap_err();
            assert!(error.contains("identity is incomplete"), "{key}: {error}");
        }
    }

    #[test]
    fn the_game_and_its_ca_must_both_be_listed() {
        for missing in [GAME_FILE, CA_FILE] {
            let mut manifest = sample();
            manifest["files"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["path"] != missing);
            let error = parse(&manifest).unwrap_err();
            assert_eq!(error, format!("manifest has no {missing}"));
        }
    }

    #[test]
    fn file_names_cannot_leave_the_install_folder() {
        for bad in [
            "../bo2mp.exe",
            "sub/bo2mp.exe",
            "C:evil.exe",
            "a\\b.exe",
            "",
            "..",
        ] {
            let mut manifest = sample();
            manifest["files"][1]["path"] = json!(bad);
            let mut renamed = sample();
            renamed["files"][0]["download"] = json!(bad);
            assert!(parse(&manifest).is_err(), "path {bad:?}");
            assert!(parse(&renamed).is_err(), "download {bad:?}");
        }
    }

    #[test]
    fn hashes_are_sixty_four_hex_digits() {
        assert_eq!(hash_digits(HASH).as_deref(), Ok(HASH));
        assert_eq!(hash_digits(&HASH.to_ascii_uppercase()).as_deref(), Ok(HASH));
        let too_long = format!("{HASH}0");
        let not_hex = HASH.replace('a', "g");
        for bad in ["", "abc", &HASH[1..], &too_long, &not_hex] {
            assert!(hash_digits(bad).is_err(), "{bad:?}");
        }
        let mut manifest = sample();
        manifest["files"][1]["sha256"] = json!("nope");
        assert!(parse(&manifest).is_err());
        let mut manifest = sample();
        manifest["files"][0]["download_sha256"] = json!("nope");
        assert!(parse(&manifest).is_err());
    }

    #[test]
    fn env_defaults_are_bo2mp_names_with_single_line_values() {
        for (name, value) in [
            ("PATH", "x"),
            ("bo2mp_lower", "x"),
            ("BO2MP_A=B", "x"),
            ("BO2MP_NUL\0", "x"),
            ("BO2MP_OK", "two\nlines"),
            ("BO2MP_OK", "cr\rhere"),
            ("BO2MP_OK", "nul\0here"),
        ] {
            let mut manifest = sample();
            manifest["env"][name] = json!(value);
            assert!(parse(&manifest).is_err(), "{name:?}={value:?}");
        }
        let mut fine = sample();
        fine["env"]["BO2MP_EXTRA"] = json!("a = b # c");
        assert!(parse(&fine).is_ok());
    }
}
