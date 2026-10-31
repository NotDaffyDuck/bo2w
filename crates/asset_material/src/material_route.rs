// Material draw route and colour-pass state-bit checks for the renderer's
// material catalog, in BO2's technique numbering (a BO2 MaterialTechniqueSet
// has 36 technique slots).

pub const RENDER_RECIPE_SLOT_TOTAL: usize = 36;

/// BO2's emissive technique (facts: dpvs-drawsurf-t6.md, the sort order).
pub const TECHNIQUE_EMISSIVE: usize = 3;

/// BO2's lit technique (facts: dpvs-drawsurf-t6.md, the sort order).
pub const TECHNIQUE_LIT: usize = 4;

/// The first colour technique: BO2's unlit, 2 (then emissive 3, lit 4, lit
/// with sun 5, lit with sun shadow 6, as `t6_materials` reads a material's
/// main technique). TODO(fact): 2, 5 and 6, and where BO2's colour techniques
/// end, are not in the facts yet.
pub const TECHNIQUE_COLOR_BAND_FIRST: usize = 2;

// BO2 has no sort key that marks sky: the skybox material of zm_nuked sorts 4, the same as
// every opaque lit material, so a route cannot tell sky from the rest by its sort key.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawPass {
    /// TODO(fact): BO2's shadow-caster technique is not in the facts, so
    /// `route` gives no material this pass yet.
    DepthShadow,

    Lit,

    Unlit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortRoute {
    pub leading_sort_bucket: u8,

    pub early_pass: u8,

    pub own_slot: u8,

    pub draw_pass: DrawPass,

    pub technique_mask: u64,

    pub uses_model_illum_const: bool,
}

impl SortRoute {

