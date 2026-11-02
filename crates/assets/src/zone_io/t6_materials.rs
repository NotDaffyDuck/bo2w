//! bo2zm: Black Ops II materials into the engine's material catalog.
//!
//! BO2 materials carry D3D11 techniques the engine cannot run, so a row here
//! is only what the renderer's fallback draw samples: the material's colour
//! map, read from the game's image packs (IWI v27) into a GPU image in its
//! own block format (BC1-3). Rows have no technique set; their surfaces keep
//! drawing through the fallback pass, which samples the colour map.

use std::collections::HashMap;
use std::sync::Arc;

use asset_core::{AssetCategoryScope, AssetPointer, T6Draw, ZoneCustodian};
use asset_material::{
    HandBuiltTexture, DesignerMaterial, SurfaceShaderRoster, SurfaceImageAttachment, SEMANTIC_COLOR,
    SEMANTIC_NORMAL, SEMANTIC_SPECULAR,
};
use asset_t6::{ImageRef, ImageSource, PackSet, ZoneCapture};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::Image;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

/// `R_HashString("colorMap")`, the colour map's slot in a texture table.
const COLOR_MAP_HASH: u32 = 0xa0ab_1041;
/// bo2mp: `DiffuseAndGloss`, the sw4 shaders' colour map.
const DIFFUSE_AND_GLOSS_HASH: u32 = 0xd50e_4161;
/// bo2mp: the picture of BO2's laser-scan 2D shader (`sw4_2d_laser_scan`:
/// the big scorestreak icons, `hud_ks_*_big`; its other two images are
/// the scan's noise and wireframe).
const LASER_SCAN_IMAGE_HASH: u32 = 0x079c_6367;

/// bo2mp: the 3-D tile-blend shaders (`sw4_3d_cod7_tile_blend*`, rocks and
/// cliffs on most multiplayer maps) have no `colorMap`: they mix two tiling
/// colour maps (`Micro_1_ColorMap`, `Micro_2_ColorMap`) by a macro map in
/// this slot, whose red, green and blue are blend weights (named `_ao`, or
/// `_c` on the cliffs: Downhill's red and yellow rocks).
const TILE_BLEND_MACRO_HASH: u32 = 0xed0a_4f83;
const TILE_BLEND_MICRO1_HASH: u32 = label_digest("Micro_1_ColorMap");
const TILE_BLEND_MICRO2_HASH: u32 = label_digest("Micro_2_ColorMap");
/// bo2mp: a lens flare's (`sw4_3d_superflare_*`) atlas of ghost shapes,
/// and the grime map its pixels are scaled by.
const SPRITE_MAP_HASH: u32 = label_digest("SpriteMap");
const GRIME_MAP_HASH: u32 = label_digest("GrimeMap");

/// The texture-slot name hash: each byte lower-cased (by setting bit 5) and
/// xored into 33 times the hash so far.
const fn label_digest(name: &str) -> u32 {
    const fn fold(bytes: &[u8], acc: u32) -> u32 {
        match bytes {
            [] => acc,
            [first, rest @ ..] => fold(rest, acc.wrapping_mul(33) ^ (*first | 0x20) as u32),
        }
    }
    fold(name.as_bytes(), 0)
}

/// A 2-D GPU texture (`layers` deep) holding `bytes`, kept on the render side only.
fn texture_2d(width: u32, height: u32, layer_n: u32, format: TextureFormat, bytes: Vec<u8>) -> Image {
    let size = Extent3d { width, height, depth_or_array_layers: layer_n };
    let mut made = Image::new_uninit(size, TextureDimension::D2, format, GPU_SIDE);
    made.data = Some(bytes);
    made
}

const GPU_SIDE: RenderAssetUsages = RenderAssetUsages::RENDER_WORLD;

/// Makes the texture bind as `kind` (a cube, a cube array) instead of a plain 2-D array.
fn bind_as(image: &mut Image, kind: TextureViewDimension) {
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(kind), ..TextureViewDescriptor::default() });
}

/// A catalog image row for a picture built here: one level, one deep, no
/// raw payload, only the decoded image.
fn built_row(name: String, semantic: u8, srgb: bool, w: u16, h: u16, image: Image) -> HandBuiltTexture {
    HandBuiltTexture {
        namespace: AssetCategoryScope::T6,
        name: AssetPointer::Real(name),
        texture_kind: 0,
        semantic,
        category: 0,
        srgb_sampled: srgb,
        width: w,
        height: h,
        depth: 1,
        tier_total: 1,
        format: 0,
        payload: Arc::default(),
        decoded: Some(Arc::new(image)),
        common_owned: false,
        decoded_variant: None,
    }
}

/// Links a linear picture made here under `name`, sized as the picture is.
fn link_built(catalog: &mut SurfaceShaderRoster, name: String, semantic: u8, picture: Image) -> usize {
    let Extent3d { width, height, .. } = picture.texture_descriptor.size;
    catalog.add_texture(built_row(name, semantic, false, width as u16, height as u16, picture))
}

/// A material's texture binding of catalog image `slot`.
fn binding(label_digest: u32, sampling: u8, semantic: u8, bay: usize) -> SurfaceImageAttachment {
    SurfaceImageAttachment {
        label_digest,
        label_head_offset: 0,
        label_tail_offset: 0,
        texture_sampling_mode: sampling,
        semantic,
        image: Some(bay),
    }
}

/// Sampling mode for the constant tables and other pictures made here.
const TABLE_SAMPLING: u8 = 0x14;

/// The material's texture in the slot named by `hash`, if any.
fn slot_texture(material: &asset_t6::MaterialRef, hash: u32) -> Option<&asset_t6::ShaderSamplerImage> {
    material.textures.iter().find(|t| t.label_digest == hash)
}

/// Same, but only if an image is attached.
fn filled_slot(material: &asset_t6::MaterialRef, hash: u32) -> Option<&asset_t6::ShaderSamplerImage> {
    material.textures.iter().find(|t| t.label_digest == hash && t.image.is_some())
}

/// The literal of the constant with `hash`, or `fallback`.
fn constant_or(material: &asset_t6::MaterialRef, hash: u32, fallback: [f32; 4]) -> [f32; 4] {
    match material.constants.iter().find(|entry| entry.0 == hash) {
        Some(entry) => entry.2,
        None => fallback,
    }
}

/// A material with nothing but a name, an owner zone and its bindings; the
/// linked rows fill in the rest over this.
fn bare_material(name: String, zone: ZoneCustodian, textures: Vec<SurfaceImageAttachment>) -> DesignerMaterial {
    DesignerMaterial {
        name: AssetPointer::Real(name),
        namespace: AssetCategoryScope::T6,
        method_set: AssetPointer::default(),
        draw_order: Default::default(),
        game_flag_info: 0,
        facet_type_bits: None,
        condition_bits: 0,
        camera_zone: 0,
        raster_bits: Vec::new(),
        raster_slot: None,
        method_table: None,
        route: Default::default(),
        textures,
        constants: Vec::new(),
        zone,
        t6_draw: None,
    }
}

/// A layered material's extra colour maps (`colorMap1`, `colorMap2`) ride
/// in the catalog under these private semantics; the fallback draw reads
/// them by semantic. Real T6 semantics stay below 0x20.
pub const T6_LAYER1_SEMANTIC: u8 = 0xf1;
pub const T6_LAYER2_SEMANTIC: u8 = 0xf2;
/// bo2mp: a sea's white-water noise and mask maps (0xf3 is the probes).
pub const T6_FOAM_NOISE_SEMANTIC: u8 = 0xf4;
pub const T6_FOAM_MASK_SEMANTIC: u8 = 0xf5;
const LAYER_MAPS: [(u32, u8); 2] = [
    (label_digest("colorMap1"), T6_LAYER1_SEMANTIC),
    (label_digest("colorMap2"), T6_LAYER2_SEMANTIC),
];

/// What linking did: capture material index -> catalog material index.
pub(crate) struct T6Materials {
    pub local: HashMap<usize, usize>,
    pub load_log: Vec<String>,
}

/// A GPU image over an IWI's own blocks: every mip level, largest first.
/// A cube map (the sky's) is six faces, each with its mips, read linear:
/// the sky shader decodes it as rgb / a in linear space.
pub(crate) fn gpu_image(iwi: &ipak_t6::IwiImage<'_>) -> Result<Image, String> {
    use ipak_t6::IwiFormat;
    let cube = iwi.is_cube();
    let texel_format = if cube {
        match iwi.format {
            IwiFormat::Dxt1 => Some(TextureFormat::Bc1RgbaUnorm),
            IwiFormat::Dxt3 => Some(TextureFormat::Bc2RgbaUnorm),
            IwiFormat::Dxt5 => Some(TextureFormat::Bc3RgbaUnorm),
            _ => None,
        }
    } else {
        match iwi.format {
            IwiFormat::Dxt1 => Some(TextureFormat::Bc1RgbaUnormSrgb),
            IwiFormat::Dxt3 => Some(TextureFormat::Bc2RgbaUnormSrgb),
            IwiFormat::Dxt5 => Some(TextureFormat::Bc3RgbaUnormSrgb),
            // Two-channel normal maps stay as stored.
            IwiFormat::Dxn => Some(TextureFormat::Bc5RgUnorm),
            // Plain pixel formats are expanded to RGBA8 below.
            IwiFormat::Rgba8 | IwiFormat::Rgb8 | IwiFormat::LuminanceAlpha | IwiFormat::Luminance | IwiFormat::Alpha => {
                Some(TextureFormat::Rgba8UnormSrgb)
            }
            _ => None,
        }
    };
    let Some(texel_format) = texel_format else {
        return Err(format!("{:?} colour maps are not drawn yet", iwi.format));
    };
    if iwi.is_volume() {
        return Err("a volume image".to_owned());
    }
    let (full_w, full_h) = (iwi.texel_width, iwi.height);
    if iwi.format.is_block_compressed() && (full_w | full_h) & 3 != 0 {
        return Err(format!("{full_w}x{full_h} is not whole 4x4 blocks"));
    }

    // Expands one face of one level to the GPU layout.
    let expand_into = |out: &mut Vec<u8>, src: &[u8]| match iwi.format {
        IwiFormat::Rgb8 => {
            for px in src.chunks_exact(3) {
                out.extend([px[0], px[1], px[2], u8::MAX]);
            }
        }
        IwiFormat::LuminanceAlpha => {
            for pair in src.chunks_exact(2) {
                let (lum, alpha) = (pair[0], pair[1]);
                out.extend([lum, lum, lum, alpha]);
            }
        }
        IwiFormat::Luminance => src.iter().for_each(|&lum| out.extend([lum, lum, lum, u8::MAX])),
        IwiFormat::Alpha => src.iter().for_each(|&alpha| out.extend([u8::MAX, u8::MAX, u8::MAX, alpha])),
        _ => out.extend_from_slice(src),
    };

    // On the fast setting a large flat picture drops its top level (still
    // whole blocks at half size); cube maps always keep every level.
    let drop_top = asset_core::t6_fast() && !cube && iwi.levels > 1 && full_w >= 128 && full_h >= 128;
    let first_level = drop_top as u32;

    // The file stores every face per level; the GPU wants every level per face.
    let face_count = iwi.faces as usize;
    let mut texels = Vec::new();
    for face in 0..face_count {
        for level in first_level..iwi.levels {
            let level_bytes = iwi.level(level);
            let stride = level_bytes.len() / face_count;
            let start = face * stride;
            expand_into(&mut texels, &level_bytes[start..start + stride]);
        }
    }

    let mut image = texture_2d(
        full_w >> first_level,
        full_h >> first_level,
        iwi.faces,
        texel_format,
        texels,
    );
    image.texture_descriptor.mip_level_count = iwi.levels - first_level;
    if cube {
        bind_as(&mut image, TextureViewDimension::Cube);
    }
    let wrap = ImageAddressMode::Repeat;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: wrap,
        address_mode_v: wrap,
        anisotropy_clamp: 8,
        ..ImageSamplerDescriptor::linear()
    });
    Ok(image)
}

