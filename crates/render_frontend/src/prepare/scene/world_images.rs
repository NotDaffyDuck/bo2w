//! Handing a world's images (material textures, reflection probes,
//! lightmaps) over to `Assets<Image>` a slice at a time, so the loading
//! screen keeps moving, and keeping them for the next load of the same map.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use asset_material::ImageVariantId;
use asset_transport::{LoadProgress, StageHandle, StageId};
use bevy::prelude::*;

use crate::assemble::drawsurf::{RuntimeLightmapHandles, RuntimeMaterialCatalog};

use super::world::{WorldLightmap, WorldScene};

/// Bytes a loading-screen frame hands over before it stops (scaled by the
/// caller), and the most one slice may take whatever the scaling says.
pub(crate) const IMAGE_BYTES_PER_OVERLAY_FRAME: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_IMAGE_BYTES_PER_SLICE: u64 = 128 * 1024 * 1024;
pub(crate) const IMAGES_PER_OVERLAY_FRAME: u32 = 16;

// ---------------------------------------------------------------------------
// Reflection probe debugging: `BO2MP_PROBE_DEBUG` paints the probe cubes flat
// so one can see which face or mip level a surface samples.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeDebug {
    /// Every texel this colour.
    Flat([u8; 4]),
    /// One colour per cube face.
    Faces,
    /// One colour per mip level, the palette rotated by `shift` levels.
    Mips { shift: usize },
}

const FACE_PALETTE: [[u8; 4]; 6] = [
    [255, 0, 0, 255],
    [0, 255, 255, 255],
    [0, 255, 0, 255],
    [255, 0, 255, 255],
    [0, 0, 255, 255],
    [255, 255, 0, 255],
];

const MIP_PALETTE: [[u8; 4]; 8] = [
    [255, 0, 0, 255],
    [255, 128, 0, 255],
    [255, 255, 0, 255],
    [0, 255, 0, 255],
    [0, 255, 255, 255],
    [0, 0, 255, 255],
    [255, 0, 255, 255],
    [255, 255, 255, 255],
];

/// Mip levels past the palette.
const BEYOND_PALETTE: [u8; 4] = [128, 128, 128, 255];

fn parse_probe_debug(raw: &str) -> Option<ProbeDebug> {
    let raw = raw.trim();
    match raw {
        "faces" => {
            diag::warn!(
                World,
                "reflection probe A/B: every cube layer flat, +X red -X cyan +Y green -Y magenta +Z blue -Z yellow"
            );
            return Some(ProbeDebug::Faces);
        }
        "mips" | "mips-shift" => {
            let shift = usize::from(raw == "mips-shift");
            diag::warn!(
                World,
                "reflection probe A/B: every mip flat, palette {}",
                if shift == 0 {
                    "0 red 1 orange 2 yellow 3 green 4 cyan 5 blue 6 magenta 7 white"
                } else {
                    "rotated by one level (0 white 1 red 2 orange 3 yellow 4 green 5 cyan 6 blue 7 magenta)"
                }
            );
            return Some(ProbeDebug::Mips { shift });
        }
        _ => {}
    }
    let mut channels = raw.split(',').map(|part| part.trim().parse::<u8>());
    if let (Some(Ok(r)), Some(Ok(g)), Some(Ok(b))) =
        (channels.next(), channels.next(), channels.next())
    {
        diag::warn!(World, "reflection probe A/B: every probe forced to {r},{g},{b},255");
        return Some(ProbeDebug::Flat([r, g, b, 255]));
    }
    diag::warn!(World, "unparseable BO2MP_PROBE_DEBUG={raw:?}; probes unchanged");
    None
}

fn probe_debug_mode() -> Option<ProbeDebug> {
    static MODE: std::sync::OnceLock<Option<ProbeDebug>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| parse_probe_debug(&std::env::var("BO2MP_PROBE_DEBUG").ok()?))
}

fn probe_colour(mode: ProbeDebug, level: usize, face: usize) -> [u8; 4] {
    match mode {
        ProbeDebug::Flat(colour) => colour,
        ProbeDebug::Faces => FACE_PALETTE[face],
        ProbeDebug::Mips { shift } => {
            let n = MIP_PALETTE.len();
            if level < n {
                MIP_PALETTE[(level + n - shift % n) % n]
            } else {
                BEYOND_PALETTE
            }
        }
    }
}

/// Paints every texel of every face of every mip of a probe cube.
fn paint_probe_debug(image: &mut Image, mode: ProbeDebug) {
    let size = image.texture_descriptor.size.width;
    let levels = image.texture_descriptor.mip_level_count as usize;
    let Some(data) = image.data.as_mut() else {
        diag::warn!(
            World,
            "BO2MP_PROBE_DEBUG: probe cube has no CPU data; left unpainted"
        );
        return;
    };
    // Bytes of one face at a level: the cube stores the six faces of a level
    // one after the other, the levels one after the other.
    let face_bytes = |level: usize| {
        let side = (size >> level).max(1) as usize;
        side * side * 4
    };
    let expected: usize = (0..levels).map(|level| face_bytes(level) * 6).sum();
    if data.len() != expected {
        diag::warn!(
            World,
            "BO2MP_PROBE_DEBUG: probe cube is {} bytes, mip walk of {size}px x{levels} needs {expected}; left unpainted",
            data.len()
        );
        return;
    }
    let mut at = 0usize;
    for level in 0..levels {
        for face in 0..6 {
            let colour = probe_colour(mode, level, face);
            let end = at + face_bytes(level);
            for texel in data[at..end].chunks_exact_mut(4) {
                texel.copy_from_slice(&colour);
            }
            at = end;
        }
    }
}

