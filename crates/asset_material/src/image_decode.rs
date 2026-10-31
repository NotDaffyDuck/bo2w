//! Decoding the images materials sample, on a pool the loader names, and
//! putting the results on their rows.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use bevy::tasks::TaskPool;

use crate::image_texels::{self, Texels};
use crate::image_wrap::{Wrap, build_image};
use asset_transport::StageHandle;
use crate::{
    AssetCategoryScope, AssetPointer, HandBuiltTexture, DesignerMaterial, TintMapRemap, MaterialDefinitions, MaterialKey,
    SurfaceImageAttachment,
    TS_2D, SEMANTIC_COLOR, SEMANTIC_FUNCTION, SEMANTIC_NORMAL, SEMANTIC_LIQUID,
};

/// How a decode stage went.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShaderTextureMetrics {
    pub requested: usize,
    pub decoded: usize,
    /// Rows with no body in the zone, or a decode cut short by a retarget.
    pub missing: usize,
    /// Rows whose body is in a format the decoder does not read.
    pub unsupported: usize,
}

/// What an asker wants out of the image beyond a plain read.
#[derive(Clone, Copy, Default)]
struct Wants(u8);

impl Wants {
    const NORMAL: u8 = 1;
    const SHARP: u8 = 2;
    const LINEAR: u8 = 4;

    fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    fn with(self, bit: u8, on: bool) -> Self {
        Self(self.0 | if on { bit } else { 0 })
    }
}

/// One image to decode and how its asker will sample it.
#[derive(Clone, Copy)]
struct Ask {
    image: usize,
    texture_sampling_mode: u8,
    wants: Wants,
}

impl Ask {
    fn plain(image: usize, texture_sampling_mode: u8) -> Self {
        Self { image, texture_sampling_mode, wants: Wants::default() }
    }

    fn read_linear(mut self) -> Self {
        self.wants = self.wants.with(Wants::LINEAR, true);
        self
    }

    /// Another asker for the same image: it keeps this one's sampler state
    /// and wants everything either of them wanted.
    fn merge_in(&mut self, other: Ask) {
        self.wants = Wants(self.wants.0 | other.wants.0);
    }
}

/// An image that came out of the pool without pixels.
#[derive(Clone, Copy)]
enum Shortfall {
    Missing,
    UnknownFormat,
}

struct Decoded {
    image: usize,
    texels: Texels,
    wrap: Wrap,
}

fn decode_ask(images: &[HandBuiltTexture], ask: Ask) -> Result<Decoded, Shortfall> {
    // Black Ops II ships no loose image files: a row without a body in the
    // zone has nothing to read.
    let row = images
        .get(ask.image)
        .filter(|row| !row.payload.is_empty())
        .ok_or(Shortfall::Missing)?;
    let texels = image_texels::decode(&row.payload, row.width.into(), row.height.into(), row.format)
        .map_err(|_| Shortfall::UnknownFormat)?;
    let wrap = Wrap {
        texture_sampling_mode: ask.texture_sampling_mode,
        is_normal: ask.wants.has(Wants::NORMAL),
        sharp: ask.wants.has(Wants::SHARP),
        force_linear: ask.wants.has(Wants::LINEAR),
        srgb_reads: row.srgb_sampled,
    };
    Ok(Decoded { image: ask.image, texels, wrap })
}

/// Decodes `asks` on `pool`. The pool is the caller's because the loader's is
/// not the frame's, and this crate sits below the loader.
fn decode_all(
    sources: &[HandBuiltTexture],
    requests: &[Ask],
    stage: &StageHandle,
    pool: &TaskPool,
) -> Vec<Result<Decoded, Shortfall>> {
    // Spread the asks evenly over the workers, in batches of 1 to 64.
    let workers = pool.thread_num().max(1);
    let batch_len = requests.len().div_ceil(workers).clamp(1, 64);
    let decode_one = |ask: &Ask| -> Result<Decoded, Shortfall> {
        // A cancelled load stops decoding; what is left counts as missing.
        let answer = if stage.was_aborted() { Err(Shortfall::Missing) } else { decode_ask(sources, *ask) };
        if let Ok(done) = &answer {
            stage.add_bytes(done.texels.bytes.len() as u64);
        }
        if !stage.was_aborted() {
            stage.advance(1);
        }
        answer
    };
    let decode_one = &decode_one;
    let batches = pool.scope(|scope| {
        requests
            .chunks(batch_len)
            .for_each(|batch| scope.spawn(async move { batch.iter().map(decode_one).collect::<Vec<_>>() }));
    });
    batches.into_iter().flatten().collect()
}

