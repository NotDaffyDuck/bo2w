//! The images the HUD draws with, decoded on first use and cached by name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use asset_core::AssetNamespace;
use assets::NamespaceTrees;
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use gamemode_t6::DAMAGE_FEEDBACK_SHADER;

use crate::gaps::ImageMiss;

/// The namespace the HUD's own chrome images are looked up in.
pub const HUD_CHROME_NAMESPACE: AssetNamespace = AssetNamespace::T6;

/// Case-insensitive, extension-free key for an image name.
fn cache_key(name: &str) -> String {
    asset_core::AssetRef::bare_name(name).to_ascii_lowercase()
}

/// Whether an image's texels are colour (sRGB) or plain data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HudSampling {
    #[default]
    Color,
    Data,
}

impl HudSampling {
    fn texture_format(self) -> TextureFormat {
        match self {
            Self::Color => TextureFormat::Rgba8UnormSrgb,
            Self::Data => TextureFormat::Rgba8Unorm,
        }
    }
}

/// What a decoded image is cached under.
type ImageKey = (AssetNamespace, String, HudSampling, Option<u8>);

/// Width, height and RGBA bytes.
type Rgba = (u32, u32, Vec<u8>);

fn image_from_rgba(
    (width, height, texels): Rgba,
    sampling: HudSampling,
    sampler_state: Option<u8>,
) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texels,
        sampling.texture_format(),
        RenderAssetUsages::default(),
    );
    if let Some(state) = sampler_state {
        image.sampler =
            ImageSampler::Descriptor(asset_material::sampler_from_state(state, 1, false));
    }
    image
}

#[derive(Resource, Default)]
pub struct HudImages {
    games_root: PathBuf,
    trees: NamespaceTrees,
    adopted_zone: Option<PathBuf>,
    map_namespace: AssetNamespace,
    cache: HashMap<ImageKey, Option<Handle<Image>>>,
    warmed: bool,
}

impl HudImages {
    /// Points the lookups at an install; a different root drops everything
    /// cached from the old one.
    pub fn set_games_root(&mut self, root: &Path) {
        if self.games_root == root {
            return;
        }
        self.games_root = root.to_path_buf();
        self.trees = NamespaceTrees::discover(&asset_transport::GamesRoot(self.games_root.clone()));
        self.adopted_zone = None;
        self.reset_cache();
        self.log_trees();
    }

    /// Adds the loaded map zone's own files to the lookups.
    pub fn adopt_map_zone(&mut self, zone_ff: &Path) {
        if self.adopted_zone.as_deref() == Some(zone_ff) {
            return;
        }
        self.adopted_zone = Some(zone_ff.to_path_buf());
        let namespace = asset_transport::zone_game_for_path(zone_ff)
            .map_or(AssetNamespace::T6, AssetNamespace::from_zone_game);
        let mut trees = self.trees.clone();
        trees.adopt_zone(zone_ff);
        if trees == self.trees && namespace == self.map_namespace {
            return;
        }
        self.trees = trees;
        self.map_namespace = namespace;
        self.reset_cache();
        self.log_trees();
    }

    fn reset_cache(&mut self) {
        self.cache.clear();
        self.warmed = false;
    }

    fn log_trees(&self) {
        for line in self.trees.report_lines() {
            diag::info!(Ui, "hud: {line}");
        }
    }

    pub fn miss_reason(&self) -> ImageMiss {
        ImageMiss::NotDecoded
    }

    pub fn get(
        &mut self,
        namespace: AssetNamespace,
        name: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.get_sampled(namespace, name, HudSampling::Color, images)
    }

    /// Every native read samples as data: there are no zone atlases left whose
    /// colour reads would need the sRGB path.
    pub(crate) fn get_native(
        &mut self,
        namespace: AssetNamespace,
        name: &str,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.get_sampled(namespace, name, HudSampling::Data, images)
    }

    pub fn get_sampled(
        &mut self,
        namespace: AssetNamespace,
        name: &str,
        sampling: HudSampling,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.get_sampled_with_sampler(namespace, name, sampling, None, images)
    }

    pub(crate) fn get_sampled_with_sampler(
        &mut self,
        namespace: AssetNamespace,
        name: &str,
        sampling: HudSampling,
        sampler_state: Option<u8>,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = (namespace, cache_key(name), sampling, sampler_state);
        if let Some(known) = self.cache.get(&key) {
            return known.clone();
        }
        let handle = decode_rgba(name)
            .map(|rgba| images.add(image_from_rgba(rgba, sampling, sampler_state)));
        self.cache.insert(key, handle.clone());
        handle
    }

    /// Loads the images the first frame will want, once per install.
    pub fn warm_present_stems(&mut self, images: &mut Assets<Image>) {
        if self.warmed || self.trees.is_empty() {
            return;
        }
        let _ = self.get_sampled_with_sampler(
            HUD_CHROME_NAMESPACE,
            DAMAGE_FEEDBACK_SHADER,
            HudSampling::Color,
            None,
            images,
        );
        self.warmed = true;
    }
}

/// The texels for a name. Only the built-in white pixel has a source so far;
/// the game ships no loose image files, and anything the zone holds has no
/// reader here yet.
fn decode_rgba(name: &str) -> Option<Rgba> {
    (cache_key(name) == "white").then(|| (1, 1, vec![255; 4]))
}
