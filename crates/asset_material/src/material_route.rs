// Material draw route and colour-pass state-bit checks for the renderer's
// material catalog, in BO2's technique numbering (a BO2 MaterialTechniqueSet
// has 36 technique slots).

pub const TECHNIQUE_SLOT_COUNT: usize = 36;

/// BO2's emissive technique (facts: dpvs-drawsurf-t6.md, the sort order).
pub const TECHNIQUE_EMISSIVE: usize = 3;

/// BO2's lit technique (facts: dpvs-drawsurf-t6.md, the sort order).
pub const TECHNIQUE_LIT: usize = 4;

/// The first colour technique: BO2's unlit, 2 (then emissive 3, lit 4, lit
/// with sun 5, lit with sun shadow 6, as `t6_materials` reads a material's
/// main technique). TODO(fact): 2, 5 and 6, and where BO2's colour techniques
/// end, are not in the facts yet.
pub const TECHNIQUE_COLOR_BAND_FIRST: usize = 2;

pub const SORT_KEY_SKY: u8 = 2;

pub const SORT_KEY_SKYBOX: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialPass {
    /// TODO(fact): BO2's shadow-caster technique is not in the facts, so
    /// `route` gives no material this pass yet.
    ShadowOnly,

    Sky(u8),

    Lit,

    Unlit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialDrawRoute {
    pub primary_sort_key: u8,

    pub prepass: u8,

    pub custom_index: u8,

    pub pass: MaterialPass,

    pub technique_slots: u64,

    pub uses_model_lighting_const: bool,
}

impl MaterialDrawRoute {

    pub const fn route(
        primary_sort_key: u8,
        prepass: u8,
        custom_index: u8,
        technique_slots: u64,
        uses_model_lighting_const: bool,
    ) -> Self {
        let bucket = primary_sort_key & 0x3f;
        let pass = if bucket == SORT_KEY_SKY || bucket == SORT_KEY_SKYBOX {
            MaterialPass::Sky(bucket)
        } else if (technique_slots >> TECHNIQUE_LIT) & 1 != 0 {
            MaterialPass::Lit
        } else {
            MaterialPass::Unlit
        };
        Self {
            primary_sort_key: bucket,
            prepass,
            custom_index,
            pass,
            technique_slots,
            uses_model_lighting_const,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPassAgreement<T> {
    Agreed(T),

    Disagrees { distinct: u8 },

    EntryTableNotWalked,

    EntryOutOfRange { entry: u8, len: u8 },

    NoColorTechnique,

    Absent,
}

impl<T> ColorPassAgreement<T> {
    #[inline]
    pub fn agreed(self) -> Option<T> {
        match self {
            Self::Agreed(value) => Some(value),
            _ => None,
        }
    }
}

pub fn color_pass_agreement<T: PartialEq>(
    state_bits_entry: Option<&[u8; TECHNIQUE_SLOT_COUNT]>,
    table: &[[u32; 2]],
    technique_slots: Option<u64>,
    decode: impl Fn([u32; 2]) -> T,
) -> ColorPassAgreement<T> {
    if table.is_empty() {
        return ColorPassAgreement::Absent;
    }
    let slots = technique_slots.unwrap_or(u64::MAX);
    if slots >> TECHNIQUE_COLOR_BAND_FIRST == 0 {
        return ColorPassAgreement::NoColorTechnique;
    }
    let out_of_range = |entry: u8| ColorPassAgreement::EntryOutOfRange {
        entry,
        len: table.len().min(u8::MAX as usize) as u8,
    };
    let Some(entries) = state_bits_entry else {
        return match table {
            [only] if table.len() == 1 => ColorPassAgreement::Agreed(decode(*only)),
            _ => ColorPassAgreement::EntryTableNotWalked,
        };
    };
    let mut answer = None;
    let mut distinct = 0u8;
    for slot in TECHNIQUE_COLOR_BAND_FIRST..TECHNIQUE_SLOT_COUNT {
        if (slots >> slot) & 1 == 0 {
            continue;
        }
        let entry = entries[slot];
        let Some(row) = table.get(entry as usize) else {
            return out_of_range(entry);
        };
        let decoded = decode(*row);

        if !colour_band_saw(entries, table, slots, slot, &decoded, &decode) {
            distinct = distinct.saturating_add(1);
        }
        if answer.is_none() {
            answer = Some(decoded);
        }
    }
    match answer {
        Some(value) if distinct <= 1 => ColorPassAgreement::Agreed(value),
        Some(_) => ColorPassAgreement::Disagrees { distinct },
        None => ColorPassAgreement::NoColorTechnique,
    }
}

fn colour_band_saw<T: PartialEq>(
    entries: &[u8; TECHNIQUE_SLOT_COUNT],
    table: &[[u32; 2]],
    slots: u64,
    before: usize,
    decoded: &T,
    decode: &impl Fn([u32; 2]) -> T,
) -> bool {
    for slot in TECHNIQUE_COLOR_BAND_FIRST..before {
        if (slots >> slot) & 1 == 0 {
            continue;
        }
        if let Some(row) = table.get(entries[slot] as usize)
            && decode(*row) == *decoded
        {
            return true;
        }
    }
    false
}