/// The normal and specular maps BO2's lit shaders read (the shine), by
/// their material slot hashes (`normalMap`, `specularMap`).
/// (`SpecularAndGloss`, 0x8c297e80: a weapon camo's specular colour and
/// gloss, which BO2's camo shader reads where the lit one reads its
/// specular map: Pack-a-Punch's blue and gold sheen.)
/// (0x942cbff0: the broken-window shards' normal map, semantic 5, in
/// mc/glass_clear_shatter_mp_model beside a 0x8c297e80 specular map.)
const SHINE_MAPS: [(u32, u8); 4] = [
    (0x59d3_0d0f, SEMANTIC_NORMAL),
    (0x34ec_ccb3, SEMANTIC_SPECULAR),
    (0x8c29_7e80, SEMANTIC_SPECULAR),
    (0x942c_bff0, SEMANTIC_NORMAL),
];

/// The private semantic the reflection probes' cube array is linked under
/// (a material named `t6/reflection_probes` draws nothing).
pub const T6_PROBES_SEMANTIC: u8 = 0xf3;
pub const T6_PROBES_MATERIAL: &str = "t6/reflection_probes";

/// The world's reflection probes as one cube-array image. BO2 keeps each
/// probe's cube in the zone: BC3, 128x128, six faces each with its whole
/// mip chain, the order a GPU cube array is uploaded in. Probe 0 on every
/// multiplayer map is a 4x4 cube whose 18 blocks are all one block (black,
/// alpha 255): a cube of one colour, so its slot is that block at every
/// size and level. A probe in any other form is black. Linked as the
/// private semantic of a material that draws nothing.
/// bo2mp: each water's own reflection cube (`water_cubes`) gets one more
/// slot after the probes, in order, the array's last.
pub(crate) fn link_reflection_probes(
    catalog: &mut SurfaceShaderRoster,
    capture: &ZoneCapture,
    packs: Option<&PackSet>,
    world: &asset_t6::WorldRef,
    zone: &str,
) -> Option<String> {
    const SIZE: u32 = 128;
    const LEVELS: u32 = 8;
    if world.probe_list.is_empty() {
        return None;
    }
    let face_bytes: usize = (0..LEVELS)
        .map(|l| {
            let blocks = ((SIZE >> l).max(1) as usize).div_ceil(4);
            blocks * blocks * 16
        })
        .sum();
    let probe_bytes = face_bytes * 6;
    let mut data = Vec::with_capacity(probe_bytes * world.probe_list.len());
    let (mut real, mut one_colour) = (0usize, 0usize);
    for probe in &world.probe_list {
        let bc3 = probe
            .image
            .and_then(|k| capture.images.get(k.index))
            .and_then(|img| img.embedded.as_ref())
            .filter(|e| matches!(e.dxgi_format, 77 | 78));
        // One block for the whole cube: every block the same, and that
        // block one colour (all its alpha and colour indices 0).
        let one_block = bc3.and_then(|e| {
            let block = e.data.get(..16)?;
            (e.data.len() % 16 == 0
                && e.data.chunks_exact(16).all(|b| b == block)
                && block[2..8].iter().chain(&block[12..16]).all(|&b| b == 0))
            .then_some(block)
        });
        match (bc3.filter(|e| e.data.len() == probe_bytes), one_block) {
            (Some(e), _) => {
                data.extend_from_slice(&e.data);
                real += 1;
            }
            (None, Some(block)) => {
                data.extend(block.iter().cycle().take(probe_bytes));
                one_colour += 1;
            }
            (None, None) => data.resize(data.len() + probe_bytes, 0),
        }
    }
    let cubes = packs.map(|p| water_cubes(capture, p)).unwrap_or_default();
    let mut sea_line = String::new();
    for (name, cube) in &cubes {
        // sea_cube_bc3 always makes a whole slot; a short one would shift
        // every water's slot, so it stands in black.
        if cube.len() == probe_bytes {
            data.extend_from_slice(cube);
        } else {
            data.resize(data.len() + probe_bytes, 0);
        }
        sea_line += &format!(", water's own cube {name}");
    }
    if !cubes.is_empty() {
        sea_line += " last";
    }
    let cube_faces = (data.len() / face_bytes) as u32;
    let mut cubes = texture_2d(SIZE, SIZE, cube_faces, TextureFormat::Bc3RgbaUnorm, data);
    cubes.texture_descriptor.mip_level_count = LEVELS;
    bind_as(&mut cubes, TextureViewDimension::CubeArray);
    let edge = SIZE as u16;
    let row = HandBuiltTexture {
        texture_kind: 5,
        tier_total: LEVELS as u8,
        ..built_row(T6_PROBES_MATERIAL.to_owned(), T6_PROBES_SEMANTIC, false, edge, edge, cubes)
    };
    let slot = catalog.add_texture(row);
    let bindings = vec![binding(0, 0, T6_PROBES_SEMANTIC, slot)];
    let owner = ZoneCustodian::intern(zone);
    catalog.add_material(bare_material(T6_PROBES_MATERIAL.to_owned(), owner, bindings));
    Some(format!(
        "t6 reflection probes: {} ({real} cube maps, {one_colour} one-colour cubes, {} stand-ins black), one {SIZE}x{SIZE} cube array, {LEVELS} mips{sea_line}",
        world.probe_list.len(),
        world.probe_list.len() - real - one_colour
    ))
}

/// bo2mp: the sea's own reflection cube. BO2's sea shader
/// (`cod7watershore`, fxc disassembly) reflects its material's
/// `customReflectionMap` (Cove's `skybox_mp_castaway_water_ft`), not the
/// world's reflection probes; the pond (`cod7waterflowreflect`) its
/// `reflectionMap` (Concert's river and pond each their own). Each
/// distinct cube that decodes, in material order: its name and its faces
/// as one of the probe array's last slots (BC3, 128x128, eight levels per
/// face; a larger cube drops its top levels, a BC1 one gets an opaque alpha
/// block per block).
pub(crate) fn water_cubes(capture: &ZoneCapture, packs: &PackSet) -> Vec<(String, Vec<u8>)> {
    let mut cubes: Vec<(String, Vec<u8>)> = Vec::new();
    for m in &capture.materials {
        let Some(image) = water_cube_image(capture, m) else { continue };
        if cubes.iter().any(|c| c.0 == image.name) {
            continue;
        }
        if let Ok(cube) = sea_cube_bc3(packs, image) {
            cubes.push((image.name.clone(), cube));
        }
    }
    cubes
}

/// A sea's `customReflectionMap` or the pond's `reflectionMap` image, if
/// it is a cube.
fn water_cube_image<'a>(capture: &'a ZoneCapture, material: &asset_t6::MaterialRef) -> Option<&'a ImageRef> {
    let method_set = material.method_set.and_then(|k| capture.technique_sets.get(k.index))?;
    let hash = match asset_core::t6_water_kind(&method_set.name)? {
        0 => return None,
        1 => 0xf973_2228,
        _ => 0x0512_ed3b,
    };
    let key = slot_texture(material, hash)?.image?;
    capture.images.get(key.index).filter(|i| i.map_kind == 5)
}

/// One cube from the packs as the probe array's BC3 slot.
fn sea_cube_bc3(packs: &PackSet, image: &ImageRef) -> Result<Vec<u8>, String> {
    use ipak_t6::IwiFormat;
    const SIZE: u32 = 128;
    const LEVELS: u32 = 8;
    let Some(ImageSource::Pack(i, entry)) = packs.locate(image) else {
        return Err("in no pack".to_owned());
    };
    let bytes = packs.packs[i].read(entry)?;
    let iwi = ipak_t6::parse_iwi(&bytes).map_err(|e| e.to_string())?;
    let w = iwi.texel_width;
    if !iwi.is_cube() || iwi.faces != 6 || w != iwi.height || w < SIZE || !w.is_power_of_two() {
        return Err(format!("not a square cube of {SIZE} or more ({w}x{})", iwi.height));
    }
    let bc1 = match iwi.format {
        IwiFormat::Dxt1 => true,
        IwiFormat::Dxt5 => false,
        other => return Err(format!("{other:?} cube")),
    };
    let skip = (w / SIZE).trailing_zeros();
    let mut out = Vec::new();
    for face in 0..6 {
        for l in 0..LEVELS {
            let blocks = ((SIZE >> l).max(1) as usize).div_ceil(4).pow(2);
            // A chain that stops at 4x4 repeats its last block below it.
            let level = (skip + l).min(iwi.levels - 1);
            let all = iwi.level(level);
            let stride = all.len() / 6;
            let src = &all[face * stride..(face + 1) * stride];
            let unit = if bc1 { 8 } else { 16 };
            if src.len() / unit != blocks && !(blocks == 1 && src.len() >= unit) {
                return Err(format!("level {level} has {} blocks, {blocks} wanted", src.len() / unit));
            }
            for block in src.chunks_exact(unit).take(blocks) {
                if bc1 {
                    out.extend_from_slice(&[255, 255, 0, 0, 0, 0, 0, 0]);
                    out.extend_from_slice(&bc1_as_four_colour(block));
                } else {
                    out.extend_from_slice(block);
                }
            }
        }
    }
    Ok(out)
}