// ---------------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------------

fn image_bytes(image: &Image) -> u64 {
    image.data.as_ref().map_or(0, |data| data.len() as u64)
}

/// Every image one lightmap page hands over, summed. A page is six `add`s, and
/// the byte counts and the budget that paces the loading screen are owed all
/// six of them.
fn lightmap_page_bytes(page: &WorldLightmap) -> u64 {
    page.primary_image.as_ref().map_or(0, image_bytes)
        + page.secondary_image.as_ref().map_or(0, image_bytes)
        + page.secondary_b_image.as_ref().map_or(0, image_bytes)
        + image_bytes(&page.ambient_image)
        + image_bytes(&page.directional_image)
        + image_bytes(&page.sun_mask_image)
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

#[derive(Default)]
struct ResidentMapImages {
    products_id: u64,
    exact: Vec<Option<Handle<Image>>>,
    probes: Vec<Option<Handle<Image>>>,
    lightmaps: Vec<Option<RuntimeLightmapHandles>>,
}

impl ResidentMapImages {
    /// True while every handle still points at an image in `images`.
    fn all_live(&self, images: &Assets<Image>) -> bool {
        let live = |handle: &Handle<Image>| images.get(handle.id()).is_some();
        let pages_live = self.lightmaps.iter().flatten().all(|page| {
            page.primary.iter().all(live)
                && page.secondary.iter().all(live)
                && page.secondary_b.iter().all(live)
                && live(&page.ambient_diagnostic)
                && live(&page.directional_diagnostic)
                && live(&page.sun_mask_diagnostic)
        });
        pages_live && self.exact.iter().chain(&self.probes).flatten().all(live)
    }
}

impl ResidentGpuImages {
    /// Shared images belong to one profile; a different one drops them.
    fn use_profile(&mut self, profile_id: u64) {
        if self.profile_id != profile_id {
            self.by_variant.clear();
            self.profile_id = profile_id;
        }
    }

    /// The shared handle for the variant, dropped from the table if its
    /// image has been freed.
    fn get_live(&mut self, variant: ImageVariantId, images: &Assets<Image>) -> Option<Handle<Image>> {
        let handle = self.by_variant.get(&variant)?;
        if images.get(handle.id()).is_some() {
            return Some(handle.clone());
        }
        self.by_variant.remove(&variant);
        None
    }
}

// ---------------------------------------------------------------------------
// The handoff
// ---------------------------------------------------------------------------

/// How much one call of `until` may do before it gives the frame back.
struct Pace {
    deadline: Option<Instant>,
    start_done: u32,
    start_bytes: u64,
    max_count: u32,
    byte_budget: u64,
    /// At least one step has been taken; a call always makes progress.
    stepped: bool,
}

impl Pace {
    /// True when the call has done something and may now stop: out of time,
    /// or out of both its count and its byte allowance.
    fn spent(&self, done: u32, bytes: u64) -> bool {
        self.stepped
            && (self.deadline.is_some_and(|end| Instant::now() >= end)
                || (done.saturating_sub(self.start_done) >= self.max_count
                    && bytes.saturating_sub(self.start_bytes) >= self.byte_budget))
    }
}

/// True when both the count and the byte allowance of a loading-screen frame
/// are used up.
pub(crate) fn overlay_count_byte_capped(
    count: u32,
    bytes: u64,
    max_count: u32,
    byte_budget: u64,
) -> bool {
    count >= max_count && bytes >= byte_budget
}

/// The materials' image slots that something can draw with.
fn reachable_exact_image_slots(catalog: &RuntimeMaterialCatalog, names: &[String]) -> HashSet<u32> {
    let mut slots: HashSet<u32> = names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.starts_with('$'))
        .map(|(index, _)| index as u32)
        .collect();
    slots.extend(
        catalog
            .materials
            .iter()
            .flat_map(|material| &material.textures)
            .filter_map(|(_, texture)| texture.as_ref())
            .map(|binding| binding.image.0),
    );
    if slots.is_empty() {
        slots.extend(0..names.len() as u32);
    }
    slots
}

/// The pipeline ids of the materials that will need an exact pipeline. A
/// material with no technique set (Black Ops II's) never gets one; it draws
/// through the fallback pass, so the spawn must not wait on one.
fn pipeline_material_ids(
    catalog: &RuntimeMaterialCatalog,
    materials: impl IntoIterator<Item = assets::MaterialIndex>,
) -> HashSet<u16> {
    materials
        .into_iter()
        .filter_map(|id| catalog.derived(id))
        .filter(|material| !material.technique_set.is_empty())
        .map(|material| material.asset_id.0)
        .collect()
}

#[derive(Default)]
pub struct WorldImageUpload {
    pub done: u32,
    pub total: u32,
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
    pub skipped: u32,
    pub pipeline_world_materials: Arc<HashSet<u16>>,
    pub pipeline_smodel_materials: Arc<HashSet<u16>>,
    /// Slots that took another slot's handle, and the bytes that saved.
    pub reused_handles: u32,
    pub reused_handle_bytes: u64,
    pub reused_common_handles: u32,
    pub reused_common_bytes: u64,
    /// The map's images were still resident and nothing was handed over.
    pub resident: bool,

