//! Decoding the images materials sample, on a pool the loader names, and
//! putting the results on their rows.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::tasks::TaskPool;

use crate::image_texels::{self, Texels};
use crate::image_wrap::{Wrap, build_image};
use asset_transport::StageHandle;
use crate::{
    AssetNamespace, AssetRef, AuthoredImage, ColorMapTransform, MaterialDefinitions, MaterialKey,
    TS_2D, TS_COLOR_MAP, TS_FUNCTION, TS_NORMAL_MAP, TS_WATER_MAP,
};

/// How a decode stage went.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaterialImageStats {
    pub requested: usize,
    pub decoded: usize,
    /// Rows with no body in the zone, or a decode cut short by a retarget.
    pub missing: usize,
    /// Rows whose body is in a format the decoder does not read.
    pub unsupported: usize,
}

/// One image to decode and how its asker will sample it.
#[derive(Clone, Copy)]
struct Ask {
    image: usize,
    sampler_state: u8,
    is_normal: bool,
    sharp: bool,
    linear: bool,
}

impl Ask {
    fn plain(image: usize, sampler_state: u8) -> Self {
        Self {
            image,
            sampler_state,
            is_normal: false,
            sharp: false,
            linear: false,
        }
    }

    /// Another asker for the same image: it keeps this one's sampler state
    /// and wants everything either of them wanted.
    fn absorb(&mut self, other: Ask) {
        self.is_normal |= other.is_normal;
        self.sharp |= other.sharp;
        self.linear |= other.linear;
    }
}

enum Answer {
    Ready {
        image: usize,
        texels: Texels,
        wrap: Wrap,
    },
    Missing,
    Unsupported,
}

fn decode_ask(images: &[AuthoredImage], ask: Ask) -> Answer {
    let Some(row) = images.get(ask.image) else {
        return Answer::Missing;
    };
    // Black Ops II ships no loose image files: a row without a body in the
    // zone has nothing to read.
    if row.payload.is_empty() {
        return Answer::Missing;
    }
    match image_texels::decode(&row.payload, u32::from(row.width), u32::from(row.height), row.format)
    {
        Ok(texels) => Answer::Ready {
            image: ask.image,
            texels,
            wrap: Wrap {
                sampler_state: ask.sampler_state,
                is_normal: ask.is_normal,
                sharp: ask.sharp,
                force_linear: ask.linear,
                srgb_reads: row.use_srgb_reads,
            },
        },
        Err(_) => Answer::Unsupported,
    }
}

/// Decodes `asks` on `pool`. The pool is the caller's because the loader's is
/// not the frame's, and this crate sits below the loader.
fn decode_all(
    images: &[AuthoredImage],
    asks: &[Ask],
    stage: &StageHandle,
    pool: &TaskPool,
) -> Vec<Answer> {
    if asks.is_empty() {
        return Vec::new();
    }
    let per_task = asks.len().div_ceil(pool.thread_num().max(1)).clamp(1, 64);
    pool.scope(|scope| {
        for part in asks.chunks(per_task) {
            scope.spawn(async move {
                let mut answers = Vec::with_capacity(part.len());
                for &ask in part {
                    if stage.is_canceled() {
                        answers.push(Answer::Missing);
                        continue;
                    }
                    let answer = decode_ask(images, ask);
                    if let Answer::Ready { texels, .. } = &answer {
                        stage.add_bytes(texels.bytes.len() as u64);
                    }
                    answers.push(answer);
                    stage.advance(1);
                }
                answers
            });
        }
    })
    .into_iter()
    .flatten()
    .collect()
}

/// Decodes `asks` and puts each result on its row. `reclaim` also takes the
/// row away from the common set, which no longer owns what was just decoded.
fn run(
    defs: &mut MaterialDefinitions,
    asks: &[Ask],
    reclaim: bool,
    stage: &StageHandle,
    pool: &TaskPool,
) -> MaterialImageStats {
    stage.set_total(asks.len() as u64);
    let answers = decode_all(&defs.images, asks, stage, pool);
    let mut stats = MaterialImageStats {
        requested: asks.len(),
        ..MaterialImageStats::default()
    };
    for answer in answers {
        match answer {
            Answer::Ready {
                image,
                texels,
                wrap,
            } => {
                let row = &mut defs.images[image];
                row.decoded = Some(Arc::new(build_image(texels, wrap)));
                if reclaim {
                    row.common_owned = false;
                }
                stats.decoded += 1;
            }
            Answer::Missing => stats.missing += 1,
            Answer::Unsupported => stats.unsupported += 1,
        }
    }
    stats
}

