//! Handing a world's images (material textures, reflection probes,
//! lightmaps) over to `Assets<Image>` a slice at a time, so the loading
//! screen keeps moving, and keeping them for the next load of the same map.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use asset_material::ImageVariantId;
use asset_transport::{LoadCompletion, StageHandle, StageId};
use bevy::prelude::*;

use crate::assemble::draw_items::{LiveLightbakeHandles, LiveMaterialLedger};

use super::world::{LevelLightbake, LevelStage};

/// Bytes a loading-screen frame hands over before it stops (scaled by the
/// caller), and the most one slice may take whatever the scaling says.
pub(crate) const IMAGE_BYTES_PER_TOPMOST_FRAME: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_IMAGE_BYTES_PER_SLICE: u64 = 128 * 1024 * 1024;
pub(crate) const IMAGES_PER_TOPMOST_FRAME: u32 = 16;

// ---------------------------------------------------------------------------
// Reflection probe debugging: `BO2MP_PROBE_DEBUG` paints the probe cubes flat
// so one can see which face or mip level a surface samples.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvProbeDebug {
    /// Every texel this colour.
    Solid([u8; 4]),
    /// One colour per cube face.
    Faces,
    /// One colour per mip level, the palette rotated by `rotate` levels.
    PerMip { rotate: usize },
}

/// Opaque colours as `0xRRGGBBAA`: +X red, -X cyan, +Y green, -Y magenta,
/// +Z blue, -Z yellow.
const FACE_PALETTE: [u32; 6] = [
    0xff00_00ff,
    0x00ff_ffff,
    0x00ff_00ff,
    0xff00_ffff,
    0x0000_ffff,
    0xffff_00ff,
];

/// Mip 0 to 7: red, orange, yellow, green, cyan, blue, magenta, white.
const MIP_PALETTE: [u32; 8] = [
    0xff00_00ff,
    0xff80_00ff,
    0xffff_00ff,
    0x00ff_00ff,
    0x00ff_ffff,
    0x0000_ffff,
    0xff00_ffff,
    u32::MAX,
];

/// Mip levels past the palette: grey.
const BEYOND_PALETTE: u32 = 0x8080_80ff;

fn rgba(colour: u32) -> [u8; 4] {
    colour.to_be_bytes()
}

/// What the loading log says when a mode is switched on.
fn probe_debug_note(mode: EnvProbeDebug) -> String {
    let what = match mode {
        EnvProbeDebug::Faces => "every cube layer flat, +X red -X cyan +Y green -Y magenta +Z blue -Z yellow".to_owned(),
        EnvProbeDebug::PerMip { rotate: 0 } => {
            "every mip flat, palette 0 red 1 orange 2 yellow 3 green 4 cyan 5 blue 6 magenta 7 white"
                .to_owned()
        }
        EnvProbeDebug::PerMip { .. } => {
            "every mip flat, palette rotated by one level (0 white 1 red 2 orange 3 yellow 4 green 5 cyan 6 blue 7 magenta)"
                .to_owned()
        }
        EnvProbeDebug::Solid([r, g, b, a]) => format!("every probe forced to {r},{g},{b},{a}"),
    };
    format!("reflection probe A/B: {what}")
}

/// `r,g,b` as an opaque colour; the parts past the third are not looked at.
fn opaque_rgb(text: &str) -> Option<[u8; 4]> {
    let parts: Vec<u8> = text
        .split(',')
        .take(3)
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [red, green, blue] = parts[..] else {
        return None;
    };
    Some([red, green, blue, u8::MAX])
}

fn parse_probe_debug(raw: &str) -> Option<EnvProbeDebug> {
    let word = raw.trim();
    let mode = match word {
        "faces" => Some(EnvProbeDebug::Faces),
        "mips" => Some(EnvProbeDebug::PerMip { rotate: 0 }),
        "mips-shift" => Some(EnvProbeDebug::PerMip { rotate: 1 }),
        _ => opaque_rgb(word).map(EnvProbeDebug::Solid),
    };
    match mode {
        Some(mode) => diag::warn!(World, "{}", probe_debug_note(mode)),
        _ => diag::warn!(
            World,
            "unparseable BO2MP_PROBE_DEBUG={word:?}; probes unchanged"
        ),
    }
    mode
}

fn env_probe_debug_mode() -> Option<EnvProbeDebug> {
    static CHOSEN: std::sync::OnceLock<Option<EnvProbeDebug>> = std::sync::OnceLock::new();
    *CHOSEN.get_or_init(|| {
        let raw = std::env::var("BO2MP_PROBE_DEBUG").ok()?;
        parse_probe_debug(&raw)
    })
}

fn probe_colour(mode: EnvProbeDebug, level: usize, face: usize) -> [u8; 4] {
    let n = MIP_PALETTE.len();
    rgba(match mode {
        EnvProbeDebug::Solid(colour) => return colour,
        EnvProbeDebug::Faces => FACE_PALETTE[face],
        EnvProbeDebug::PerMip { .. } if level >= n => BEYOND_PALETTE,
        EnvProbeDebug::PerMip { rotate } => MIP_PALETTE[(level + n - rotate % n) % n],
    })
}

/// Bytes of one RGBA8 cube face at mip `level` of a `size`-pixel cube.
fn cube_face_bytes(size: u32, level: usize) -> usize {
    let edge = (size >> level).max(1) as usize;
    4 * edge * edge
}

/// Paints every texel of every face of every mip of a probe cube. The cube
/// stores the six faces of a level one after the other, the levels one after
/// the other.
fn paint_env_probe_debug(image: &mut Image, mode: EnvProbeDebug) {
    let edge = image.texture_descriptor.size.width;
    let levels = image.texture_descriptor.mip_level_count as usize;
    let faces: Vec<(usize, usize, usize)> = (0..levels)
        .flat_map(|level| (0..6).map(move |face| (level, face, cube_face_bytes(edge, level))))
        .collect();
    let needed: usize = faces.iter().map(|&(_, _, len)| len).sum();
    let Some(cube) = image.data.as_mut() else {
        diag::warn!(
            World,
            "BO2MP_PROBE_DEBUG: the probe cube holds no CPU bytes, so it stays unpainted"
        );
        return;
    };
    let have = cube.len();
    if have != needed {
        diag::warn!(
            World,
            "BO2MP_PROBE_DEBUG: the probe cube holds {have} bytes but {levels} mips from {edge}px want {needed}, so it stays unpainted"
        );
        return;
    }
    let mut offset = 0;
    for (level, face, len) in faces {
        let colour = probe_colour(mode, level, face);
        let face_bytes = &mut cube[offset..offset + len];
        face_bytes
            .chunks_exact_mut(colour.len())
            .for_each(|texel| texel.copy_from_slice(&colour));
        offset += len;
    }
}