    exact_images: Vec<Option<Arc<Image>>>,
    exact_variants: Vec<Option<ImageVariantId>>,
    /// One asset per prepared variant. Two catalog slots that hold the same
    /// variant hold byte-identical images with the same sampler and colour
    /// space; giving each its own `Assets<Image>` entry uploads the same
    /// texture twice and keeps two copies of it resident, so the second slot
    /// takes a clone of the first one's handle and drops its own copy.
    exact_by_variant: HashMap<ImageVariantId, Handle<Image>>,
    exact_handles: Vec<Option<Handle<Image>>>,
    /// Slots whose image belongs to the shared profile.
    exact_common: Vec<bool>,
    common_profile_id: u64,
    products_id: u64,
    exact_at: usize,
    reachable_exact: HashSet<u32>,
    probes: Vec<Option<Image>>,
    probe_handles: Vec<Option<Handle<Image>>>,
    probe_at: usize,
    lightmaps: Vec<Option<WorldLightmap>>,
    lightmap_handles: Vec<Option<RuntimeLightmapHandles>>,
    lightmap_at: usize,
    upload_stage: Option<StageHandle>,
}

impl WorldImageUpload {
    pub fn unfinished(&self) -> bool {
        self.done < self.total
    }

    pub fn tess_handles(
        &self,
    ) -> (
        &[Option<Handle<Image>>],
        &[Option<RuntimeLightmapHandles>],
        &[Option<Handle<Image>>],
    ) {
        (
            &self.exact_handles,
            &self.lightmap_handles,
            &self.probe_handles,
        )
    }

    pub fn exact_handles(&self) -> &[Option<Handle<Image>>] {
        &self.exact_handles
    }

    pub fn probe_handles(&self) -> &[Option<Handle<Image>>] {
        &self.probe_handles
    }

    pub fn lightmap_handles(&self) -> &[Option<RuntimeLightmapHandles>] {
        &self.lightmap_handles
    }

    /// Takes the images out of the scene and gets ready to hand them over.
    pub fn arm(&mut self, scene: &mut WorldScene, progress: Option<&LoadProgress>) {
        let catalog = &scene.runtime_material_catalog;
        let mut world_pipelines = pipeline_material_ids(
            catalog,
            scene.batches.iter().filter_map(|batch| batch.material),
        );
        let model_pipelines = pipeline_material_ids(
            catalog,
            scene
                .static_model_meshes
                .iter()
                .chain(scene.sky_model.iter())
                .flat_map(|mesh| mesh.surface_materials())
                .flatten(),
        );

        self.exact_images = std::mem::take(&mut scene.exact_material_images);
        let slot_count = self.exact_images.len();
        self.exact_common = std::mem::take(&mut scene.exact_material_common);
        self.exact_common.resize(slot_count, false);
        self.exact_variants = std::mem::take(&mut scene.exact_material_variants);
        self.exact_variants.resize(slot_count, None);
        self.exact_handles = vec![None; slot_count];
        self.exact_by_variant.clear();
        self.common_profile_id = scene.common_profile_id;
        self.products_id = scene.products_id;
        self.resident = false;

        self.reachable_exact =
            reachable_exact_image_slots(&scene.runtime_material_catalog, &scene.exact_material_names);
        self.reachable_exact.extend(
            scene
                .primary_light_attenuation
                .iter()
                .filter_map(|light| light.image),
        );
        self.reachable_exact
            .extend(scene.dynamic_light.and_then(|light| light.attenuation.image));
        self.reachable_exact.extend(scene.outdoor_image);
        if let Some(sun) = scene.sun_effects {
            self.reachable_exact.extend(sun.sprite_image);
            self.reachable_exact.extend(sun.flare_image);
            world_pipelines.extend(
                [sun.sprite_material, sun.flare_material]
                    .into_iter()
                    .flatten()
                    .filter_map(|material| u16::try_from(material).ok()),
            );
        }
        self.pipeline_world_materials = Arc::new(world_pipelines);
        self.pipeline_smodel_materials = Arc::new(model_pipelines);

        self.probes = std::mem::take(&mut scene.reflection_probes);
        self.probe_handles = vec![None; self.probes.len()];
        self.lightmaps = std::mem::take(&mut scene.lightmaps);
        self.lightmap_handles = vec![None; self.lightmaps.len()];

        let pages = self.lightmaps.iter().flatten().count();
        self.total = (slot_count + self.probes.len() + pages) as u32;
        self.done = 0;
        self.skipped = 0;
        self.handed_bytes = 0;
        self.reused_handles = 0;
        self.reused_handle_bytes = 0;
        self.reused_common_handles = 0;
        self.reused_common_bytes = 0;
        self.exact_at = 0;
        self.probe_at = 0;
        self.lightmap_at = 0;

        // A rearm after a load that never got here leaves the old bar behind;
        // it was interrupted, not finished.
        if let Some(bar) = self.upload_stage.take() {
            bar.cancel();
        }
        self.upload_stage = progress
            .map(|progress| progress.begin(StageId::WorldImages, Some(u64::from(self.total))));
    }

    /// Records one handoff that could not be interrupted. Only the largest is
    /// kept: the question is what the smallest keepable budget is, and that
    /// is the largest step, not the distribution.
    fn note_step(&mut self, took: Duration, bytes: u64) {
        let ns = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
        if ns > self.largest_step_ns {
            self.largest_step_ns = ns;
            self.largest_step_bytes = bytes;
        }
    }

    /// Counts one slot as handled and tells the loading bar.
    fn finish_step(&mut self, pace: &mut Pace) {
        self.done = self.done.saturating_add(1);
        pace.stepped = true;
        if let Some(bar) = &self.upload_stage {
            bar.set_completed(u64::from(self.done));
            bar.set_bytes(self.handed_bytes);
        }
    }

