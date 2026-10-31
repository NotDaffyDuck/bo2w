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
    pub cols: usize,
    pub lines: usize,
    pub squares: Vec<String>,
}

impl StringTable {
    pub fn text_at(&self, row: usize, column: usize) -> Option<&str> {
        if row >= self.lines || column >= self.cols {
            return None;
        }
        self.squares.get(row * self.cols + column).map(String::as_str)
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
    const KINDS: [(&str, KeyType); 4] = [
        ("int", KeyType::Int),
        ("float", KeyType::Float),
        ("string", KeyType::String),
        ("vector", KeyType::Vector),
    ];
    let plain = without_comments(text);
    let words: Vec<&str> = plain.split_whitespace().collect();
    let mut table = BTreeMap::new();
    for pair in words.chunks(2) {
        let kind = KINDS
            .iter()
            .find(|(word, _)| *word == pair[0])
            .map(|&(_, kind)| kind)
            .ok_or_else(|| format!("unknown key type {}", pair[0]))?;
        let Some(name) = pair.get(1).map(|word| word.to_ascii_lowercase()) else {
            return Err("missing key name".to_owned());
        };
        if table.contains_key(&name) {
            return Err(format!("duplicate key {name}"));
        }
        table.insert(name, kind);
    }
    Ok(table)
}

/// The marks the entity lump is made of.
enum Mark {
    Open,
    Close,
    Quoted(String),
}

/// The lump's braces and quoted strings, in order; anything else is skipped.
/// A quote left open runs to the end of the text.
fn marks(text: &str) -> impl Iterator<Item = Mark> + '_ {
    let mut remainder = text;
    std::iter::from_fn(move || {
        let at = remainder.find(['{', '}', '"'])?;
        let sign = remainder.as_bytes()[at];
        remainder = &remainder[at + 1..];
        Some(match sign {
            b'{' => Mark::Open,
            b'}' => Mark::Close,
            _ => {
                let (inside, later) = remainder.split_once('"').unwrap_or((remainder, ""));
                remainder = later;
                Mark::Quoted(inside.to_owned())
            }
        })
    })
}

/// The map's entity lump: `{ "key" "value" ... }` records. A brace drops a
/// key still waiting for its value; pairs outside braces are lost.
pub fn parse_entity_string(text: &str) -> Vec<Vec<(String, String)>> {
    let mut finished = Vec::new();
    let mut current: Option<Vec<(String, String)>> = None;
    let mut key_waiting: Option<String> = None;
    for mark in marks(text) {
        let word = match mark {
            Mark::Open => {
                current = Some(Vec::new());
                key_waiting = None;
                continue;
            }
            Mark::Close => {
                finished.extend(current.take());
                key_waiting = None;
                continue;
            }
            Mark::Quoted(word) => word,
        };
        let Some(key) = key_waiting.take() else {
            key_waiting = Some(word);
            continue;
        };
        if let Some(open) = current.as_mut() {
            open.push((key, word));
        }
    }
    finished
}

/// The leading number of `text`, the way C's `atof` reads it.
fn leading_number(words: &str) -> f64 {
    let words = words.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let bytes = words.as_bytes();
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
    if !words[mantissa..end].bytes().any(|b| b.is_ascii_digit()) {
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
    words[..end].parse().unwrap_or(0.0)
}

/// `"x y z"`; parts that are missing read as 0.
fn triple(text: &str) -> [f32; 3] {
    let mut parts = text.split_ascii_whitespace().map(|part| leading_number(part) as f32);
    std::array::from_fn(|_| parts.next().unwrap_or(0.0))
}

/// A `script_model` or `script_brushmodel` the map authored.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Prop {
    pub(crate) presence: crate::ScriptedModel,
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
    // The last key of a kind wins; `angle` (yaw only) and `angles` share one.
    let newest = |pick: &dyn Fn(&str, &str) -> Option<[f32; 3]>| {
        record.iter().rev().find_map(|(field, value)| pick(&field.to_ascii_lowercase(), value))
    };
    let origin = newest(&|field, value| (field == "origin").then(|| triple(value)));
    let angles = newest(&|field, value| match field {
        "angles" => Some(triple(value)),
        "angle" => Some([0.0, leading_number(value) as f32, 0.0]),
        _ => None,
    });
    let shown = record
        .iter()
        .rev()
        .find(|(field, _)| field.eq_ignore_ascii_case("model"))
        .map(|(_, value)| value.as_str())
        .filter(|name| !name.is_empty() && !name.starts_with('*'));
    let presence = crate::ScriptedModel::from_authored_source_ordinal(u32::try_from(ordinal).ok()?);
    Some(Prop {
        presence,
        origin: origin.unwrap_or_default(),
        angles: angles.unwrap_or_default(),
        model: shown.map(Arc::from),
    })
}

/// Walk the lump: count what takes an entity number, keep the props.
pub(crate) fn spawn_map(level: &LevelData) -> Result<Spawned, String> {
    let mut taken = 0usize;
    let mut props = Vec::new();
    for (rank, record) in level.entities.iter().enumerate() {
        let class = record
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case("classname"))
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
            props.extend(read_prop(rank, record));
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
            crate::ScriptedModel::from_authored_source_ordinal(5)
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
