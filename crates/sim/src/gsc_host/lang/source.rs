//! Where script text comes from.

use std::collections::BTreeMap;

/// Hands out a script module's text by name (`maps/mp/gametypes/dm`).
pub trait SourceResolver {
    fn read(&self, module: &str) -> Result<String, String>;

    /// The module's raw bytes, which is what its identity hashes.
    fn read_bytes(&self, module: &str) -> Result<Vec<u8>, String> {
        self.read(module).map(String::into_bytes)
    }
}

impl SourceResolver for BTreeMap<String, String> {
    fn read(&self, module: &str) -> Result<String, String> {
        self.get(module)
            .cloned()
            .ok_or_else(|| format!("missing script module {module}"))
    }
}

/// UTF-8 when it is, else one character per byte (Latin-1).
pub fn decode_source(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// A module name in its one canonical form: forward slashes, lower case, no
/// `.gsc`. Anything that could climb out of the script root is refused.
pub fn normalize_module(module: &str) -> Result<String, String> {
    let flat = module.replace('\\', "/").to_ascii_lowercase();
    let name = flat.strip_suffix(".gsc").unwrap_or(&flat);
    let sound = !name.is_empty()
        && name.split('/').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    if sound {
        Ok(name.to_owned())
    } else {
        Err(format!("invalid script module {module:?}"))
    }
}
