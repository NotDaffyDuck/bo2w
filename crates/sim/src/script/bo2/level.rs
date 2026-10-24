//! What the map file hands the match, and the props it leaves standing.
//!
//! The entity lump is a list of key/value records. The start-up scripts never
//! read them, so the only records that matter to the match are the ones that
//! take an entity number (they count against the cap) and the authored
//! `script_model` / `script_brushmodel` records, which become [`Prop`]s whose
//! engine-side presence the script runtime keeps in step.

use std::collections::BTreeMap;
use std::sync::Arc;

/// How many entity numbers scripts may take: everything below the reserved top
/// of the table.
pub(crate) const ENTITY_CAP: usize = entity_t6::MAX_GENTITIES as usize - 64;

/// A table of strings as the map's string files store them, row after row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StringTable {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<String>,
}

impl StringTable {
    pub fn cell(&self, row: usize, column: usize) -> Option<&str> {
        if row >= self.rows || column >= self.columns {
            return None;
        }
        self.cells.get(row * self.columns + column).map(String::as_str)
    }
}

/// The type a map editor declares for an entity key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    Int,
    Float,
    String,
    Vector,
}

/// Everything the engine reads from the map and hands to the scripts.
#[derive(Clone, Debug, Default)]
pub struct LevelData {
    pub entities: Vec<Vec<(String, String)>>,
    pub tables: BTreeMap<String, StringTable>,
    pub keys: BTreeMap<String, KeyType>,
}

/// Drop `//` line comments and `/* */` blocks (a block reads as a space).
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |end| &after[end..]);
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
            out.push(' ');
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The editor's key list: pairs of `kind name`, kinds `int float string vector`.
pub fn parse_radiant_keys(text: &str) -> Result<BTreeMap<String, KeyType>, String> {
    let plain = without_comments(text);
    let mut words = plain.split_whitespace();
    let mut keys = BTreeMap::new();
    while let Some(kind) = words.next() {
        let kind = match kind {
            "int" => KeyType::Int,
            "float" => KeyType::Float,
            "string" => KeyType::String,
            "vector" => KeyType::Vector,
            other => return Err(format!("unknown key type {other}")),
        };
        let name = words.next().ok_or("missing key name")?.to_ascii_lowercase();
        if keys.insert(name.clone(), kind).is_some() {
            return Err(format!("duplicate key {name}"));
        }
    }
    Ok(keys)
}

