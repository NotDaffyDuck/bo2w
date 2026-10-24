//! VPK archives: a `_dir.vpk` tree of every file, its bytes in numbered
//! `_NNN.vpk` archives beside it (or after the tree, archive 0x7fff).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SIGNATURE: u32 = 0x55aa_1234;
/// The archive index meaning "in the directory file, after the tree".
const IN_DIR: u16 = 0x7fff;

#[derive(Clone, Debug)]
struct Entry {
    preload: Vec<u8>,
    archive: u16,
    offset: u32,
    length: u32,
}

/// One opened VPK: its tree, read once.
#[derive(Debug)]
pub struct Vpk {
    dir: PathBuf,
    /// Where archive 0x7fff's bytes start in the directory file.
    data_start: u64,
    entries: HashMap<String, Entry>,
}

fn cstr(data: &[u8], at: &mut usize) -> Option<String> {
    let end = data.get(*at..)?.iter().position(|&b| b == 0)? + *at;
    let s = String::from_utf8_lossy(&data[*at..end]).into_owned();
    *at = end + 1;
    Some(s)
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The lower-case, forward-slash name a file is looked up by.
pub fn key(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

impl Vpk {
    /// Opens a `*_dir.vpk`.
    pub fn open(dir: &Path) -> Option<Self> {
        let data = std::fs::read(dir).ok()?;
        if u32_at(&data, 0)? != SIGNATURE {
            return None;
        }
        let version = u32_at(&data, 4)?;
        let tree_size = u32_at(&data, 8)? as usize;
        let header = match version {
            1 => 12,
            2 => 28,
            _ => return None,
        };
        let tree_end = header + tree_size;
        let mut at = header;
        let mut entries = HashMap::new();
        loop {
            let ext = cstr(&data, &mut at)?;
            if ext.is_empty() {
                break;
            }
            loop {
                let folder = cstr(&data, &mut at)?;
                if folder.is_empty() {
                    break;
                }
                loop {
                    let name = cstr(&data, &mut at)?;
                    if name.is_empty() {
                        break;
                    }
                    let preload_len = u16_at(&data, at + 4)? as usize;
                    let archive = u16_at(&data, at + 6)?;
                    let offset = u32_at(&data, at + 8)?;
                    let length = u32_at(&data, at + 12)?;
                    at += 18;
                    let preload = data.get(at..at + preload_len)?.to_vec();
                    at += preload_len;
                    let mut path = String::new();
                    if folder != " " {
                        path.push_str(&folder);
                        path.push('/');
                    }
                    path.push_str(&name);
                    if ext != " " {
                        path.push('.');
                        path.push_str(&ext);
                    }
                    entries.insert(key(&path), Entry { preload, archive, offset, length });
                }
            }
            if at > tree_end {
                return None;
            }
        }
        Some(Self { dir: dir.to_path_buf(), data_start: tree_end as u64, entries })
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&key(path))
    }

    /// Every file under a folder (`models/weapons/`), by its full name.
    pub fn under<'a>(&'a self, folder: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let folder = key(folder);
        self.entries.keys().filter(move |k| k.starts_with(&folder)).map(String::as_str)
    }

    /// A file's bytes.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let entry = self.entries.get(&key(path))?;
        let mut out = entry.preload.clone();
        if entry.length == 0 {
            return Some(out);
        }
        let (file, start) = if entry.archive == IN_DIR {
            (self.dir.clone(), self.data_start + u64::from(entry.offset))
        } else {
            let name = self.dir.file_name()?.to_str()?;
            let base = name.strip_suffix("_dir.vpk")?;
            (self.dir.with_file_name(format!("{base}_{:03}.vpk", entry.archive)), u64::from(entry.offset))
        };
        let mut f = File::open(file).ok()?;
        f.seek(SeekFrom::Start(start)).ok()?;
        let mut body = vec![0; entry.length as usize];
        f.read_exact(&mut body).ok()?;
        out.extend_from_slice(&body);
        Some(out)
    }
}