/// Decodes `asks` and puts each result on its row. `reclaim` also takes the
/// row away from the common set, which no longer owns what was just decoded.
fn run(
    defs: &mut MaterialDefinitions,
    asks: &[Ask],
    reclaim: bool,
    stage: &StageHandle,
    pool: &TaskPool,
) -> ShaderTextureMetrics {
    stage.set_total(asks.len() as u64);
    let answers = decode_all(&defs.images, asks, stage, pool);
    let mut stats = ShaderTextureMetrics::default();
    stats.requested = asks.len();
    stats.decoded = answers
        .into_iter()
        .filter_map(|answer| answer.map_err(|short| stats.note_shortfall(short)).ok())
        .map(|done| install_decoded(&mut defs.images, done, reclaim))
        .count();
    stats
}

impl ShaderTextureMetrics {
    fn note_shortfall(&mut self, short: Shortfall) {
        let tally = match short {
            Shortfall::Missing => &mut self.missing,
            Shortfall::UnknownFormat => &mut self.unsupported,
        };
        *tally = tally.saturating_add(1);
    }
}

fn install_decoded(images: &mut [HandBuiltTexture], done: Decoded, reclaim: bool) {
    let Decoded { image, texels, wrap } = done;
    let row = &mut images[image];
    row.decoded = Some(Arc::new(build_image(texels, wrap)));
    row.common_owned &= !reclaim;
}

/// Whether a texture of this semantic holds a normal map; `None` for the
/// ones that are not decoded as colour or normal at all.
fn decodes_as_normal(semantic: u8) -> Option<bool> {
    let decoded = !matches!(semantic, SEMANTIC_FUNCTION | SEMANTIC_LIQUID);
    decoded.then_some(semantic == SEMANTIC_NORMAL)
}

/// The row exists and holds no pixels yet.
fn awaiting(images: &[HandBuiltTexture], image: usize) -> bool {
    images.get(image).is_some_and(|row| row.decoded.is_none())
}

/// The row already holds pixels.
fn has_pixels(images: &[HandBuiltTexture], image: usize) -> bool {
    images.get(image).is_some_and(|row| row.decoded.is_some())
}

/// Asks in the order offered, one per image.
#[derive(Default)]
struct Queue {
    seen: HashSet<usize>,
    asks: Vec<Ask>,
}

impl Queue {
    fn offer(&mut self, ask: Ask) {
        if self.seen.insert(ask.image) {
            self.asks.push(ask);
        }
    }

    /// Runs what was offered; nothing offered means no stage work at all.
    fn decode(
        &self,
        defs: &mut MaterialDefinitions,
        reclaim: bool,
        stage: &StageHandle,
        pool: &TaskPool,
    ) -> usize {
        match self.asks.as_slice() {
            [] => 0,
            asks => run(defs, asks, reclaim, stage, pool).decoded,
        }
    }
}

/// Decodes every image some material samples that is not decoded yet.
pub fn unpack_surface_tint_maps(
    defs: &mut MaterialDefinitions,
    stage: &StageHandle,
    pool: &TaskPool,
) -> ShaderTextureMetrics {
    let mut merged: BTreeMap<usize, Ask> = BTreeMap::new();
    for material in &defs.materials {
        let alpha_tested = matches!(defs.shared_alpha_cutoff(material), Some(Some(_)));
        let squared = defs.tint_map_remap(material) == TintMapRemap::Square;
        let wanted = material.textures.iter().filter_map(|texture| {
            let image = texture.image?;
            let is_normal = decodes_as_normal(texture.semantic)?;
            awaiting(&defs.images, image).then_some((image, texture.texture_sampling_mode, is_normal))
        });
        for (image, texture_sampling_mode, is_normal) in wanted {
            let wants = Wants::default()
                .with(Wants::NORMAL, is_normal)
                .with(Wants::SHARP, alpha_tested && !is_normal)
                .with(Wants::LINEAR, squared && !is_normal);
            let ask = Ask { image, texture_sampling_mode, wants };
            merged
                .entry(image)
                .and_modify(|first| first.merge_in(ask))
                .or_insert(ask);
        }
    }
    let asks: Vec<Ask> = merged.into_values().collect();
    run(defs, &asks, true, stage, pool)
}

