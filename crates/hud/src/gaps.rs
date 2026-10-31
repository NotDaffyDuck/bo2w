//! The HUD's ledger of things it cannot draw (yet): which gaps exist, what
//! can cause each, and the one-line report logged when the live set changes.

use core::fmt;

use bevy::prelude::*;
use diag::gap::Gap;
use diag::gap::LackLedger;
use diag::gap::ShortfallOrigin;
use diag::info;

const HUD_SHORTAGE_COUNT: usize = <HudShortage as Gap>::ALL_KINDS.len();

/// What is known about one shortage besides its identity.
struct Facts {
    log_name: &'static str,
    standing: bool,
}

/// One row per shortage, in declaration order (the enum value is the index).
const FACTS: [Facts; HUD_SHORTAGE_COUNT] = [
    Facts {
        log_name: "scripted-hudelem",
        standing: true,
    },
    Facts {
        log_name: "reticle-weapondef",
        standing: false,
    },
    Facts {
        log_name: "reticle-material",
        standing: false,
    },
    Facts {
        log_name: "reticle-side-quad-offsets",
        standing: true,
    },
    Facts {
        log_name: "localized-text",
        standing: false,
    },
    Facts {
        log_name: "game-message",
        standing: true,
    },
    Facts {
        log_name: "flash-whiteout",
        standing: false,
    },
    Facts {
        log_name: "hudelem-material",
        standing: false,
    },
    Facts {
        log_name: "ads-overlay",
        standing: false,
    },
    Facts {
        log_name: "death-icons",
        standing: true,
    },
    Facts {
        log_name: "asset-font",
        standing: false,
    },
    Facts {
        log_name: "menu-visexp",
        standing: false,
    },
    Facts {
        log_name: "overhead-names",
        standing: false,
    },
    Facts {
        log_name: "text-decode-fx",
        standing: false,
    },
];

/// A class of HUD output that is missing or approximate.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum HudShortage {
    ScriptedElem,
    ReticleGunDef,
    ReticleArt,
    ReticleArmOffsets,
    LocalizedStrings,
    GameMessages,
    FlashWhite,
    ElemMaterial,
    AdsScope,
    KillIcons,
    FontAsset,
    MenuVisibility,
    NameTags,
    DecodeFx,
}

impl HudShortage {
    fn facts(self) -> &'static Facts {
        &FACTS[self as usize]
    }
}

impl Gap for HudShortage {
    const ALL_KINDS: &'static [HudShortage] = &[
        HudShortage::ScriptedElem,
        HudShortage::ReticleGunDef,
        HudShortage::ReticleArt,
        HudShortage::ReticleArmOffsets,
        HudShortage::LocalizedStrings,
        HudShortage::GameMessages,
        HudShortage::FlashWhite,
        HudShortage::ElemMaterial,
        HudShortage::AdsScope,
        HudShortage::KillIcons,
        HudShortage::FontAsset,
        HudShortage::MenuVisibility,
        HudShortage::NameTags,
        HudShortage::DecodeFx,
    ];

    fn name(self) -> &'static str {
        self.facts().log_name
    }

    /// A standing gap is a known hole in the feature itself, not a failure to
    /// find some asset at run time.
    fn still_open(self) -> bool {
        self.facts().standing
    }
}

/// Which of a weapon's two reticle images is meant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReticlePart {
    Center,
    Side,
}

impl ReticlePart {
    const fn word(self) -> &'static str {
        ["centre", "side"][self as usize]
    }
}

impl fmt::Display for ReticlePart {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.pad(self.word())
    }
}

/// Why an image lookup came back empty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LookupFailure {
    NoRoot,
    Undecoded,
}

impl fmt::Display for LookupFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let words = [
            "no games root is set",
            "not a zone atlas and not in the IWD",
        ];
        out.pad(words[*self as usize])
    }
}