/// The map's entity lump: `{ "key" "value" ... }` records.
pub fn parse_entity_string(text: &str) -> Vec<Vec<(String, String)>> {
    let mut records = Vec::new();
    let mut open: Option<Vec<(String, String)>> = None;
    let mut pending: Option<String> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                open = Some(Vec::new());
                pending = None;
            }
            '}' => {
                records.extend(open.take());
                pending = None;
            }
            '"' => {
                let token: String = chars.by_ref().take_while(|&c| c != '"').collect();
                match pending.take() {
                    None => pending = Some(token),
                    Some(key) => {
                        if let Some(record) = open.as_mut() {
                            record.push((key, token));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    records
}

/// The leading number of `text`, the way C's `atof` reads it.
fn leading_number(text: &str) -> f64 {
    let text = text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let bytes = text.as_bytes();
    let digits_at = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
    let mut end = usize::from(matches!(bytes.first(), Some(b'-' | b'+')));
    let mantissa = end;
    while digits_at(end) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while digits_at(end) {
            end += 1;
        }
    }
    if !text[mantissa..end].bytes().any(|b| b.is_ascii_digit()) {
        return 0.0;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut at = end + 1 + usize::from(matches!(bytes.get(end + 1), Some(b'-' | b'+')));
        if digits_at(at) {
            while digits_at(at) {
                at += 1;
            }
            end = at;
        }
    }
    text[..end].parse().unwrap_or(0.0)
}

/// `"x y z"`; parts that are missing read as 0.
fn triple(text: &str) -> [f32; 3] {
    let mut parts = text.split_ascii_whitespace().map(|part| leading_number(part) as f32);
    std::array::from_fn(|_| parts.next().unwrap_or(0.0))
}

/// A `script_model` or `script_brushmodel` the map authored.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Prop {
    pub(crate) presence: crate::ScriptModelId,
    pub(crate) origin: [f32; 3],
    pub(crate) angles: [f32; 3],
    /// The model to show; brush models (`*N`) and unset models show none.
    pub(crate) model: Option<Arc<str>>,
}

/// What reading the entity lump left behind.
pub(crate) struct Spawned {
    pub(crate) props: Vec<Prop>,
    /// Entity numbers the lump took.
    pub(crate) entities: usize,
}

fn read_prop(ordinal: usize, record: &[(String, String)]) -> Option<Prop> {
    let mut origin = [0.0; 3];
    let mut angles = [0.0; 3];
    let mut model = None;
    for (key, value) in record {
        match key.to_ascii_lowercase().as_str() {
            "origin" => origin = triple(value),
            "angles" => angles = triple(value),
            "angle" => angles = [0.0, leading_number(value) as f32, 0.0],
            "model" => model = Some(value.as_str()),
            _ => {}
        }
    }
    Some(Prop {
        presence: crate::ScriptModelId::from_authored_source_ordinal(u32::try_from(ordinal).ok()?),
        origin,
        angles,
        model: model.filter(|m| !m.is_empty() && !m.starts_with('*')).map(Arc::from),
    })
}

/// Walk the lump: count what takes an entity number, keep the props.
pub(crate) fn spawn_map(level: &LevelData) -> Result<Spawned, String> {
    let mut taken = 0usize;
    let mut props = Vec::new();
    for (ordinal, record) in level.entities.iter().enumerate() {
        let class = record
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("classname"))
            .map_or("", |(_, value)| value.as_str());
        // Nodes and the structs scripts walk (`script_struct`) take no number;
        // the world record takes one and is not a prop.
        if class != "worldspawn"
            && (class.is_empty() || class.starts_with("node_") || class == "script_struct")
        {
            continue;
        }
        if taken >= ENTITY_CAP {
            return Err("no free entities".into());
        }
        taken += 1;
        if matches!(class, "script_model" | "script_brushmodel") {
            props.extend(read_prop(ordinal, record));
        }
    }
    Ok(Spawned { props, entities: taken })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn key_lists_skip_comments_and_refuse_repeats() {
        let keys = parse_radiant_keys("// header\nint Count /* note */ float speed\nvector origin string target").unwrap();
        assert_eq!(keys["count"], KeyType::Int);
        assert_eq!(keys["speed"], KeyType::Float);
        assert_eq!(keys["origin"], KeyType::Vector);
        assert_eq!(keys["target"], KeyType::String);
        assert_eq!(parse_radiant_keys("bool x").unwrap_err(), "unknown key type bool");
        assert_eq!(parse_radiant_keys("int").unwrap_err(), "missing key name");
        assert_eq!(parse_radiant_keys("int a float A").unwrap_err(), "duplicate key a");
    }

    #[test]
    fn the_lump_reads_back_as_records() {
        let lump = "{ \"classname\" \"worldspawn\" }\n{ \"classname\" \"script_model\" \"origin\" \"1 2 3\" }\n{ \"stray\" }";
        let records = parse_entity_string(lump);
        assert_eq!(records.len(), 3);
        assert!(records[2].is_empty());
        assert_eq!(records[1], record(&[("classname", "script_model"), ("origin", "1 2 3")]));
    }

    #[test]
    fn numbers_read_like_c() {
        assert_eq!(leading_number("  12.5abc"), 12.5);
        assert_eq!(leading_number("-3e2"), -300.0);
        assert_eq!(leading_number("e5"), 0.0);
        assert_eq!(leading_number("4e"), 4.0);
        assert_eq!(triple("1 2"), [1.0, 2.0, 0.0]);
    }

    #[test]
    fn only_numbered_records_count_and_only_models_are_props() {
        let level = LevelData {
            entities: vec![
                record(&[("classname", "worldspawn")]),
                record(&[("classname", "node_pathnode")]),
                record(&[("classname", "script_struct")]),
                record(&[("targetname", "no class")]),
                record(&[
                    ("classname", "script_model"),
                    ("origin", "10 20 30"),
                    ("angle", "90"),
                    ("model", "defaultvehicle"),
                ]),
                record(&[("classname", "script_brushmodel"), ("model", "*7")]),
                record(&[("classname", "trigger_radius")]),
            ],
            ..LevelData::default()
        };
        let spawned = spawn_map(&level).unwrap();
        assert_eq!(spawned.entities, 4);
        assert_eq!(spawned.props.len(), 2);
        assert_eq!(spawned.props[0].origin, [10.0, 20.0, 30.0]);
        assert_eq!(spawned.props[0].angles, [0.0, 90.0, 0.0]);
        assert_eq!(spawned.props[0].model.as_deref(), Some("defaultvehicle"));
        assert_eq!(spawned.props[1].model, None);
        assert_eq!(
            spawned.props[1].presence,
            crate::ScriptModelId::from_authored_source_ordinal(5)
        );
    }

    #[test]
    fn a_full_table_refuses_the_next_record() {
        let level = LevelData {
            entities: vec![record(&[("classname", "info_null")]); ENTITY_CAP + 1],
            ..LevelData::default()
        };
        assert_eq!(spawn_map(&level).err().as_deref(), Some("no free entities"));
    }
}
