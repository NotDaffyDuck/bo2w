use bevy::asset::AssetId;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_asset::{ExtractedAssets, RenderAssets};
use bevy::render::render_resource::{TextureView, TextureViewDimension, TextureViewId};
use bevy::render::texture::GpuImage;
use render_material::MaterialGenerationId;

use assets::image_handles::{
    RuntimeImageHandles, RuntimeImageHandlesData, RuntimeLightmapHandles,
};

#[derive(Resource, Clone, Debug, Default)]
pub struct ExtractedRuntimeImageHandles {
    pub handles: RuntimeImageHandles,
}

impl std::ops::Deref for ExtractedRuntimeImageHandles {
    type Target = RuntimeImageHandles;

    fn deref(&self) -> &Self::Target {
        &self.handles
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeUploadedLightmapViews {
    pub primary: Result<TextureView, UploadedViewRefusal>,
    pub secondary: Result<TextureView, UploadedViewRefusal>,
    pub secondary_b: Result<TextureView, UploadedViewRefusal>,
}

#[derive(Clone, Debug)]
pub struct UploadedTextureView {
    pub view: TextureView,
    pub dimension: TextureViewDimension,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct RuntimeUploadedImageRegistry {
    source_handles: Option<RuntimeImageHandles>,
    slot_index: UploadedSlotIndex,
    pending: UploadedPending,
    pub generation_id: MaterialGenerationId,

    views_revision: u64,

    replaced_revision: u64,
    pub material_images: Vec<Option<Result<UploadedTextureView, UploadedViewRefusal>>>,
    pub reflection_probes: Vec<Option<Result<TextureView, UploadedViewRefusal>>>,
    pub lightmaps: Vec<Option<RuntimeUploadedLightmapViews>>,
    pub model_lighting: Option<Result<TextureView, UploadedViewRefusal>>,

    pub resolved_post_sun: Option<Result<TextureView, UploadedViewRefusal>>,
    pub float_z: Option<Result<TextureView, UploadedViewRefusal>>,

    pub sun_shadow: Option<Result<TextureView, UploadedViewRefusal>>,

    pub spot_shadow: Option<Result<TextureView, UploadedViewRefusal>>,

    pub ui3d: Option<Result<TextureView, UploadedViewRefusal>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum ViewChange {
    None,

    Published,

    Replaced,
}

impl ViewChange {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Replaced, _) | (_, Self::Replaced) => Self::Replaced,
            (Self::Published, _) | (_, Self::Published) => Self::Published,
            _ => Self::None,
        }
    }

    fn of_view(before: Option<TextureViewId>, after: Option<TextureViewId>) -> Self {
        match (before, after) {
            (a, b) if a == b => Self::None,
            (Some(_), Some(_)) => Self::Replaced,
            _ => Self::Published,
        }
    }
}

impl RuntimeUploadedImageRegistry {