/// A BC1 colour block as BC3's colour half, which always reads four
/// colours: a three-colour block (first colour not above the second) swaps
/// its colours and maps its middle colour to a third and its black to the
/// darker end.
fn bc1_as_four_colour(block: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    out.copy_from_slice(&block[..8]);
    let (c0, c1) = (u16::from_le_bytes([block[0], block[1]]), u16::from_le_bytes([block[2], block[3]]));
    if c0 > c1 {
        return out;
    }
    out[..2].copy_from_slice(&block[2..4]);
    out[2..4].copy_from_slice(&block[..2]);
    let luma = |c: u16| (c >> 11) as u32 * 2 + ((c >> 5) & 63) as u32 + (c & 31) as u32;
    let dark = if luma(c1) < luma(c0) { 0 } else { 1 };
    let mut bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let mut mapped = 0u32;
    for texel in 0..16 {
        let new = match bits & 3 {
            0 => 1,
            1 => 0,
            2 => 2,
            _ => dark,
        };
        mapped |= new << (texel * 2);
        bits >>= 2;
    }
    out[4..].copy_from_slice(&mapped.to_le_bytes());
    out
}

/// Decode one captured image from the packs.
pub(crate) fn decode(packs: &PackSet, image: &ImageRef) -> Result<Image, String> {
    match packs.locate(image) {
        Some(ImageSource::Pack(i, entry)) => {
            let bytes = packs.packs[i].read(entry)?;
            let iwi = ipak_t6::parse_iwi(&bytes).map_err(|e| e.to_string())?;
            gpu_image(&iwi)
        }
        Some(ImageSource::Embedded) => embedded_image(image),
        Some(ImageSource::Empty) => Err("no pixels".to_owned()),
        None => Err("in no pack".to_owned()),
    }
}

/// An image whose pixels ride in the zone itself (`GfxImageLoadDef`):
/// Zombies Declassified keeps its fire, embers and chalk there. The data is
/// each face's levels, largest first, as the GPU takes it.
fn embedded_image(image: &ImageRef) -> Result<Image, String> {
    let e = image.embedded.as_ref().ok_or("no pixels")?;
    // DXGI formats: BC1-3 and BC5 (UNORM / SRGB), R8G8B8A8.
    let (tex_format, unit, block) = match e.dxgi_format {
        71 | 72 => (TextureFormat::Bc1RgbaUnormSrgb, 8, 4),
        74 | 75 => (TextureFormat::Bc2RgbaUnormSrgb, 16, 4),
        77 | 78 => (TextureFormat::Bc3RgbaUnormSrgb, 16, 4),
        83 => (TextureFormat::Bc5RgUnorm, 16, 4),
        // bo2mp: a 1x1 DXGI 28 (`$identitynormalmap`, `$white`, `$black`) is
        // read as stored; the bigger ones (embers, chalk) are colour.
        28 if image.width == 1 && image.height == 1 => (TextureFormat::Rgba8Unorm, 4, 1),
        28 | 29 => (TextureFormat::Rgba8UnormSrgb, 4, 1),
        // bo2mp Raid: A8 (the world's `$outdoor` lookup), read as one channel.
        65 => (TextureFormat::R8Unorm, 1, 1),
        other => return Err(format!("in-zone DXGI format {other} is not drawn yet")),
    };
    let (w, h) = (u32::from(image.width), u32::from(image.height));
    let levels = u32::from(e.tier_total.max(1));
    let size = |l: u32| {
        let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
        (lw.div_ceil(block) * lh.div_ceil(block)) as usize * unit
    };
    let need: usize = (0..levels).map(size).sum();
    // A cube (the sky's; `MAPTYPE_CUBE`) carries six faces, each with its
    // mips, in GPU upload order; read linear, as the pack path's sky.
    let cube = image.map_kind == 5 || (need > 0 && e.data.len() == need * 6);
    let (faces, format) = match (cube, tex_format) {
        (true, TextureFormat::Bc1RgbaUnormSrgb) => (6, TextureFormat::Bc1RgbaUnorm),
        (true, TextureFormat::Bc2RgbaUnormSrgb) => (6, TextureFormat::Bc2RgbaUnorm),
        (true, TextureFormat::Bc3RgbaUnormSrgb) => (6, TextureFormat::Bc3RgbaUnorm),
        (true, TextureFormat::Rgba8UnormSrgb) => (6, TextureFormat::Rgba8Unorm),
        (true, TextureFormat::Rgba8Unorm) => (6, TextureFormat::Rgba8Unorm),
        (true, _) => {
            return Err(format!(
                "in-zone DXGI {} cube is not drawn yet",
                e.dxgi_format
            ));
        }
        (false, f) => (1, f),
    };
    let need = need * faces as usize;
    if w == 0 || h == 0 || (block == 4 && (w % 4 != 0 || h % 4 != 0)) {
        return Err(format!("in-zone {w}x{h} is not whole 4x4 blocks"));
    }
    if e.data.len() < need {
        return Err(format!(
            "in-zone pixels {} bytes, {need} needed",
            e.data.len()
        ));
    }
    let mut out = texture_2d(w, h, faces, format, e.data[..need].to_vec());
    out.texture_descriptor.mip_level_count = levels;
    if cube {
        bind_as(&mut out, TextureViewDimension::Cube);
    }
    out.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..Default::default()
    });
    Ok(out)
}

/// The texture a material is coloured with: its `colorMap`; else, among its
/// colour-map-semantic textures (layered and ember materials name their
/// slots differently), the one whose image is named as a colour map (`_c`,
/// `_col`), not an ember or glow layer (`_e`); else the first of them.
/// A tile-blend material is coloured by its `<name>_cheap` stand-in's
/// colour map (BO2's own mix of the two tiles, `..._c_cmb`), else by its
/// first tile; never by its macro map.
pub(crate) fn colour_texture<'a>(
    material: &'a asset_t6::MaterialRef,
    capture: &'a ZoneCapture,
) -> Option<&'a asset_t6::ShaderSamplerImage> {
    if slot_texture(material, TILE_BLEND_MACRO_HASH).is_some() {
        let cheap = format!("{}_cheap", material.name);
        let mixed = capture
            .materials
            .iter()
            .find(|m| m.name == cheap)
            .and_then(|m| filled_slot(m, COLOR_MAP_HASH));
        if let Some(t) = mixed.or_else(|| filled_slot(material, TILE_BLEND_MICRO1_HASH)) {
            return Some(t);
        }
    }
    // A colour map is named `_c` / `_col`; the burning-ember materials also
    // carry an animated glow noise named `_c` (`glowcycle_random_01_c`),
    // which sorts first in their table, so it never counts as colour.
    // Masks (`c_gen_arm_rim_mask_c`, 32x32 on the viewmodel arms) are named
    // like colour maps too; the real one is the largest.
    let colour_named = |t: &&asset_t6::ShaderSamplerImage| {
        t.image
            .and_then(|k| capture.images.get(k.index))
            .is_some_and(|img| {
                (img.name.ends_with("_c") || img.name.ends_with("_col"))
                    && !img.name.contains("glowcycle")
                    && !img.name.contains("_mask")
            })
    };
    let area = |t: &&asset_t6::ShaderSamplerImage| {
        t.image
            .and_then(|k| capture.images.get(k.index))
            .map_or(0u32, |img| u32::from(img.width) * u32::from(img.height))
    };
    let semantic = || {
        material
            .textures
            .iter()
            .filter(|t| t.semantic == SEMANTIC_COLOR && t.image.is_some())
    };
    // bo2mp: the sw4 shaders' `DiffuseAndGloss` (Dig's tarps: their frayed
    // edge texture is named `_c`, the diffuse is not).
    filled_slot(material, COLOR_MAP_HASH)
        .or_else(|| filled_slot(material, DIFFUSE_AND_GLOSS_HASH))
        .or_else(|| filled_slot(material, SPRITE_MAP_HASH))
        .or_else(|| semantic().filter(colour_named).max_by_key(area))
        .or_else(|| semantic().next())
        .or_else(|| filled_slot(material, LASER_SCAN_IMAGE_HASH))
        // bo2mp: a 2D picture's one image (BO2's menu backings,
        // `sw4_2d_tile`: semantic 0, no colour-map name).
        .or_else(|| {
            let mut t = material.textures.iter().filter(|t| t.semantic == 0 && t.image.is_some());
            let first = t.next();
            first.filter(|_| t.next().is_none())
        })
}

/// A 1x1 stand-in for a colour map with no pixels here, by its name: white,
/// black, else mid grey ("grey").
fn flat_image(name: &str) -> Image {
    let folded = name.to_ascii_lowercase();
    // bo2zm: an effect image with no pixels (`fxt_*`, BO1 effects) must
    // draw nothing, not a grey or white quad.
    let fx_gap = folded.starts_with("fxt_") || folded.starts_with("fx_");
    let shade: u8 = match () {
        _ if fx_gap => 0x00,
        _ if folded.contains("white") => 0xff,
        _ if folded.contains("black") => 0x00,
        _ => 0x80,
    };
    texture_2d(1, 1, 1, TextureFormat::Rgba8UnormSrgb, vec![shade, shade, shade, if fx_gap { 0x00 } else { 0xff }])
}

/// bo2mp Raid: the world's outdoor lookup picture (`GfxWorld::outdoorImage`,
/// `$outdoor`), which no material names, linked under its own name.
pub(crate) fn link_outdoor_image(
    catalog: &mut SurfaceShaderRoster,
    packs: &PackSet,
    image: &ImageRef,
) -> Result<usize, String> {
    let gpu = decode(packs, image)?;
    let row = built_row(image.name.clone(), SEMANTIC_COLOR, false, image.width, image.height, gpu);
    Ok(catalog.add_texture(row))
}

/// bo2mp maps: a shine map named `,<image>` is a picture another zone holds
/// (common_mp's `global_normal_flat_16x16` in Aftermath's puddle decals).
/// Until the earlier-zones pass gives it its pixels it draws as an empty
/// slot does (the renderer's flat normal, no specular), under the `#flat`
/// name the colour maps' stand-ins use.
fn foreign_shine_stand_in(
    catalog: &mut SurfaceShaderRoster,
    capture: &ZoneCapture,
    key: asset_t6::AssetIdentity,
    semantic: u8,
) -> Option<usize> {
    let name = capture.images.get(key.index)?.name.as_str();
    if !name.starts_with(',') {
        return None;
    }
    let texel: Vec<u8> = match semantic {
        SEMANTIC_NORMAL => vec![128, 128, 255, 255],
        _ => vec![0; 4],
    };
    let flat = texture_2d(1, 1, 1, TextureFormat::Rgba8Unorm, texel);
    Some(catalog.add_texture(built_row(format!("{name}#flat"), semantic, false, 1, 1, flat)))
}