/// Decodes the picture of each named material: its colour map, else its 2D
/// texture. Returns how many images came out.
pub fn decode_color_or_2d_for_keys(
    defs: &mut MaterialDefinitions,
    keys: impl IntoIterator<Item = MaterialKey>,
    stage: &StageHandle,
    pool: &TaskPool,
) -> usize {
    let mut named: HashMap<AssetCategoryScope, HashSet<String>> = HashMap::new();
    keys.into_iter().for_each(|key| {
        let bare = AssetPointer::bare_name(&key.name);
        if !bare.is_empty() {
            named.entry(key.namespace).or_default().insert(bare.to_owned());
        }
    });
    let is_listed = |material: &&DesignerMaterial| {
        let label = material.name.as_str();
        let known = named.get(&material.namespace).is_some_and(|names| names.contains(label));
        known && material.name.is_genuine() && !label.is_empty()
    };
    let picture_of = |material: &DesignerMaterial| {
        let pick = |semantic: u8| {
            let texture_of = |t: &&SurfaceImageAttachment| t.semantic == semantic && t.image.is_some();
            material.textures.iter().find(texture_of)
        };
        let texture = pick(SEMANTIC_COLOR).or_else(|| pick(TS_2D))?;
        Some((texture.image?, texture.texture_sampling_mode))
    };
    let mut queue = Queue::default();
    let pictures = defs.materials.iter().filter(is_listed).filter_map(picture_of);
    for (image, texture_sampling_mode) in pictures.filter(|&(image, _)| !has_pixels(&defs.images, image)) {
        queue.offer(Ask::plain(image, texture_sampling_mode));
    }
    queue.decode(defs, true, stage, pool)
}

/// Decodes the listed images (row, sampler state) that are not decoded yet,
/// read without the sRGB curve.
pub fn decode_catalog_images(
    defs: &mut MaterialDefinitions,
    gpu_images: impl IntoIterator<Item = (usize, u8)>,
    stage: &StageHandle,
    pool: &TaskPool,
) -> usize {
    let mut queue = Queue::default();
    let missing = gpu_images.into_iter().filter(|&(image, _)| !has_pixels(&defs.images, image));
    missing.for_each(|(image, texture_sampling_mode)| queue.offer(Ask::plain(image, texture_sampling_mode).read_linear()));
    queue.decode(defs, false, stage, pool)
}

/// Decodes the zone's own `$`-named images that carry a body, on this thread.
pub fn decode_in_zone_builtin_images(defs: &mut MaterialDefinitions) -> usize {
    let builtin = |row: &&mut HandBuiltTexture| {
        row.decoded.is_none()
            && row.texture_kind == 3
            && !row.payload.is_empty()
            && row.name.is_genuine()
            && row.name.as_str().starts_with('$')
    };
    defs.images
        .iter_mut()
        .filter(builtin)
        .filter_map(decode_builtin_row)
        .count()
}

/// Decodes one builtin image's embedded payload in place; `None` when it will not decode.
fn decode_builtin_row(row: &mut HandBuiltTexture) -> Option<()> {
    let texels = image_texels::decode(&row.payload, row.width.into(), row.height.into(), row.format).ok()?;
    let wrap = Wrap {
        texture_sampling_mode: 0,
        is_normal: row.name.as_str().contains("normal"),
        sharp: false,
        force_linear: false,
        srgb_reads: row.srgb_sampled,
    };
    row.decoded = Some(Arc::new(build_image(texels, wrap)));
    Some(())
}