/// The cube with its debug colours painted in, when a mode is on.
fn debug_painted(mut probe: Image) -> Image {
    if let Some(mode) = env_probe_debug_mode() {
        paint_env_probe_debug(&mut probe, mode);
    }
    probe
}

// ---------------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------------

fn cpu_bytes(of: &Image) -> u64 {
    of.data.as_ref().map_or(0, |data| data.len() as u64)
}

/// Every image one lightmap page hands over, summed. A page is six `add`s, and
/// the byte counts and the budget that paces the loading screen are owed all
/// six of them.
fn lightmap_page_bytes(page: &LevelLightbake) -> u64 {
    let optional = [
        &page.first_slot_texture,
        &page.auxiliary_texture,
        &page.second_b_map,
    ];
    let always = [
        &page.background_light_texture,
        &page.heading_texture,
        &page.daylight_mask_image,
    ];
    optional
        .into_iter()
        .flatten()
        .chain(always)
        .map(cpu_bytes)
        .sum()
}

// ---------------------------------------------------------------------------
// What stays resident between loads
// ---------------------------------------------------------------------------


/// Images that outlive a world: the shared-profile ones every map uses, and
/// the whole set of the last map so a reload of it needs no handoff.
#[derive(Resource, Default)]
pub struct ResidentGpuImages {
    profile_id: u64,
    by_variant: HashMap<ImageVariantId, Handle<Image>>,
    map: ResidentMapImages,
}

/// The handles of the last map handed over in full.
#[derive(Default)]
struct ResidentMapImages {
    products_id: u64,
    exact: Vec<Option<Handle<Image>>>,
    probes: Vec<Option<Handle<Image>>>,
    lightmaps: Vec<Option<LiveLightbakeHandles>>,
}

/// Every image handle of a lightmap page.
fn page_handles(sheet: &LiveLightbakeHandles) -> impl Iterator<Item = &Handle<Image>> {
    let optional = [&sheet.primary, &sheet.secondary, &sheet.auxiliary_second];
    let always = [
        &sheet.background_probe_note,
        &sheet.heading_probe_note,
        &sheet.sun_mask_debug,
    ];
    optional.into_iter().flatten().chain(always)
}

impl ResidentMapImages {
    /// True while every handle still points at an image in `images`.
    fn all_live(&self, images: &Assets<Image>) -> bool {
        let slots = self.exact.iter().chain(&self.probes).flatten();
        let pages = self.lightmaps.iter().flatten().flat_map(page_handles);
        slots
            .chain(pages)
            .all(|handle| images.get(handle.id()).is_some())
    }

    /// Whether these are the handles of the map `products_id` with the given
    /// slot, probe and page counts.
    fn fits_in(&self, products_id: u64, tallies: [usize; 3]) -> bool {
        products_id != 0
            && self.products_id == products_id
            && [self.exact.len(), self.probes.len(), self.lightmaps.len()] == tallies
    }
}

impl ResidentGpuImages {
    /// Shared images belong to one profile; a different one drops them.
    fn use_profile(&mut self, profile_id: u64) {
        if std::mem::replace(&mut self.profile_id, profile_id) != profile_id {
            self.by_variant.clear();
        }
    }

    /// The shared handle for the variant, dropped from the table if its
    /// image has been freed.
    fn get_live(
        &mut self,
        variant: ImageVariantId,
        gpu_images: &Assets<Image>,
    ) -> Option<Handle<Image>> {
        let live = gpu_images.get(self.by_variant.get(&variant)?.id()).is_some();
        if !live {
            self.by_variant.remove(&variant);
            return None;
        }
        self.by_variant.get(&variant).cloned()
    }
}

// ---------------------------------------------------------------------------
// The handoff
// ---------------------------------------------------------------------------

/// Slots handed over and bytes handed over, as a pair.
#[derive(Clone, Copy, Debug, Default)]
struct Progress {
    slots: u32,
    bytes: u64,
}

/// How much one call of `until` may do before it gives the frame back.
struct Pace {
    due_at: Option<Instant>,
    /// Where the call started.
    from: Progress,
    /// What it may do: it stops only once it has done both.
    allowance: Progress,
    /// At least one step has been taken; a call always makes progress.
    advanced: bool,
}

impl Pace {
    /// True when the call has done something and may now stop: out of time,
    /// or out of both its count and its byte allowance.
    fn spent(&self, now: Progress) -> bool {
        if !self.advanced {
            return false;
        }
        let late = self.due_at.is_some_and(|end| Instant::now() >= end);
        let slots_used = now.slots.saturating_sub(self.from.slots) >= self.allowance.slots;
        let bytes_used = now.bytes.saturating_sub(self.from.bytes) >= self.allowance.bytes;
        late || (slots_used && bytes_used)
    }
}

/// The materials' image slots that something can draw with: those named
/// `$...` and those a material binds. With none marked, all of them.
fn reachable_exact_image_slots(catalog: &LiveMaterialLedger, names: &[String]) -> HashSet<u32> {
    let named = (0u32..)
        .zip(names)
        .filter(|(_, name)| name.starts_with('$'))
        .map(|(seat, _)| seat);
    let mats = &catalog.materials;
    let bound = mats
        .iter()
        .flat_map(|live| live.textures.iter())
        .filter_map(|entry| entry.1.as_ref().map(|binding| binding.image.0));
    let wanted: HashSet<u32> = named.chain(bound).collect();
    match wanted.is_empty() {
        true => (0..names.len() as u32).collect(),
        false => wanted,
    }
}

/// The pipeline ids of the materials that will need an exact pipeline. A
/// material with no technique set (Black Ops II's) never gets one; it draws
/// through the fallback pass, so the spawn must not wait on one.
fn pipeline_material_ids(
    catalog: &LiveMaterialLedger,
    materials: impl IntoIterator<Item = assets::SurfaceMaterialId>,
) -> HashSet<u16> {
    materials
        .into_iter()
        .filter_map(|index| catalog.derived(index))
        .filter(|derived| !derived.method_set.is_empty())
        .map(|derived| derived.asset_id.0)
        .collect()
}