/// bo2mp: an f32 as an IEEE half float (round to nearest; tiny values to
/// zero, huge ones to infinity).
fn half_bits(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = b & 0x7f_ffff;
    if v.is_nan() {
        return 0x7e00;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    if exp <= 0 {
        return sign;
    }
    let half = sign | ((exp as u16) << 10) | (mant >> 13) as u16;
    // Round to nearest (a carry into the exponent is still right).
    if mant & 0x1000 != 0 { half + 1 } else { half }
}

/// bo2mp: a water material's constants for the fallback draw's water
/// shader, one RGBA16F texel each (BO2's own `cod7water` / `cod7watershore`
/// shaders, fxc disassembly; the constants by their hashes, as the shaders'
/// register arguments name them):
/// 0..3 the four normal-map layers (world xy scale, scroll per second),
/// 4 normal scale and bias (first map, second map), 5 the layers' weights,
/// 6 highlight powers, fresnel bias and scale, 7 highlight strengths, their
/// overall scale and the kind (0 pool, 1 sea, 2 open sea: no vertex
/// alpha), 8 and 9 the water colour and opacity (pool: grazing and facing;
/// sea: scatter and body, deep),
/// 10 the sea's shadow light, 11..13 the sea's own reflection cube
/// (`water_cubes`): horizonColor and its slot counted from the probe
/// array's end (1 = last; 0 = none), customReflectionProbeScale, and the
/// distance fade (the alphas of waterColorShallow and waterColorDeepDelta)
/// with 1 when its shader takes no sun shadow; the pool's 13 its
/// refraction: the thickness scale (waterControlVar1.w: 1 / the depth
/// behind it at which it turns fully opaque) and the offset in pixels (8).
/// The pond (`cod7waterflowreflect`, kind 3) packs its own: 0..2 its three
/// normal layers (Lo, Lo, Hi), 4 raw, 5 the weights, 6 fresnel bias and
/// scale, 7 the kind, 8 and 9 scatter and body (deep), 10 normalMapScale
/// (fresnel normal, reflection normal), 11 its cube's slot, 12
/// probeScale (its w the opacity), 13 its refraction (thickness scale:
/// waterColorDeepScatterDelta.w; offset: waterColorShallowScatter.w), and
/// in 7's xyz its screen reflection's three ray lengths (rayDistance).
fn water_table(material: &asset_t6::MaterialRef, kind: u8, own_cube: usize, unshadowed: bool) -> Vec<[f32; 4]> {
    let c = |hash: u32| {
        material
            .constants
            .iter()
            .find(|c| c.0 == hash)
            .map_or([0.0; 4], |c| c.2)
    };
    let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    if kind == 0 {
        // cod7water: waterNormalPositionScale, waterLoNormalScroll,
        // waterHiNormalScroll, waterNormalScrollSpeed (vertex shader:
        // uv = pos.xy * scale + gameTime * scroll * speed), waterNormalScale,
        // waterControlVar0/1, waterColorN, waterColorS.
        let scale = c(0xc728_b4a7);
        let lo = c(0x6e97_1968);
        let hi = c(0x01a2_0bea);
        let speed = c(0x134c_d3cc);
        let control1 = c(0xa7d6_56a6);
        vec![
            [scale[0], scale[0], lo[0] * speed[0], lo[1] * speed[0]],
            [scale[1], scale[1], lo[2] * speed[1], lo[3] * speed[1]],
            [scale[2], scale[2], hi[0] * speed[2], hi[1] * speed[2]],
            [scale[3], scale[3], hi[2] * speed[3], hi[3] * speed[3]],
            c(0x8632_8e1e),
            [1.0; 4],
            c(0xa7d6_56a7),
            [control1[0], control1[1], control1[2], 0.0],
            c(0xbd74_8e86),
            c(0xbd74_8e9b),
            [0.0; 4],
            [0.0; 4],
            [0.0; 4],
            [control1[3], 8.0, 0.0, 0.0],
        ]
    } else if kind == 2 {
        // cod7waterflowreflect: normalUVScrollA (normalMapHi), B and C
        // (normalMapLo; uv = pos.xy * scroll.x + gameTime * scroll.yz),
        // normalMapScale, the deep colours (its control map is white),
        // fresnel bias and scale (the alphas of waterColorShallow and
        // waterColorDeepDelta), probeScale.
        let scroll = |hash: u32| {
            let s = c(hash);
            [s[0], s[0], s[1], s[2]]
        };
        let (shallow, deep) = (c(0x7429_46aa), c(0x9092_abc4));
        vec![
            scroll(0x1b34_e27f),
            scroll(0x1b34_e27e),
            scroll(0x1b34_e27c),
            [0.0; 4],
            [1.0, 0.0, 1.0, 0.0],
            [1.0, 1.0, 1.0, 0.0],
            [0.0, 0.0, shallow[3], deep[3]],
            {
                let ray = c(0x9356_8369);
                [ray[0], ray[1], ray[2], 3.0]
            },
            add(c(0xe887_d68c), c(0xeef4_77a2)),
            add(shallow, deep),
            c(0xe618_4bb7),
            [0.0, 0.0, 0.0, own_cube as f32],
            c(0x27d0_bbb2),
            [c(0xeef4_77a2)[3], c(0xe887_d68c)[3], 0.0, 0.0],
        ]
    } else {
        // cod7watershore: normalUVscroll_0123, normalUVcontrol0/2/3 (uv =
        // pos.xy * control.xy + gameTime * scroll * control.zw; layer 1 is
        // unused), normalScaleLoHi, controlVar0/1, the deep colours
        // (shallow + deep delta: the control map's depth reads 1 on open
        // water), waterOpacityControl.w, customLightmapShadowColor.
        let scroll = c(0x95ba_cba2);
        let c0 = c(0x9ea1_a767);
        let c2 = c(0x9ea1_a765);
        let c3 = c(0x9ea1_a764);
        let control1 = c(0x483f_c933);
        // The open-sea variants (Carrier, Takeoff, Frostbite's distant sea)
        // have no opacity control: they draw opaque and never read the
        // vertex colour (kind 2; their shader refracts nothing).
        let opacity_control = material.constants.iter().find(|c| c.0 == 0x8898_e1ab);
        let opacity = opacity_control.map_or(1.0, |c| c.2[3]);
        let kind = if opacity_control.is_some() { 1.0 } else { 2.0 };
        let mut scatter = add(c(0xe887_d68c), c(0xeef4_77a2));
        let mut body = add(c(0x7429_46aa), c(0x9092_abc4));
        let (shallow_w, deep_w) = (c(0x7429_46aa)[3], c(0x9092_abc4)[3]);
        scatter[3] = opacity;
        body[3] = opacity;
        let horizon = c(0x4980_5a5a);
        let scale = c(0x0b19_8186);
        vec![
            [c0[0], c0[1], scroll[0] * c0[2], scroll[0] * c0[3]],
            [0.0; 4],
            [c2[0], c2[1], scroll[2] * c2[2], scroll[2] * c2[3]],
            [c3[0], c3[1], scroll[3] * c3[2], scroll[3] * c3[3]],
            c(0xb47c_f4c9),
            [1.0, 0.0, 1.0, 1.0],
            c(0x483f_c932),
            [control1[0], control1[1], 0.3184, kind],
            scatter,
            body,
            c(0xc9e7_ae4a),
            [horizon[0], horizon[1], horizon[2], own_cube as f32],
            [scale[0], scale[1], scale[2], 0.0],
            [shallow_w, deep_w, f32::from(u8::from(unshadowed)), 0.0],
            // The shore: shoreUVcontrol (uv = pos.xy * xy + zw), controlVar1's
            // control-map and shore-picture distortions, and the two deep
            // deltas (body and scatter = base + delta * the control map's g).
            c(0x19cc_0727),
            [control1[2], control1[3], 0.0, 0.0],
            c(0x9092_abc4),
            c(0xeef4_77a2),
            // White water: whiteWaterNoiseScale/Scroll (two noise uvs, xy
            // and zw = pos.xy * scale + gameTime * scroll),
            // whiteWaterMaskScale01/23 and MaskScroll01/23 (four mask uvs),
            // whiteWaterColor (rgb; w scales the body under the foam).
            c(0xc794_6294),
            c(0xba17_7021),
            c(0x4c53_b0ff),
            c(0x4c53_b0bf),
            c(0xa5ba_130a),
            c(0xa5ba_12ca),
            c(0x2cc3_720f),
        ]
    }
}

/// bo2mp: an emissive flow material's (molten lava) extra pictures, from
/// BO2's `cod7emissiveflow` shaders (fxc disassembly; textures and
/// constants by the hashes its register arguments name): its masks packed
/// in one RGBA8 picture (emissiveMapHi alpha, emissiveMapLo alpha,
/// noiseMap green, noiseMap alpha; resampled to the high mask's size), and
/// its colour remap with a last row of constants (RGBA16F): diffuse uv
/// control, emissive uv scale, emissive uv scroll, noise uv control,
/// diffuse control, emissive remap uv, diffuse filter, emissive filter.
fn flow_images(
    material: &asset_t6::MaterialRef,
    decoded: &dyn Fn(u32) -> Option<(usize, usize, Vec<u8>)>,
) -> Option<(Image, Image)> {
    let (w, h, hi) = decoded(0xd1bf_3646)?;
    let lo = decoded(0xd1bf_35c4)?;
    let noise = decoded(0x7350_2222)?;
    let at = |p: &(usize, usize, Vec<u8>), x: usize, y: usize, c: usize| {
        let (pw, ph, px) = p;
        px[((y * ph / h) * pw + x * pw / w) * 4 + c]
    };
    let mut mask = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            mask.extend_from_slice(&[
                hi[(y * w + x) * 4 + 3],
                at(&lo, x, y, 3),
                at(&noise, x, y, 1),
                at(&noise, x, y, 3),
            ]);
        }
    }
    let mask = texture_2d(w as u32, h as u32, 1, TextureFormat::Rgba8Unorm, mask);
    let (rw, rh, remap) = decoded(0xd0f6_1c30)?;
    if rw < 8 {
        return None;
    }
    let c = |hash: u32| constant_or(material, hash, [0.0; 4]);
    let constants = [
        c(0xb7a9_a24a),
        c(0xe75d_ae00),
        c(0xd31b_b535),
        c(0x2e12_415a),
        c(0x557d_6789),
        c(0xe6ed_ea93),
        c(0xd4f6_818e),
        c(0x1d90_b95b),
    ];
    let mut rows: Vec<[f32; 4]> = remap
        .chunks_exact(4)
        .map(|p| [0, 1, 2, 3].map(|k| f32::from(p[k]) / 255.0))
        .collect();
    let mut last = vec![[0.0f32; 4]; rw];
    last[..constants.len()].copy_from_slice(&constants);
    rows.extend(last);
    let mut table = water_image(&rows);
    table.texture_descriptor.size = Extent3d {
        width: rw as u32,
        height: rh as u32 + 1,
        depth_or_array_layers: 1,
    };
    Some((mask, table))
}

