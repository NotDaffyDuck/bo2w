//! The images the HUD draws with, decoded on first use and cached by name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use asset_core::AssetCategoryScope;
use assets::CategoryTreeSet;
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use gamemode_t6::HIT_MARKER_MATERIAL;

use crate::gaps::LookupFailure;
use diag::info;

/// The namespace the HUD's own chrome images are looked up in.
pub const CHROME_NAMESPACE: AssetCategoryScope = AssetCategoryScope::T6;

/// Case-insensitive, extension-free key for an image name.
fn normalized_name(name: &str) -> String {
    asset_core::AssetPointer::bare_name(name).to_ascii_lowercase()
}

/// Whether an image's texels are colour (sRGB) or plain data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PictureSampling {
    #[default]
    Color,
    Data,
}

impl PictureSampling {
    fn texture_format(self) -> TextureFormat {
        match self {
            Self::Color => TextureFormat::Rgba8UnormSrgb,
            Self::Data => TextureFormat::Rgba8Unorm,
        }
    }
}

/// What a decoded image is cached under.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PictureKey {
    namespace: AssetCategoryScope,
    name: String,
    sampling: PictureSampling,
    texture_sampling_mode: Option<u8>,
}

/// Decoded RGBA texels with their size.
#[derive(Debug, PartialEq, Eq)]
struct Texels {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

impl Texels {
    fn into_image(self, key: &PictureKey) -> Image {
        let size = Extent3d {
            width: self.width,
            height: self.height,
            ..default()
        };
        let mut image = Image::new(
            size,
            TextureDimension::D2,
            self.bytes,
            key.sampling.texture_format(),
            RenderAssetUsages::default(),
        );
        if let Some(descriptor) = key
            .texture_sampling_mode
            .map(|state| asset_material::sampler_from_state(state, 1, false))
        {
            image.sampler = ImageSampler::Descriptor(descriptor);
        }
        image
    }
}

/// Where lookups can read from: the install, its file trees and the map zone.
#[derive(Default)]
struct Sources {
    root: PathBuf,
    trees: CategoryTreeSet,
    zone_file: Option<PathBuf>,
    zone_namespace: AssetCategoryScope,
}

impl Sources {
    fn announce(&self) {
        let rows = self.trees.summary_rows();
        for line in rows {
            info!(Ui, "hud: {}", line);
        }
    }
}

/// Everything decoded so far (misses included) and whether the first-frame
/// images were requested.
#[derive(Default)]
struct Cache {
    entries: HashMap<PictureKey, Option<Handle<Image>>>,
    warmed: bool,
}

/// Images the first frame will want, as (name, sampling).
const FIRST_FRAME: [(&str, PictureSampling); 1] = [(HIT_MARKER_MATERIAL, PictureSampling::Color)];

#[derive(Resource, Default)]
pub struct HudPictures {
    sources: Sources,
    cache: Cache,
}

impl HudPictures {
    /// Points the lookups at an install; a different root drops everything
    /// cached from the old one.
    pub fn point_at_install(&mut self, install_root: &Path) {
        if self.sources.root.as_path() != install_root {
            let trees = CategoryTreeSet::discover(&asset_transport::GameInstallDir(install_root.to_owned()));
            self.sources = Sources {
                root: install_root.to_owned(),
                trees,
                zone_file: None,
                ..std::mem::take(&mut self.sources)
            };
            self.reset_after_sources_changed();
        }
    }

    /// Adds the loaded map zone's own files to the lookups.
    pub fn add_map_zone(&mut self, zone_archive_file: &Path) {
        if self.sources.zone_file.as_deref() == Some(zone_archive_file) {
            return;
        }
        self.sources.zone_file = Some(zone_archive_file.to_owned());
        let namespace = asset_transport::title_of_zone_path(zone_archive_file)
            .map(AssetCategoryScope::from_zone_game)
            .unwrap_or(AssetCategoryScope::T6);
        let mut grown = self.sources.trees.clone();
        grown.take_zone_root(zone_archive_file);
        let changed = grown != self.sources.trees || namespace != self.sources.zone_namespace;
        if changed {
            self.sources.trees = grown;
            self.sources.zone_namespace = namespace;
            self.reset_after_sources_changed();
        }
    }

    fn reset_after_sources_changed(&mut self) {
        self.cache = Cache::default();
        self.sources.announce();
    }

    pub fn lookup_failure(&self) -> LookupFailure {
        LookupFailure::Undecoded
    }

    pub fn get(
        &mut self,
        namespace: AssetCategoryScope,
        name: &str,
        image_store: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.get_as(namespace, name, PictureSampling::default(), image_store)
    }

    /// Every native read samples as data: there are no zone atlases left whose
    /// colour reads would need the sRGB path.
    pub(crate) fn get_data(
        &mut self,
        namespace: AssetCategoryScope,
        name: &str,
        image_store: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.get_as(namespace, name, PictureSampling::Data, image_store)
    }

    pub fn get_as(
        &mut self,
        namespace: AssetCategoryScope,
        name: &str,
        sampling: PictureSampling,
        image_store: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.get_as_with(namespace, name, sampling, None, image_store)
    }

    pub(crate) fn get_as_with(
        &mut self,
        namespace: AssetCategoryScope,
        name: &str,
        sampling: PictureSampling,
        texture_sampling_mode: Option<u8>,
        image_store: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let key = PictureKey {
            namespace,
            name: normalized_name(name),
            sampling,
            texture_sampling_mode,
        };
        if !self.cache.entries.contains_key(&key) {
            let made = decode_rgba(name).map(|texels| image_store.add(texels.into_image(&key)));
            self.cache.entries.insert(key.clone(), made);
        }
        self.cache.entries[&key].clone()
    }

