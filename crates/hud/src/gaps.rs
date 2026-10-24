//! The HUD's ledger of things it cannot draw (yet): which gaps exist, what
//! can cause each, and the one-line report logged when the live set changes.

use core::fmt;

use bevy::prelude::*;
use diag::gap::{self as ledger, Gap as _, GapLedger};

const HUD_GAP_COUNT: usize = <HudGap as ledger::Gap>::ALL.len();

/// A class of HUD output that is missing or approximate.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum HudGap {
    ScriptedHudElem,
    ReticleWeaponDef,
    ReticleMaterial,
    ReticleSideQuadOffsets,
    LocalizedText,
    GameMessage,
    FlashWhiteout,
    HudElemMaterial,
    AdsOverlay,
    DeathIcons,
    AssetFont,
    MenuVisExp,
    OverheadNames,
    TextDecodeFx,
}

impl ledger::Gap for HudGap {
    const ALL: &'static [HudGap] = &[
        HudGap::ScriptedHudElem,
        HudGap::ReticleWeaponDef,
        HudGap::ReticleMaterial,
        HudGap::ReticleSideQuadOffsets,
        HudGap::LocalizedText,
        HudGap::GameMessage,
        HudGap::FlashWhiteout,
        HudGap::HudElemMaterial,
        HudGap::AdsOverlay,
        HudGap::DeathIcons,
        HudGap::AssetFont,
        HudGap::MenuVisExp,
        HudGap::OverheadNames,
        HudGap::TextDecodeFx,
    ];

    fn name(self) -> &'static str {
        use HudGap::*;
        match self {
            ScriptedHudElem => "scripted-hudelem",
            ReticleWeaponDef => "reticle-weapondef",
            ReticleMaterial => "reticle-material",
            ReticleSideQuadOffsets => "reticle-side-quad-offsets",
            LocalizedText => "localized-text",
            GameMessage => "game-message",
            FlashWhiteout => "flash-whiteout",
            HudElemMaterial => "hudelem-material",
            AdsOverlay => "ads-overlay",
            DeathIcons => "death-icons",
            AssetFont => "asset-font",
            MenuVisExp => "menu-visexp",
            OverheadNames => "overhead-names",
            TextDecodeFx => "text-decode-fx",
        }
    }

    /// A standing gap is a known hole in the feature itself, not a failure to
    /// find some asset at run time.
    fn is_standing(self) -> bool {
        use HudGap::*;
        matches!(
            self,
            ScriptedHudElem | ReticleSideQuadOffsets | GameMessage | DeathIcons
        )
    }
}

/// Which of a weapon's two reticle images is meant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReticleSlot {
    Center,
    Side,
}

impl fmt::Display for ReticleSlot {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(match self {
            ReticleSlot::Center => "centre",
            ReticleSlot::Side => "side",
        })
    }
}

/// Why an image lookup came back empty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageMiss {
    NoGamesRoot,
    NotDecoded,
}

impl fmt::Display for ImageMiss {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(match self {
            ImageMiss::NoGamesRoot => "no games root is set",
            ImageMiss::NotDecoded => "not a zone atlas and not in the IWD",
        })
    }
}

/// The concrete reason a gap is live right now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GapCause {
    ReticleNoWeaponCatalog,
    ReticleWeaponNotInCatalog { viewmodel_index: u32 },
    ReticleNoAuthoredMaterials { viewmodel_index: u32 },
    ReticleSlotNamesNoImage { slot: ReticleSlot },
    ReticleImageMissing { slot: ReticleSlot, name: String, miss: ImageMiss },
    NoStringTable,
    LocalizedRowMissing { key: String },
    FlashWhiteoutImageMissing { name: String, miss: ImageMiss },
    HudElemImageMissing { name: String, miss: ImageMiss },
    HudElemMaterialUnbound { material_index: i32 },
    AdsOverlayNamesNoImage { material: Option<String> },
    AdsOverlayImageMissing { name: String, miss: ImageMiss },
    AdsOverlayNoSize,
    VisExpUneval { menu: String, item: usize, err: String },
    MenuExpression { menu: String, item: usize, err: String },
    MenuScriptUnsupported { menu: String, command: String },
    TextDecodeFxAtlasMissing { material: String, image: Option<String> },
    TextDecodeFxDecayTooShort { fx_decay_duration: i32 },
    OverheadNoPresentedClient { client: u32 },
    OverheadVisibilityUnavailable,
    OverheadHeadUnavailable { entnum: u16 },
    OverheadFlashUnavailable,
    OverheadPartyUnavailable { client: u32 },
    OverheadFontUnavailable,
}

impl ledger::GapCause for GapCause {
    type Gap = HudGap;

    fn gap(&self) -> HudGap {
        use GapCause as C;
        match self {
            C::ReticleNoWeaponCatalog | C::ReticleWeaponNotInCatalog { .. } => {
                HudGap::ReticleWeaponDef
            }
            C::ReticleNoAuthoredMaterials { .. }
            | C::ReticleSlotNamesNoImage { .. }
            | C::ReticleImageMissing { .. } => HudGap::ReticleMaterial,
            C::NoStringTable | C::LocalizedRowMissing { .. } => HudGap::LocalizedText,
            C::FlashWhiteoutImageMissing { .. } => HudGap::FlashWhiteout,
            C::HudElemImageMissing { .. } | C::HudElemMaterialUnbound { .. } => {
                HudGap::HudElemMaterial
            }
            C::AdsOverlayNamesNoImage { .. }
            | C::AdsOverlayImageMissing { .. }
            | C::AdsOverlayNoSize => HudGap::AdsOverlay,
            C::VisExpUneval { .. } | C::MenuExpression { .. } | C::MenuScriptUnsupported { .. } => {
                HudGap::MenuVisExp
            }
            C::TextDecodeFxAtlasMissing { .. } | C::TextDecodeFxDecayTooShort { .. } => {
                HudGap::TextDecodeFx
            }
            C::OverheadNoPresentedClient { .. }
            | C::OverheadVisibilityUnavailable
            | C::OverheadHeadUnavailable { .. }
            | C::OverheadFlashUnavailable
            | C::OverheadPartyUnavailable { .. }
            | C::OverheadFontUnavailable => HudGap::OverheadNames,
        }
    }
}