/// bo2mp: an emissive tile material's (Magma's lava rocks) glow as one
/// RGBA16F picture, from BO2's `sw4_3d_cod7_emissive_tile` shaders (fxc
/// disassembly): (emissive A + emissive B at Micro_Scale) * A's alpha *
/// hdrAmount, before the shader squares it; B averaged over the texels one
/// A texel covers. Then its constants (Micro_Scale, Micro_Height,
/// Macro_Height) as a 1x1 RGBA16F picture.
fn tile_images(
    material: &asset_t6::MaterialRef,
    decoded: &dyn Fn(u32) -> Option<(usize, usize, Vec<u8>)>,
) -> Option<(Image, Image)> {
    let (w, h, a) = decoded(0x7c73_d0d6)?;
    let (bw, bh, b) = decoded(0xa4af_0cde)?;
    let c = |hash: u32| constant_or(material, hash, [0.0; 4]);
    let hdr = c(0x00e2_62b2)[0];
    let micro = c(0x00ef_311d);
    let (sx, sy) = (micro[0].max(0.0), micro[1].max(0.0));
    let unit = |byte: u8| f32::from(byte) / 255.0;
    // The B texels (wrapping) that A texel `p` covers along one axis once
    // B is tiled `scale` times over A.
    let covered = |p: usize, scale: f32, a_len: usize, b_len: usize| {
        let edge = |q: usize| q as f32 * scale * b_len as f32 / a_len as f32;
        let first = edge(p).floor() as i64;
        let end = edge(p + 1).ceil() as i64;
        (first, end.max(first + 1))
    };
    let mut rows = Vec::with_capacity(w * h);
    for (index, a_px) in a.chunks_exact(4).take(w * h).enumerate() {
        let (x, y) = (index % w, index / w);
        let (x_from, x_to) = covered(x, sx, w, bw);
        let (y_from, y_to) = covered(y, sy, h, bh);
        let mut total = [0.0f32; 3];
        let mut count = 0.0f32;
        for by in y_from..y_to {
            let row = by.rem_euclid(bh as i64) as usize * bw;
            for bx in x_from..x_to {
                let at = (row + bx.rem_euclid(bw as i64) as usize) * 4;
                total.iter_mut().zip(&b[at..at + 3]).for_each(|(t, &v)| *t += unit(v));
                count += 1.0;
            }
        }
        let coverage = unit(a_px[3]);
        let mix = |k: usize| (unit(a_px[k]) + total[k] / count.max(1.0)) * coverage * hdr;
        rows.push([mix(0), mix(1), mix(2), 1.0]);
    }
    let mut glow = water_image(&rows);
    glow.texture_descriptor.size = Extent3d {
        width: w as u32,
        height: h as u32,
        depth_or_array_layers: 1,
    };
    let consts = water_image(&[[sx, sy, c(0x0e10_685a)[0], c(0x1f32_ca52)[0]]]);
    Some((glow, consts))
}

/// bo2mp: the water table as an RGBA16F picture (read by texel).
fn water_image(rows: &[[f32; 4]]) -> Image {
    let mut halves = Vec::with_capacity(rows.len() * 8);
    for value in rows.iter().flatten() {
        halves.extend_from_slice(&half_bits(*value).to_le_bytes());
    }
    texture_2d(rows.len() as u32, 1, 1, TextureFormat::Rgba16Float, halves)
}

/// An objective material's two colours as a 2x1 sRGB picture (linear
/// colours in, so a sampled texel gives them back).
fn objective_image(lo: [f32; 4], hi: [f32; 4]) -> Image {
    let enc = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    };
    let pixels: Vec<u8> = [lo, hi]
        .iter()
        .flat_map(|colour| [enc(colour[0]), enc(colour[1]), enc(colour[2]), 255])
        .collect();
    texture_2d(2, 1, 1, TextureFormat::Rgba8UnormSrgb, pixels)
}

/// `R_HashString` of the water colours (`waterColorNear`, `waterColorS..`).
const WATER_COLOR_NEAR_HASH: u32 = 0xbd74_8e86;
const WATER_COLOR_S_HASH: u32 = 0xbd74_8e9b;
/// Black Ops 1's water (the Declassified maps' `wpc_cod7waterflowreflect`):
/// its colour in the shallows and the change to its colour in the deep.
const WATER_SHALLOW_HASH: u32 = label_digest("waterColorShallow");
const WATER_DEEP_DELTA_HASH: u32 = label_digest("waterColorDeepDelta");