/// Images of one kind waiting to be handed over, the handles they became,
/// and the next one to go. Handles are image handles unless named.
struct Slots<T, H = Handle<Image>> {
    pending: Vec<Option<T>>,
    handles: Vec<Option<H>>,
    next: usize,
}

impl<T, H> Default for Slots<T, H> {
    fn default() -> Self {
        Self::holding(Vec::new())
    }
}

impl<T, H> Slots<T, H> {
    fn holding(pending: Vec<Option<T>>) -> Self {
        Self {
            handles: pending.iter().map(|_| None).collect(),
            pending,
            next: 0,
        }
    }

    /// Already handed over: nothing pending, only these handles.
    fn handed(handles: Vec<Option<H>>) -> Self {
        Self {
            handles,
            ..Self::default()
        }
    }

    fn exhausted(&self) -> bool {
        self.next >= self.pending.len()
    }

    /// The next slot and what it holds, moving past it.
    fn take_next(&mut self) -> Option<(usize, Option<T>)> {
        let at = self.next;
        let item = self.pending.get_mut(at)?.take();
        self.next += 1;
        Some((at, item))
    }
}

/// Where an exact image's handle came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HandleSource {
    /// Another slot of this map with the same variant.
    ThisMap,
    /// The shared images every map of the profile uses.
    Shared,
    /// A new asset.
    Added,
}

#[derive(Default)]
pub struct LevelImageUpload {
    pub finished: u32,
    pub expected: u32,
    /// Bytes handed to `Assets<Image>`. **Not** bytes on the GPU: the asset
    /// server takes the image here, the render world turns it into a texture
    /// some frames later, and the driver copies it later still. Calling this
    /// "uploaded" is how a handoff rate gets reported as a transfer rate.
    ///
    /// A slot that takes another slot's handle hands nothing over and is not
    /// in here; its bytes are `reused_handle_bytes`.
    pub handed_bytes: u64,
    /// The longest single handoff, and how big it was. A frame budget can
    /// only be kept to the granularity of the largest thing that cannot be
    /// split, so when a 40 ms slice runs to 227 ms this is the number that
    /// says whether the budget was ignored or was never keepable.
    pub largest_step_ns: u64,
    pub largest_step_bytes: u64,
    /// Image slots nothing can draw with, passed over.
    pub passed_over: u32,
    pub pipeline_level_materials: Arc<HashSet<u16>>,
    pub pipeline_prop_materials: Arc<HashSet<u16>>,
    /// Slots that took another slot's handle, and the bytes that saved.
    pub reused_handles: u32,
    pub reused_handle_bytes: u64,
    pub reused_common_handles: u32,
    pub reused_common_bytes: u64,
    /// The map's images were still resident and nothing was handed over.
    pub resident: bool,

    exact: Slots<Arc<Image>>,
    exact_variants: Vec<Option<ImageVariantId>>,
    /// One asset per prepared variant. Two catalog slots that hold the same
    /// variant hold byte-identical images with the same sampler and colour
    /// space; giving each its own `Assets<Image>` entry uploads the same
    /// texture twice and keeps two copies of it resident, so the second slot
    /// takes a clone of the first one's handle and drops its own copy.
    exact_by_variant: HashMap<ImageVariantId, Handle<Image>>,
    /// Slots whose image belongs to the shared profile.
    exact_common: Vec<bool>,
    /// Slots something draws with; empty means all.
    reachable_precise: HashSet<u32>,
    common_profile_id: u64,
    products_id: u64,
    probes: Slots<Image>,
    pages: Slots<LevelLightbake, LiveLightbakeHandles>,
    upload_stage: Option<StageHandle>,
}

/// The slots of the exact images a draw can reach, beyond the materials':
/// light attenuation, the outdoor mask and the sun's sprites.
fn reachable_scene_images(scene: &LevelStage) -> impl Iterator<Item = u32> + '_ {
    let lights = scene
        .key_light_attenuation
        .iter()
        .filter_map(|lamp| lamp.image);
    let dynamic = scene
        .dynamic_light
        .and_then(|light| light.attenuation.image);
    let sun = scene
        .sun_effects
        .into_iter()
        .flat_map(|sun| [sun.sprite_image, sun.flare_image]);
    lights
        .chain(dynamic)
        .chain(scene.openair_image)
        .chain(sun.flatten())
}

/// Hands one lightmap page's six images over.
fn hand_over_page(
    at: usize,
    page: LevelLightbake,
    gpu_images: &mut Assets<Image>,
) -> LiveLightbakeHandles {
    diag::info!(
        World,
        "world lightmap[{at}]: secondary={} 2x{}x{} mask={} {}x{}",
        page.background_light_origin_label,
        page.background_light_extent.x,
        page.background_light_extent.y,
        page.daylight_mask_source_name,
        page.daylight_mask_size.x,
        page.daylight_mask_size.y
    );
    let mut add = |image: Image| gpu_images.add(image);
    LiveLightbakeHandles {
        primary: page.first_slot_texture.map(&mut add),
        secondary: page.auxiliary_texture.map(&mut add),
        auxiliary_second: page.second_b_map.map(&mut add),
        background_probe_note: add(page.background_light_texture),
        heading_probe_note: add(page.heading_texture),
        sun_mask_debug: add(page.daylight_mask_image),
    }
}

/// What `stitch_handles` lends out: material images, lightmap pages, probes.
pub type StitchHandles<'a> = (
    &'a [Option<Handle<Image>>],
    &'a [Option<LiveLightbakeHandles>],
    &'a [Option<Handle<Image>>],
);

impl LevelImageUpload {
    pub fn has_slots_left(&self) -> bool {
        self.finished < self.expected
    }