impl fmt::Display for GapCause {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        use GapCause as C;
        match self {
            C::ReticleNoWeaponCatalog => out.write_str("no weapon catalog is loaded"),
            C::ReticleWeaponNotInCatalog { viewmodel_index } => {
                write!(out, "viewmodel weapon index {viewmodel_index} is not in the catalog")
            }
            C::ReticleNoAuthoredMaterials { viewmodel_index } => write!(
                out,
                "weapon index {viewmodel_index} authored no reticle material in either slot"
            ),
            C::ReticleSlotNamesNoImage { slot } => {
                write!(out, "the {slot} material names no image")
            }
            C::ReticleImageMissing { slot, name, miss } => {
                write!(out, "{slot} image `{name}` is {miss}")
            }
            C::NoStringTable => out.write_str("no localized string table is loaded"),
            C::LocalizedRowMissing { key } => {
                write!(out, "key `{key}` has no row in the loaded language")
            }
            C::FlashWhiteoutImageMissing { name, miss } => {
                write!(out, "flash white image `{name}` is {miss}")
            }
            C::HudElemMaterialUnbound { material_index } => write!(
                out,
                "HudElem materialIndex {material_index} resolves to no CS_HUDMATERIALS name"
            ),
            C::HudElemImageMissing { name, miss } => {
                write!(out, "hud elem material `{name}` is {miss}")
            }
            C::AdsOverlayNamesNoImage { material: Some(name) } => {
                write!(out, "overlay material `{name}` names no image")
            }
            C::AdsOverlayNamesNoImage { material: None } => {
                out.write_str("overlayMaterial is authored and names no material")
            }
            C::AdsOverlayImageMissing { name, miss } => {
                write!(out, "iris image `{name}` is {miss}")
            }
            C::AdsOverlayNoSize => out.write_str("adsOverlayWidth×Height is not a drawable rect"),
            C::VisExpUneval { menu, item, err } => write!(out, "{menu}[{item}] visExp: {err}"),
            C::MenuExpression { menu, item, err } => {
                write!(out, "{menu}[{item}] expression: {err}")
            }
            C::MenuScriptUnsupported { menu, command } => {
                write!(out, "{menu}: menu script `{command}` is not executed")
            }
            C::TextDecodeFxAtlasMissing { material, image: Some(image) } => write!(
                out,
                "decode FX material `{material}` image `{image}` did not upload"
            ),
            C::TextDecodeFxAtlasMissing { material, image: None } => {
                write!(out, "decode FX material `{material}` did not decode")
            }
            C::TextDecodeFxDecayTooShort { fx_decay_duration } => write!(
                out,
                "fxDecayDuration {fx_decay_duration} ms is under one 30 Hz decay tick"
            ),
            C::OverheadNoPresentedClient { client } => {
                write!(out, "client {client} has no presented clientState row")
            }
            C::OverheadVisibilityUnavailable => {
                out.write_str("client point trace or FX visibility is unavailable")
            }
            C::OverheadHeadUnavailable { entnum } => {
                write!(out, "entity {entnum} has no same-Present posed-head product")
            }
            C::OverheadFlashUnavailable => {
                out.write_str("local presented playerState is missing for the flashbang check")
            }
            C::OverheadPartyUnavailable { client } => {
                write!(out, "party relation to client {client} is unknown")
            }
            C::OverheadFontUnavailable => {
                out.write_str("fonts/hudsmallfont or its atlas is unavailable")
            }
        }
    }
}

/// The live gaps and their causes.
#[derive(Resource, Debug, Default)]
pub struct HudPresentationGaps(GapLedger<GapCause, HUD_GAP_COUNT>);

impl HudPresentationGaps {
    pub fn raise(&mut self, cause: GapCause) {
        self.0.raise(cause);
    }

    pub fn clear(&mut self, gap: HudGap) {
        self.0.clear(gap);
    }

    pub fn is_live(&self, gap: HudGap) -> bool {
        self.0.is_live(gap)
    }

    pub fn cause(&self, gap: HudGap) -> Option<&GapCause> {
        self.0.cause(gap)
    }

    pub fn live(&self) -> impl Iterator<Item = HudGap> + '_ {
        self.0.live()
    }

    pub fn count(&self) -> usize {
        self.0.count()
    }

    fn signature(&self) -> String {
        let mut text = String::new();
        self.0
            .write_signature(&mut text)
            .expect("writing into a String cannot fail");
        text
    }
}

/// Logs the live gaps whenever the set (or a cause) differs from what was
/// logged last.
pub(crate) fn report_hud_gaps(gaps: Res<HudPresentationGaps>, mut logged: Local<Option<String>>) {
    let now = gaps.signature();
    if logged.as_deref() == Some(now.as_str()) {
        return;
    }
    let names: Vec<&str> = gaps.live().map(HudGap::name).collect();
    diag::info!(
        Ui,
        "hud: {} presentation gaps live: {}",
        gaps.count(),
        names.join(" ")
    );
    for gap in gaps.live() {
        if let Some(cause) = gaps.cause(gap) {
            diag::info!(Ui, "hud gap {}: {}", gap.name(), cause);
        }
    }
    *logged = Some(now);
}