/// A water material's colour as a 1x1 picture: waterColorNear and
/// waterColorS mixed half and half (the shader's mix by N.V at about 60
/// degrees), less its fresnel share there (0.05), alpha theirs. The world
/// shader squares the texel's raw value, so the raw value is the root of the
/// linear colour. Black Ops 1's water has a shallow colour and a deep one
/// (shallow plus the delta, by the depth behind it): drawn at half depth
/// (Shi No Numa's swamp: a muddy brown, clear at the banks, solid deep).
fn cod7_water_image(material: &asset_t6::MaterialRef) -> Image {
    let colour = |hash: u32| material.constants.iter().find(|c| c.0 == hash).map(|c| c.2);
    let mix: [f32; 4] = match (colour(WATER_COLOR_NEAR_HASH), colour(WATER_SHALLOW_HASH)) {
        (None, Some(shallow)) => {
            let delta = colour(WATER_DEEP_DELTA_HASH).unwrap_or([0.0; 4]);
            std::array::from_fn(|k| shallow[k] + 0.5 * delta[k])
        }
        (near, _) => {
            let near = near.unwrap_or([0.08, 0.07, 0.04, 0.85]);
            let far = colour(WATER_COLOR_S_HASH).unwrap_or(near);
            std::array::from_fn(|k| 0.5 * (near[k] + far[k]))
        }
    };
    let raw = |k: usize| ((mix[k] * 0.95).clamp(0.0, 1.0).sqrt() * 255.0).round() as u8;
    let alpha = mix[3].clamp(0.0, 1.0);
    let mut image = Image::new_uninit(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(vec![raw(0), raw(1), raw(2), (alpha * 255.0).round() as u8]);
    image
}

/// Link the wanted capture materials into `catalog`, each with its colour
/// map decoded.
pub(crate) fn link_materials(
    catalog: &mut SurfaceShaderRoster,
    capture: &ZoneCapture,
    packs: Option<&PackSet>,
    wanted: impl IntoIterator<Item = usize>,
    zone: &str,
) -> T6Materials {
    link_materials_with(catalog, capture, packs, wanted, zone, None)
}

/// `link_materials`, with the real images of other zones by name: a
/// comma-named image is a reference to one another zone defines.
pub(crate) fn link_materials_with(
    catalog: &mut SurfaceShaderRoster,
    capture: &ZoneCapture,
    packs: Option<&PackSet>,
    wanted: impl IntoIterator<Item = usize>,
    zone: &str,
    foreign_images: Option<&HashMap<String, ImageRef>>,
) -> T6Materials {
    let owner = ZoneCustodian::intern(zone);
    let mut images: HashMap<usize, Option<usize>> = HashMap::new();
    let mut local = HashMap::new();
    let (mut coloured, mut uncoloured, mut decoded) = (0usize, 0usize, 0usize);
    let mut emissive_only = 0usize;
    let mut emissive_names: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    // bo2mp Raid: pictures a shadow-only material names but no zone here
    // holds (`wpc/shadowcaster`'s `,shadow`); never drawn in colour, so no gap.
    let mut shadow_only: Vec<String> = Vec::new();
    let mut draws: HashMap<T6Draw, usize> = HashMap::new();
    let mut cube_names: Option<Vec<String>> = None;
    for mi in wanted {
        if local.contains_key(&mi) {
            continue;
        }
        let Some(mat_def) = capture.materials.get(mi) else {
            continue;
        };
        let method_set = mat_def
            .method_set
            .and_then(|k| capture.technique_sets.get(k.index))
            .map_or("", |t| t.name.as_str());
        // The main technique's draw state: lit with sun shadow, lit with
        // sun, lit, emissive, then unlit (the first the material draws in).
        let state = [6usize, 5, 4, 3, 2].iter().find_map(|&t| {
            let entry = *mat_def.state_bits_entry.get(t)?;
            mat_def.state_words.get(usize::from(entry)).copied()
        });
        let mut draw = T6Draw::from_technique_set(method_set, mat_def.sort_rank, state);
        // bo2mp test aid: `BO2MP_T6_HIDE_MAT=a,b` leaves out every material
        // whose name contains one of them (bisecting a stray surface).
        if let Ok(hide) = std::env::var("BO2MP_T6_HIDE_MAT")
            && hide
                .split(',')
                .any(|h| !h.is_empty() && mat_def.name.contains(h))
        {
            draw.blend = asset_core::T6Blend::CasterOnly;
        }
        // bo2mp: a material whose surface flags say "only cast shadow"
        // (0x80000, the `onlycastshadow` infoParm: `wpc/shadowcaster` and
        // trees' shadow cards like Raid's `mtl_ctl_tree_shadow_caster`)
        // never draws in colour.
        if mat_def.surf_bits & 0x8_0000 != 0 {
            draw.blend = asset_core::T6Blend::CasterOnly;
        }
        // bo2mp: surface flag 0x80 (`nodraw`) is never drawn in colour either:
        // set only on clip, caulk and shadow-caster materials, and on
        // Carrier's `mtl_t6_veh_drone_quad_rotor_blades_dead` (the Blackhawk's
        // rotor discs, an all-black texture that drew as a black octagon).
        if mat_def.surf_bits & 0x80 != 0 {
            draw.blend = asset_core::T6Blend::CasterOnly;
        }
        // bo2zm: a colour map that is Treyarch's `deleteme` placeholder (the
        // magenta "DELETE ME" card) stands where a ported asset lost its
        // image (Declassified's frozen-zombie ice slush shell, BO1 Winter's
        // Howl); it draws nothing, so the ice shell over it shows.
        if colour_texture(mat_def, capture)
            .and_then(|t| t.image)
            .and_then(|k| capture.images.get(k.index))
            .is_some_and(|img| img.name.to_ascii_lowercase().ends_with("deleteme"))
        {
            draw.blend = asset_core::T6Blend::CasterOnly;
        }
        // bo2zm 27c: a material that draws in the emissive technique and in
        // none of the lit ones (TECHNIQUE_LIT_BEGIN 4 to TECHNIQUE_LIT_END 26),
        // like Die Rise's cloud sheets, takes no light: BO2 draws it unlit.
        let drawn = |t: usize| {
            mat_def
                .state_bits_entry
                .get(t)
                .is_some_and(|&e| usize::from(e) < mat_def.state_words.len())
        };
        if !draw.unlit && drawn(3) && !(4..26).any(drawn) {
            draw.unlit = true;
            emissive_only += 1;
            if emissive_names.len() < 8 {
                let constants: Vec<_> = mat_def
                    .constants
                    .iter()
                    .map(|c| (c.1.as_str(), c.2))
                    .collect();
                emissive_names.push(format!("{} [{method_set}] {constants:?}", mat_def.name));
            }
        }
        // Unlit shaders scale their colour by the material's `scaleRGB`
        // (the light dome's is 32); powers of two up to 128 are kept.
        // bo2mp: the `sw4_3d_unlit` shaders (screens, backlit glass) scale
        // the texel by `hdrAmount` before squaring it into linear (fxc
        // disassembly: (rgb * a * hdrAmount)^2), so by its square.
        if draw.unlit {
            let sw4 = method_set.split('_').any(|t| t == "sw4")
                && method_set.split('_').any(|t| t == "unlit");
            let hdr_amount = mat_def
                .constants
                .iter()
                .find(|c| c.1 == "hdrAmount")
                .map(|c| c.2[0] * c.2[0])
                .filter(|_| sw4);
            let scale = mat_def
                .constants
                .iter()
                .find(|c| c.1.starts_with("scaleRGB"))
                .map(|c| c.2[0])
                .or(hdr_amount)
                .unwrap_or(1.0);
            draw.unlit_scale_exp = scale.max(1.0).log2().round().clamp(0.0, 7.0) as u8;
            // bo2zm 27c: Die Rise's cloud sheets (`scrolling_zfeather`) drift
            // and fade where the scene behind them is near; their colour
            // scale (`hdrAmount` squared) rides in their instance.
            draw.sheet = method_set.contains("scrolling_zfeather");
            // bo2zm round 1: a zombie's eyes, scaled by its eye brightness.
            draw.eyes = method_set.contains("zombie_eyes");
        }
        // Tally the draw kinds with the per-material details folded away.
        let mut kind = draw;
        kind.draw_order = 0;
        kind.polygon_offset = 0;
        kind.unlit_scale_exp = 0;
        kind.layers = [0; 2];
        kind.cull = asset_core::T6Cull::None;
        *draws.entry(kind).or_default() += 1;

        let mut textures: Vec<SurfaceImageAttachment> = Vec::with_capacity(4);
        let caster = draw.blend == asset_core::T6Blend::CasterOnly;
        // Decodes a capture image (following a comma reference into another
        // zone) and links it into the catalog, once per image.
        let mut link = |key: asset_t6::AssetIdentity,
                        catalog: &mut SurfaceShaderRoster,
                        failures: &mut Vec<String>,
                        decoded: &mut usize|
         -> Option<usize> {
            if let Some(&known) = images.get(&key.index) {
                return known;
            }
            let linked = (|| {
                let own = capture.images.get(key.index)?;
                let picked = own
                    .name
                    .strip_prefix(',')
                    .and_then(|other| foreign_images.and_then(|by_name| by_name.get(other)))
                    .unwrap_or(own);
                let gpu = match decode(packs?, picked) {
                    Ok(gpu) => gpu,
                    Err(why) => {
                        let gap = format!("{} (material {}): {why}", picked.name, mat_def.name);
                        if caster { shadow_only.push(gap) } else { failures.push(gap) }
                        return None;
                    }
                };
                *decoded += 1;
                let row = HandBuiltTexture {
                    texture_kind: picked.map_kind,
                    depth: picked.depth,
                    tier_total: picked.tier_total,
                    ..built_row(picked.name.clone(), SEMANTIC_COLOR, true, picked.width, picked.height, gpu)
                };
                Some(catalog.add_texture(row))
            })();
            images.insert(key.index, linked);
            linked
        };
        // bo2mp: a water material draws from its constants (a picture of
        // them in the colour map's place) and its two normal maps (in the
        // normal and specular maps' places).
        let water = asset_core::t6_water_kind(method_set).filter(|_| draw.water);
        if let Some(kind) = water {
            // The water reflects its own cube when it is linked: its slot
            // counted from the probe array's end (1 = last; 0 = none).
            let names = cube_names.get_or_insert_with(|| {
                packs.map(|p| water_cubes(capture, p).into_iter().map(|c| c.0).collect()).unwrap_or_default()
            });
            let own_cube = water_cube_image(capture, mat_def)
                .and_then(|img| names.iter().position(|n| *n == img.name))
                .map_or(0, |i| names.len() - i);
            // A sea whose sun-shadow technique runs its lit technique's
            // pixel shader takes no shadow (Hijacked's distant sea).
            let pixel = |t: usize| {
                mat_def
                    .method_set
                    .and_then(|k| capture.technique_sets.get(k.index))
                    .and_then(|s| s.techniques.get(t)?.as_ref()?.passes.first())
                    .map(|p| p.pixel_shader.as_str())
            };
            let unshadowed = kind == 1 && pixel(6).is_some() && pixel(6) == pixel(4);
            let rows = water_table(mat_def, kind, own_cube, unshadowed);
            let width = rows.len() as u16;
            let table = water_image(&rows);
            let row = built_row(format!("{}#water", mat_def.name), SEMANTIC_COLOR, false, width, 1, table);
            let slot = catalog.add_texture(row);
            textures.push(binding(COLOR_MAP_HASH, TABLE_SAMPLING, SEMANTIC_COLOR, slot));
            // Pool: normalMap00, normalMap01; sea: normalMapLo, normalMapHi.
            let maps: [u32; 2] = match kind {
                0 => [0x1aca_8a8f, 0x1aca_8a8e],
                _ => [0x1aca_7e8c, 0x1aca_7f0e],
            };
            for (hash, semantic) in maps.into_iter().zip([SEMANTIC_NORMAL, SEMANTIC_SPECULAR]) {
                let Some(texture) = slot_texture(mat_def, hash) else { continue };
                let Some(key) = texture.image else { continue };
                if let Some(slot) = link(key, catalog, &mut failures, &mut decoded) {
                    let mode = texture.texture_sampling_mode;
                    textures.push(binding(texture.label_digest, mode, semantic, slot));
                }
            }
            // The sea's control map, shore picture and white-water noise and
            // mask (noiseMap, maskMap); seas without them keep the 1x1
            // stand-ins.
            if kind == 1 {
                let sea_maps = [
                    (0x6709_8a9b, T6_LAYER1_SEMANTIC),
                    (0xb1e0_21df, T6_LAYER2_SEMANTIC),
                    (0x7350_2222, T6_FOAM_NOISE_SEMANTIC),
                    (0x9e7b_a008, T6_FOAM_MASK_SEMANTIC),
                ];
                for (hash, semantic) in sea_maps {
                    let Some(texture) = slot_texture(mat_def, hash) else { continue };
                    let Some(key) = texture.image else { continue };
                    if let Some(slot) = link(key, catalog, &mut failures, &mut decoded) {
                        let mode = texture.texture_sampling_mode;
                        textures.push(binding(texture.label_digest, mode, semantic, slot));
                    }
                }
            }
        } else if draw.objective {
            // bo2zm M3: an objective material (the box's question marks)
            // draws its two pulse colours, not its (black) colour map: they
            // go in a 2x1 picture its shader reads (texel 0 colorObjMin, 1
            // colorObjMax).
            let named = |label: &str| match mat_def.constants.iter().find(|entry| entry.1 == label) {
                Some(entry) => entry.2,
                None => [0.0; 4],
            };
            let pulse = objective_image(named("colorObjMin"), named("colorObjMax"));
            let row = built_row(format!("{}#objective", mat_def.name), SEMANTIC_COLOR, true, 2, 1, pulse);
            let slot = catalog.add_texture(row);
            textures.push(binding(COLOR_MAP_HASH, TABLE_SAMPLING, SEMANTIC_COLOR, slot));
        } else if method_set.contains("cod7water")
            && colour_texture(mat_def, capture).is_none_or(|t| {
                // Black Ops 1's water keeps its sky reflection (a cube map)
                // in the colour slot; drawn as a colour map it was a flat
                // bright-blue sheet (Shi No Numa's swamp).
                t.image
                    .and_then(|k| capture.images.get(k.index))
                    .is_some_and(|img| img.map_kind == 5)
            })
        {
            // bo2zm visuals: water (`cod7water`) without a colour map. Its pixel
            // shader (fxc disassembly) mixes the scene behind towards
            // waterColorNear..waterColorS (by the view angle) times the
            // lightmap, by their alpha, under a fresnel reflection; drawn
            // here as that colour at a middle view angle, blended by its
            // alpha (it drew as the untextured blue diagnostic).
            draw.blend = asset_core::T6Blend::Blend;
            let water = cod7_water_image(mat_def);
            let row = built_row(format!("{}#water", mat_def.name), SEMANTIC_COLOR, true, 1, 1, water);
            let slot = catalog.add_texture(row);
            textures.push(binding(COLOR_MAP_HASH, TABLE_SAMPLING, SEMANTIC_COLOR, slot));
        } else if let Some(texture) = colour_texture(mat_def, capture)
            && let Some(key) = texture.image
        {
            // bo2zm M3 fix list 1: a colour map whose pixels are in no pack
            // here (a reference into another zone, e.g. the leftover Maya
            // `lambert1` material's "grey") draws flat, as its name says,
            // not in the untextured diagnostic colours (his "weird
            // splotches ... of random colors").
            let slot = match link(key, catalog, &mut failures, &mut decoded) {
                Some(slot) => slot,
                None => {
                    let name = capture.images.get(key.index).map(|i| i.name.as_str()).unwrap_or_default();
                    let flat = flat_image(name);
                    catalog.add_texture(built_row(format!("{name}#flat"), SEMANTIC_COLOR, true, 1, 1, flat))
                }
            };
            let mode = texture.texture_sampling_mode;
            textures.push(binding(texture.label_digest, mode, SEMANTIC_COLOR, slot));
        }
        // A layered material's extra colour maps (the fast quality setting
        // draws the base layer alone).
        if asset_core::t6_fast() {
            draw.layers = [0, 0];
        }
        let layered = draw.layers != [0, 0];
        for (hash, semantic) in LAYER_MAPS.into_iter().filter(|_| layered) {
            let Some(texture) = slot_texture(mat_def, hash) else { continue };
            let Some(key) = texture.image else { continue };
            if let Some(slot) = link(key, catalog, &mut failures, &mut decoded) {
                let mode = texture.texture_sampling_mode;
                textures.push(binding(texture.label_digest, mode, semantic, slot));
            }
        }
        // bo2mp: a lens flare's grime map, in layer 1's place.
        if draw.flare
            && let Some(texture) = filled_slot(mat_def, GRIME_MAP_HASH)
            && let Some(key) = texture.image
            && let Some(slot) = link(key, catalog, &mut failures, &mut decoded)
        {
            let mode = texture.texture_sampling_mode;
            textures.push(binding(texture.label_digest, mode, T6_LAYER1_SEMANTIC, slot));
        }
        // The normal and specular maps BO2's lit shaders read (the shine);
        // the fast quality setting draws without them.
        // (`BO2MP_T6_NO_SHINE` leaves them out: a test aid for comparing.)
        if !asset_core::t6_fast()
            && !draw.unlit
            && water.is_none()
            && std::env::var_os("BO2MP_T6_NO_SHINE").is_none()
        {
            for (hash, semantic) in SHINE_MAPS {
                let Some(texture) = slot_texture(mat_def, hash) else { continue };
                let Some(key) = texture.image else { continue };
                let slot = match link(key, catalog, &mut failures, &mut decoded) {
                    Some(slot) => Some(slot),
                    None => foreign_shine_stand_in(catalog, capture, key, semantic),
                };
                if let Some(slot_ix) = slot {
                    let mode = texture.texture_sampling_mode;
                    textures.push(binding(texture.label_digest, mode, semantic, slot_ix));
                }
            }
        }
        // bo2mp: an emissive flow material's masks (in the specular map's
        // place) and colour remap with constants (in layer 1's).
        // (`BO2MP_T6_NO_FLOW` leaves them out: a test aid for comparing.)
        // bo2mp: an emissive tile model material's glow (in the specular
        // map's place), macro normal map and constants (in layer 1's).
        // Top level of the picture in slot `hash`, as RGBA8.
        let slot_pixels = |hash: u32| {
            let key = slot_texture(mat_def, hash)?.image?;
            let picture = decode(packs?, capture.images.get(key.index)?).ok()?;
            top_level_rgba8(&picture)
        };
        let model_shader = !method_set.starts_with("wpc_");
        let tile_pictures = match draw.tile && model_shader {
            true => tile_images(mat_def, &slot_pixels),
            false => None,
        };
        draw.tile = tile_pictures.is_some();
        if let Some((glow, consts)) = tile_pictures {
            for (picture, suffix, semantic) in [(glow, "glow", SEMANTIC_SPECULAR), (consts, "tile", T6_LAYER1_SEMANTIC)] {
                let slot = link_built(catalog, format!("{}#tile_{suffix}", mat_def.name), semantic, picture);
                textures.push(binding(0, TABLE_SAMPLING, semantic, slot));
            }
            // The macro normal map.
            if let Some(texture) = slot_texture(mat_def, 0x13a7_8442)
                && let Some(key) = texture.image
                && let Some(slot) = link(key, catalog, &mut failures, &mut decoded)
            {
                let mode = texture.texture_sampling_mode;
                textures.push(binding(texture.label_digest, mode, SEMANTIC_NORMAL, slot));
            }
        }
        // bo2mp: an animated screen's constants in layer 1's place: FPS,
        // Grid (columns, rows), Padding, then hdrAmount squared (BO2's
        // `sw4_3d_animated_unlit` pixel shader, fxc disassembly), a 2x1
        // RGBA16F picture.
        let found = |hash: u32| mat_def.constants.iter().find(|entry| entry.0 == hash).map(|entry| entry.2);
        let animated_screen = draw.anim && draw.unlit && !draw.effect;
        let flipbook = match (found(0x0001_b865), found(0x0038_2b58)) {
            (Some(fps), Some(grid)) if animated_screen && grid[0] >= 1.0 && grid[1] >= 1.0 => Some((fps, grid)),
            _ => None,
        };
        draw.anim = flipbook.is_some();
        if let Some((fps, grid)) = flipbook {
            let padding = found(0xd235_be11).map_or(1.0, |p| p[0]);
            let hdr = found(0x00e2_62b2).map_or(1.0, |h| h[0]);
            let table = water_image(&[[fps[0], grid[0], grid[1], padding], [hdr * hdr, 0.0, 0.0, 0.0]]);
            let slot = link_built(catalog, format!("{}#anim", mat_def.name), T6_LAYER1_SEMANTIC, table);
            textures.retain(|t| t.semantic != T6_LAYER1_SEMANTIC);
            textures.push(binding(0, TABLE_SAMPLING, T6_LAYER1_SEMANTIC, slot));
        }
        // bo2mp: BO2's three-layer unlit (`sw4_3d_unlit_3layer`: Nightclub's
        // starfield ceiling) draws as an animated screen whose table says so
        // (texel 1's y). Three maps (t2, t1, t0: layers 1, 2 and 3), each
        // scaled, offset, turned, scrolled and pushed by parallax, mixed by
        // their alphas (fxc disassembly, technique 4). Layers 1 and 2 share
        // one image, the colour map; layer 3's is in layer 2's place; the
        // table in layer 1's: [0, 1, 1, 1] (a still screen), [hdrAmount
        // squared, 1], then per layer [Scale, Offset], [Scroll, Origin],
        // [Rotation, Alpha, Depth].
        let three_layer = draw.unlit && !draw.effect && method_set.contains("_3layer");
        let layer_image = |hash: u32| slot_texture(mat_def, hash).and_then(|t| t.image);
        let layer_keys = match (layer_image(0xdd64_512e), layer_image(0xdd64_dd0d), layer_image(0xdd65_596c)) {
            (Some(l1), Some(l2), Some(l3)) if three_layer && l1.index == l2.index => Some((l2, l3)),
            _ => None,
        };
        let layer_slots = layer_keys.and_then(|(l12, l3)| {
            let shared = link(l12, catalog, &mut failures, &mut decoded)?;
            Some((shared, link(l3, catalog, &mut failures, &mut decoded)?))
        });
        if let Some((shared, third)) = layer_slots {
            draw.anim = true;
            let layer = |scale: u32, offset: u32, scroll: u32, origin: u32, turn: u32, alpha: u32, depth: u32| {
                let (s, o) = (constant_or(mat_def, scale, [1.0; 4]), constant_or(mat_def, offset, [0.0; 4]));
                let (v, p) = (constant_or(mat_def, scroll, [0.0; 4]), constant_or(mat_def, origin, [0.0; 4]));
                let r = constant_or(mat_def, turn, [0.0; 4]);
                let (a, d) = (constant_or(mat_def, alpha, [1.0; 4])[0], constant_or(mat_def, depth, [0.0; 4])[0]);
                [[s[0], s[1], o[0], o[1]], [v[0], v[1], p[0], p[1]], [r[0], r[1], a, d]]
            };
            let hdr = constant_or(mat_def, 0x00e2_62b2, [1.0; 4])[0];
            let mut rows = vec![[0.0, 1.0, 1.0, 1.0], [hdr * hdr, 1.0, 0.0, 0.0]];
            rows.extend(layer(0xc5e8_256a, 0xc3e2_dbff, 0x82f2_d61f, 0xc307_1a06, 0x7078_cb46, 0xc6ea_3186, 0xc73e_96ff));
            rows.extend(layer(0xc7f4_ebc9, 0x07af_cf7c, 0xc69d_ca5c, 0x06d0_e645, 0x5a98_8a05, 0xc93e_49a5, 0xc96d_655c));
            rows.extend(layer(0xca8e_2428, 0x5d4a_f3fd, 0x1c5a_ee1d, 0x5c6d_0184, 0x4fd6_eb44, 0xcb4e_41c4, 0xcb9e_653d));
            let table = link_built(catalog, format!("{}#3layer", mat_def.name), T6_LAYER1_SEMANTIC, water_image(&rows));
            textures.retain(|t| ![SEMANTIC_COLOR, T6_LAYER1_SEMANTIC, T6_LAYER2_SEMANTIC].contains(&t.semantic));
            textures.push(binding(COLOR_MAP_HASH, TABLE_SAMPLING, SEMANTIC_COLOR, shared));
            textures.push(binding(0, TABLE_SAMPLING, T6_LAYER1_SEMANTIC, table));
            textures.push(binding(0, TABLE_SAMPLING, T6_LAYER2_SEMANTIC, third));
        }
        // bo2mp: a tile blend model material (cliffs, rocks, vista mountains):
        // Micro_1 as its colour map, Micro_2 in the specular map's place, the
        // macro AO/mix map in the normal map's, and Micro_1_Scale,
        // Micro_2_Scale, AO_Diffuse_Adj, EdgeHighlight (2x1 RGBA16F) in layer
        // 1's. World tile blends keep the stand-in colour map.
        let blend_maps = if draw.tile_blend && model_shader {
            // All three are linked (in this order) even if an earlier one is missing.
            let mut fetch = |hash: u32| {
                let key = slot_texture(mat_def, hash)?.image?;
                link(key, catalog, &mut failures, &mut decoded)
            };
            let micro1 = fetch(TILE_BLEND_MICRO1_HASH);
            let micro2 = fetch(TILE_BLEND_MICRO2_HASH);
            let macro_map = fetch(TILE_BLEND_MACRO_HASH);
            micro1.zip(micro2).zip(macro_map)
        } else {
            None
        };
        draw.tile_blend = blend_maps.is_some();
        if let Some(((micro1, micro2), macro_map)) = blend_maps {
            let scale1 = constant_or(mat_def, 0x1818_f373, [1.0; 4]);
            let scale2 = constant_or(mat_def, 0x7d8c_def0, [1.0; 4]);
            let ao_adjust = constant_or(mat_def, 0x9923_36af, [0.0; 4])[0];
            let edge_glint = constant_or(mat_def, 0x259b_0793, [1.0; 4])[0];
            let table = water_image(&[
                [scale1[0], scale1[1], scale2[0], scale2[1]],
                [ao_adjust, edge_glint, 0.0, 0.0],
            ]);
            let consts = link_built(catalog, format!("{}#tile_blend", mat_def.name), T6_LAYER1_SEMANTIC, table);
            textures.retain(|t| t.semantic != SEMANTIC_COLOR);
            textures.push(binding(COLOR_MAP_HASH, TABLE_SAMPLING, SEMANTIC_COLOR, micro1));
            textures.push(binding(0, TABLE_SAMPLING, SEMANTIC_SPECULAR, micro2));
            textures.push(binding(0, TABLE_SAMPLING, SEMANTIC_NORMAL, macro_map));
            textures.push(binding(0, TABLE_SAMPLING, T6_LAYER1_SEMANTIC, consts));
        }
        // bo2mp: a tattered flag's frayed edge texture (in the specular map's
        // place) and its EdgeScale (a 1x1 picture in layer 1's place).
        if draw.flag {
            let frayed_edge = slot_texture(mat_def, 0xcdab_26c0)
                .and_then(|texture| texture.image)
                .and_then(|key| link(key, catalog, &mut failures, &mut decoded));
            draw.flag = frayed_edge.is_some();
            if let Some(edge) = frayed_edge {
                let scale = constant_or(mat_def, 0x8240_aa7b, [1.0; 4]);
                let table = water_image(&[[scale[0], scale[1], 0.0, 0.0]]);
                let consts = link_built(catalog, format!("{}#flag", mat_def.name), T6_LAYER1_SEMANTIC, table);
                textures.push(binding(0, TABLE_SAMPLING, SEMANTIC_SPECULAR, edge));
                textures.push(binding(0, TABLE_SAMPLING, T6_LAYER1_SEMANTIC, consts));
            }
        }
        let flow_pictures = match draw.flow && std::env::var_os("BO2MP_T6_NO_FLOW").is_none() {
            true => flow_images(mat_def, &slot_pixels),
            false => None,
        };
        draw.flow = flow_pictures.is_some();
        if let Some((mask, table)) = flow_pictures {
            for (picture, suffix, semantic) in [(mask, "mask", SEMANTIC_SPECULAR), (table, "remap", T6_LAYER1_SEMANTIC)] {
                let slot = link_built(catalog, format!("{}#flow_{suffix}", mat_def.name), semantic, picture);
                textures.push(binding(0, TABLE_SAMPLING, semantic, slot));
            }
        }
        if textures.is_empty() {
            uncoloured += 1;
        } else {
            coloured += 1;
        }
        let parameters = mat_def
            .constants
            .iter()
            .map(|&(label_digest, ref label, literal)| {
                // Names longer than the slot are cut; shorter ones zero-padded.
                let mut name = [0u8; 12];
                let kept = label.len().min(name.len());
                name[..kept].copy_from_slice(&label.as_bytes()[..kept]);
                asset_material::ShaderParameterValue { label_digest, name, literal }
            })
            .collect();
        let row = DesignerMaterial {
            draw_order: mat_def.sort_rank,
            // Only the low byte of the game flags matters downstream: the
            // no-marks bit (0x4) and the static-model sun-shadow bits (0xc0).
            game_flag_info: mat_def.game_flags as u8,
            facet_type_bits: Some(mat_def.facet_type_bits),
            constants: parameters,
            t6_draw: Some(draw),
            ..bare_material(mat_def.name.clone(), owner.clone(), textures)
        };
        local.insert(mi, catalog.add_material(row));
    }
    let summary = format!(
        "t6 materials: {} linked ({coloured} with a colour map, {uncoloured} without), {decoded} colour maps decoded from packs, {emissive_only} emissive-only drawn unlit, {} failed, {} shadow-only without their picture; draws {:?}",
        local.len(),
        failures.len(),
        shadow_only.len(),
        draws
    );
    let gaps = failures.iter().take(40).map(|gap| format!("t6 colour map gap: {gap}"));
    let gaps = gaps.chain(shadow_only.iter().take(40).map(|gap| format!("t6 shadow-only picture gap: {gap}")));
    let emissive = emissive_names.iter().map(|n| format!("t6 emissive-only material: {n}"));
    let load_log = std::iter::once(summary).chain(gaps).chain(emissive).collect();
    T6Materials { local, load_log }
}

/// bo2zm M4: whether a material adds its colour onto what is under it
/// (blend ONE/ONE, or SRC_ALPHA/ONE) - BO2's menu brackets and glows,
/// black where nothing shows.
pub(crate) fn is_additive(material: &asset_t6::MaterialRef) -> bool {
    (0..36)
        .filter_map(|t| material.draw_state(t))
        .any(|d| d.dst_blend == 2 && matches!(d.src_blend, 2 | 5))
}

/// bo2mp vehicle screens: a 2D material's AddMap picture (texture name
/// hash 0xe98a255d, the `sw4_2d_color_add` pixel shader's `AddMap`, t0),
/// when it has one: the VTOL Warship's reticles (`mp_hud_sentry_outer_white`
/// = `mp_hud_sentry_outer_blend` colour map + `mp_hud_sentry_outer_add`).
pub(crate) fn add_map<'a>(material: &asset_t6::MaterialRef, capture: &'a ZoneCapture) -> Option<&'a ImageRef> {
    let t = material.textures.iter().find(|t| t.label_digest == ADD_MAP_HASH)?;
    capture.images.get(t.image?.index)
}

