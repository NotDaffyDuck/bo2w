//! A finished material population and the questions it answers.

use bevy::prelude::Resource;

use crate::{
    AssetNamespace, AssetRef, AssetRefCensus, AuthoredImage, AuthoredMaterial, AuthoredShader,
    AuthoredVertexDecl, ColorMapTransform, MaterialCullFace, MaterialDrawMode,
    MaterialIndex, MaterialKey, MaterialPass, TS_2D, TS_COLOR_MAP, TechniqueSetFacts, TechsetKey,
    TechsetResolve, ZoneOwner, alpha_test_cutoff_from_state_bits, color_pass_agreement,
    cull_face_from_state_bits, material_constant_name,
};

/// What a build finished with: every name resolved and every row final. The
/// build state that produced it is not here, so nothing holding this can go on
/// linking where the importer stopped.
#[derive(Clone, Debug, Default)]
pub struct MaterialDefinitions {
    pub materials: Vec<AuthoredMaterial>,
    pub images: Vec<AuthoredImage>,
    pub shaders: Vec<AuthoredShader>,
    pub vertex_decls: Vec<AuthoredVertexDecl>,
    /// Arguments a pass fed that no loader could place. Always zero: the one
    /// loader that counted them is gone.
    pub leftover_t5_arg_n: u32,
}

/// How many of each kind of name a population holds, real against reference.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetRefDumpCensus {
    pub materials: AssetRefCensus,
    pub images: AssetRefCensus,
    pub shaders: AssetRefCensus,
    pub decls: AssetRefCensus,
}

impl AssetRefDumpCensus {
    pub fn from_catalog(catalog: &MaterialDefinitions) -> Self {
        fn census<'a>(names: impl Iterator<Item = &'a AssetRef>) -> AssetRefCensus {
            let mut census = AssetRefCensus::default();
            names.for_each(|name| census.push(name));
            census
        }
        Self {
            materials: census(catalog.materials.iter().map(|m| &m.name)),
            images: census(catalog.images.iter().map(|i| &i.name)),
            shaders: census(catalog.shaders.iter().map(|s| &s.name)),
            decls: census(catalog.vertex_decls.iter().map(|d| &d.name)),
        }
    }
}

/// The name a row is looked up by, or `None` when it names nothing.
fn lookup_name(name: &str) -> Option<&str> {
    let bare = AssetRef::bare_name(name);
    (!bare.is_empty()).then_some(bare)
}

impl MaterialDefinitions {
    pub fn zone_of(&self, index: usize) -> ZoneOwner {
        self.materials.get(index).map(|m| m.zone).unwrap_or_default()
    }

    pub fn material_index_by_key(&self, key: &MaterialKey) -> Option<MaterialIndex> {
        self.material_index_by_ns(key.namespace, &key.name)
    }

    pub fn material_index_by_ns(&self, namespace: AssetNamespace, name: &str) -> Option<MaterialIndex> {
        let name = lookup_name(name)?;
        self.materials
            .iter()
            .position(|m| m.namespace == namespace && m.name.is_real() && m.name.as_str() == name)
            .map(MaterialIndex::from_order)
    }

    pub fn image_index_by_key(&self, namespace: AssetNamespace, name: &str) -> Option<usize> {
        let name = lookup_name(name)?;
        self.images
            .iter()
            .position(|i| i.namespace == namespace && i.name.is_real() && i.name.as_str() == name)
    }

    /// What the material's colour passes all decode to, when they agree.
    fn agreed_over<T: PartialEq>(
        material: &AuthoredMaterial,
        decode: impl Fn([u32; 2]) -> T,
    ) -> Option<T> {
        color_pass_agreement(
            material.state_bits_entry.as_ref(),
            &material.state_bits,
            material.route.map(|route| route.technique_slots),
            decode,
        )
        .agreed()
    }

    fn agreed_draw_mode(material: &AuthoredMaterial) -> Option<MaterialDrawMode> {
        Self::agreed_over(material, MaterialDrawMode::from_state_bits)
    }

    pub fn agreed_alpha_test_cutoff(&self, material: &AuthoredMaterial) -> Option<Option<f32>> {
        Self::agreed_over(material, alpha_test_cutoff_from_state_bits)
    }

