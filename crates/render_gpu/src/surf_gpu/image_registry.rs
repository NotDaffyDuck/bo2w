//! What the GPU has finished uploading of the runtime images (material maps,
//! reflection probes, lightmap pages, the model-lighting volume).
//!
//! The asset layer hands the render world a set of image handles; Bevy uploads
//! the images on its own schedule. The registry answers one question for the
//! draw code: "which texture view may I bind for this slot right now?". Slots
//! whose upload has not landed yet are remembered and retried, and a changed
//! image is looked up through an owner index, so a frame that changes nothing
//! touches nothing.

use std::iter;
use std::sync::atomic::{AtomicU8, Ordering};

use bevy::asset::AssetId;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_asset::{ExtractedAssets, RenderAssets};
use bevy::render::render_resource::{TextureView, TextureViewDimension};
use bevy::render::texture::GpuImage;
use render_material::MaterialEpochId;

use assets::surf_images::{LiveImageHandles, LiveImageHandlesData, LiveLightbakeHandles};

#[derive(Resource, Clone, Debug, Default)]
pub struct ExtractedLiveImageHandles {
    pub handles: LiveImageHandles,
}

impl std::ops::Deref for ExtractedLiveImageHandles {
    type Target = LiveImageHandles;

    fn deref(&self) -> &Self::Target {
        let Self { handles } = self;
        handles
    }
}

/// A texture view together with the dimension it was uploaded as.
#[derive(Clone, Debug)]
pub struct UploadedTextureEye {
    pub tex_view: TextureView,
    pub dimension: TextureViewDimension,
}

/// Why a slot has nothing bindable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadedEyeRejection {
    /// The handle set carries no image for a plane that must have one.
    PreciseImageNotHeld,
    /// The image is known but Bevy has not uploaded it yet.
    UploadQueued,
    /// The image uploaded as another dimension than the slot demands.
    WrongDimension {
        want_dims: TextureViewDimension,
        got_dim: TextureViewDimension,
    },
}

/// The three images a lightmap page is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightmapPlane {
    Primary,
    Backup,
    SecondaryB,
}

impl LightmapPlane {
    const EVERY: [Self; 3] = [Self::Primary, Self::Backup, Self::SecondaryB];

    fn image(self, page: &LiveLightbakeHandles) -> Option<&Handle<Image>> {
        match self {
            Self::Primary => page.primary.as_ref(),
            Self::Backup => page.secondary.as_ref(),
            Self::SecondaryB => page.auxiliary_second.as_ref(),
        }
    }
}

/// One place in the handle set whose view the registry keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Material(u32),
    Probe(u32),
    Lightmap(u32, LightmapPlane),
    ModelLighting,
}

impl Target {
    fn every(handles: &LiveImageHandlesData) -> Vec<Self> {
        let pages = handles.lightmaps.len() as u32;
        (0..handles.shader_textures.len() as u32)
            .map(Self::Material)
            .chain((0..handles.probe_images.len() as u32).map(Self::Probe))
            .chain((0..pages).flat_map(|page| {
                LightmapPlane::EVERY.map(move |plane| Self::Lightmap(page, plane))
            }))
            .chain(iter::once(Self::ModelLighting))
            .collect()
    }

    fn image(self, handles: &LiveImageHandlesData) -> Option<&Handle<Image>> {
        match self {
            Self::Material(slot) => handles.shader_textures.get(slot as usize)?.as_ref(),
            Self::Probe(slot) => handles.probe_images.get(slot as usize)?.as_ref(),
            Self::Lightmap(page, plane) => {
                plane.image(handles.lightmaps.get(page as usize)?.as_ref()?)
            }
            Self::ModelLighting => handles.model_light.as_ref(),
        }
    }

    /// The dimension this slot insists on; material maps take what they are.
    fn demanded_dimension(self) -> Option<TextureViewDimension> {
        match self {
            Self::Material(_) => None,
            Self::Probe(_) => Some(TextureViewDimension::Cube),
            Self::Lightmap(..) => Some(TextureViewDimension::D2),
            Self::ModelLighting => Some(TextureViewDimension::D3),
        }
    }