const ADD_MAP_HASH: u32 = 0xe98a_255d;

/// bo2mp vehicle screens: BO2's `sw4_2d_color_add` pixel shader reads only
/// the colour map's alpha (`rgb = (ColorMap.a * colour.rgb + AddMap.rgb) *
/// colour.a`, blend ONE/ONE). Packed: (AddMap colour, colour map alpha) for
/// the additive pass, and a white-lit flat copy (colour map alpha plus the
/// AddMap's colour as brightness-alpha) for layers that only alpha-blend.
pub(crate) fn pack_color_add(tint: &Image, added: &Image) -> Option<(Image, Image)> {
    let (w, h, mut rgba) = top_level_rgba8(tint)?;
    let (aw, ah, add) = top_level_rgba8(added)?;
    let mut flat = rgba.clone();
    for (index, (packed_px, flat_px)) in rgba.chunks_exact_mut(4).zip(flat.chunks_exact_mut(4)).enumerate() {
        let (x, y) = (index % w, index / w);
        // Nearest AddMap texel at the same spot.
        let near = ((y * ah / h) * aw + x * aw / w) * 4;
        let added_rgb = &add[near..near + 3];
        let coverage = packed_px[3];
        packed_px[..3].copy_from_slice(added_rgb);
        for (out, &extra) in flat_px[..3].iter_mut().zip(added_rgb) {
            *out = coverage.saturating_add(extra);
        }
        flat_px[3] = u8::MAX;
    }
    let packed = rgba8_image(tint, w, h, rgba);
    let flat = additive_to_alpha(&rgba8_image(tint, w, h, flat))?;
    Some((packed, flat))
}

