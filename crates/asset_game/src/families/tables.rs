//! Reading the authored tables: which attachments exist, which pairs clash,
//! and which families the stats table lists.

use std::collections::{BTreeMap, HashSet};

use super::{AttachmentChoice, FamilyContent, FamilyKey, FamilySlot, WeaponFamilies, WeaponFamily};
use crate::cac::stats_col;
use crate::{AssetNamespace, CacAuthoredCategory, CacOffhandBucket, CapturedStringTable};

/// Columns of `attachmentTable.csv`.
mod attach_col {
    pub const POINT: i32 = 2;
    pub const CAPTION: i32 = 3;
    pub const NAME: i32 = 4;
    pub const ICON: i32 = 6;
    pub const DESC: i32 = 7;
}

/// The stats table lists a weapon's attachment names in these columns.
const OFFERED_COLS: std::ops::RangeInclusive<i32> = 11..=21;

/// Everything the attachment tables say about one game.
#[derive(Clone, Debug, Default)]
pub(super) struct AttachmentRules {
    pub(super) known: BTreeMap<String, AttachmentChoice>,
    /// Unordered pairs that cannot share a weapon, stored smaller name first.
    clashes: HashSet<(String, String)>,
}

impl AttachmentRules {
    pub(super) fn allows_together(&self, a: &str, b: &str) -> bool {
        !self.clashes.contains(&pair_of(a, b))
    }

    fn read_attachments(&mut self, table: &CapturedStringTable) {
        for row in 1..table.rows as i32 {
            let name = table.cell(row, attach_col::NAME).to_ascii_lowercase();
            let point = table.cell(row, attach_col::POINT);
            // A "none" attachment point marks a bookkeeping row, not an attachment.
            if name.is_empty() || point == "none" {
                continue;
            }
            self.known.entry(name.clone()).or_insert_with(|| AttachmentChoice {
                name,
                point: point.to_owned(),
                caption_key: table.cell(row, attach_col::CAPTION).to_owned(),
                icon: table.cell(row, attach_col::ICON).to_owned(),
                desc_key: table.cell(row, attach_col::DESC).to_owned(),
            });
        }
    }

    /// The combo table is a square grid; a "no" cell bans the row's name with
    /// the name that heads the cell's column.
    fn read_clashes(&mut self, table: &CapturedStringTable) {
        for row in 1..table.rows as i32 {
            let left = table.cell(row, 0).to_ascii_lowercase();
            for col in 1..table.columns as i32 {
                if !table.cell(row, col).eq_ignore_ascii_case("no") {
                    continue;
                }
                let right = table.cell(col, 0).to_ascii_lowercase();
                if !left.is_empty() && !right.is_empty() && left != right {
                    self.clashes.insert(pair_of(&left, &right));
                }
            }
        }
    }
}

fn pair_of(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

fn table_is(table: &CapturedStringTable, authored_name: &str) -> bool {
    table.name.eq_ignore_ascii_case(authored_name)
}

fn slot_of(category: Option<CacAuthoredCategory>, offhand: Option<CacOffhandBucket>) -> FamilySlot {
    use CacAuthoredCategory as Cat;
    match category {
        Some(Cat::Pistol | Cat::MachinePistol | Cat::Projectile | Cat::Special) => FamilySlot::Secondary,
        Some(_) => FamilySlot::Primary,
        None => match offhand {
            Some(CacOffhandBucket::Lethal) => FamilySlot::Lethal,
            Some(CacOffhandBucket::Tactical) => FamilySlot::Tactical,
            None => FamilySlot::Other,
        },
    }
}

impl WeaponFamilies {
    /// The first attachment table and the first combo table of each game win.
    pub(super) fn read_attachment_tables(&mut self, tables: &[(AssetNamespace, CapturedStringTable)]) {
        for (namespace, table) in tables {
            if table_is(table, "mp/attachmentTable.csv") && !self.rules.contains_key(namespace) {
                self.rules.entry(*namespace).or_default().read_attachments(table);
            }
        }
        for (namespace, table) in tables {
            if table_is(table, "mp/attachmentCombos.csv") {
                let rules = self.rules.entry(*namespace).or_default();
                if rules.clashes.is_empty() {
                    rules.read_clashes(table);
                }
            }
        }
    }

    /// One family per weapon row of a stats table; a base seen before is kept
    /// as first read.
    pub(super) fn read_family_rows(
        &mut self,
        namespace: AssetNamespace,
        table: &CapturedStringTable,
        content: &impl FamilyContent,
    ) {
        let rules = self.rules.remove(&namespace).unwrap_or_default();
        for row in 1..table.rows as i32 {
            let group = table.cell(row, stats_col::GROUP);
            let base = table.cell(row, stats_col::REF);
            if !group.starts_with("weapon_") || base.is_empty() || base == "weapon_null" {
                continue;
            }
            let key = FamilyKey::new(namespace, base);
            if rules.known.contains_key(&key.base) || self.index.contains_key(&key) {
                continue;
            }
            let base_id = content.lookup(namespace, &key.base);
            let offhand = base_id
                .map(|id| content.offhand_class(id))
                .and_then(crate::cac_offhand_bucket);
            let slot = slot_of(crate::cac_category_from_item_group(group), offhand);

            // Alphabetical, each attachment once.
            let mut attachments: Vec<AttachmentChoice> = OFFERED_COLS
                .map(|col| table.cell(row, col).to_ascii_lowercase())
                .filter_map(|name| rules.known.get(&name).cloned())
                .collect();
            attachments.sort_by(|a, b| a.name.cmp(&b.name));
            attachments.dedup_by(|a, b| a.name == b.name);

            let left_out = match base_id {
                None => Some(format!("authored base `{}` is not in the loaded content", key.base)),
                Some(id) => match content.admission(id) {
                    Err(refusal) => Some(refusal.to_string()),
                    Ok(()) if slot == FamilySlot::Other => Some(format!("`{group}` is not a class slot")),
                    Ok(()) => None,
                },
            };
            if let Some(reason) = left_out {
                self.excluded.push((key.asset_key(), reason));
            }
            self.index.insert(key.clone(), self.families.len());
            self.families.push(WeaponFamily {
                key,
                item_group: group.to_owned(),
                slot,
                display_key: table.cell(row, stats_col::NAME).to_owned(),
                image: table.cell(row, stats_col::IMAGE).to_owned(),
                base: base_id,
                attachments,
            });
        }
        self.rules.insert(namespace, rules);
    }
}
