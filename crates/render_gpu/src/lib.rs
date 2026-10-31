pub mod gpu_diag;
pub mod surf_gpu;
mod plugin;

pub use surf_gpu::{
    // Frame data and what the extract publishes.
    ENGINE_TEXTURE_FLOATZ, ENGINE_TEXTURE_RESOLVED_POST_DAYLIGHT, ENGINE_TEXTURE_SHADOW_MAP_CONE,
    ENGINE_TEXTURE_SHADOW_MAP_DAYLIGHT, ExtractedInspectGeometry, ExtractedPostEffect,
    ExtractedRenderFrameOutputs, ExtractedLiveImageHandles, StaticGeometryExtract,
    ExtractedT6Dynamic, InstalledRenderWorld, PublishedRenderFrame, RenderFrameData,
    RenderWorldData, T6DeferredFill, T6DynamicDraw, T6SkinShape, T6Skinned, T6StaticDraw,
    // Post effects and guest views.
    FocusBlur, BokehFrame, BLOOM_APPLY_MATERIAL, BLOOM_SETUP_MATERIAL, BloomFrame, GuestViewFrame,
    GuestViewTexture,
    // Images, samplers and the shared texture tables.
    CodeSamplerChannel, LiveImageHandles, LiveImageHandlesData, LiveLightbakeHandles,
    LiveTextureBinding, LiveUploadedImageRegistry,
    SamplerOrigin, SamplerTexShape, FacetSamplerInputs, FLAT_TEXTURE_TABLE_SLOTS,
    VOLUME_TEXTURE_TABLE_SIZE, CUBEMAP_TEXTURE_TABLE_SIZE, SAMPLER_TABLE_SIZE,
    MethodType, UploadedTextureEye, UploadedEyeRejection,
    // Shadow maps and the sun's casters.
    SHADOW_MAP_CONE_COLOR_FORMAT, SPOT_SHADOW_DEPTH_FMT, SHADOWMAP_SPOT_LABEL,
    SHADOW_MAP_DAYLIGHT_COLOR_FORMAT, SUN_SHADOW_DEPTH_FMT, SHADOWMAP_SUN_LABEL,
    SUN_CASTER_TECH, SUNLIGHT_CASCADE_TOTAL,
    // Buffers, uploads and the pass layout contract.
    SubmitReadyGpu, LENS_EYE_FORMAT, ModelLightingTileUpload, ModelLightingTileUploads,
    HeldDrawItem, HeldDrawKind, PropCachePatchWriteReject, PropCacheGpu, SEND_GRANULE,
    GpuBindSlotDescriptor, GpuBindResourceKind, WgpuLayoutRejection, GpuPassBindingPlan, GpuStageVisibility,
    GpuVertexField, GpuVertexStreamLayout, GpuVertexElementKind, plan_gpu_pass_bindings,
    geometry_inspect_enabled, gpu_types, padded_len, table_bind_entries,
    write_padded, write_range_into,
};
pub use plugin::DrawGpuPlugin;

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Every `embedded://render_gpu/...` shader path must name a file under
    /// `src/`, or Bevy never loads the shader and its pipelines wait forever
    /// (a folder rename once hung every map on the loading screen).
    #[test]
    fn embedded_shader_paths_name_real_files() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src.clone()];
        // Split so this file does not match its own needle.
        let needle = concat!("\"embedded://", "render_gpu/");
        let mut checked = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for piece in text.split(needle).skip(1) {
                    let rel = &piece[..piece.find('"').unwrap()];
                    assert!(src.join(rel).is_file(), "{}: no shader file src/{rel}", path.display());
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }
}