    pub const fn route(sort_key: u8, pre: u8, custom: u8, slots: u64, model_light: bool) -> Self {
        let lit = (slots >> TECHNIQUE_LIT) & 1 == 1;
        Self {
            leading_sort_bucket: sort_key & 0x3f,
            early_pass: pre,
            own_slot: custom,
            draw_pass: if lit { DrawPass::Lit } else { DrawPass::Unlit },
            technique_mask: slots,
            uses_model_illum_const: model_light,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPassVerdict<T> {
    Agreed(T),

    Split { distinct: u8 },

    SlotTableUnread,

    SlotBeyondTable { slot_entry: u8, table_len: u8 },

    NoColorPass,

    NothingThere,
}

impl<T> ColorPassVerdict<T> {
    #[inline]
    pub fn consensus_reached(self) -> Option<T> {
        match self {
            Self::Agreed(found) => Some(found),
            _ => None,
        }
    }
}

fn slot_is_set(mask: u64, bit: usize) -> bool {
    mask >> bit & 1 != 0
}

pub fn judge_color_passes<T: PartialEq>(
    per_slot: Option<&[u8; RENDER_RECIPE_SLOT_TOTAL]>,
    entries: &[[u32; 2]],
    slots: Option<u64>,
    read_bits: impl Fn([u32; 2]) -> T,
) -> ColorPassVerdict<T> {
    if entries.first().is_none() {
        return ColorPassVerdict::NothingThere;
    }
    let mask = slots.unwrap_or(u64::MAX);
    if mask >> TECHNIQUE_COLOR_BAND_FIRST == 0 {
        return ColorPassVerdict::NoColorPass;
    }
    let Some(per_slot) = per_slot else {
        return match entries {
            [only] => ColorPassVerdict::Agreed(read_bits(*only)),
            _ => ColorPassVerdict::SlotTableUnread,
        };
    };
    let len = u8::try_from(entries.len()).unwrap_or(u8::MAX);
    // Each different value the set colour slots decode to, first seen first.
    let mut values: Vec<T> = vec![];
    for slot in (TECHNIQUE_COLOR_BAND_FIRST..RENDER_RECIPE_SLOT_TOTAL).filter(|&s| slot_is_set(mask, s)) {
        let entry = per_slot[slot];
        let Some(&raw) = entries.get(usize::from(entry)) else {
            return ColorPassVerdict::SlotBeyondTable { slot_entry: entry, table_len: len };
        };
        let value = read_bits(raw);
        if !values.contains(&value) {
            values.push(value);
        }
    }
    match values.len() {
        0 => ColorPassVerdict::NoColorPass,
        1 => ColorPassVerdict::Agreed(values.swap_remove(0)),
        n => ColorPassVerdict::Split { distinct: n as u8 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(raw: [u32; 2]) -> u32 {
        raw[0]
    }

    #[test]
    fn a_route_masks_its_sort_key_and_reads_lit_from_its_slots() {
        let lit = SortRoute::route(0xff, 1, 2, 1 << TECHNIQUE_LIT, true);
        assert_eq!((lit.leading_sort_bucket, lit.early_pass, lit.own_slot), (0x3f, 1, 2));
        assert_eq!(lit.draw_pass, DrawPass::Lit);
        assert_eq!((lit.technique_mask, lit.uses_model_illum_const), (16, true));
        let unlit = SortRoute::route(4, 0, 0, 1 << TECHNIQUE_EMISSIVE, false);
        assert_eq!(unlit.draw_pass, DrawPass::Unlit);
        assert_eq!(unlit.leading_sort_bucket, 4);
    }

    #[test]
    fn only_an_agreement_gives_a_value() {
        assert_eq!(ColorPassVerdict::Agreed(5).consensus_reached(), Some(5));
        assert_eq!(ColorPassVerdict::<u8>::NothingThere.consensus_reached(), None);
        assert_eq!(ColorPassVerdict::<u8>::Split { distinct: 2 }.consensus_reached(), None);
    }

    #[test]
    fn colour_passes_agree_or_say_why_not() {
        let table = [[1, 0], [2, 0], [1, 9]];
        let mut rows = [0u8; RENDER_RECIPE_SLOT_TOTAL];
        let slots = Some((1u64 << 2) | (1 << 4) | (1 << 5));
        assert_eq!(judge_color_passes(Some(&rows), &[], slots, pick), ColorPassVerdict::NothingThere);
        assert_eq!(judge_color_passes(Some(&rows), &table, Some(0b11), pick), ColorPassVerdict::NoColorPass);
        assert_eq!(judge_color_passes(Some(&rows), &table, Some(0), pick), ColorPassVerdict::NoColorPass);
        // No per-slot rows: only a one-row table can be trusted.
        assert_eq!(judge_color_passes(None, &table[..1], slots, pick), ColorPassVerdict::Agreed(1));
        assert_eq!(judge_color_passes(None, &table, slots, pick), ColorPassVerdict::SlotTableUnread);
        // Every set slot on rows that decode alike (0 and 2 both give 1).
        rows[4] = 2;
        assert_eq!(judge_color_passes(Some(&rows), &table, slots, pick), ColorPassVerdict::Agreed(1));
        rows[5] = 1;
        assert_eq!(judge_color_passes(Some(&rows), &table, slots, pick), ColorPassVerdict::Split { distinct: 2 });
        rows[5] = 7;
        assert_eq!(
            judge_color_passes(Some(&rows), &table, slots, pick),
            ColorPassVerdict::SlotBeyondTable { slot_entry: 7, table_len: 3 }
        );
        // Unset slots are never read, and no mask means every slot counts.
        rows[2] = 1;
        rows[3] = 9;
        let two_alike = Some((1u64 << 3) | (1 << 2));
        assert_eq!(judge_color_passes(Some(&rows), &table, two_alike, pick), ColorPassVerdict::SlotBeyondTable { slot_entry: 9, table_len: 3 });
        let none_set = Some(1u64 << 40);
        assert_eq!(judge_color_passes(Some(&rows), &table, none_set, pick), ColorPassVerdict::NoColorPass);
        let all = [0u8; RENDER_RECIPE_SLOT_TOTAL];
        assert_eq!(judge_color_passes(Some(&all), &table, None, pick), ColorPassVerdict::Agreed(1));
    }
}