    /// Loads the images the first frame will want, once per install.
    pub fn warm_first_frame(&mut self, image_store: &mut Assets<Image>) {
        if self.cache.warmed || self.sources.trees.is_empty() {
            return;
        }
        for (name, sampling) in FIRST_FRAME {
            self.get_as(CHROME_NAMESPACE, name, sampling, image_store);
        }
        self.cache.warmed = true;
    }
}

/// The texels for a name. Only the built-in white pixel has a source so far;
/// the game ships no loose image files, and anything the zone holds has no
/// reader here yet.
fn decode_rgba(name: &str) -> Option<Texels> {
    if normalized_name(name) != "white" {
        return None;
    }
    Some(Texels {
        width: 1,
        height: 1,
        bytes: [255; 4].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white_pixel() -> Texels {
        Texels {
            width: 1,
            height: 1,
            bytes: vec![255; 4],
        }
    }

    fn key(name: &str, sampling: PictureSampling, texture_sampling_mode: Option<u8>) -> PictureKey {
        PictureKey {
            namespace: CHROME_NAMESPACE,
            name: name.to_owned(),
            sampling,
            texture_sampling_mode,
        }
    }

    #[test]
    fn names_are_normalised_for_lookup() {
        assert_eq!(normalized_name("White"), "white");
    }

    #[test]
    fn only_the_white_pixel_decodes() {
        assert_eq!(decode_rgba("white"), Some(white_pixel()));
        assert_eq!(decode_rgba("WHITE"), Some(white_pixel()));
        assert_eq!(decode_rgba("compass_map_default"), None);
    }

    #[test]
    fn sampling_picks_the_texture_format() {
        assert_eq!(PictureSampling::Color.texture_format(), TextureFormat::Rgba8UnormSrgb);
        assert_eq!(PictureSampling::Data.texture_format(), TextureFormat::Rgba8Unorm);
        assert_eq!(PictureSampling::default(), PictureSampling::Color);
    }

    #[test]
    fn image_carries_its_size_and_format() {
        let texels = Texels {
            width: 2,
            height: 1,
            bytes: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let image = texels.into_image(&key("x", PictureSampling::Data, None));
        assert_eq!((image.width(), image.height()), (2, 1));
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8Unorm);
    }

    #[test]
    fn lookups_are_cached_including_misses() {
        let mut pictures = HudPictures::default();
        let mut image_store = Assets::<Image>::default();
        let first = pictures.get(CHROME_NAMESPACE, "white", &mut image_store);
        let again = pictures.get(CHROME_NAMESPACE, "WHITE", &mut image_store);
        assert_eq!(first, again);
        assert!(first.is_some());
        assert_eq!(image_store.len(), 1);
        assert!(pictures.get(CHROME_NAMESPACE, "nothing", &mut image_store).is_none());
        assert!(pictures.get(CHROME_NAMESPACE, "nothing", &mut image_store).is_none());
        assert_eq!(pictures.cache.entries.len(), 2);
        assert_eq!(image_store.len(), 1);
    }

    #[test]
    fn sampling_and_sampler_make_separate_entries() {
        let mut pictures = HudPictures::default();
        let mut image_store = Assets::<Image>::default();
        let colour = pictures.get_as(CHROME_NAMESPACE, "white", PictureSampling::Color, &mut image_store);
        let data = pictures.get_data(CHROME_NAMESPACE, "white", &mut image_store);
        assert_ne!(colour, data);
        assert_eq!(image_store.len(), 2);
    }

    #[test]
    fn forgetting_clears_the_cache_and_priming() {
        let mut pictures = HudPictures::default();
        let mut image_store = Assets::<Image>::default();
        pictures.get(CHROME_NAMESPACE, "white", &mut image_store);
        pictures.cache.warmed = true;
        pictures.reset_after_sources_changed();
        assert!(pictures.cache.entries.is_empty());
        assert!(!pictures.cache.warmed);
    }

    #[test]
    fn warming_waits_for_an_install() {
        let mut pictures = HudPictures::default();
        let mut image_store = Assets::<Image>::default();
        pictures.warm_first_frame(&mut image_store);
        assert!(!pictures.cache.warmed);
    }

    #[test]
    fn a_sampler_state_makes_its_own_entry_and_overrides_the_sampler() {
        let mut pictures = HudPictures::default();
        let mut image_store = Assets::<Image>::default();
        let plain = pictures.get_as_with(CHROME_NAMESPACE, "white", PictureSampling::Color, None, &mut image_store);
        let stated = pictures.get_as_with(CHROME_NAMESPACE, "white", PictureSampling::Color, Some(3), &mut image_store);
        assert_ne!(plain, stated);
        let plain_image = image_store.get(plain.as_ref().unwrap()).unwrap();
        let stated_image = image_store.get(stated.as_ref().unwrap()).unwrap();
        assert!(!matches!(plain_image.sampler, ImageSampler::Descriptor(_)));
        assert!(matches!(stated_image.sampler, ImageSampler::Descriptor(_)));
    }

    #[test]
    fn pointing_at_the_same_root_keeps_the_cache() {
        let mut pictures = HudPictures::default();
        let mut image_store = Assets::<Image>::default();
        pictures.get(CHROME_NAMESPACE, "white", &mut image_store);
        pictures.point_at_install(Path::new(""));
        assert_eq!(pictures.cache.entries.len(), 1);
    }

    #[test]
    fn lookup_failure_is_undecoded() {
        assert!(matches!(HudPictures::default().lookup_failure(), LookupFailure::Undecoded));
    }
}