/// bo2zm M4: an additive picture for a 2D layer that only alpha-blends: its
/// top level as RGBA with the brightness as alpha (black = see-through),
/// colour divided back out.
pub(crate) fn additive_to_alpha(image: &Image) -> Option<Image> {
    let (w, h, mut rgba) = top_level_rgba8(image)?;
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[0].max(px[1]).max(px[2]));
        let alpha = a * u32::from(px[3]) / 255;
        if a > 0 {
            for c in &mut px[..3] {
                *c = (u32::from(*c) * 255 / a).min(255) as u8;
            }
        }
        px[3] = alpha as u8;
    }
    Some(rgba8_image(image, w, h, rgba))
}

/// bo2zm M4: the left `keep` of a picture's columns, the rest see-through
/// (the lobby's quiet speaker: `voice_on` without its sound waves).
pub(crate) fn left_part(image: &Image, keep: f32) -> Option<Image> {
    let (w, h, mut rgba) = top_level_rgba8(image)?;
    let cut = (w as f32 * keep).round() as usize;
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        if i % w >= cut {
            px[3] = 0;
        }
    }
    Some(rgba8_image(image, w, h, rgba))
}

/// bo2mp: a picture's top level as plain RGBA8 (the minimap reads its
/// pixels to turn it).
pub(crate) fn to_rgba8(image: &Image) -> Option<Image> {
    let (w, h, rgba) = top_level_rgba8(image)?;
    Some(rgba8_image(image, w, h, rgba))
}

fn rgba8_image(like: &Image, w: usize, h: usize, rgba: Vec<u8>) -> Image {
    let mut copy = texture_2d(w as u32, h as u32, 1, TextureFormat::Rgba8UnormSrgb, rgba);
    copy.sampler = like.sampler.clone();
    copy
}

/// A decoded picture's top level as RGBA8 bytes (block formats decoded, the
/// bytes as stored: no sRGB conversion): width, height, pixels.
fn top_level_rgba8(image: &Image) -> Option<(usize, usize, Vec<u8>)> {
    let w = image.texture_descriptor.size.width as usize;
    let h = image.texture_descriptor.size.height as usize;
    let data = image.data.as_ref()?;
    let (block, decode): (usize, fn(&[u8], &mut [u8], usize)) = match image.texture_descriptor.format {
        TextureFormat::Bc1RgbaUnormSrgb | TextureFormat::Bc1RgbaUnorm => (8, bcdec_rs::bc1),
        TextureFormat::Bc2RgbaUnormSrgb | TextureFormat::Bc2RgbaUnorm => (16, bcdec_rs::bc2),
        TextureFormat::Bc3RgbaUnormSrgb | TextureFormat::Bc3RgbaUnorm => (16, bcdec_rs::bc3),
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm => (0, |_, _, _| {}),
        _ => return None,
    };
    let byte_len = w * h * 4;
    if block == 0 {
        return Some((w, h, data.get(..byte_len)?.to_vec()));
    }
    let mut rgba = vec![0u8; byte_len];
    let blocks_across = w.div_ceil(4);
    let blocks_down = h.div_ceil(4);
    let mut unpacked = [0u8; 4 * 4 * 4];
    for block_index in 0..blocks_across * blocks_down {
        let start = block_index * block;
        decode(data.get(start..start + block)?, &mut unpacked, 16);
        let (left, top) = ((block_index % blocks_across) * 4, (block_index / blocks_across) * 4);
        for (texel, px) in unpacked.chunks_exact(4).enumerate() {
            let (col, row) = (left + texel % 4, top + texel / 4);
            if col < w && row < h {
                let dest = (row * w + col) * 4;
                rgba[dest..dest + 4].copy_from_slice(px);
            }
        }
    }
    Some((w, h, rgba))
}
