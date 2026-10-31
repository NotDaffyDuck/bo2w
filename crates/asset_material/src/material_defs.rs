//! A finished material population and the questions it answers.

use bevy::prelude::Resource;

use crate::{
    AssetCategoryScope, AssetPointer, AssetPointerTally, HandBuiltTexture, DesignerMaterial, HandBuiltShader,
    VertexDeclRow, TintMapRemap, FaceCullingMode, ShaderDrawStyle,
    SurfaceMaterialId, MaterialKey, DrawPass, TS_2D, SEMANTIC_COLOR, RecipeBundleSummary, RecipeBundleId,
    RecipeBundleLookup, ZoneCustodian, cutout_level_of, judge_color_passes,
    face_culling_of, surface_parameter_label,
};

/// What a build finished with: every name resolved and every row final. The
/// build state that produced it is not here, so nothing holding this can go on
/// linking where the importer stopped.
#[derive(Clone, Debug, Default)]
pub struct MaterialDefinitions {
    pub materials: Vec<DesignerMaterial>,
    pub images: Vec<HandBuiltTexture>,
    pub shaders: Vec<HandBuiltShader>,
    pub vertex_decls: Vec<VertexDeclRow>,
}

/// How many of each kind of name a population holds, real against reference.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetPointerDumpTally {
    pub materials: AssetPointerTally,
    pub images: AssetPointerTally,
    pub shaders: AssetPointerTally,
    pub decls: AssetPointerTally,
}

impl AssetPointerDumpTally {
    pub fn build_from_index(catalog: &MaterialDefinitions) -> Self {
        fn tally<'a>(refs: impl IntoIterator<Item = &'a AssetPointer>) -> AssetPointerTally {
            refs.into_iter().fold(AssetPointerTally::default(), |mut acc, one| {
                acc.push(one);
                acc
            })
        }
        let materials = tally(catalog.materials.iter().map(|row| &row.name));
        let images = tally(catalog.images.iter().map(|row| &row.name));
        let shaders = tally(catalog.shaders.iter().map(|row| &row.name));
        let decls = tally(catalog.vertex_decls.iter().map(|row| &row.name));
        Self {
            materials,
            images,
            shaders,
            decls,
        }
    }
}

/// The name a row is looked up by, or `None` when it names nothing.
fn lookup_name(name: &str) -> Option<&str> {
    match AssetPointer::bare_name(name) {
        "" => None,
        trimmed => Some(trimmed),
    }
}

impl MaterialDefinitions {
    pub fn owning_zone(&self, index: usize) -> ZoneCustodian {
        match self.materials.get(index) {
            Some(row) => row.zone,
            None => ZoneCustodian::default(),
        }
    }

    pub fn material_index_by_key(&self, key: &MaterialKey) -> Option<SurfaceMaterialId> {
        self.material_index_by_ns(key.namespace, &key.name)
    }

    pub fn material_index_by_ns(&self, namespace: AssetCategoryScope, wanted_name: &str) -> Option<SurfaceMaterialId> {
        let wanted = lookup_name(wanted_name)?;
        let slot = self.materials.iter().position(|row| {
            row.name.is_genuine() && row.namespace == namespace && row.name.as_str() == wanted
        })?;
        Some(SurfaceMaterialId::from_sequence(slot))
    }

    pub fn image_index_by_key(&self, namespace: AssetCategoryScope, wanted_name: &str) -> Option<usize> {
        let wanted = lookup_name(wanted_name)?;
        self.images.iter().position(|row| {
            row.name.is_genuine() && row.namespace == namespace && row.name.as_str() == wanted
        })
    }

    /// What the material's colour passes all decode to, when they agree.
    fn agreed_over<T: PartialEq>(
        mat: &DesignerMaterial,
        read_bits: impl Fn([u32; 2]) -> T,
    ) -> Option<T> {
        let slots = mat.route.map(|r| r.technique_mask);
        let table = mat.raster_bits.as_slice();
        let verdict = judge_color_passes(mat.raster_slot.as_ref(), table, slots, read_bits);
        verdict.consensus_reached()
    }