    /// Hands over as many images as `max_this_frame` and the byte budget
    /// allow, or until `deadline`. `u32::MAX` for the count means no byte
    /// limit. True once everything is handed over.
    pub fn until(
        &mut self,
        images: &mut Assets<Image>,
        common: &mut ResidentGpuImages,
        deadline: Option<Instant>,
        max_this_frame: u32,
        paced_byte_budget: u64,
    ) -> bool {
        common.use_profile(self.common_profile_id);
        if self.done == 0 && self.take_resident(common, images) {
            return true;
        }
        let mut pace = Pace {
            deadline,
            start_done: self.done,
            start_bytes: self.handed_bytes,
            max_count: max_this_frame,
            byte_budget: if max_this_frame == u32::MAX {
                u64::MAX
            } else {
                paced_byte_budget
            },
            stepped: false,
        };
        let finished = self.hand_over_exact(images, common, &mut pace)
            && self.hand_over_probes(images, &mut pace)
            && self.hand_over_lightmaps(images, &mut pace);
        if !finished {
            return false;
        }
        self.keep_for_reload(common);
        if let Some(bar) = self.upload_stage.take() {
            bar.set_completed(u64::from(self.done));
            bar.set_bytes(self.handed_bytes);
            bar.done();
        }
        true
    }

    fn hand_over_exact(
        &mut self,
        images: &mut Assets<Image>,
        common: &mut ResidentGpuImages,
        pace: &mut Pace,
    ) -> bool {
        while self.exact_at < self.exact_images.len() {
            if pace.spent(self.done, self.handed_bytes) {
                return false;
            }
            let slot = self.exact_at;
            let image = self.exact_images[slot].take();
            self.exact_at += 1;
            if !self.reachable_exact.is_empty() && !self.reachable_exact.contains(&(slot as u32)) {
                self.skipped = self.skipped.saturating_add(1);
                self.finish_step(pace);
                continue;
            }
            let started = Instant::now();
            let mut handed = 0;
            if let Some(image) = image {
                let (handle, bytes) = self.exact_handle(slot, &image, images, common);
                self.exact_handles[slot] = Some(handle);
                handed = bytes;
            }
            self.handed_bytes += handed;
            self.note_step(started.elapsed(), handed);
            self.finish_step(pace);
        }
        true
    }

    /// The handle for the slot's image and the bytes actually handed over to
    /// get it. A slot whose variant already is an asset, here or in the
    /// shared set, takes that handle and lets its own copy go.
    fn exact_handle(
        &mut self,
        slot: usize,
        image: &Arc<Image>,
        images: &mut Assets<Image>,
        common: &mut ResidentGpuImages,
    ) -> (Handle<Image>, u64) {
        let bytes = image_bytes(image);
        let Some(variant) = self.exact_variants[slot] else {
            return (images.add((**image).clone()), bytes);
        };
        let shared = self.exact_common[slot] && self.common_profile_id != 0;

        if let Some(handle) = self.exact_by_variant.get(&variant) {
            if shared {
                common.by_variant.insert(variant, handle.clone());
            }
            self.reused_handles = self.reused_handles.saturating_add(1);
            self.reused_handle_bytes = self.reused_handle_bytes.saturating_add(bytes);
            return (handle.clone(), 0);
        }
        if shared && let Some(handle) = common.get_live(variant, images) {
            self.reused_handles = self.reused_handles.saturating_add(1);
            self.reused_handle_bytes = self.reused_handle_bytes.saturating_add(bytes);
            self.reused_common_handles = self.reused_common_handles.saturating_add(1);
            self.reused_common_bytes = self.reused_common_bytes.saturating_add(bytes);
            self.exact_by_variant.insert(variant, handle.clone());
            return (handle, 0);
        }
        let handle = images.add((**image).clone());
        if shared {
            common.by_variant.insert(variant, handle.clone());
        }
        self.exact_by_variant.insert(variant, handle.clone());
        (handle, bytes)
    }

    fn hand_over_probes(&mut self, images: &mut Assets<Image>, pace: &mut Pace) -> bool {
        while self.probe_at < self.probes.len() {
            if pace.spent(self.done, self.handed_bytes) {
                return false;
            }
            let at = self.probe_at;
            self.probe_at += 1;
            let image = self.probes[at].take();
            let bytes = image.as_ref().map_or(0, image_bytes);
            self.handed_bytes += bytes;
            let started = Instant::now();
            self.probe_handles[at] = image.map(|mut image| {
                if let Some(mode) = probe_debug_mode() {
                    paint_probe_debug(&mut image, mode);
                }
                images.add(image)
            });
            self.note_step(started.elapsed(), bytes);
            self.finish_step(pace);
        }
        true
    }

    fn hand_over_lightmaps(&mut self, images: &mut Assets<Image>, pace: &mut Pace) -> bool {
        while self.lightmap_at < self.lightmaps.len() {
            if pace.spent(self.done, self.handed_bytes) {
                return false;
            }
            let at = self.lightmap_at;
            self.lightmap_at += 1;
            let page = self.lightmaps[at].take();
            // A page is six adds that cannot be split, so the page is the
            // indivisible unit and its bytes are the six summed: the budget
            // has to be told about them even though it cannot stop halfway.
            let bytes = page.as_ref().map_or(0, lightmap_page_bytes);
            let started = Instant::now();
            let handles = page.map(|page| {
                diag::info!(
                    World,
                    "world lightmap[{at}]: secondary={} 2x{}x{} mask={} {}x{}",
                    page.ambient_source_name,
                    page.ambient_size.x,
                    page.ambient_size.y,
                    page.sun_mask_source_name,
                    page.sun_mask_size.x,
                    page.sun_mask_size.y
                );
                RuntimeLightmapHandles {
                    primary: page.primary_image.map(|image| images.add(image)),
                    secondary: page.secondary_image.map(|image| images.add(image)),
                    secondary_b: page.secondary_b_image.map(|image| images.add(image)),
                    ambient_diagnostic: images.add(page.ambient_image),
                    directional_diagnostic: images.add(page.directional_image),
                    sun_mask_diagnostic: images.add(page.sun_mask_image),
                }
            });
            let counts = handles.is_some();
            self.lightmap_handles[at] = handles;
            self.handed_bytes += bytes;
            self.note_step(started.elapsed(), bytes);
            if counts {
                self.done = self.done.saturating_add(1);
            }
            pace.stepped = true;
            if let Some(bar) = &self.upload_stage {
                bar.set_completed(u64::from(self.done));
                bar.set_bytes(self.handed_bytes);
            }
        }
        true
    }