    /// The material images', the lightmap pages' and the probes' handles.
    pub fn stitch_handles(&self) -> StitchHandles<'_> {
        (
            self.precise_handles(),
            self.lightbake_handles(),
            self.probe_refs(),
        )
    }

    pub fn precise_handles(&self) -> &[Option<Handle<Image>>] {
        &self.exact.handles
    }

    pub fn probe_refs(&self) -> &[Option<Handle<Image>>] {
        &self.probes.handles
    }

    pub fn lightbake_handles(&self) -> &[Option<LiveLightbakeHandles>] {
        &self.pages.handles
    }

    /// Takes the images out of the scene and gets ready to hand them over.
    /// The record of the largest step survives a rearm.
    pub fn take_from(&mut self, scene: &mut LevelStage, progress: Option<&LoadCompletion>) {
        // A rearm after a load that never got here leaves the old bar behind;
        // it was interrupted, not finished.
        if let Some(bar) = self.upload_stage.take() {
            bar.cancel();
        }
        let catalog = &scene.live_material_ledger;
        let mut world_pipelines = pipeline_material_ids(
            catalog,
            scene.batches.iter().filter_map(|batch| batch.material),
        );
        let sun_materials = scene
            .sun_effects
            .into_iter()
            .flat_map(|sun| [sun.sprite_material, sun.flare_material])
            .flatten()
            .filter_map(|material| u16::try_from(material).ok());
        world_pipelines.extend(sun_materials);
        let model_materials = scene
            .prop_meshes
            .iter()
            .chain(scene.sky_dome_model.iter())
            .flat_map(|mesh| mesh.facet_materials())
            .flatten();
        let model_pipelines = pipeline_material_ids(catalog, model_materials);
        let mut reachable = reachable_exact_image_slots(catalog, &scene.precise_material_names);
        reachable.extend(reachable_scene_images(scene));

        let exact_images = std::mem::take(&mut scene.precise_material_images);
        let slot_total = exact_images.len();
        let mut exact_common = std::mem::take(&mut scene.exact_material_common);
        exact_common.resize(slot_total, false);
        let mut exact_variants = std::mem::take(&mut scene.exact_material_variants);
        exact_variants.resize(slot_total, None);
        let probes = std::mem::take(&mut scene.env_probe_images);
        let pages = std::mem::take(&mut scene.lightmaps);
        let total = (slot_total + probes.len() + pages.iter().flatten().count()) as u32;

        let fresh = Self {
            expected: total,
            largest_step_ns: self.largest_step_ns,
            largest_step_bytes: self.largest_step_bytes,
            pipeline_level_materials: Arc::new(world_pipelines),
            pipeline_prop_materials: Arc::new(model_pipelines),
            exact: Slots::holding(exact_images),
            exact_variants,
            exact_common,
            reachable_precise: reachable,
            common_profile_id: scene.common_profile_id,
            products_id: scene.products_id,
            probes: Slots::holding(probes),
            pages: Slots::holding(pages),
            upload_stage: progress
                .map(|bar| bar.begin(StageId::WorldImages, Some(u64::from(total)))),
            ..Self::default()
        };
        *self = fresh;
    }

    fn progress(&self) -> Progress {
        Progress {
            slots: self.finished,
            bytes: self.handed_bytes,
        }
    }

    /// Records one handoff that could not be interrupted. Only the largest is
    /// kept: the question is what the smallest keepable budget is, and that
    /// is the largest step, not the distribution.
    fn note_step(&mut self, took: Duration, bytes: u64) {
        let ns = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
        if ns <= self.largest_step_ns {
            return;
        }
        (self.largest_step_ns, self.largest_step_bytes) = (ns, bytes);
    }

    /// Ends one step: counts the slot as done when `counts`, and tells the
    /// loading bar.
    fn end_step(&mut self, pace: &mut Pace, counts: bool) {
        self.finished = self.finished.saturating_add(u32::from(counts));
        pace.advanced = true;
        if let Some(bar) = &self.upload_stage {
            bar.set_completed(u64::from(self.finished));
            bar.set_bytes(self.handed_bytes);
        }
    }

    /// Hands over as many images as `slot_cap` and the byte budget
    /// allow, or until `deadline`. `u32::MAX` for the count means no byte
    /// limit. True once everything is handed over.
    pub fn hand_over_until(
        &mut self,
        gpu_images: &mut Assets<Image>,
        common: &mut ResidentGpuImages,
        stop_at: Option<Instant>,
        slot_cap: u32,
        paced_byte_budget: u64,
    ) -> bool {
        common.use_profile(self.common_profile_id);
        if self.finished == 0 && self.take_resident(common, gpu_images) {
            return true;
        }
        let unpaced = slot_cap == u32::MAX;
        let mut pace = Pace {
            due_at: stop_at,
            from: self.progress(),
            allowance: Progress {
                slots: slot_cap,
                bytes: if unpaced { u64::MAX } else { paced_byte_budget },
            },
            advanced: false,
        };
        // Material images first, then the probes, then the lightmap pages.
        loop {
            let stage_left = [
                !self.exact.exhausted(),
                !self.probes.exhausted(),
                !self.pages.exhausted(),
            ];
            let Some(stage) = stage_left.iter().position(|&left| left) else {
                break;
            };
            if pace.spent(self.progress()) {
                return false;
            }
            if stage == 0 {
                self.step_exact(gpu_images, common, &mut pace);
            } else if stage == 1 {
                self.step_probe(gpu_images, &mut pace);
            } else {
                self.step_page(gpu_images, &mut pace);
            }
        }
        self.keep_for_reload(common);
        if let Some(bar) = self.upload_stage.take() {
            bar.set_completed(u64::from(self.finished));
            bar.set_bytes(self.handed_bytes);
            bar.done();
        }
        true
    }

    fn step_exact(
        &mut self,
        store: &mut Assets<Image>,
        common: &mut ResidentGpuImages,
        pace: &mut Pace,
    ) {
        let Some((slot, image)) = self.exact.take_next() else {
            return;
        };
        let drawn =
            self.reachable_precise.is_empty() || self.reachable_precise.contains(&(slot as u32));
        if !drawn {
            self.passed_over += 1;
            self.end_step(pace, true);
            return;
        }
        let began = Instant::now();
        let handed = image.map_or(0, |image| {
            let (tile, bytes) = self.exact_handle(slot, &image, store, common);
            self.exact.handles[slot] = Some(tile);
            bytes
        });
        self.handed_bytes += handed;
        self.note_step(began.elapsed(), handed);
        self.end_step(pace, true);
    }

    /// The handle for the slot's image and the bytes actually handed over to
    /// get it. A slot whose variant already is an asset, here or in the
    /// shared set, takes that handle and lets its own copy go.
    fn exact_handle(
        &mut self,
        seat: usize,
        image: &Arc<Image>,
        gpu_images: &mut Assets<Image>,
        common: &mut ResidentGpuImages,
    ) -> (Handle<Image>, u64) {
        let bytes = cpu_bytes(image);
        let Some(variant) = self.exact_variants[seat] else {
            return (gpu_images.add(Image::clone(image)), bytes);
        };
        let shared = self.exact_common[seat] && self.common_profile_id != 0;
        let (found, source) = match self.exact_by_variant.get(&variant) {
            Some(found) => (found.clone(), HandleSource::ThisMap),
            None => match shared.then(|| common.get_live(variant, gpu_images)).flatten() {
                Some(found) => (found, HandleSource::Shared),
                None => (gpu_images.add(Image::clone(image)), HandleSource::Added),
            },
        };
        if shared && source != HandleSource::Shared {
            common.by_variant.insert(variant, found.clone());
        }
        if source != HandleSource::ThisMap {
            self.exact_by_variant.insert(variant, found.clone());
        }
        if source == HandleSource::Added {
            return (found, bytes);
        }
        self.reused_handles = self.reused_handles.saturating_add(1);
        self.reused_handle_bytes = self.reused_handle_bytes.saturating_add(bytes);
        if source == HandleSource::Shared {
            self.reused_common_handles = self.reused_common_handles.saturating_add(1);
            self.reused_common_bytes = self.reused_common_bytes.saturating_add(bytes);
        }
        (found, 0)
    }

    fn step_probe(&mut self, gpu_images: &mut Assets<Image>, pace: &mut Pace) {
        let Some((at, probe)) = self.probes.take_next() else {
            return;
        };
        let bytes = probe.as_ref().map_or(0, cpu_bytes);
        self.handed_bytes += bytes;
        let began = Instant::now();
        self.probes.handles[at] = probe.map(|probe| gpu_images.add(debug_painted(probe)));
        self.note_step(began.elapsed(), bytes);
        self.end_step(pace, true);
    }

    /// A page is six adds that cannot be split, so the page is the
    /// indivisible unit and its bytes are the six summed: the budget has to
    /// be told about them even though it cannot stop halfway. A missing page
    /// takes a step but is not counted done.
    fn step_page(&mut self, store: &mut Assets<Image>, pace: &mut Pace) {
        let Some((at, page)) = self.pages.take_next() else {
            return;
        };
        let bytes = page.as_ref().map_or(0, lightmap_page_bytes);
        let began = Instant::now();
        let handles = page.map(|page| hand_over_page(at, page, store));
        let present = handles.is_some();
        self.pages.handles[at] = handles;
        self.handed_bytes += bytes;
        self.note_step(began.elapsed(), bytes);
        self.end_step(pace, present);
    }

    /// Every slot is handed over: remember the handles for a reload of the
    /// same map, and let the decoded CPU copies go now the asset server has
    /// its own.
    fn keep_for_reload(&mut self, common: &mut ResidentGpuImages) {
        let products_id = self.products_id;
        if products_id == 0 {
            return;
        }
        common.map = ResidentMapImages {
            products_id,
            exact: self.exact.handles.clone(),
            probes: self.probes.handles.clone(),
            lightmaps: self.pages.handles.clone(),
        };
        assets::note_gpu_resident(products_id);
        let Some((count, freed)) = assets::release_resident_cpu_images(products_id) else {
            return;
        };
        let freed_mb = freed as f64 / (1024.0 * 1024.0);
        diag::info!(
            World,
            "resident map: walk #{products_id} let go of {count} decoded images ({freed_mb:.0} MB freed) once the card had them"
        );
    }

    /// Reuses the previous load's handles if they are for this very map and
    /// all still alive. Otherwise forgets them.
    fn take_resident(&mut self, common: &mut ResidentGpuImages, images: &Assets<Image>) -> bool {
        let tallies = [
            self.exact.handles.len(),
            self.probes.handles.len(),
            self.pages.handles.len(),
        ];
        let map = &common.map;
        if !(map.fits_in(self.products_id, tallies) && map.all_live(images)) {
            common.map = ResidentMapImages::default();
            assets::note_gpu_resident(0);
            return false;
        }
        self.exact = Slots::handed(map.exact.clone());
        self.probes = Slots::handed(map.probes.clone());
        self.pages = Slots::handed(map.lightmaps.clone());
        self.finished = self.expected;
        self.resident = true;
        if let Some(bar) = self.upload_stage.take() {
            bar.set_completed(u64::from(self.finished));
            bar.reuse();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image whose data is exactly `bytes` long.
    fn image_of(bytes: usize) -> Image {
        let mut image = Image::default();
        image.data = Some(vec![0; bytes]);
        image
    }

    fn arc_images(count: usize) -> Vec<Option<Arc<Image>>> {
        (0..count).map(|_| Some(Arc::new(image_of(4)))).collect()
    }

    /// An upload with `exact` slots to hand over, as `arm` would leave it.
    fn upload_of(exact: Vec<Option<Arc<Image>>>) -> LevelImageUpload {
        let n = exact.len();
        LevelImageUpload {
            expected: n as u32,
            exact_variants: vec![None; n],
            exact_common: vec![false; n],
            exact: Slots::holding(exact),
            ..LevelImageUpload::default()
        }
    }

    /// An upload of one slot whose image is variant 7, shared under `profile`.
    fn shared_upload(profile: u64) -> LevelImageUpload {
        LevelImageUpload {
            exact_variants: vec![Some(variant(7))],
            exact_common: vec![true],
            common_profile_id: profile,
            ..upload_of(arc_images(1))
        }
    }

    fn variant(payload: u64) -> ImageVariantId {
        ImageVariantId { payload, usage: 0 }
    }

    /// A fresh asset store and resident set.
    fn stores() -> (Assets<Image>, ResidentGpuImages) {
        (Assets::default(), ResidentGpuImages::default())
    }

    fn run(
        upload: &mut LevelImageUpload,
        gpu_images: &mut Assets<Image>,
        common: &mut ResidentGpuImages,
    ) -> bool {
        upload.hand_over_until(gpu_images, common, None, u32::MAX, 0)
    }

    fn ids(handles: &[Option<Handle<Image>>]) -> Vec<Option<AssetId<Image>>> {
        handles.iter().map(|h| h.as_ref().map(Handle::id)).collect()
    }

    /// A probe cube of `size` pixels with `levels` mips, filled with zeros.
    fn probe_cube(size: u32, mip_count: u32) -> Image {
        let total: usize = (0..mip_count as usize)
            .map(|level| 6 * cube_face_bytes(size, level))
            .sum();
        let mut image = image_of(total);
        image.texture_descriptor.size.width = size;
        image.texture_descriptor.mip_level_count = mip_count;
        image
    }

    fn texel(image: &Image, at: usize) -> [u8; 4] {
        let data = image.data.as_deref().unwrap_or_default();
        std::array::from_fn(|i| data[at + i])
    }

    fn page(
        primary: Option<usize>,
        ambient: usize,
        directional: usize,
        sun_mask: usize,
    ) -> LevelLightbake {
        LevelLightbake {
            first_slot_texture: primary.map(image_of),
            auxiliary_texture: None,
            second_b_map: None,
            background_light_texture: image_of(ambient),
            heading_texture: image_of(directional),
            daylight_mask_image: image_of(sun_mask),
            background_light_origin_label: String::new(),
            daylight_mask_source_name: String::new(),
            background_light_extent: UVec2::ONE,
            daylight_mask_size: UVec2::ONE,
        }
    }

    #[test]
    fn probe_debug_names_and_colours_parse() {
        let cases = [
            ("faces", EnvProbeDebug::Faces),
            (" mips ", EnvProbeDebug::PerMip { rotate: 0 }),
            ("mips-shift", EnvProbeDebug::PerMip { rotate: 1 }),
            ("12, 34,56", EnvProbeDebug::Solid([12, 34, 56, 255])),
        ];
        for (raw, mode) in cases {
            assert_eq!(parse_probe_debug(raw), Some(mode), "{raw:?}");
        }
    }

    #[test]
    fn probe_debug_refuses_what_it_cannot_read() {
        for raw in ["", "red", "1,2", "300,0,0", "-1,0,0"] {
            assert_eq!(parse_probe_debug(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn probe_colours_follow_the_mode() {
        assert_eq!(
            probe_colour(EnvProbeDebug::Solid([1, 2, 3, 255]), 5, 4),
            [1, 2, 3, 255]
        );
        assert_eq!(
            probe_colour(EnvProbeDebug::Faces, 3, 2),
            rgba(FACE_PALETTE[2])
        );
        let mips = EnvProbeDebug::PerMip { rotate: 0 };
        assert_eq!(probe_colour(mips, 0, 0), rgba(MIP_PALETTE[0]));
        assert_eq!(probe_colour(mips, 7, 5), rgba(MIP_PALETTE[7]));
        assert_eq!(probe_colour(mips, 8, 0), rgba(BEYOND_PALETTE));
        // The rotated palette moves every level one colour along.
        let shifted = EnvProbeDebug::PerMip { rotate: 1 };
        assert_eq!(probe_colour(shifted, 0, 0), rgba(MIP_PALETTE[7]));
        assert_eq!(probe_colour(shifted, 1, 0), rgba(MIP_PALETTE[0]));
        assert_eq!(probe_colour(shifted, 8, 0), rgba(BEYOND_PALETTE));
    }

    #[test]
    fn the_palettes_are_opaque_and_named_right() {
        assert_eq!(rgba(FACE_PALETTE[0]), [255, 0, 0, 255]);
        assert_eq!(rgba(MIP_PALETTE[1]), [255, 128, 0, 255]);
        assert_eq!(rgba(BEYOND_PALETTE), [128, 128, 128, 255]);
        assert!(
            FACE_PALETTE
                .iter()
                .chain(&MIP_PALETTE)
                .all(|&c| rgba(c)[3] == 255)
        );
    }

    #[test]
    fn painting_a_probe_fills_each_face_of_each_level() {
        let mut cube = probe_cube(4, 2);
        paint_env_probe_debug(&mut cube, EnvProbeDebug::Faces);
        let face0 = 4 * 4 * 4;
        // Level 0: six faces of 4x4, then level 1: six faces of 2x2.
        let level1 = 6 * face0;
        let face1 = 2 * 2 * 4;
        for (face, &colour) in FACE_PALETTE.iter().enumerate() {
            let at = [
                face * face0,
                face * face0 + face0 - 4,
                level1 + face * face1,
            ];
            assert!(
                at.iter().all(|&at| texel(&cube, at) == rgba(colour)),
                "face {face}"
            );
        }
        assert_eq!(cube.data.as_ref().map(Vec::len), Some(level1 + 6 * face1));
    }

    #[test]
    fn painting_by_mip_gives_each_level_its_own_colour() {
        let mut cube = probe_cube(4, 3);
        paint_env_probe_debug(&mut cube, EnvProbeDebug::PerMip { rotate: 0 });
        let level0 = 6 * 4 * 4 * 4;
        let level1 = 6 * 2 * 2 * 4;
        for (level, at) in [0, level0, level0 + level1].into_iter().enumerate() {
            assert_eq!(texel(&cube, at), rgba(MIP_PALETTE[level]));
        }
    }

    #[test]
    fn a_probe_of_the_wrong_size_is_left_as_it_was() {
        let mut cube = probe_cube(4, 1);
        if let Some(data) = cube.data.as_mut() {
            data.push(9);
        }
        let before = cube.data.clone();
        paint_env_probe_debug(&mut cube, EnvProbeDebug::Solid([7, 7, 7, 255]));
        assert_eq!(cube.data, before);

        let mut empty = probe_cube(4, 1);
        empty.data = None;
        paint_env_probe_debug(&mut empty, EnvProbeDebug::Faces);
        assert!(empty.data.is_none());
    }

    #[test]
    fn an_image_counts_the_bytes_it_holds() {
        assert_eq!(cpu_bytes(&image_of(0)), 0);
        assert_eq!(cpu_bytes(&image_of(40)), 40);
        let mut gone = image_of(8);
        gone.data = None;
        assert_eq!(cpu_bytes(&gone), 0);
    }

    #[test]
    fn a_lightmap_page_counts_all_six_images() {
        let with = |secondary: bool, auxiliary_second: bool, primary: bool| LevelLightbake {
            auxiliary_texture: secondary.then(|| image_of(20)),
            second_b_map: auxiliary_second.then(|| image_of(40)),
            ..page(primary.then_some(10), 100, 200, 400)
        };
        assert_eq!(lightmap_page_bytes(&with(true, true, true)), 770);
        assert_eq!(lightmap_page_bytes(&with(false, false, false)), 700);
        assert_eq!(lightmap_page_bytes(&with(false, true, true)), 750);
    }

    fn pace(
        stepped: bool,
        stop_at: Option<Instant>,
        from: (u32, u64),
        allowance: (u32, u64),
    ) -> Pace {
        Pace {
            due_at: stop_at,
            from: Progress {
                slots: from.0,
                bytes: from.1,
            },
            allowance: Progress {
                slots: allowance.0,
                bytes: allowance.1,
            },
            advanced: stepped,
        }
    }

    fn at(slots: u32, bytes: u64) -> Progress {
        Progress { slots, bytes }
    }

    #[test]
    fn a_call_always_makes_progress_before_it_may_stop() {
        let past = Instant::now().checked_sub(Duration::from_secs(1));
        assert!(!pace(false, past, (0, 0), (1, 0)).spent(at(5, 5)));
        assert!(pace(true, past, (0, 0), (1, 0)).spent(at(0, 0)));
        // No deadline: the count and the bytes decide.
        assert!(!pace(true, None, (0, 0), (1, 0)).spent(at(0, 0)));
        assert!(pace(true, None, (0, 0), (1, 0)).spent(at(1, 0)));
    }

    #[test]
    fn the_pace_counts_from_where_the_call_started() {
        let pace = pace(true, None, (10, 1000), (2, 50));
        assert!(!pace.spent(at(11, 1100)));
        assert!(!pace.spent(at(12, 1049)));
        assert!(pace.spent(at(12, 1050)));
    }

    #[test]
    fn shared_images_belong_to_one_profile() {
        let (mut images, mut common) = stores();
        common.use_profile(5);
        common
            .by_variant
            .insert(variant(1), images.add(image_of(4)));
        common.use_profile(5);
        assert_eq!(common.by_variant.len(), 1);
        common.use_profile(6);
        assert!(common.by_variant.is_empty());
    }

    #[test]
    fn a_shared_handle_whose_image_was_freed_is_dropped() {
        let (mut images, mut common) = stores();
        let held = images.add(image_of(4));
        common.by_variant.insert(variant(1), held.clone());
        assert_eq!(
            common.get_live(variant(1), &images).map(|h| h.id()),
            Some(held.id())
        );
        assert!(common.get_live(variant(2), &images).is_none());
        images.remove(&held);
        assert!(common.get_live(variant(1), &images).is_none());
        assert!(common.by_variant.is_empty());
    }

    #[test]
    fn exact_slots_that_can_be_drawn_with() {
        let catalog = LiveMaterialLedger::default();
        let names = ["$black", "wall", "$white"].map(str::to_owned);
        assert_eq!(
            reachable_exact_image_slots(&catalog, &names),
            HashSet::from([0, 2])
        );
        // Nothing marks a slot reachable: all of them are.
        let plain = ["a", "b"].map(str::to_owned);
        assert_eq!(
            reachable_exact_image_slots(&catalog, &plain),
            HashSet::from([0, 1])
        );
        assert!(reachable_exact_image_slots(&catalog, &[]).is_empty());
    }

    #[test]
    fn every_slot_is_handed_over_in_one_unpaced_call() {
        let (mut gpu_images, mut common) = stores();
        let mut upload = upload_of(arc_images(3));
        assert!(upload.has_slots_left());
        assert!(run(&mut upload, &mut gpu_images, &mut common));
        assert!(!upload.has_slots_left());
        assert_eq!(
            (upload.finished, upload.handed_bytes, gpu_images.len()),
            (3, 12, 3)
        );
        assert!(upload.precise_handles().iter().all(Option::is_some));
    }

    #[test]
    fn a_slot_nothing_draws_with_is_passed_over() {
        let (mut gpu_images, mut common) = stores();
        let mut upload = LevelImageUpload {
            reachable_precise: HashSet::from([1]),
            ..upload_of(arc_images(3))
        };
        assert!(run(&mut upload, &mut gpu_images, &mut common));
        assert_eq!(
            (upload.passed_over, upload.finished, upload.handed_bytes),
            (2, 3, 4)
        );
        let made: Vec<bool> = upload
            .precise_handles()
            .iter()
            .map(Option::is_some)
            .collect();
        assert_eq!(made, [false, true, false]);
        assert_eq!(gpu_images.len(), 1);
    }

    #[test]
    fn a_slot_with_no_image_is_done_and_hands_nothing() {
        let (mut gpu_images, mut common) = stores();
        let mut upload = upload_of(vec![None, Some(Arc::new(image_of(4)))]);
        assert!(run(&mut upload, &mut gpu_images, &mut common));
        assert_eq!(upload.finished, 2);
        let made: Vec<bool> = upload
            .precise_handles()
            .iter()
            .map(Option::is_some)
            .collect();
        assert_eq!(made, [false, true]);
    }

    #[test]
    fn two_slots_with_one_variant_share_one_asset() {
        let (mut gpu_images, mut common) = stores();
        let mut upload = LevelImageUpload {
            exact_variants: vec![Some(variant(1)), Some(variant(1)), Some(variant(2))],
            ..upload_of(arc_images(3))
        };
        assert!(run(&mut upload, &mut gpu_images, &mut common));
        let handles = ids(upload.precise_handles());
        assert_eq!(handles[0], handles[1]);
        assert_ne!(handles[0], handles[2]);
        assert_eq!(gpu_images.len(), 2);
        assert_eq!((upload.reused_handles, upload.reused_handle_bytes), (1, 4));
        // The reused slot hands nothing over.
        assert_eq!(upload.handed_bytes, 8);
        assert_eq!(upload.reused_common_handles, 0);
    }

    #[test]
    fn a_shared_image_is_handed_over_once_for_every_map() {
        let (mut gpu_images, mut common) = stores();
        let mut first = shared_upload(3);
        assert!(run(&mut first, &mut gpu_images, &mut common));
        assert_eq!((gpu_images.len(), common.by_variant.len()), (1, 1));

        let mut second = shared_upload(3);
        assert!(run(&mut second, &mut gpu_images, &mut common));
        assert_eq!((gpu_images.len(), second.handed_bytes), (1, 0));
        assert_eq!(
            (second.reused_common_handles, second.reused_common_bytes),
            (1, 4)
        );
        assert_eq!(ids(first.precise_handles()), ids(second.precise_handles()));
    }

    #[test]
    fn a_new_profile_does_not_reuse_the_old_shared_images() {
        let (mut gpu_images, mut common) = stores();
        assert!(run(&mut shared_upload(3), &mut gpu_images, &mut common));
        let mut other = shared_upload(4);
        assert!(run(&mut other, &mut gpu_images, &mut common));
        assert_eq!((gpu_images.len(), other.reused_common_handles), (2, 0));
    }

    #[test]
    fn profile_zero_shares_nothing() {
        let (mut gpu_images, mut common) = stores();
        assert!(run(&mut shared_upload(0), &mut gpu_images, &mut common));
        assert!(common.by_variant.is_empty());
    }

    #[test]
    fn a_paced_call_takes_one_slot_when_the_count_allows_one() {
        let (mut images, mut common) = stores();
        let mut upload = upload_of(arc_images(3));
        for (expect_finished, expect_done) in [(false, 1), (false, 2), (true, 3)] {
            assert_eq!(
                upload.hand_over_until(&mut images, &mut common, None, 1, 0),
                expect_finished
            );
            assert_eq!(upload.finished, expect_done);
        }
    }

    #[test]
    fn a_paced_call_goes_on_until_the_byte_allowance_is_used_too() {
        let (mut images, mut common) = stores();
        let mut upload = upload_of(arc_images(3));
        // The count is reached after one slot, but 100 bytes are not.
        assert!(upload.hand_over_until(&mut images, &mut common, None, 1, 100));
        assert_eq!(upload.finished, 3);

        let mut upload = upload_of(arc_images(4));
        // Two slots make 8 bytes, which is the allowance.
        assert!(!upload.hand_over_until(&mut images, &mut common, None, 1, 8));
        assert_eq!((upload.finished, upload.handed_bytes), (2, 8));
    }

    #[test]
    fn an_expired_deadline_still_lets_one_slot_through() {
        let (mut images, mut common) = stores();
        let mut upload = upload_of(arc_images(3));
        let past = Instant::now().checked_sub(Duration::from_secs(1));
        assert!(!upload.hand_over_until(&mut images, &mut common, past, u32::MAX, 0));
        assert_eq!(upload.finished, 1);
    }

    #[test]
    fn probes_and_lightmap_pages_follow_the_slots() {
        let (mut gpu_images, mut common) = stores();
        let mut upload = LevelImageUpload {
            probes: Slots::holding(vec![Some(image_of(16)), None]),
            pages: Slots::holding(vec![None, Some(page(Some(10), 1, 2, 4))]),
            // One slot, two probes, one page that exists.
            expected: 1 + 2 + 1,
            ..upload_of(arc_images(1))
        };
        assert!(run(&mut upload, &mut gpu_images, &mut common));
        assert_eq!(upload.finished, 4);
        let probes: Vec<bool> = upload.probe_refs().iter().map(Option::is_some).collect();
        assert_eq!(probes, [true, false]);
        assert!(upload.lightbake_handles()[0].is_none());
        let made = upload.lightbake_handles()[1].as_ref().map(|page| {
            (
                page.primary.is_some(),
                page.secondary.is_some(),
                page.auxiliary_second.is_some(),
            )
        });
        assert_eq!(made, Some((true, false, false)));
        // 4 exact + 16 probe + (10 + 1 + 2 + 4) page.
        assert_eq!(upload.handed_bytes, 4 + 16 + 17);
        assert_eq!(gpu_images.len(), 1 + 1 + 4);
        let (exact, lightmaps, probes) = upload.stitch_handles();
        assert_eq!((exact.len(), lightmaps.len(), probes.len()), (1, 2, 2));
    }

    #[test]
    fn the_largest_step_is_the_one_that_is_kept() {
        let mut upload = LevelImageUpload::default();
        let steps = [
            (5, 50, (5_000_000, 50)),
            (2, 999, (5_000_000, 50)),
            (9, 7, (9_000_000, 7)),
        ];
        for (ms, bytes, kept) in steps {
            upload.note_step(Duration::from_millis(ms), bytes);
            assert_eq!((upload.largest_step_ns, upload.largest_step_bytes), kept);
        }
    }

    #[test]
    fn a_reload_of_the_same_map_hands_nothing_over() {
        let (mut gpu_images, mut common) = stores();
        let make = || LevelImageUpload {
            products_id: 9,
            probes: Slots::holding(vec![Some(image_of(4))]),
            expected: 3,
            ..upload_of(arc_images(2))
        };
        let mut first = make();
        assert!(run(&mut first, &mut gpu_images, &mut common));
        assert!(!first.resident);
        assert_eq!((common.map.products_id, gpu_images.len()), (9, 3));

        let mut again = make();
        assert!(run(&mut again, &mut gpu_images, &mut common));
        assert!(again.resident);
        assert_eq!(again.finished, again.expected);
        assert_eq!((gpu_images.len(), again.handed_bytes), (3, 0));
        assert_eq!(ids(again.precise_handles()), ids(first.precise_handles()));
        assert!(again.probe_refs()[0].is_some());
    }

    #[test]
    fn a_reload_whose_images_were_freed_hands_them_over_again() {
        let (mut gpu_images, mut common) = stores();
        let make = || LevelImageUpload {
            products_id: 11,
            ..upload_of(arc_images(2))
        };
        let mut first = make();
        assert!(run(&mut first, &mut gpu_images, &mut common));
        if let Some(freed) = &first.precise_handles()[0] {
            gpu_images.remove(freed);
        }

        let mut again = make();
        assert!(run(&mut again, &mut gpu_images, &mut common));
        assert!(!again.resident);
        assert_eq!(again.handed_bytes, 8);
        assert!(
            again
                .precise_handles()
                .iter()
                .flatten()
                .all(|h| gpu_images.get(h.id()).is_some())
        );
    }

    #[test]
    fn a_different_map_does_not_reuse_the_resident_images() {
        let (mut gpu_images, mut common) = stores();
        let make = |products_id| LevelImageUpload {
            products_id,
            ..upload_of(arc_images(1))
        };
        assert!(run(&mut make(21), &mut gpu_images, &mut common));
        let mut other = make(22);
        assert!(run(&mut other, &mut gpu_images, &mut common));
        assert!(!other.resident);
        assert_eq!((common.map.products_id, gpu_images.len()), (22, 2));
    }

    #[test]
    fn a_map_with_no_products_id_is_not_kept_for_a_reload() {
        let (mut gpu_images, mut common) = stores();
        assert!(run(&mut upload_of(arc_images(1)), &mut gpu_images, &mut common));
        assert_eq!(common.map.products_id, 0);
        assert!(common.map.exact.is_empty());
    }
}