/// Whether a texture of this semantic holds a normal map; `None` for the
/// ones that are not decoded as colour or normal at all.
fn decodes_as_normal(semantic: u8) -> Option<bool> {
    match semantic {
        TS_FUNCTION | TS_WATER_MAP => None,
        TS_NORMAL_MAP => Some(true),
        _ => Some(false),
    }
}

/// Decodes every image some material samples that is not decoded yet.
pub fn decode_material_color_maps(
    defs: &mut MaterialDefinitions,
    stage: &StageHandle,
    pool: &TaskPool,
) -> MaterialImageStats {
    let mut asks: Vec<Option<Ask>> = vec![None; defs.images.len()];
    for material in &defs.materials {
        let alpha_tested = defs
            .agreed_alpha_test_cutoff(material)
            .is_some_and(|cutoff| cutoff.is_some());
        let squared = defs.color_map_transform(material) == ColorMapTransform::Square;
        for texture in &material.textures {
            let (Some(is_normal), Some(image)) =
                (decodes_as_normal(texture.semantic), texture.image)
            else {
                continue;
            };
            if defs.images.get(image).is_none_or(|row| row.decoded.is_some()) {
                continue;
            }
            let ask = Ask {
                image,
                sampler_state: texture.sampler_state,
                is_normal,
                sharp: alpha_tested && !is_normal,
                linear: squared && !is_normal,
            };
            asks[image].get_or_insert(ask).absorb(ask);
        }
    }
    let asks: Vec<Ask> = asks.into_iter().flatten().collect();
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
    let mut wanted: HashMap<AssetNamespace, HashSet<String>> = HashMap::new();
    for key in keys {
        let name = AssetRef::bare_name(&key.name);
        if !name.is_empty() {
            wanted.entry(key.namespace).or_default().insert(name.to_owned());
        }
    }
    if wanted.is_empty() {
        return 0;
    }
    let mut seen = HashSet::new();
    let mut asks = Vec::new();
    for material in &defs.materials {
        let name = material.name.as_str();
        let named = material.name.is_real()
            && !name.is_empty()
            && wanted
                .get(&material.namespace)
                .is_some_and(|names| names.contains(name));
        if !named {
            continue;
        }
        let picture = |semantic: u8| {
            material
                .textures
                .iter()
                .find(|texture| texture.semantic == semantic && texture.image.is_some())
        };
        let Some(texture) = picture(TS_COLOR_MAP).or_else(|| picture(TS_2D)) else {
            continue;
        };
        let Some(image) = texture.image else {
            continue;
        };
        let decoded = defs.images.get(image).is_some_and(|row| row.decoded.is_some());
        if !decoded && seen.insert(image) {
            asks.push(Ask::plain(image, texture.sampler_state));
        }
    }
    if asks.is_empty() {
        return 0;
    }
    run(defs, &asks, true, stage, pool).decoded
}

/// Decodes the listed images (row, sampler state) that are not decoded yet,
/// read without the sRGB curve.
pub fn decode_catalog_images(
    defs: &mut MaterialDefinitions,
    images: impl IntoIterator<Item = (usize, u8)>,
    stage: &StageHandle,
    pool: &TaskPool,
) -> usize {
    let mut seen = HashSet::new();
    let mut asks = Vec::new();
    for (image, sampler_state) in images {
        let decoded = defs.images.get(image).is_some_and(|row| row.decoded.is_some());
        if !decoded && seen.insert(image) {
            asks.push(Ask {
                linear: true,
                ..Ask::plain(image, sampler_state)
            });
        }
    }
    if asks.is_empty() {
        return 0;
    }
    run(defs, &asks, false, stage, pool).decoded
}

/// Decodes the zone's own `$`-named images that carry a body, on this thread.
pub fn decode_in_zone_builtin_images(defs: &mut MaterialDefinitions) -> usize {
    let mut decoded = 0;
    for row in &mut defs.images {
        let name = row.name.as_str();
        let builtin = row.decoded.is_none()
            && !row.payload.is_empty()
            && row.map_type == 3
            && row.name.is_real()
            && name.starts_with('$');
        if !builtin {
            continue;
        }
        let Ok(texels) = image_texels::decode(
            &row.payload,
            u32::from(row.width),
            u32::from(row.height),
            row.format,
        ) else {
            continue;
        };
        let wrap = Wrap {
            sampler_state: 0,
            is_normal: name.contains("normal"),
            sharp: false,
            force_linear: false,
            srgb_reads: row.use_srgb_reads,
        };
        row.decoded = Some(Arc::new(build_image(texels, wrap)));
        decoded += 1;
    }
    decoded
}