    /// Every slot is handed over: remember the handles for a reload of the
    /// same map, and let the decoded CPU copies go now the asset server has
    /// its own.
    fn keep_for_reload(&mut self, common: &mut ResidentGpuImages) {
        if self.products_id == 0 {
            return;
        }
        common.map = ResidentMapImages {
            products_id: self.products_id,
            exact: self.exact_handles.clone(),
            probes: self.probe_handles.clone(),
            lightmaps: self.lightmap_handles.clone(),
        };
        assets::note_gpu_resident(self.products_id);
        if let Some((count, freed)) = assets::release_resident_cpu_images(self.products_id) {
            diag::info!(
                World,
                "resident map: walk #{} let go of {count} decoded images ({:.0} MB freed) once the card had them",
                self.products_id,
                freed as f64 / (1024.0 * 1024.0)
            );
        }
    }

    /// Reuses the previous load's handles if they are for this very map and
    /// all still alive. Otherwise forgets them.
    fn take_resident(&mut self, common: &mut ResidentGpuImages, images: &Assets<Image>) -> bool {
        let map = &common.map;
        let same_map = self.products_id != 0
            && map.products_id == self.products_id
            && map.exact.len() == self.exact_handles.len()
            && map.probes.len() == self.probe_handles.len()
            && map.lightmaps.len() == self.lightmap_handles.len();
        if !same_map || !map.all_live(images) {
            common.map = ResidentMapImages::default();
            assets::note_gpu_resident(0);
            return false;
        }
        self.exact_handles.clone_from(&map.exact);
        self.probe_handles.clone_from(&map.probes);
        self.lightmap_handles.clone_from(&map.lightmaps);
        self.exact_images.clear();
        self.probes.clear();
        self.lightmaps.clear();
        self.exact_at = 0;
        self.probe_at = 0;
        self.lightmap_at = 0;
        self.done = self.total;
        self.resident = true;
        if let Some(bar) = self.upload_stage.take() {
            bar.set_completed(u64::from(self.done));
            bar.reuse();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    use super::*;

    /// An image whose data is exactly `bytes` long.
    fn image_of(bytes: usize) -> Image {
        let mut image = Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        image.data = Some(vec![0; bytes]);
        image
    }

    fn arc_images(count: usize) -> Vec<Option<Arc<Image>>> {
        (0..count).map(|_| Some(Arc::new(image_of(4)))).collect()
    }

    /// An upload with `exact` slots to hand over, as `arm` would leave it.
    fn upload_of(exact: Vec<Option<Arc<Image>>>) -> WorldImageUpload {
        let n = exact.len();
        WorldImageUpload {
            total: n as u32,
            exact_variants: vec![None; n],
            exact_common: vec![false; n],
            exact_handles: vec![None; n],
            exact_images: exact,
            ..WorldImageUpload::default()
        }
    }

    fn variant(payload: u64) -> ImageVariantId {
        ImageVariantId { payload, usage: 0 }
    }

    fn run(upload: &mut WorldImageUpload, images: &mut Assets<Image>, common: &mut ResidentGpuImages) -> bool {
        upload.until(images, common, None, u32::MAX, 0)
    }

    /// A probe cube of `size` pixels with `levels` mips, filled with zeros.
    fn probe_cube(size: u32, levels: u32) -> Image {
        let mut image = Image::new_fill(
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
            TextureDimension::D2,
            &[0, 0, 0, 0],
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        let total: usize = (0..levels)
            .map(|level| {
                let side = (size >> level).max(1) as usize;
                side * side * 4 * 6
            })
            .sum();
        image.texture_descriptor.mip_level_count = levels;
        image.data = Some(vec![0; total]);
        image
    }

    fn texel(image: &Image, at: usize) -> [u8; 4] {
        let data = image.data.as_ref().expect("data");
        [data[at], data[at + 1], data[at + 2], data[at + 3]]
    }

    #[test]
    fn probe_debug_names_and_colours_parse() {
        assert_eq!(parse_probe_debug("faces"), Some(ProbeDebug::Faces));
        assert_eq!(parse_probe_debug(" mips "), Some(ProbeDebug::Mips { shift: 0 }));
        assert_eq!(
            parse_probe_debug("mips-shift"),
            Some(ProbeDebug::Mips { shift: 1 })
        );
        assert_eq!(
            parse_probe_debug("12, 34,56"),
            Some(ProbeDebug::Flat([12, 34, 56, 255]))
        );
    }

    #[test]
    fn probe_debug_refuses_what_it_cannot_read() {
        assert_eq!(parse_probe_debug(""), None);
        assert_eq!(parse_probe_debug("red"), None);
        assert_eq!(parse_probe_debug("1,2"), None);
        assert_eq!(parse_probe_debug("300,0,0"), None);
        assert_eq!(parse_probe_debug("-1,0,0"), None);
    }

    #[test]
    fn probe_colours_follow_the_mode() {
        let flat = ProbeDebug::Flat([1, 2, 3, 255]);
        assert_eq!(probe_colour(flat, 5, 4), [1, 2, 3, 255]);
        assert_eq!(probe_colour(ProbeDebug::Faces, 3, 2), FACE_PALETTE[2]);
        let mips = ProbeDebug::Mips { shift: 0 };
        assert_eq!(probe_colour(mips, 0, 0), MIP_PALETTE[0]);
        assert_eq!(probe_colour(mips, 7, 5), MIP_PALETTE[7]);
        assert_eq!(probe_colour(mips, 8, 0), BEYOND_PALETTE);
        // The rotated palette moves every level one colour along.
        let shifted = ProbeDebug::Mips { shift: 1 };
        assert_eq!(probe_colour(shifted, 0, 0), MIP_PALETTE[7]);
        assert_eq!(probe_colour(shifted, 1, 0), MIP_PALETTE[0]);
        assert_eq!(probe_colour(shifted, 8, 0), BEYOND_PALETTE);
    }

    #[test]
    fn painting_a_probe_fills_each_face_of_each_level() {
        let mut cube = probe_cube(4, 2);
        paint_probe_debug(&mut cube, ProbeDebug::Faces);
        let face0 = 4 * 4 * 4;
        // Level 0: six faces of 4x4, then level 1: six faces of 2x2.
        for face in 0..6 {
            assert_eq!(texel(&cube, face * face0), FACE_PALETTE[face]);
            assert_eq!(texel(&cube, face * face0 + face0 - 4), FACE_PALETTE[face]);
        }
        let level1 = 6 * face0;
        let face1 = 2 * 2 * 4;
        for face in 0..6 {
            assert_eq!(texel(&cube, level1 + face * face1), FACE_PALETTE[face]);
        }
        assert_eq!(cube.data.as_ref().map(Vec::len), Some(level1 + 6 * face1));
    }

    #[test]
    fn painting_by_mip_gives_each_level_its_own_colour() {
        let mut cube = probe_cube(4, 3);
        paint_probe_debug(&mut cube, ProbeDebug::Mips { shift: 0 });
        let level0 = 6 * 4 * 4 * 4;
        let level1 = 6 * 2 * 2 * 4;
        assert_eq!(texel(&cube, 0), MIP_PALETTE[0]);
        assert_eq!(texel(&cube, level0), MIP_PALETTE[1]);
        assert_eq!(texel(&cube, level0 + level1), MIP_PALETTE[2]);
    }

    #[test]
    fn a_probe_of_the_wrong_size_is_left_as_it_was() {
        let mut cube = probe_cube(4, 1);
        cube.data.as_mut().expect("data").push(9);
        let before = cube.data.clone();
        paint_probe_debug(&mut cube, ProbeDebug::Flat([7, 7, 7, 255]));
        assert_eq!(cube.data, before);

        let mut empty = probe_cube(4, 1);
        empty.data = None;
        paint_probe_debug(&mut empty, ProbeDebug::Faces);
        assert!(empty.data.is_none());
    }

    #[test]
    fn an_image_counts_the_bytes_it_holds() {
        assert_eq!(image_bytes(&image_of(0)), 0);
        assert_eq!(image_bytes(&image_of(40)), 40);
        let mut gone = image_of(8);
        gone.data = None;
        assert_eq!(image_bytes(&gone), 0);
    }

    #[test]
    fn a_lightmap_page_counts_all_six_images() {
        let page = |primary: bool, secondary: bool, secondary_b: bool| WorldLightmap {
            primary_image: primary.then(|| image_of(10)),
            secondary_image: secondary.then(|| image_of(20)),
            secondary_b_image: secondary_b.then(|| image_of(40)),
            ambient_image: image_of(100),
            directional_image: image_of(200),
            sun_mask_image: image_of(400),
            ambient_source_name: String::new(),
            sun_mask_source_name: String::new(),
            ambient_size: UVec2::ONE,
            sun_mask_size: UVec2::ONE,
        };
        assert_eq!(lightmap_page_bytes(&page(true, true, true)), 770);
        assert_eq!(lightmap_page_bytes(&page(false, false, false)), 700);
        assert_eq!(lightmap_page_bytes(&page(true, false, true)), 750);
    }

    #[test]
    fn a_loading_frame_is_capped_only_when_count_and_bytes_are_both_used() {
        assert!(overlay_count_byte_capped(16, 100, 16, 100));
        assert!(overlay_count_byte_capped(20, 500, 16, 100));
        assert!(!overlay_count_byte_capped(15, 500, 16, 100));
        assert!(!overlay_count_byte_capped(20, 99, 16, 100));
    }

    #[test]
    fn a_call_always_makes_progress_before_it_may_stop() {
        let pace = |stepped, deadline: Option<Instant>| Pace {
            deadline,
            start_done: 0,
            start_bytes: 0,
            max_count: 1,
            byte_budget: 0,
            stepped,
        };
        let past = Instant::now().checked_sub(Duration::from_secs(1));
        assert!(!pace(false, past).spent(5, 5));
        assert!(pace(true, past).spent(0, 0));
        // No deadline: the count and the bytes decide.
        assert!(!pace(true, None).spent(0, 0));
        assert!(pace(true, None).spent(1, 0));
    }

    #[test]
    fn the_pace_counts_from_where_the_call_started() {
        let pace = Pace {
            deadline: None,
            start_done: 10,
            start_bytes: 1000,
            max_count: 2,
            byte_budget: 50,
            stepped: true,
        };
        assert!(!pace.spent(11, 1100));
        assert!(!pace.spent(12, 1049));
        assert!(pace.spent(12, 1050));
    }

    #[test]
    fn shared_images_belong_to_one_profile() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        common.use_profile(5);
        common.by_variant.insert(variant(1), images.add(image_of(4)));
        common.use_profile(5);
        assert_eq!(common.by_variant.len(), 1);
        common.use_profile(6);
        assert!(common.by_variant.is_empty());
    }

    #[test]
    fn a_shared_handle_whose_image_was_freed_is_dropped() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let handle = images.add(image_of(4));
        common.by_variant.insert(variant(1), handle.clone());
        assert_eq!(
            common.get_live(variant(1), &images).map(|h| h.id()),
            Some(handle.id())
        );
        assert!(common.get_live(variant(2), &images).is_none());
        images.remove(&handle);
        assert!(common.get_live(variant(1), &images).is_none());
        assert!(common.by_variant.is_empty());
    }

    #[test]
    fn exact_slots_that_can_be_drawn_with() {
        let catalog = RuntimeMaterialCatalog::default();
        let names = ["$black".to_owned(), "wall".to_owned(), "$white".to_owned()];
        let slots = reachable_exact_image_slots(&catalog, &names);
        assert_eq!(slots, HashSet::from([0, 2]));
        // Nothing marks a slot reachable: all of them are.
        let plain = ["a".to_owned(), "b".to_owned()];
        assert_eq!(
            reachable_exact_image_slots(&catalog, &plain),
            HashSet::from([0, 1])
        );
        assert!(reachable_exact_image_slots(&catalog, &[]).is_empty());
    }

    #[test]
    fn every_slot_is_handed_over_in_one_unpaced_call() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(3));
        assert!(upload.unfinished());
        assert!(run(&mut upload, &mut images, &mut common));
        assert!(!upload.unfinished());
        assert_eq!((upload.done, upload.handed_bytes), (3, 12));
        assert_eq!(images.len(), 3);
        assert!(upload.exact_handles().iter().all(Option::is_some));
    }

    #[test]
    fn a_slot_nothing_draws_with_is_passed_over() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(3));
        upload.reachable_exact = HashSet::from([1]);
        assert!(run(&mut upload, &mut images, &mut common));
        assert_eq!(upload.skipped, 2);
        assert_eq!(upload.done, 3);
        assert_eq!(upload.handed_bytes, 4);
        let made: Vec<bool> = upload.exact_handles().iter().map(Option::is_some).collect();
        assert_eq!(made, [false, true, false]);
        assert_eq!(images.len(), 1);
    }

    #[test]
    fn a_slot_with_no_image_is_done_and_hands_nothing() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(vec![None, Some(Arc::new(image_of(4)))]);
        assert!(run(&mut upload, &mut images, &mut common));
        assert_eq!(upload.done, 2);
        assert!(upload.exact_handles()[0].is_none());
        assert!(upload.exact_handles()[1].is_some());
    }

    #[test]
    fn two_slots_with_one_variant_share_one_asset() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(3));
        upload.exact_variants = vec![Some(variant(1)), Some(variant(1)), Some(variant(2))];
        assert!(run(&mut upload, &mut images, &mut common));
        let handles = upload.exact_handles();
        assert_eq!(
            handles[0].as_ref().map(|h| h.id()),
            handles[1].as_ref().map(|h| h.id())
        );
        assert_ne!(
            handles[0].as_ref().map(|h| h.id()),
            handles[2].as_ref().map(|h| h.id())
        );
        assert_eq!(images.len(), 2);
        assert_eq!((upload.reused_handles, upload.reused_handle_bytes), (1, 4));
        // The reused slot hands nothing over.
        assert_eq!(upload.handed_bytes, 8);
        assert_eq!(upload.reused_common_handles, 0);
    }

    #[test]
    fn a_shared_image_is_handed_over_once_for_every_map() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let shared = |profile| {
            let mut upload = upload_of(arc_images(1));
            upload.exact_variants = vec![Some(variant(7))];
            upload.exact_common = vec![true];
            upload.common_profile_id = profile;
            upload
        };
        let mut first = shared(3);
        assert!(run(&mut first, &mut images, &mut common));
        assert_eq!(images.len(), 1);
        assert_eq!(common.by_variant.len(), 1);

        let mut second = shared(3);
        assert!(run(&mut second, &mut images, &mut common));
        assert_eq!(images.len(), 1);
        assert_eq!(second.handed_bytes, 0);
        assert_eq!((second.reused_common_handles, second.reused_common_bytes), (1, 4));
        assert_eq!(
            first.exact_handles()[0].as_ref().map(|h| h.id()),
            second.exact_handles()[0].as_ref().map(|h| h.id())
        );
    }

    #[test]
    fn a_new_profile_does_not_reuse_the_old_shared_images() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let shared = |profile| {
            let mut upload = upload_of(arc_images(1));
            upload.exact_variants = vec![Some(variant(7))];
            upload.exact_common = vec![true];
            upload.common_profile_id = profile;
            upload
        };
        assert!(run(&mut shared(3), &mut images, &mut common));
        let mut other = shared(4);
        assert!(run(&mut other, &mut images, &mut common));
        assert_eq!(images.len(), 2);
        assert_eq!(other.reused_common_handles, 0);
    }

    #[test]
    fn profile_zero_shares_nothing() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(1));
        upload.exact_variants = vec![Some(variant(7))];
        upload.exact_common = vec![true];
        assert!(run(&mut upload, &mut images, &mut common));
        assert!(common.by_variant.is_empty());
    }

    #[test]
    fn a_paced_call_takes_one_slot_when_the_count_allows_one() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(3));
        assert!(!upload.until(&mut images, &mut common, None, 1, 0));
        assert_eq!(upload.done, 1);
        assert!(!upload.until(&mut images, &mut common, None, 1, 0));
        assert_eq!(upload.done, 2);
        assert!(upload.until(&mut images, &mut common, None, 1, 0));
        assert_eq!(upload.done, 3);
    }

    #[test]
    fn a_paced_call_goes_on_until_the_byte_allowance_is_used_too() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(3));
        // The count is reached after one slot, but 100 bytes are not.
        assert!(upload.until(&mut images, &mut common, None, 1, 100));
        assert_eq!(upload.done, 3);

        let mut upload = upload_of(arc_images(4));
        // Two slots make 8 bytes, which is the allowance.
        assert!(!upload.until(&mut images, &mut common, None, 1, 8));
        assert_eq!((upload.done, upload.handed_bytes), (2, 8));
    }

    #[test]
    fn an_expired_deadline_still_lets_one_slot_through() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(3));
        let past = Instant::now().checked_sub(Duration::from_secs(1));
        assert!(!upload.until(&mut images, &mut common, past, u32::MAX, 0));
        assert_eq!(upload.done, 1);
    }

    #[test]
    fn probes_and_lightmap_pages_follow_the_slots() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(1));
        upload.probes = vec![Some(image_of(16)), None];
        upload.probe_handles = vec![None; 2];
        upload.lightmaps = vec![
            None,
            Some(WorldLightmap {
                primary_image: Some(image_of(10)),
                secondary_image: None,
                secondary_b_image: None,
                ambient_image: image_of(1),
                directional_image: image_of(2),
                sun_mask_image: image_of(4),
                ambient_source_name: String::new(),
                sun_mask_source_name: String::new(),
                ambient_size: UVec2::ONE,
                sun_mask_size: UVec2::ONE,
            }),
        ];
        upload.lightmap_handles = vec![None; 2];
        // One slot, two probes, one page that exists.
        upload.total = 1 + 2 + 1;
        assert!(run(&mut upload, &mut images, &mut common));
        assert_eq!(upload.done, 4);
        assert!(upload.probe_handles()[0].is_some());
        assert!(upload.probe_handles()[1].is_none());
        assert!(upload.lightmap_handles()[0].is_none());
        let page = upload.lightmap_handles()[1].as_ref().expect("page");
        assert!(page.primary.is_some());
        assert!(page.secondary.is_none());
        assert!(page.secondary_b.is_none());
        // 4 exact + 16 probe + (10 + 1 + 2 + 4) page.
        assert_eq!(upload.handed_bytes, 4 + 16 + 17);
        assert_eq!(images.len(), 1 + 1 + 4);
        let (exact, lightmaps, probes) = upload.tess_handles();
        assert_eq!((exact.len(), lightmaps.len(), probes.len()), (1, 2, 2));
    }

    #[test]
    fn the_largest_step_is_the_one_that_is_kept() {
        let mut upload = WorldImageUpload::default();
        upload.note_step(Duration::from_millis(5), 50);
        upload.note_step(Duration::from_millis(2), 999);
        assert_eq!(upload.largest_step_ns, 5_000_000);
        assert_eq!(upload.largest_step_bytes, 50);
        upload.note_step(Duration::from_millis(9), 7);
        assert_eq!((upload.largest_step_ns, upload.largest_step_bytes), (9_000_000, 7));
    }

    #[test]
    fn a_reload_of_the_same_map_hands_nothing_over() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let make = || {
            let mut upload = upload_of(arc_images(2));
            upload.products_id = 9;
            upload.probes = vec![Some(image_of(4))];
            upload.probe_handles = vec![None];
            upload.total = 3;
            upload
        };
        let mut first = make();
        assert!(run(&mut first, &mut images, &mut common));
        assert!(!first.resident);
        assert_eq!(common.map.products_id, 9);
        assert_eq!(images.len(), 3);

        let mut again = make();
        assert!(run(&mut again, &mut images, &mut common));
        assert!(again.resident);
        assert_eq!(again.done, again.total);
        assert_eq!(images.len(), 3);
        assert_eq!(again.handed_bytes, 0);
        assert_eq!(
            again.exact_handles()[1].as_ref().map(|h| h.id()),
            first.exact_handles()[1].as_ref().map(|h| h.id())
        );
        assert!(again.probe_handles()[0].is_some());
    }

    #[test]
    fn a_reload_whose_images_were_freed_hands_them_over_again() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let make = || {
            let mut upload = upload_of(arc_images(2));
            upload.products_id = 11;
            upload
        };
        let mut first = make();
        assert!(run(&mut first, &mut images, &mut common));
        let freed = first.exact_handles()[0].clone().expect("handle");
        images.remove(&freed);

        let mut again = make();
        assert!(run(&mut again, &mut images, &mut common));
        assert!(!again.resident);
        assert_eq!(again.handed_bytes, 8);
        assert!(again.exact_handles().iter().flatten().all(|h| images.get(h.id()).is_some()));
    }

    #[test]
    fn a_different_map_does_not_reuse_the_resident_images() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let make = |products_id| {
            let mut upload = upload_of(arc_images(1));
            upload.products_id = products_id;
            upload
        };
        assert!(run(&mut make(21), &mut images, &mut common));
        let mut other = make(22);
        assert!(run(&mut other, &mut images, &mut common));
        assert!(!other.resident);
        assert_eq!(common.map.products_id, 22);
        assert_eq!(images.len(), 2);
    }

    #[test]
    fn a_map_with_no_products_id_is_not_kept_for_a_reload() {
        let mut images = Assets::<Image>::default();
        let mut common = ResidentGpuImages::default();
        let mut upload = upload_of(arc_images(1));
        assert!(run(&mut upload, &mut images, &mut common));
        assert_eq!(common.map.products_id, 0);
        assert!(common.map.exact.is_empty());
    }
}
