//! The 60 T6 PC asset pools, in pool-id order. Names are the ones the
//! game's own asset tools print, so a census can be compared against them.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u32)]
pub enum XAssetKind {
    XModelPieces,
    PhysPreset,
    PhysConstraints,
    Destructible,
    XAnimParts,
    XModel,
    Material,
    TechniqueSet,
    Image,
    Sound,
    SoundPatch,
    ClipMap,
    ClipMapPvs,
    ComWorld,
    GameWorldSp,
    GameWorldMp,
    MapEnts,
    GfxWorld,
    LightDef,
    UiMap,
    Font,
    FontIcon,
    MenuList,
    Menu,
    Localize,
    Weapon,
    WeaponDef,
    WeaponVariant,
    WeaponFull,
    Attachment,
    AttachmentUnique,
    WeaponCamo,
    SndDriverGlobals,
    Fx,
    ImpactFx,
    AiType,
    MpType,
    MpBody,
    MpHead,
    Character,
    XModelAlias,
    RawFile,
    StringTable,
    Leaderboard,
    XGlobals,
    Ddl,
    Glasses,
    EmblemSet,
    ScriptParseTree,
    KeyValuePairs,
    VehicleDef,
    MemoryBlock,
    AddonMapEnts,
    Tracer,
    SkinnedVerts,
    Qdb,
    Slug,
    FootstepTable,
    FootstepFxTable,
    ZBarrier,
}

pub const ASSET_TYPE_COUNT: usize = 0x3C;

/// Every pool as (type, printed name); position in the table is the pool id.
const POOLS: [(XAssetKind, &str); ASSET_TYPE_COUNT] = [
    (XAssetKind::XModelPieces, "xmodelpieces"),
    (XAssetKind::PhysPreset, "physpreset"),
    (XAssetKind::PhysConstraints, "physconstraints"),
    (XAssetKind::Destructible, "destructibledef"),
    (XAssetKind::XAnimParts, "xanim"),
    (XAssetKind::XModel, "xmodel"),
    (XAssetKind::Material, "material"),
    (XAssetKind::TechniqueSet, "techniqueset"),
    (XAssetKind::Image, "image"),
    (XAssetKind::Sound, "soundbank"),
    (XAssetKind::SoundPatch, "soundpatch"),
    (XAssetKind::ClipMap, "clipmap"),
    (XAssetKind::ClipMapPvs, "clipmap_pvs"),
    (XAssetKind::ComWorld, "comworld"),
    (XAssetKind::GameWorldSp, "gameworldsp"),
    (XAssetKind::GameWorldMp, "gameworldmp"),
    (XAssetKind::MapEnts, "mapents"),
    (XAssetKind::GfxWorld, "gfxworld"),
    (XAssetKind::LightDef, "gfxlightdef"),
    (XAssetKind::UiMap, "uimap"),
    (XAssetKind::Font, "font"),
    (XAssetKind::FontIcon, "fonticon"),
    (XAssetKind::MenuList, "menulist"),
    (XAssetKind::Menu, "menu"),
    (XAssetKind::Localize, "localize"),
    (XAssetKind::Weapon, "weapon"),
    (XAssetKind::WeaponDef, "weapondef"),
    (XAssetKind::WeaponVariant, "weaponvariant"),
    (XAssetKind::WeaponFull, "weaponfull"),
    (XAssetKind::Attachment, "attachment"),
    (XAssetKind::AttachmentUnique, "attachmentunique"),
    (XAssetKind::WeaponCamo, "weaponcamo"),
    (XAssetKind::SndDriverGlobals, "snddriverglobals"),
    (XAssetKind::Fx, "fx"),
    (XAssetKind::ImpactFx, "fximpacttable"),
    (XAssetKind::AiType, "aitype"),
    (XAssetKind::MpType, "mptype"),
    (XAssetKind::MpBody, "mpbody"),
    (XAssetKind::MpHead, "mphead"),
    (XAssetKind::Character, "character"),
    (XAssetKind::XModelAlias, "xmodelalias"),
    (XAssetKind::RawFile, "rawfile"),
    (XAssetKind::StringTable, "stringtable"),
    (XAssetKind::Leaderboard, "leaderboard"),
    (XAssetKind::XGlobals, "xglobals"),
    (XAssetKind::Ddl, "ddl"),
    (XAssetKind::Glasses, "glasses"),
    (XAssetKind::EmblemSet, "emblemset"),
    (XAssetKind::ScriptParseTree, "script"),
    (XAssetKind::KeyValuePairs, "keyvaluepairs"),
    (XAssetKind::VehicleDef, "vehicle"),
    (XAssetKind::MemoryBlock, "memoryblock"),
    (XAssetKind::AddonMapEnts, "addonmapents"),
    (XAssetKind::Tracer, "tracer"),
    (XAssetKind::SkinnedVerts, "skinnedverts"),
    (XAssetKind::Qdb, "qdb"),
    (XAssetKind::Slug, "slug"),
    (XAssetKind::FootstepTable, "footsteptable"),
    (XAssetKind::FootstepFxTable, "footstepfxtable"),
    (XAssetKind::ZBarrier, "zbarrier"),
];

impl XAssetKind {
    pub const ALL: [XAssetKind; ASSET_TYPE_COUNT] = {
        let mut all = [POOLS[0].0; ASSET_TYPE_COUNT];
        let mut slot = 1;
        while slot < ASSET_TYPE_COUNT {
            all[slot] = POOLS[slot].0;
            slot += 1;
        }
        all
    };

    pub const fn from_u32(raw: u32) -> Option<XAssetKind> {
        if (raw as usize) < ASSET_TYPE_COUNT {
            Some(POOLS[raw as usize].0)
        } else {
            None
        }
    }

    pub const fn index(self) -> usize {
        self as u32 as usize
    }

    pub const fn name(self) -> &'static str {
        POOLS[self.index()].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_stop_at_the_pool_count() {
        for (i, ty) in XAssetKind::ALL.iter().enumerate() {
            assert_eq!(ty.index(), i);
            assert_eq!(XAssetKind::from_u32(i as u32), Some(*ty));
        }
        assert_eq!(XAssetKind::from_u32(ASSET_TYPE_COUNT as u32), None);
        assert_eq!(XAssetKind::ZBarrier.index(), 0x3B);
        assert_eq!(XAssetKind::ScriptParseTree.index(), 0x30);
    }

    #[test]
    fn the_table_is_in_discriminant_order_with_distinct_names() {
        for (slot, (ty, name)) in POOLS.iter().enumerate() {
            assert_eq!(ty.index(), slot);
            assert!(!name.is_empty());
            assert_eq!(POOLS.iter().filter(|(_, other)| other == name).count(), 1);
        }
        assert_eq!(XAssetKind::ScriptParseTree.name(), "script");
        assert_eq!(XAssetKind::XAnimParts.name(), "xanim");
    }
}
