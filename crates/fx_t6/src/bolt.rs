//! The packed bolt / sort-order word on each BO2 effect.
//!
//! Bolted form: sort order 8 bits, type 2, dobj handle 11, temporal 1,
//! bone index 9. Dynent form: sort order 8, type 2, debris_bit id 21.
// BO2 unknown: the facts give the fields and widths but not their bit order.
// Packed here in the listed order from bit 0 (see facts/QUESTIONS.md).

pub const FX_BOLT_SORT_BITS: u32 = 8;
pub const FX_BOLT_TYPE_SHIFT: u32 = 8;
pub const FX_BOLT_TYPE_BITS: u32 = 2;
pub const FX_BOLT_DOBJ_SHIFT: u32 = 10;
pub const FX_BOLT_DOBJ_BITS: u32 = 11;
pub const FX_BOLT_TEMPORAL_SHIFT: u32 = 21;
pub const ATTACH_JOINT_BIT_OFFSET: u32 = 22;
pub const FX_BOLT_BONE_BITS: u32 = 9;
pub const FX_BOLT_DYNENT_SHIFT: u32 = 10;
pub const FX_BOLT_DYNENT_BITS: u32 = 21;

pub const FX_BOLT_DOBJ_NONE: u32 = 0x7ff;
pub const FX_BOLT_BONE_NONE: u32 = 0x1ff;
pub const FX_BOLT_MARK_ENTITY_NONE: u32 = 0x3ff;
pub const FX_BOLT_MARK_ENTITY_WORLD: u32 = 0x3fe;
pub const FX_BOLT_SORT_DEFAULT: u8 = 0xff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FxBoltType {
    None = 0,
    Bolted = 1,
    Dynent = 2,
    MarkEntity = 3,
}

impl FxBoltType {
    #[inline]
    pub fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => FxBoltType::None,
            1 => FxBoltType::Bolted,
            2 => FxBoltType::Dynent,
            _ => FxBoltType::MarkEntity,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxBolt {
    None,
    Bolted {
        dobj: u32,
        temporal: bool,
        bone: u32,
    },
    Dynent {
        id: u32,
    },
    /// The mark-entity form's fields are not in the facts; the bits above the
    /// type are kept as they are.
    MarkEntity {
        raw: u32,
    },
}

#[inline]
const fn mask(bits: u32) -> u32 {
    (1 << bits) - 1
}

#[inline]
pub fn bolt_sort_order(word: u32) -> u8 {
    (word & mask(FX_BOLT_SORT_BITS)) as u8
}

#[inline]
pub fn bolt_type(word: u32) -> FxBoltType {
    FxBoltType::from_bits(word >> FX_BOLT_TYPE_SHIFT)
}

pub fn pack_bolt(paint_rank: u8, bolt: FxBolt) -> u32 {
    let sort = u32::from(paint_rank);
    let ty = |t: FxBoltType| (t as u32) << FX_BOLT_TYPE_SHIFT;
    match bolt {
        FxBolt::None => sort | ty(FxBoltType::None),
        FxBolt::Bolted {
            dobj,
            temporal,
            bone,
        } => {
            sort | ty(FxBoltType::Bolted)
                | ((dobj & mask(FX_BOLT_DOBJ_BITS)) << FX_BOLT_DOBJ_SHIFT)
                | (u32::from(temporal) << FX_BOLT_TEMPORAL_SHIFT)
                | ((bone & mask(FX_BOLT_BONE_BITS)) << ATTACH_JOINT_BIT_OFFSET)
        }
        FxBolt::Dynent { id } => {
            sort | ty(FxBoltType::Dynent)
                | ((id & mask(FX_BOLT_DYNENT_BITS)) << FX_BOLT_DYNENT_SHIFT)
        }
        FxBolt::MarkEntity { raw } => {
            sort | ty(FxBoltType::MarkEntity) | (raw << FX_BOLT_DOBJ_SHIFT)
        }
    }
}

pub fn unpack_bolt(word: u32) -> FxBolt {
    match bolt_type(word) {
        FxBoltType::None => FxBolt::None,
        FxBoltType::Bolted => FxBolt::Bolted {
            dobj: (word >> FX_BOLT_DOBJ_SHIFT) & mask(FX_BOLT_DOBJ_BITS),
            temporal: (word >> FX_BOLT_TEMPORAL_SHIFT) & 1 != 0,
            bone: (word >> ATTACH_JOINT_BIT_OFFSET) & mask(FX_BOLT_BONE_BITS),
        },
        FxBoltType::Dynent => FxBolt::Dynent {
            id: (word >> FX_BOLT_DYNENT_SHIFT) & mask(FX_BOLT_DYNENT_BITS),
        },
        FxBoltType::MarkEntity => FxBolt::MarkEntity {
            raw: word >> FX_BOLT_DOBJ_SHIFT,
        },
    }
}

/// The word of an effect bolted to nothing, with the default sort order.
pub const FX_BOLT_WORD_UNBOLTED: u32 = FX_BOLT_SORT_DEFAULT as u32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_fill_31_bits() {
        assert_eq!(ATTACH_JOINT_BIT_OFFSET + FX_BOLT_BONE_BITS, 31);
        assert_eq!(FX_BOLT_DYNENT_SHIFT + FX_BOLT_DYNENT_BITS, 31);
        assert_eq!(FX_BOLT_DOBJ_NONE, mask(FX_BOLT_DOBJ_BITS));
        assert_eq!(FX_BOLT_BONE_NONE, mask(FX_BOLT_BONE_BITS));
    }

    #[test]
    fn round_trips() {
        let cases = [
            FxBolt::None,
            FxBolt::Bolted {
                dobj: FX_BOLT_DOBJ_NONE,
                temporal: true,
                bone: FX_BOLT_BONE_NONE,
            },
            FxBolt::Bolted {
                dobj: 17,
                temporal: false,
                bone: 3,
            },
            FxBolt::Dynent { id: 0x1f_ffff },
            FxBolt::MarkEntity {
                raw: FX_BOLT_MARK_ENTITY_WORLD,
            },
        ];
        for b in cases {
            let w = pack_bolt(42, b);
            assert_eq!(unpack_bolt(w), b);
            assert_eq!(bolt_sort_order(w), 42);
        }
        assert_eq!(unpack_bolt(FX_BOLT_WORD_UNBOLTED), FxBolt::None);
        assert_eq!(bolt_sort_order(FX_BOLT_WORD_UNBOLTED), 0xff);
    }
}