/// The concrete reason a gap is live right now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GapReason {
    ReticleNoCatalog,
    ReticleGunMissing { viewmodel_index: u32 },
    ReticleNoArt { viewmodel_index: u32 },
    ReticlePartUnnamed { slot: ReticlePart },
    ReticleImageAbsent { slot: ReticlePart, asset_key: String, miss: LookupFailure },
    StringTableAbsent,
    StringRowAbsent { key: String },
    FlashImageAbsent { asset_key: String, miss: LookupFailure },
    ElemImageAbsent { asset_key: String, miss: LookupFailure },
    ElemMaterialUnbound { material_index: i32 },
    ScopeUnnamed { material: Option<String> },
    ScopeImageAbsent { asset_key: String, miss: LookupFailure },
    ScopeNoSize,
    VisibilityUneval { menu_name: String, item_no: usize, why: String },
    MenuExprFailed { menu_name: String, item_no: usize, why: String },
    MenuScriptSkipped { menu_name: String, command: String },
    DecodeAtlasAbsent { paint_material: String, image_name: Option<String> },
    DecodeDecayShort { fx_decay_duration: i32 },
    TagNoClient { client: u32 },
    TagVisibilityOff,
    TagHeadOff { entnum: u16 },
    TagFlashOff,
    TagPartyOff { client: u32 },
    TagFontOff,
}

impl ShortfallOrigin for GapReason {
    type Gap = HudShortage;

    fn gap(&self) -> HudShortage {
        match self {
            GapReason::ReticleNoCatalog | GapReason::ReticleGunMissing { .. } => HudShortage::ReticleGunDef,
            GapReason::ReticleNoArt { .. }
            | GapReason::ReticlePartUnnamed { .. }
            | GapReason::ReticleImageAbsent { .. } => HudShortage::ReticleArt,
            GapReason::StringTableAbsent | GapReason::StringRowAbsent { .. } => HudShortage::LocalizedStrings,
            GapReason::FlashImageAbsent { .. } => HudShortage::FlashWhite,
            GapReason::ElemImageAbsent { .. } | GapReason::ElemMaterialUnbound { .. } => HudShortage::ElemMaterial,
            GapReason::ScopeUnnamed { .. } | GapReason::ScopeImageAbsent { .. } | GapReason::ScopeNoSize => HudShortage::AdsScope,
            GapReason::VisibilityUneval { .. } | GapReason::MenuExprFailed { .. } | GapReason::MenuScriptSkipped { .. } => {
                HudShortage::MenuVisibility
            }
            GapReason::DecodeAtlasAbsent { .. } | GapReason::DecodeDecayShort { .. } => HudShortage::DecodeFx,
            GapReason::TagNoClient { .. }
            | GapReason::TagVisibilityOff
            | GapReason::TagHeadOff { .. }
            | GapReason::TagFlashOff
            | GapReason::TagPartyOff { .. }
            | GapReason::TagFontOff => HudShortage::NameTags,
        }
    }
}

impl GapReason {
    fn reticle_line(&self) -> Option<String> {
        Some(match self {
            Self::ReticleNoCatalog => "no weapon catalog is loaded".to_owned(),
            Self::ReticleGunMissing {
                viewmodel_index: idx,
            } => {
                format!("viewmodel weapon index {idx} is not in the catalog")
            }
            Self::ReticleNoArt {
                viewmodel_index: idx,
            } => {
                format!("weapon index {idx} authored no reticle material in either slot")
            }
            Self::ReticlePartUnnamed { slot } => format!("the {slot} material names no image"),
            Self::ReticleImageAbsent { asset_key, slot, miss } => {
                format!("{slot} image `{asset_key}` is {miss}")
            }
            _ => return None,
        })
    }

    fn text_line(&self) -> Option<String> {
        Some(match self {
            Self::StringTableAbsent => "no localized string table is loaded".to_owned(),
            Self::StringRowAbsent { key } => {
                format!("key `{key}` has no row in the loaded language")
            }
            _ => return None,
        })
    }