    fn note(&mut self, change: ViewChange) {
        match change {
            ViewChange::None => {}
            ViewChange::Published => {
                self.views_revision = self.views_revision.wrapping_add(1);
            }
            ViewChange::Replaced => {
                self.views_revision = self.views_revision.wrapping_add(1);
                self.replaced_revision = self.replaced_revision.wrapping_add(1);
            }
        }
    }

}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadedViewRefusal {
    ExactImageNotRetained,
    StaleGeneration {
        retained: MaterialGenerationId,
        current: MaterialGenerationId,
    },
    UploadPending,
    DimensionMismatch {
        expected: TextureViewDimension,
        actual: TextureViewDimension,
    },
}

#[derive(Clone, Debug, Default)]
struct UploadedSlotIndex {
    material: HashMap<AssetId<Image>, Vec<u32>>,
    probes: HashMap<AssetId<Image>, Vec<u32>>,
    lightmaps: HashMap<AssetId<Image>, Vec<(u32, u8)>>,
    model_lighting: Option<AssetId<Image>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UploadedPending {
    material: Vec<u32>,
    probes: Vec<u32>,
    lightmaps: Vec<(u32, u8)>,
    model_lighting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UploadedRefresh {
    None,
    Slots(UploadedRefreshSlots),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UploadedRefreshSlots {
    material: Vec<u32>,
    probes: Vec<u32>,
    lightmaps: Vec<(u32, u8)>,
    model_lighting: bool,
}

pub(crate) fn prepare_uploaded_image_registry(
    handles: Option<Res<ExtractedRuntimeImageHandles>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    extracted: Option<Res<ExtractedAssets<GpuImage>>>,
    mut registry: ResMut<RuntimeUploadedImageRegistry>,
) {
    let Some(handles) = handles else {
        if registry.source_handles.is_none() {
            return;
        }
        let (views, replaced) = (registry.views_revision, registry.replaced_revision);
        *registry = RuntimeUploadedImageRegistry::default();
        registry.views_revision = views.wrapping_add(1);
        registry.replaced_revision = replaced.wrapping_add(1);
        return;
    };
    let source_unchanged = registry
        .source_handles
        .as_ref()
        .is_some_and(|retained| retained.ptr_eq(&handles.handles));
    if !source_unchanged {
        registry.source_handles = Some(handles.handles.clone());
        registry.slot_index = UploadedSlotIndex::from_handles(&handles.handles);
        registry.generation_id = handles.generation_id;
        let change = refresh_all_uploaded_slots(&mut registry, &handles, &gpu_images);
        registry.note(change);
        registry.pending = uploaded_pending_from_registry(&registry);
        return;
    }

    let plan = match extracted.as_ref() {
        Some(extracted) => uploaded_refresh_plan(
            extracted
                .added
                .iter()
                .copied()
                .chain(extracted.modified.iter().copied()),
            &registry.pending,
            &registry.slot_index,
        ),
        None => uploaded_refresh_plan(core::iter::empty(), &registry.pending, &registry.slot_index),
    };
    match plan {
        UploadedRefresh::None => {}
        UploadedRefresh::Slots(slots) => {
            let change = refresh_uploaded_slots(&mut registry, &handles, &gpu_images, &slots);
            registry.note(change);
            retarget_uploaded_pending(&mut registry, &slots);
        }
    }
}

impl UploadedSlotIndex {
    fn from_handles(handles: &RuntimeImageHandlesData) -> Self {
        let mut index = Self::default();
        for (slot, handle) in handles.material_images.iter().enumerate() {
            if let Some(handle) = handle {
                index
                    .material
                    .entry(handle.id())
                    .or_default()
                    .push(slot as u32);
            }
        }
        for (slot, handle) in handles.reflection_probes.iter().enumerate() {
            if let Some(handle) = handle {
                index
                    .probes
                    .entry(handle.id())
                    .or_default()
                    .push(slot as u32);
            }
        }
        for (slot, page) in handles.lightmaps.iter().enumerate() {
            let Some(page) = page else {
                continue;
            };
            let slot = slot as u32;
            for (plane, handle) in [
                (0u8, page.primary.as_ref()),
                (1, page.secondary.as_ref()),
                (2, page.secondary_b.as_ref()),
            ] {
                if let Some(handle) = handle {
                    index
                        .lightmaps
                        .entry(handle.id())
                        .or_default()
                        .push((slot, plane));
                }
            }
        }
        index.model_lighting = handles.model_lighting.as_ref().map(Handle::id);
        index
    }
}

fn uploaded_refresh_plan(
    changed: impl IntoIterator<Item = AssetId<Image>>,
    pending: &UploadedPending,
    index: &UploadedSlotIndex,
) -> UploadedRefresh {
    let mut slots = UploadedRefreshSlots {
        material: pending.material.clone(),
        probes: pending.probes.clone(),
        lightmaps: pending.lightmaps.clone(),
        model_lighting: pending.model_lighting,
    };
    for id in changed {
        if let Some(owned) = index.material.get(&id) {
            for &slot in owned {
                push_unique_u32(&mut slots.material, slot);
            }
        }
        if let Some(owned) = index.probes.get(&id) {
            for &slot in owned {
                push_unique_u32(&mut slots.probes, slot);
            }
        }
        if let Some(owned) = index.lightmaps.get(&id) {
            for &key in owned {
                push_unique_lightmap(&mut slots.lightmaps, key);
            }
        }
        if index.model_lighting == Some(id) {
            slots.model_lighting = true;
        }
    }
    if slots.material.is_empty()
        && slots.probes.is_empty()
        && slots.lightmaps.is_empty()
        && !slots.model_lighting
    {
        UploadedRefresh::None
    } else {
        UploadedRefresh::Slots(slots)
    }
}

fn push_unique_u32(out: &mut Vec<u32>, slot: u32) {
    if !out.contains(&slot) {
        out.push(slot);
    }
}

fn push_unique_lightmap(out: &mut Vec<(u32, u8)>, key: (u32, u8)) {
    if !out.contains(&key) {
        out.push(key);
    }
}

fn refresh_all_uploaded_slots(
    registry: &mut RuntimeUploadedImageRegistry,
    handles: &RuntimeImageHandlesData,
    gpu_images: &RenderAssets<GpuImage>,
) -> ViewChange {
    let mut change = if registry.material_images.len() != handles.material_images.len()
        || registry.reflection_probes.len() != handles.reflection_probes.len()
        || registry.lightmaps.len() != handles.lightmaps.len()
    {
        ViewChange::Published
    } else {
        ViewChange::None
    };
    registry
        .material_images
        .resize_with(handles.material_images.len(), || None);
    for (slot, handle) in registry
        .material_images
        .iter_mut()
        .zip(&handles.material_images)
    {
        change = change.merge(refresh_native_view(slot, handle.as_ref(), gpu_images));
    }
    registry
        .reflection_probes
        .resize_with(handles.reflection_probes.len(), || None);
    for (index, (slot, handle)) in registry
        .reflection_probes
        .iter_mut()
        .zip(&handles.reflection_probes)
        .enumerate()
    {
        if let Some(handle) = handle {
            log_uploaded_probe_once(gpu_images, handle, index);
        }
        change = change.merge(refresh_view(
            slot,
            handle.as_ref(),
            gpu_images,
            TextureViewDimension::Cube,
        ));
    }
    registry
        .lightmaps
        .resize_with(handles.lightmaps.len(), || None);
    for (slot, page) in registry.lightmaps.iter_mut().zip(&handles.lightmaps) {
        change = change.merge(refresh_lightmap_page(slot, page.as_ref(), gpu_images));
    }
    change.merge(refresh_view(
        &mut registry.model_lighting,
        handles.model_lighting.as_ref(),
        gpu_images,
        TextureViewDimension::D3,
    ))
}

fn refresh_uploaded_slots(
    registry: &mut RuntimeUploadedImageRegistry,
    handles: &RuntimeImageHandlesData,
    gpu_images: &RenderAssets<GpuImage>,
    slots: &UploadedRefreshSlots,
) -> ViewChange {
    let mut change = ViewChange::None;
    for &slot in &slots.material {
        let Some(dst) = registry.material_images.get_mut(slot as usize) else {
            continue;
        };
        change = change.merge(refresh_native_view(
            dst,
            handles
                .material_images
                .get(slot as usize)
                .and_then(Option::as_ref),
            gpu_images,
        ));
    }
    for &slot in &slots.probes {
        let Some(dst) = registry.reflection_probes.get_mut(slot as usize) else {
            continue;
        };
        let handle = handles
            .reflection_probes
            .get(slot as usize)
            .and_then(Option::as_ref);
        if let Some(handle) = handle {
            log_uploaded_probe_once(gpu_images, handle, slot as usize);
        }
        change = change.merge(refresh_view(
            dst,
            handle,
            gpu_images,
            TextureViewDimension::Cube,
        ));
    }
    for &(slot, plane) in &slots.lightmaps {
        let Some(page) = handles.lightmaps.get(slot as usize) else {
            continue;
        };
        let Some(page) = page else {
            if let Some(dst) = registry.lightmaps.get_mut(slot as usize) {
                change = change.merge(refresh_lightmap_page(dst, None, gpu_images));
            }
            continue;
        };
        if registry
            .lightmaps
            .get(slot as usize)
            .is_some_and(Option::is_none)
        {
            change = change.merge(ViewChange::Published);
        }
        let Some(views) = registry.lightmaps.get_mut(slot as usize).map(|slot| {
            slot.get_or_insert_with(|| RuntimeUploadedLightmapViews {
                primary: Err(UploadedViewRefusal::ExactImageNotRetained),
                secondary: Err(UploadedViewRefusal::ExactImageNotRetained),
                secondary_b: Err(UploadedViewRefusal::ExactImageNotRetained),
            })
        }) else {
            continue;
        };
        change = change.merge(match plane {
            0 => refresh_required_view(
                &mut views.primary,
                page.primary.as_ref(),
                gpu_images,
                TextureViewDimension::D2,
            ),
            1 => refresh_required_view(
                &mut views.secondary,
                page.secondary.as_ref(),
                gpu_images,
                TextureViewDimension::D2,
            ),
            _ => refresh_required_view(
                &mut views.secondary_b,
                page.secondary_b.as_ref(),
                gpu_images,
                TextureViewDimension::D2,
            ),
        });
    }
    if slots.model_lighting {
        change = change.merge(refresh_view(
            &mut registry.model_lighting,
            handles.model_lighting.as_ref(),
            gpu_images,
            TextureViewDimension::D3,
        ));
    }
    change
}

fn refresh_lightmap_page(
    slot: &mut Option<RuntimeUploadedLightmapViews>,
    page: Option<&RuntimeLightmapHandles>,
    gpu_images: &RenderAssets<GpuImage>,
) -> ViewChange {
    let Some(page) = page else {
        return match slot.take() {
            Some(_) => ViewChange::Published,
            None => ViewChange::None,
        };
    };
    let mut change = if slot.is_none() {
        ViewChange::Published
    } else {
        ViewChange::None
    };
    let views = slot.get_or_insert_with(|| RuntimeUploadedLightmapViews {
        primary: Err(UploadedViewRefusal::ExactImageNotRetained),
        secondary: Err(UploadedViewRefusal::ExactImageNotRetained),
        secondary_b: Err(UploadedViewRefusal::ExactImageNotRetained),
    });
    change = change.merge(refresh_required_view(
        &mut views.primary,
        page.primary.as_ref(),
        gpu_images,
        TextureViewDimension::D2,
    ));
    change = change.merge(refresh_required_view(
        &mut views.secondary,
        page.secondary.as_ref(),
        gpu_images,
        TextureViewDimension::D2,
    ));
    change.merge(refresh_required_view(
        &mut views.secondary_b,
        page.secondary_b.as_ref(),
        gpu_images,
        TextureViewDimension::D2,
    ))
}

fn uploaded_pending_from_registry(registry: &RuntimeUploadedImageRegistry) -> UploadedPending {
    let mut pending = UploadedPending::default();
    for (slot, view) in registry.material_images.iter().enumerate() {
        if material_slot_pending(view) {
            pending.material.push(slot as u32);
        }
    }
    for (slot, view) in registry.reflection_probes.iter().enumerate() {
        if probe_slot_pending(view) {
            pending.probes.push(slot as u32);
        }
    }
    for (slot, page) in registry.lightmaps.iter().enumerate() {
        let Some(page) = page else {
            continue;
        };
        if matches!(page.primary, Err(UploadedViewRefusal::UploadPending)) {
            pending.lightmaps.push((slot as u32, 0));
        }
        if matches!(page.secondary, Err(UploadedViewRefusal::UploadPending)) {
            pending.lightmaps.push((slot as u32, 1));
        }
        if matches!(page.secondary_b, Err(UploadedViewRefusal::UploadPending)) {
            pending.lightmaps.push((slot as u32, 2));
        }
    }
    pending.model_lighting = matches!(
        registry.model_lighting,
        Some(Err(UploadedViewRefusal::UploadPending))
    );
    pending
}

fn retarget_uploaded_pending(
    registry: &mut RuntimeUploadedImageRegistry,
    slots: &UploadedRefreshSlots,
) {
    registry.pending.material.retain(|&slot| {
        material_slot_pending(registry.material_images.get(slot as usize).unwrap_or(&None))
    });
    registry.pending.probes.retain(|&slot| {
        probe_slot_pending(
            registry
                .reflection_probes
                .get(slot as usize)
                .unwrap_or(&None),
        )
    });
    registry.pending.lightmaps.retain(|&(slot, plane)| {
        lightmap_plane_pending(
            registry
                .lightmaps
                .get(slot as usize)
                .and_then(|page| page.as_ref()),
            plane,
        )
    });
    registry.pending.model_lighting = matches!(
        registry.model_lighting,
        Some(Err(UploadedViewRefusal::UploadPending))
    );
    for &slot in &slots.material {
        if material_slot_pending(registry.material_images.get(slot as usize).unwrap_or(&None)) {
            push_unique_u32(&mut registry.pending.material, slot);
        }
    }
    for &slot in &slots.probes {
        if probe_slot_pending(
            registry
                .reflection_probes
                .get(slot as usize)
                .unwrap_or(&None),
        ) {
            push_unique_u32(&mut registry.pending.probes, slot);
        }
    }
    for &key in &slots.lightmaps {
        if lightmap_plane_pending(
            registry
                .lightmaps
                .get(key.0 as usize)
                .and_then(|page| page.as_ref()),
            key.1,
        ) {
            push_unique_lightmap(&mut registry.pending.lightmaps, key);
        }
    }
    if slots.model_lighting {
        registry.pending.model_lighting = matches!(
            registry.model_lighting,
            Some(Err(UploadedViewRefusal::UploadPending))
        );
    }
}

fn material_slot_pending(slot: &Option<Result<UploadedTextureView, UploadedViewRefusal>>) -> bool {
    matches!(slot, Some(Err(UploadedViewRefusal::UploadPending)))
}

fn probe_slot_pending(slot: &Option<Result<TextureView, UploadedViewRefusal>>) -> bool {
    matches!(slot, Some(Err(UploadedViewRefusal::UploadPending)))
}

fn lightmap_plane_pending(page: Option<&RuntimeUploadedLightmapViews>, plane: u8) -> bool {
    let Some(page) = page else {
        return false;
    };
    let slot = match plane {
        0 => &page.primary,
        1 => &page.secondary,
        _ => &page.secondary_b,
    };
    matches!(slot, Err(UploadedViewRefusal::UploadPending))
}

fn refresh_native_view(
    slot: &mut Option<Result<UploadedTextureView, UploadedViewRefusal>>,
    handle: Option<&Handle<Image>>,
    gpu_images: &RenderAssets<GpuImage>,
) -> ViewChange {
    let bound = |slot: &Option<Result<UploadedTextureView, UploadedViewRefusal>>| match slot {
        Some(Ok(current)) => Some((current.view.id(), current.dimension)),
        _ => None,
    };
    let before = bound(slot);
    let Some(handle) = handle else {
        *slot = None;
        return ViewChange::of_view(before.map(|(id, _)| id), None);
    };
    let Some(gpu) = gpu_images.get(handle) else {
        if before.is_none() && matches!(slot, Some(Err(UploadedViewRefusal::UploadPending))) {
            return ViewChange::None;
        }
        *slot = Some(Err(UploadedViewRefusal::UploadPending));
        return ViewChange::of_view(before.map(|(id, _)| id), None);
    };
    let dimension = gpu
        .texture_view_descriptor
        .as_ref()
        .and_then(|descriptor| descriptor.dimension)
        .unwrap_or(TextureViewDimension::D2);
    if before == Some((gpu.texture_view.id(), dimension)) {
        return ViewChange::None;
    }
    *slot = Some(Ok(UploadedTextureView {
        view: gpu.texture_view.clone(),
        dimension,
    }));
    ViewChange::of_view(before.map(|(id, _)| id), Some(gpu.texture_view.id()))
}

fn refresh_view(
    slot: &mut Option<Result<TextureView, UploadedViewRefusal>>,
    handle: Option<&Handle<Image>>,
    gpu_images: &RenderAssets<GpuImage>,
    expected: TextureViewDimension,
) -> ViewChange {
    let Some(handle) = handle else {
        let before = match slot.as_ref() {
            Some(Ok(view)) => Some(view.id()),
            _ => None,
        };
        let existed = slot.take().is_some();
        return match (before, existed) {
            (Some(_), _) => ViewChange::Replaced,
            (None, true) => ViewChange::Published,
            (None, false) => ViewChange::None,
        };
    };
    let fresh = slot.is_none();
    let change = refresh_required_view(
        slot.get_or_insert(Err(UploadedViewRefusal::UploadPending)),
        Some(handle),
        gpu_images,
        expected,
    );
    if fresh {
        change.merge(ViewChange::Published)
    } else {
        change
    }
}

fn refresh_required_view(
    slot: &mut Result<TextureView, UploadedViewRefusal>,
    handle: Option<&Handle<Image>>,
    gpu_images: &RenderAssets<GpuImage>,
    expected: TextureViewDimension,
) -> ViewChange {
    let refuse = |slot: &mut Result<TextureView, UploadedViewRefusal>, next| match slot.as_ref() {
        Err(current) if *current == next => ViewChange::None,
        _ => {
            *slot = Err(next);
            ViewChange::Published
        }
    };
    let Some(handle) = handle else {
        return refuse(slot, UploadedViewRefusal::ExactImageNotRetained);
    };
    let Some(gpu) = gpu_images.get(handle) else {
        return refuse(slot, UploadedViewRefusal::UploadPending);
    };
    let actual = gpu
        .texture_view_descriptor
        .as_ref()
        .and_then(|descriptor| descriptor.dimension)
        .unwrap_or(TextureViewDimension::D2);
    if actual != expected {
        return refuse(
            slot,
            UploadedViewRefusal::DimensionMismatch { expected, actual },
        );
    }
    let before = slot.as_ref().ok().map(TextureView::id);
    if before == Some(gpu.texture_view.id()) {
        return ViewChange::None;
    }
    *slot = Ok(gpu.texture_view.clone());
    ViewChange::of_view(before, Some(gpu.texture_view.id()))
}

fn log_uploaded_probe_once(
    gpu_images: &RenderAssets<GpuImage>,
    handle: &Handle<Image>,
    index: usize,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static LOGGED: AtomicUsize = AtomicUsize::new(0);
    if index >= 4 || LOGGED.fetch_max(index + 1, Ordering::Relaxed) > index {
        return;
    }
    let Some(gpu) = gpu_images.get(handle) else {
        return;
    };
    diag::warn!(
        World,
        "probe cube[{index}] uploaded {}x{} layers={} mips={} format={:?} had_data={} view_dim={:?} view_mips={:?}",
        gpu.texture_descriptor.size.width,
        gpu.texture_descriptor.size.height,
        gpu.texture_descriptor.size.depth_or_array_layers,
        gpu.texture_descriptor.mip_level_count,
        gpu.texture_descriptor.format,
        gpu.had_data,
        gpu.texture_view_descriptor
            .as_ref()
            .and_then(|descriptor| descriptor.dimension),
        gpu.texture_view_descriptor
            .as_ref()
            .and_then(|descriptor| descriptor.mip_level_count),
    );
}