    fn agreed_draw_mode(material: &DesignerMaterial) -> Option<ShaderDrawStyle> {
        Self::agreed_over(material, ShaderDrawStyle::from_state_bits)
    }

    pub fn shared_alpha_cutoff(&self, material: &DesignerMaterial) -> Option<Option<f32>> {
        Self::agreed_over(material, cutout_level_of)
    }

    pub fn face_culling(&self, material: &DesignerMaterial) -> Option<FaceCullingMode> {
        Self::agreed_over(material, face_culling_of)
    }

    fn pass_of(material: &DesignerMaterial) -> Option<DrawPass> {
        let route = material.route?;
        Some(route.draw_pass)
    }

    pub fn casts_shadow_only(&self, material: &DesignerMaterial) -> bool {
        Self::pass_of(material) == Some(DrawPass::DepthShadow)
    }

    /// `None` while the material has no route to say.
    pub fn draws_flat(&self, material: &DesignerMaterial) -> Option<bool> {
        let pass = Self::pass_of(material)?;
        Some(pass == DrawPass::Unlit)
    }

    pub fn constant(material: &DesignerMaterial, param_name: &str) -> Option<[f32; 4]> {
        for entry in &material.constants {
            if surface_parameter_label(&entry.name) == param_name {
                return Some(entry.literal);
            }
        }
        None
    }

    /// The four rows the animated-material shader reads: uv animation, then the
    /// falloff parameters, begin colour and end colour when the material has
    /// all three.
    pub fn material_animation(material: &DesignerMaterial) -> [[f32; 4]; 4] {
        let scroll = Self::constant(material, "uvAnimParms").unwrap_or_default();
        let parms = Self::constant(material, "falloffParms");
        let begin = Self::constant(material, "falloffBegin");
        let end = Self::constant(material, "falloffEndCo");
        let (Some(falloff), Some(open_at), Some(end)) = (parms, begin, end) else {
            return [scroll, [0.0; 4], [0.0; 4], [0.0; 4]];
        };
        let begin_opaque = [open_at[0], open_at[1], open_at[2], 1.0];
        [scroll, falloff, begin_opaque, end]
    }

    pub fn tint_map_remap(&self, material: &DesignerMaterial) -> TintMapRemap {
        let by_pass = || match Self::pass_of(material) {
            Some(DrawPass::DepthShadow) | None => TintMapRemap::Unknown,
            Some(DrawPass::Lit | DrawPass::Unlit) => TintMapRemap::Square,
        };
        let Some(mode) = Self::agreed_draw_mode(material) else {
            return by_pass();
        };
        if matches!(
            mode,
            ShaderDrawStyle::Blend | ShaderDrawStyle::Additive | ShaderDrawStyle::Screen
        ) {
            TintMapRemap::Unknown
        } else {
            TintMapRemap::Square
        }
    }

    /// The name of the image a HUD material draws: its 2D texture, else its
    /// colour map.
    pub fn overlay_picture_name(&self, material: &DesignerMaterial) -> Option<&str> {
        let name_for_semantic = |wanted: u8| -> Option<&str> {
            let mut candidates = material.textures.iter();
            let binding = candidates.find(|b| b.image.is_some() && b.semantic == wanted)?;
            let row = binding.image?;
            let image_name = self.images.get(row)?.name.as_str();
            if image_name.is_empty() { None } else { Some(image_name) }
        };
        name_for_semantic(TS_2D).or_else(|| name_for_semantic(SEMANTIC_COLOR))
    }

    /// Technique sets a population holds; none do, see `technique_graph`.
    pub fn method_set_facts(&self) -> &[RecipeBundleSummary] {
        &[]
    }

    pub fn resolve_method_set(&self, _key: RecipeBundleId<'_>) -> RecipeBundleLookup<'_> {
        RecipeBundleLookup::Missing
    }
}

/// A technique-set name with its `x0`/`x1` feature tokens taken out.
pub fn without_t5_feature_tag(name: &str) -> String {
    let mut out = name.to_owned();
    for token in ["x0", "x1"] {
        out = out.replace(token, "");
    }
    out
}