    fn element_line(&self) -> Option<String> {
        Some(match self {
            Self::FlashImageAbsent { asset_key, miss } => {
                format!("flash white image `{asset_key}` is {miss}")
            }
            Self::ElemImageAbsent { asset_key, miss } => format!("hud elem material `{asset_key}` is {miss}"),
            Self::ElemMaterialUnbound {
                material_index: idx,
            } => {
                format!("HudElem materialIndex {idx} resolves to no CS_HUDMATERIALS name")
            }
            _ => return None,
        })
    }

    fn scope_line(&self) -> Option<String> {
        Some(match self {
            Self::ScopeUnnamed { material } => match material {
                Some(name) => format!("overlay material `{name}` names no image"),
                None => "overlayMaterial is authored and names no material".to_owned(),
            },
            Self::ScopeImageAbsent { asset_key, miss } => format!("iris image `{asset_key}` is {miss}"),
            Self::ScopeNoSize => "adsOverlayWidth×Height is not a drawable rect".to_owned(),
            _ => return None,
        })
    }

    fn menu_line(&self) -> Option<String> {
        let (menu, item, what, err) = match self {
            Self::VisibilityUneval { item_no: item, menu_name: menu, why: err } => (menu, item, "visExp", err),
            Self::MenuExprFailed { item_no: item, menu_name: menu, why: err } => (menu, item, "expression", err),
            Self::MenuScriptSkipped { menu_name: menu, command } => {
                return Some(format!("{menu}: menu script `{command}` is not executed"));
            }
            _ => return None,
        };
        Some(format!("{menu}[{item}] {what}: {err}"))
    }

    fn decode_line(&self) -> Option<String> {
        match self {
            Self::DecodeAtlasAbsent { image_name: image, paint_material: material } => {
                let tail = image.as_ref().map_or_else(
                    || "did not decode".to_owned(),
                    |i| format!("image `{i}` did not upload"),
                );
                Some(format!("decode FX material `{material}` {tail}"))
            }
            Self::DecodeDecayShort {
                fx_decay_duration: ms,
            } => Some(format!(
                "fxDecayDuration {ms} ms is under one 30 Hz decay tick"
            )),
            _ => None,
        }
    }

    fn tag_line(&self) -> Option<String> {
        let line = match self {
            Self::TagNoClient { client } => {
                format!("client {client} has no presented clientState row")
            }
            Self::TagVisibilityOff => "client point trace or FX visibility is unavailable".into(),
            Self::TagHeadOff { entnum } => {
                format!("entity {entnum} has no same-Present posed-head product")
            }
            Self::TagFlashOff => {
                "local presented playerState is missing for the flashbang check".into()
            }
            Self::TagPartyOff { client } => format!("party relation to client {client} is unknown"),
            Self::TagFontOff => "fonts/hudsmallfont or its atlas is unavailable".into(),
            _ => return None,
        };
        Some(line)
    }

    /// The sentence for this reason: the first topic that recognises it wins.
    fn sentence(&self) -> String {
        let topics: [fn(&GapReason) -> Option<String>; 7] = [
            Self::reticle_line,
            Self::text_line,
            Self::element_line,
            Self::scope_line,
            Self::menu_line,
            Self::decode_line,
            Self::tag_line,
        ];
        topics
            .iter()
            .find_map(|topic| topic(self))
            .unwrap_or_else(|| format!("{:?}", self))
    }
}

impl fmt::Display for GapReason {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.sentence())
    }
}

/// The live gaps and their causes.
#[derive(Resource, Debug, Default)]
pub struct HudGapBook(LackLedger<GapReason, HUD_SHORTAGE_COUNT>);

impl HudGapBook {
    pub fn log_gap(&mut self, reason: GapReason) {
        self.0.raise(reason)
    }

    pub fn settle(&mut self, which: HudShortage) {
        self.0.clear(which)
    }

    pub fn still_counting(&self, which: HudShortage) -> bool {
        self.0.is_live(which)
    }

    pub fn reason_for(&self, which: HudShortage) -> Option<&GapReason> {
        self.0.cause(which)
    }

