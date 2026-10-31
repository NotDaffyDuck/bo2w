//! bo2zm: Black Ops II assets for the old engine. Takes what a T6 zone walk loads and
//! what the image packs hold, and turns it into data the engine uses.
//! Not part of the upstream engine.

pub mod zone_grab;
mod clip_brushes;
pub mod level_ents;
/// Old path, kept until bo2zm_tools t6mat reads `DrawState` from the crate root
/// (RENAMES.txt row, pile 6).
#[doc(hidden)]
pub mod capture {
    pub use super::DrawState;
}
pub mod gsc;
pub mod images;
pub mod m2;
mod vertex;

pub use zone_grab::{
    AabbTreeRef, AssetIdentity, CellRef, ClipAabbRef, ClipBrushRef, ClipCmodelRef, ClipLeafBrushNodeRef, ClipLeafRef,
    ClipMaterialRef, ClipNodeRef, ClipRef, ClipStaticModelRef, DpvsRanges, DrawState, EmbeddedImage, GlassRef, ImageRef, LightGridRef, MaterialRef,
    DestructiblePieceRef, DestructibleRef, DestructibleStageRef, HeliDrive, VehicleDrive, VehicleRef,
    HeroLightRef, HeroLightTreeRef, ZBarrierDefRef,
    WorldFogRef, WorldRef, WorldSurface, XAnimRef, XModelCollSurfRef, XModelRef, XSurfaceRef, ZoneCapture, read_image,
};
pub use zone_grab::ShaderSamplerImage;
pub use zone_grab::PassRef;
pub use zone_grab::PathNodeRef;
pub use zone_grab::PathTablesRef;
pub use zone_grab::PortalRef;
pub use zone_grab::PrimaryLightRef;
pub use zone_grab::ReflectionProbeRef;
pub use zone_grab::ShaderArgRef;
pub use zone_grab::SkyGridVolumeRef;
pub use zone_grab::StaticModel;
pub use zone_grab::StringTableRef;
pub use zone_grab::SunRef;
pub use zone_grab::TechniqueRef;
pub use zone_grab::TechniqueSetRef;
pub use level_ents::{ArtFog, MapEntity, parse_art_fog, parse_entities};
pub use gsc::{GscObject, GscRun, GscValue, MapScriptFacts, PlacedFx};
pub use images::{ImageSource, Pack, PackSet};
pub use m2::{
    AttachmentRef, AttachmentUniqueRef,
    FootstepFxTableRef, FootstepTableRef, FxEffectRef, FxElemRef, FxTrailRef, FxVisualRef, ImpactTableRef,
    SndAliasListRef, SndAliasRef, SndBankRef, SndCurveRef, SndDriverGlobalsRef, SndDynamicsRef, SndEqBandRef, SndGroupMixRef, SndMasterRef, SndPanRef, SndRadverbRef, TracerRef,
    FontRef, GlyphRef, WeaponCamoMaterialRef, WeaponCamoRef, WeaponCamoSetRef, WeaponRef,
};
pub use clip_brushes::CollTreeLeaf;
pub use clip_brushes::CollTreeNode;
pub use vertex::{
    T6_PACKED_VERTEX, T6_WORLD_VERTEX, T6PackedVertex,
};
pub use vertex::T6WorldVertex;
pub use vertex::f32_to_half;
pub use vertex::short_float_to_full;
pub use vertex::pack_unit_vec;
pub use vertex::decode_unit_normal;

use std::path::Path;

use asset_transport::T6ZoneMemory;
use asset_transport::ZoneTitleKind;
use asset_transport::ZoneTextureData;
use asset_transport::mount_zone;

/// Open, walk and capture one zone file.
pub fn capture_zone(path: &Path) -> Result<ZoneCapture, String> {
    let image = mount_zone(path).map_err(|e| e.to_string())?;
    capture_image(path, &image)
}

/// `capture_zone`, keeping every pass's shader bytecode too (probes).
pub fn capture_zone_with_shaders(path: &Path) -> Result<ZoneCapture, String> {
    let image = mount_zone(path).map_err(|e| e.to_string())?;
    capture_image_with(path, &image, true)
}

/// Walk and capture an already opened zone image.
pub fn capture_image(path: &Path, image: &ZoneTextureData) -> Result<ZoneCapture, String> {
    capture_image_with(path, image, false)
}

fn capture_image_with(
    path: &Path,
    image: &ZoneTextureData,
    keep_shaders: bool,
) -> Result<ZoneCapture, String> {
    if image.game != ZoneTitleKind::T6 {
        return Err(format!("{shown}: not a Black Ops II zone", shown = path.display()));
    }
    let header = image.t6_header().map_err(|e| e.to_string())?;
    let mut memory = T6ZoneMemory::for_header(&header);
    let mut zone_reader = memory.stream(&image.bytes).map_err(|e| e.to_string())?;
    let table = fastfile_t6::read_asset_directory(&mut zone_reader).map_err(|e| e.to_string())?;
    let mut capture = ZoneCapture::default();
    // bo2zm M2: script strings, for bone names and notetracks.
    capture.strings = (0..table.strings.count())
        .map(|i| table.strings.get(&zone_reader, i).unwrap_or("").to_owned())
        .collect();
    capture.keep_shaders = keep_shaders;
    capture.zone = images::stem_of(path);
    let result = {
        let mut walker = fastfile_t6::Walker::new(&mut zone_reader, &mut capture);
        walker.walk_assets(&table).map_err(|e| {
            let trail: Vec<&str> = walker.trail().collect();
            format!(
                "{shown}: walk failed: {e} (in {})",
                trail.join(" > "),
                shown = path.display(),
            )
        })
    };
    result?;
    let ended = zone_reader.cursor();
    if ended != image.bytes.len() {
        return Err(format!(
            "{shown}: walk ended at {ended:#x} of {:#x}",
            image.bytes.len(),
            shown = path.display(),
        ));
    }
    Ok(capture)
}