    fn settle(self, handles: &LiveImageHandlesData, gpu_images: &RenderAssets<GpuImage>) -> Option<Result<UploadedTextureEye, UploadedEyeRejection>> {
        if let Self::Lightmap(page, _) = self {
            // A plane only exists inside a page the handle set has.
            if handles.lightmaps.get(page as usize).is_none_or(Option::is_none) {
                return None;
            }
        }
        let Some(image) = self.image(handles) else {
            let is_plane = matches!(self, Self::Lightmap(..));
            return is_plane.then_some(Err(UploadedEyeRejection::PreciseImageNotHeld));
        };
        let Some(gpu_img) = gpu_images.get(image) else {
            return Some(Err(UploadedEyeRejection::UploadQueued));
        };
        // No view descriptor, or one without a dimension, means a plain 2D upload.
        let uploaded_as = gpu_img.texture_view_descriptor.as_ref().and_then(|view| view.dimension);
        let seen_dim = uploaded_as.unwrap_or(TextureViewDimension::D2);
        if let Some(expected) = self.demanded_dimension().filter(|wanted| *wanted != seen_dim) {
            return Some(Err(UploadedEyeRejection::WrongDimension { want_dims: expected, got_dim: seen_dim }));
        }
        Some(Ok(UploadedTextureEye { dimension: seen_dim, tex_view: gpu_img.texture_view.clone() }))
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct LiveUploadedImageRegistry {
    held: Option<LiveImageHandles>,
    /// Which targets a changed image feeds.
    owners: HashMap<AssetId<Image>, Vec<Target>>,
    /// Targets whose image is known but not uploaded yet.
    waiting: Vec<Target>,
    pub revision_stamp: MaterialEpochId,
    material: Vec<Option<Result<UploadedTextureEye, UploadedEyeRejection>>>,
    shine_probes: Vec<Option<Result<UploadedTextureEye, UploadedEyeRejection>>>,
    /// Three planes per page, page-major.
    lightmaps: Vec<Option<Result<UploadedTextureEye, UploadedEyeRejection>>>,
    model_lighting: Option<Result<UploadedTextureEye, UploadedEyeRejection>>,
}

fn ready(bay: &Option<Result<UploadedTextureEye, UploadedEyeRejection>>) -> Option<&UploadedTextureEye> {
    bay.as_ref()?.as_ref().ok()
}

impl LiveUploadedImageRegistry {
    pub fn material_view(&self, bay: u32) -> Option<&UploadedTextureEye> {
        ready(self.material.get(bay as usize)?)
    }

    /// A plain 2D map, the only kind a flat surface can sample.
    pub fn flat_map_view(&self, bay: u32) -> Option<&TextureView> {
        let uploaded = self.material_view(bay)?;
        (uploaded.dimension == TextureViewDimension::D2).then_some(&uploaded.tex_view)
    }

    pub fn probe_view(&self, bay: u32) -> Option<&UploadedTextureEye> {
        ready(self.shine_probes.get(bay as usize)?)
    }

    pub fn lightmap_view(&self, page: usize, plane: LightmapPlane) -> Option<&TextureView> {
        let index = page * LightmapPlane::EVERY.len() + plane as usize;
        ready(self.lightmaps.get(index)?).map(|uploaded| &uploaded.tex_view)
    }

    pub fn model_lighting_view(&self) -> Option<&UploadedTextureEye> {
        ready(&self.model_lighting)
    }

    fn slot_for(&mut self, which: Target) -> Option<&mut Option<Result<UploadedTextureEye, UploadedEyeRejection>>> {
        let (list, index) = match which {
            Target::ModelLighting => return Some(&mut self.model_lighting),
            Target::Material(slot_ix) => (&mut self.material, slot_ix as usize),
            Target::Probe(slot_no) => (&mut self.shine_probes, slot_no as usize),
            Target::Lightmap(page, plane) => (&mut self.lightmaps, page as usize * LightmapPlane::EVERY.len() + plane as usize),
        };
        list.get_mut(index)
    }

    /// Start over for a new handle set.
    fn adopt_live(&mut self, wanted: &ExtractedLiveImageHandles, gpu_images: &RenderAssets<GpuImage>) {
        let handles: &LiveImageHandlesData = &wanted.handles;
        self.held = Some(wanted.handles.clone());
        self.revision_stamp = wanted.revision_stamp;
        self.material = vec![None; handles.shader_textures.len()];
        self.shine_probes = vec![None; handles.probe_images.len()];
        self.lightmaps = vec![None; handles.lightmaps.len() * LightmapPlane::EVERY.len()];
        self.model_lighting = None;
        self.owners.clear();
        let targets = Target::every(handles);
        for &target in &targets {
            if let Some(image) = target.image(handles) {
                self.owners.entry(image.id()).or_default().push(target);
            }
        }
        self.settle(targets, handles, gpu_images);
    }

    /// Everything that has to be looked at again: what was still waiting plus
    /// whatever the changed images feed.
    fn due(&self, changed: impl Iterator<Item = AssetId<Image>>) -> Vec<Target> {
        let mut due = self.waiting.clone();
        let fed = changed.filter_map(|image| self.owners.get(&image)).flatten().copied();
        fed.for_each(|target| {
            if !due.contains(&target) {
                due.push(target);
            }
        });
        due
    }

    /// Resolve `targets` and keep exactly the unfinished ones for next time.
    fn settle(
        &mut self,
        targets: Vec<Target>,
        handles: &LiveImageHandlesData,
        image_assets: &RenderAssets<GpuImage>,
    ) {
        self.waiting.clear();
        for target in targets {
            if let (Target::Probe(slot), Some(image)) = (target, target.image(handles)) {
                note_probe_upload(image_assets, image, slot);
            }
            let outcome = target.settle(handles, image_assets);
            let unfinished = matches!(outcome, Some(Err(UploadedEyeRejection::UploadQueued)));
            let Some(slot) = self.slot_for(target) else {
                continue;
            };
            *slot = outcome;
            if unfinished {
                self.waiting.push(target);
            }
        }
    }
}

pub(crate) fn refresh_uploaded_images(wanted: Option<Res<ExtractedLiveImageHandles>>, uploads: Res<RenderAssets<GpuImage>>, fresh: Option<Res<ExtractedAssets<GpuImage>>>, mut registry: ResMut<LiveUploadedImageRegistry>) {
    let gpu_images = &*uploads;
    let Some(wanted) = wanted else {
        if registry.held.is_some() {
            *registry = LiveUploadedImageRegistry::default();
        }
        return;
    };
    let same_set = registry
        .held
        .as_ref()
        .is_some_and(|held| held.same_handles(&wanted.handles));
    if !same_set {
        registry.adopt_live(&wanted, gpu_images);
        return;
    }
    // Newly uploaded and re-uploaded images both count as changed.
    let changed = fresh.iter().flat_map(|assets| [&assets.added, &assets.modified]).flatten().copied();
    let due = registry.due(changed);
    if !due.is_empty() {
        registry.settle(due, &wanted.handles, gpu_images);
    }
}

/// Log the first upload of each of the first four reflection probes.
fn note_probe_upload(gpu_images: &RenderAssets<GpuImage>, image: &Handle<Image>, bay: u32) {
    static NOTED: AtomicU8 = AtomicU8::new(0);
    let Some(bit) = 1u8.checked_shl(bay).filter(|bit| *bit < 1 << 4) else {
        return;
    };
    let Some(gpu) = gpu_images.get(image) else {
        return;
    };
    if NOTED.fetch_or(bit, Ordering::Relaxed) & bit != 0 {
        return;
    }
    let texture = &gpu.texture_descriptor;
    let (size, mips, format, make_filled) = (texture.size, texture.mip_level_count, texture.format, gpu.had_data);
    let view = gpu.texture_view_descriptor.as_ref();
    let (view_dim, view_mips) = (view.and_then(|v| v.dimension), view.and_then(|v| v.mip_level_count));
    diag::warn!(
        World,
        "probe cube[{bay}] uploaded {}x{} layers={} mips={} format={:?} had_data={} view_dim={:?} view_mips={:?}",
        size.width, size.height, size.depth_or_array_layers, mips, format, make_filled, view_dim, view_mips,
    );
}