    pub fn live(&self) -> impl Iterator<Item = HudShortage> + '_ {
        let book = &self.0;
        book.live()
    }

    pub fn count(&self) -> usize {
        self.0.count()
    }

    /// A text that changes exactly when the live set or one of its causes does.
    fn signature(&self) -> String {
        let mut text = String::new();
        self.0
            .print_fingerprint(&mut text)
            .map(|()| text)
            .expect("a String write never fails")
    }
}

/// Logs the live gaps whenever the set (or a cause) differs from what was
/// logged last.
pub(crate) fn log_hud_shortages(book: Res<HudGapBook>, mut logged: Local<Option<String>>) {
    let stamp = book.signature();
    if logged.as_ref() == Some(&stamp) {
        return;
    }
    let live: Vec<HudShortage> = book.live().collect();
    let listing = live
        .iter()
        .map(|gap| gap.name())
        .collect::<Vec<_>>()
        .join(" ");
    info!(
        Ui,
        "hud: {} live presentation gaps: {}",
        live.len(),
        listing
    );
    let with_cause = live
        .iter()
        .filter_map(|&gap| book.reason_for(gap).map(|why| (gap, why)));
    for (gap, why) in with_cause {
        info!(Ui, "hud gap {name}: {why}", name = gap.name());
    }
    *logged = Some(stamp);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> HudGapBook {
        HudGapBook::default()
    }

    #[test]
    fn every_shortage_has_a_distinct_log_name() {
        let mut seen: Vec<&str> = HudShortage::ALL_KINDS.iter().map(|s| s.name()).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total);
        assert_eq!(total, HUD_SHORTAGE_COUNT);
    }

    #[test]
    fn standing_shortages_are_the_four_known_holes() {
        let standing: Vec<&str> = HudShortage::ALL_KINDS
            .iter()
            .filter(|s| s.still_open())
            .map(|s| s.name())
            .collect();
        assert_eq!(
            standing,
            [
                "scripted-hudelem",
                "reticle-side-quad-offsets",
                "game-message",
                "death-icons"
            ]
        );
    }

    #[test]
    fn reasons_map_to_their_shortage() {
        use ShortfallOrigin as _;
        let cases = [
            (GapReason::ReticleNoCatalog, HudShortage::ReticleGunDef),
            (
                GapReason::StringRowAbsent { key: "k".into() },
                HudShortage::LocalizedStrings,
            ),
            (GapReason::ScopeNoSize, HudShortage::AdsScope),
            (GapReason::TagFontOff, HudShortage::NameTags),
            (
                GapReason::DecodeDecayShort {
                    fx_decay_duration: 3,
                },
                HudShortage::DecodeFx,
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(reason.gap(), expected);
        }
    }

    #[test]
    fn raise_then_clear_roundtrips() {
        let mut b = book();
        let standing = HudShortage::ALL_KINDS.iter().filter(|s| s.still_open()).count();
        assert_eq!(b.count(), standing);
        b.log_gap(GapReason::StringTableAbsent);
        assert!(b.still_counting(HudShortage::LocalizedStrings));
        assert_eq!(
            b.reason_for(HudShortage::LocalizedStrings),
            Some(&GapReason::StringTableAbsent)
        );
        b.settle(HudShortage::LocalizedStrings);
        assert!(!b.still_counting(HudShortage::LocalizedStrings));
        assert_eq!(b.live().count(), standing);
    }

    #[test]
    fn signature_follows_the_cause_not_just_the_set() {
        let mut b = book();
        b.log_gap(GapReason::StringRowAbsent { key: "A".into() });
        let first = b.signature();
        b.log_gap(GapReason::StringRowAbsent { key: "B".into() });
        assert_ne!(first, b.signature());
    }

    #[test]
    fn display_text_names_the_part_and_the_miss() {
        let reason = GapReason::ReticleImageAbsent {
            slot: ReticlePart::Side,
            asset_key: "x".into(),
            miss: LookupFailure::NoRoot,
        };
        assert_eq!(reason.to_string(), "side image `x` is no games root is set");
        assert_eq!(ReticlePart::Center.to_string(), "centre");
    }

    #[test]
    fn name_table_is_pinned_in_order() {
        let names: Vec<&str> = HudShortage::ALL_KINDS.iter().map(|s| s.name()).collect();
        assert_eq!(
            names,
            [
                "scripted-hudelem",
                "reticle-weapondef",
                "reticle-material",
                "reticle-side-quad-offsets",
                "localized-text",
                "game-message",
                "flash-whiteout",
                "hudelem-material",
                "ads-overlay",
                "death-icons",
                "asset-font",
                "menu-visexp",
                "overhead-names",
                "text-decode-fx",
            ]
        );
    }

    #[test]
    fn every_display_text_is_pinned() {
        let img = || LookupFailure::Undecoded;
        let cases: Vec<(GapReason, &str)> = vec![
            (GapReason::ReticleNoCatalog, "no weapon catalog is loaded"),
            (
                GapReason::ReticleGunMissing { viewmodel_index: 4 },
                "viewmodel weapon index 4 is not in the catalog",
            ),
            (
                GapReason::ReticleNoArt { viewmodel_index: 5 },
                "weapon index 5 authored no reticle material in either slot",
            ),
            (
                GapReason::ReticlePartUnnamed {
                    slot: ReticlePart::Center,
                },
                "the centre material names no image",
            ),
            (
                GapReason::ReticleImageAbsent {
                    slot: ReticlePart::Center,
                    asset_key: "n".into(),
                    miss: img(),
                },
                "centre image `n` is not a zone atlas and not in the IWD",
            ),
            (
                GapReason::StringTableAbsent,
                "no localized string table is loaded",
            ),
            (
                GapReason::StringRowAbsent { key: "K".into() },
                "key `K` has no row in the loaded language",
            ),
            (
                GapReason::FlashImageAbsent {
                    asset_key: "f".into(),
                    miss: img(),
                },
                "flash white image `f` is not a zone atlas and not in the IWD",
            ),
            (
                GapReason::ElemMaterialUnbound { material_index: -1 },
                "HudElem materialIndex -1 resolves to no CS_HUDMATERIALS name",
            ),
            (
                GapReason::ElemImageAbsent {
                    asset_key: "e".into(),
                    miss: LookupFailure::NoRoot,
                },
                "hud elem material `e` is no games root is set",
            ),
            (
                GapReason::ScopeUnnamed {
                    material: Some("m".into()),
                },
                "overlay material `m` names no image",
            ),
            (
                GapReason::ScopeUnnamed { material: None },
                "overlayMaterial is authored and names no material",
            ),
            (
                GapReason::ScopeImageAbsent {
                    asset_key: "i".into(),
                    miss: img(),
                },
                "iris image `i` is not a zone atlas and not in the IWD",
            ),
            (
                GapReason::ScopeNoSize,
                "adsOverlayWidth\u{d7}Height is not a drawable rect",
            ),
            (
                GapReason::VisibilityUneval {
                    menu_name: "m".into(),
                    item_no: 2,
                    why: "e".into(),
                },
                "m[2] visExp: e",
            ),
            (
                GapReason::MenuExprFailed {
                    menu_name: "m".into(),
                    item_no: 2,
                    why: "e".into(),
                },
                "m[2] expression: e",
            ),
            (
                GapReason::MenuScriptSkipped {
                    menu_name: "m".into(),
                    command: "c".into(),
                },
                "m: menu script `c` is not executed",
            ),
            (
                GapReason::DecodeAtlasAbsent {
                    paint_material: "m".into(),
                    image_name: Some("i".into()),
                },
                "decode FX material `m` image `i` did not upload",
            ),
            (
                GapReason::DecodeAtlasAbsent {
                    paint_material: "m".into(),
                    image_name: None,
                },
                "decode FX material `m` did not decode",
            ),
            (
                GapReason::DecodeDecayShort {
                    fx_decay_duration: 7,
                },
                "fxDecayDuration 7 ms is under one 30 Hz decay tick",
            ),
            (
                GapReason::TagNoClient { client: 3 },
                "client 3 has no presented clientState row",
            ),
            (
                GapReason::TagVisibilityOff,
                "client point trace or FX visibility is unavailable",
            ),
            (
                GapReason::TagHeadOff { entnum: 9 },
                "entity 9 has no same-Present posed-head product",
            ),
            (
                GapReason::TagFlashOff,
                "local presented playerState is missing for the flashbang check",
            ),
            (
                GapReason::TagPartyOff { client: 1 },
                "party relation to client 1 is unknown",
            ),
            (
                GapReason::TagFontOff,
                "fonts/hudsmallfont or its atlas is unavailable",
            ),
        ];
        for (reason, text) in &cases {
            assert_eq!(&reason.to_string(), text);
        }
    }

    #[test]
    fn every_reason_maps_to_its_shortage() {
        use ShortfallOrigin as _;
        let m = |s: &str| s.to_string();
        let img = || LookupFailure::NoRoot;
        let cases = vec![
            (
                GapReason::ReticleGunMissing { viewmodel_index: 0 },
                HudShortage::ReticleGunDef,
            ),
            (
                GapReason::ReticleNoArt { viewmodel_index: 0 },
                HudShortage::ReticleArt,
            ),
            (
                GapReason::ReticlePartUnnamed {
                    slot: ReticlePart::Side,
                },
                HudShortage::ReticleArt,
            ),
            (
                GapReason::ReticleImageAbsent {
                    slot: ReticlePart::Side,
                    asset_key: m("a"),
                    miss: img(),
                },
                HudShortage::ReticleArt,
            ),
            (GapReason::StringTableAbsent, HudShortage::LocalizedStrings),
            (
                GapReason::FlashImageAbsent {
                    asset_key: m("a"),
                    miss: img(),
                },
                HudShortage::FlashWhite,
            ),
            (
                GapReason::ElemImageAbsent {
                    asset_key: m("a"),
                    miss: img(),
                },
                HudShortage::ElemMaterial,
            ),
            (
                GapReason::ElemMaterialUnbound { material_index: 1 },
                HudShortage::ElemMaterial,
            ),
            (
                GapReason::ScopeUnnamed { material: None },
                HudShortage::AdsScope,
            ),
            (
                GapReason::ScopeImageAbsent {
                    asset_key: m("a"),
                    miss: img(),
                },
                HudShortage::AdsScope,
            ),
            (
                GapReason::VisibilityUneval {
                    menu_name: m("a"),
                    item_no: 0,
                    why: m("e"),
                },
                HudShortage::MenuVisibility,
            ),
            (
                GapReason::MenuExprFailed {
                    menu_name: m("a"),
                    item_no: 0,
                    why: m("e"),
                },
                HudShortage::MenuVisibility,
            ),
            (
                GapReason::MenuScriptSkipped {
                    menu_name: m("a"),
                    command: m("c"),
                },
                HudShortage::MenuVisibility,
            ),
            (
                GapReason::DecodeAtlasAbsent {
                    paint_material: m("a"),
                    image_name: None,
                },
                HudShortage::DecodeFx,
            ),
            (GapReason::TagNoClient { client: 0 }, HudShortage::NameTags),
            (GapReason::TagVisibilityOff, HudShortage::NameTags),
            (GapReason::TagHeadOff { entnum: 0 }, HudShortage::NameTags),
            (GapReason::TagFlashOff, HudShortage::NameTags),
            (GapReason::TagPartyOff { client: 0 }, HudShortage::NameTags),
        ];
        for (reason, expected) in cases {
            assert_eq!(reason.gap(), expected);
        }
    }

    #[test]
    fn live_set_after_raising_every_kind_lists_each_once() {
        let mut b = book();
        b.log_gap(GapReason::TagFontOff);
        b.log_gap(GapReason::TagFlashOff);
        b.log_gap(GapReason::ScopeNoSize);
        let live: Vec<_> = b.live().collect();
        assert!(live.contains(&HudShortage::NameTags));
        assert!(live.contains(&HudShortage::AdsScope));
        assert_eq!(live.len(), b.count());
        assert_eq!(
            b.reason_for(HudShortage::NameTags),
            Some(&GapReason::TagFlashOff)
        );
    }
}