    pub fn cull_face(&self, material: &AuthoredMaterial) -> Option<MaterialCullFace> {
        Self::agreed_over(material, cull_face_from_state_bits)
    }

    fn pass_of(material: &AuthoredMaterial) -> Option<MaterialPass> {
        material.route.map(|route| route.pass)
    }

    pub fn is_sky(&self, material: &AuthoredMaterial) -> bool {
        matches!(Self::pass_of(material), Some(MaterialPass::Sky(_)))
    }

    pub fn is_shadowcaster(&self, material: &AuthoredMaterial) -> bool {
        matches!(Self::pass_of(material), Some(MaterialPass::ShadowOnly))
    }

    /// `None` while the material has no route to say.
    pub fn is_unlit(&self, material: &AuthoredMaterial) -> Option<bool> {
        Some(matches!(
            Self::pass_of(material)?,
            MaterialPass::Unlit | MaterialPass::Sky(_)
        ))
    }

    pub fn constant(material: &AuthoredMaterial, name: &str) -> Option<[f32; 4]> {
        material
            .constants
            .iter()
            .find(|c| material_constant_name(&c.name) == name)
            .map(|c| c.literal)
    }

    /// The four rows the animated-material shader reads: uv animation, then the
    /// falloff parameters, begin colour and end colour when the material has
    /// all three.
    pub fn material_animation(material: &AuthoredMaterial) -> [[f32; 4]; 4] {
        let uv_anim = Self::constant(material, "uvAnimParms").unwrap_or_default();
        let falloff = (
            Self::constant(material, "falloffParms"),
            Self::constant(material, "falloffBegin"),
            Self::constant(material, "falloffEndCo"),
        );
        match falloff {
            (Some(parms), Some(begin), Some(end)) => {
                [uv_anim, parms, [begin[0], begin[1], begin[2], 1.0], end]
            }
            _ => [uv_anim, [0.0; 4], [0.0; 4], [0.0; 4]],
        }
    }

    pub fn color_map_transform(&self, material: &AuthoredMaterial) -> ColorMapTransform {
        match Self::agreed_draw_mode(material) {
            Some(MaterialDrawMode::Blend | MaterialDrawMode::Additive | MaterialDrawMode::Screen) => {
                ColorMapTransform::Unknown
            }
            Some(_) => ColorMapTransform::Square,
            None => match Self::pass_of(material) {
                Some(MaterialPass::Lit | MaterialPass::Unlit | MaterialPass::Sky(_)) => {
                    ColorMapTransform::Square
                }
                Some(MaterialPass::ShadowOnly) | None => ColorMapTransform::Unknown,
            },
        }
    }

    /// The name of the image a HUD material draws: its 2D texture, else its
    /// colour map.
    pub fn hud_image_name(&self, material: &AuthoredMaterial) -> Option<&str> {
        let image_named_for = |semantic: u8| {
            let texture = material
                .textures
                .iter()
                .find(|t| t.semantic == semantic && t.image.is_some())?;
            let name = self.images.get(texture.image?)?.name.as_str();
            (!name.is_empty()).then_some(name)
        };
        image_named_for(TS_2D).or_else(|| image_named_for(TS_COLOR_MAP))
    }

    /// Technique sets a population holds; none do, see `technique_graph`.
    pub fn technique_set_facts(&self) -> &[TechniqueSetFacts] {
        &[]
    }

    pub fn resolve_technique_set(&self, _key: TechsetKey<'_>) -> TechsetResolve<'_> {
        TechsetResolve::Missing
    }

    pub fn leftover_t5_arg_top(&self) -> Option<String> {
        None
    }

    pub fn leftover_t5_arg_ranked(&self, _rank: usize) -> Option<String> {
        None
    }

    pub fn leftover_t5_arg_dest(&self, _dest: u16) -> Option<String> {
        None
    }
}

/// A technique-set name with its `x0`/`x1` feature tokens taken out.
pub fn t5_feature_token_stripped(name: &str) -> String {
    name.replace("x0", "").replace("x1", "")
}
