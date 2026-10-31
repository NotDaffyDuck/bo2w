//! The Black Ops II front-end content a match carries: HUD fonts and pictures
//! and the UI scripts that use them.
//!
//! These are compared when a prepared match is swapped in, so equality here
//! means "the same content", judged by names and sizes rather than by
//! every byte.

use std::sync::Arc;

use bevy::prelude::{Image, Resource};

/// One bitmap font from `fonts/720/*`: the pixel height it was cut at, and
/// its glyphs. A letter_shape is (letter, [x0, y0 from the pen and baseline,
/// advance, width, height in pixels], [s0, t0, s1, t1 on the sheet]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6HudFont {
    pub name: String,
    pub raster_rows: f32,
    pub glyphs: Vec<(u16, [i16; 5], [f32; 4])>,
}

/// The HUD's English fonts. They share one white sheet whose alpha is the
/// coverage, and each goes by the name the HUD scripts call it
/// (`ui/t6/codbase.lua`: Default is normalFont, Condensed is smallFont, Big is
/// bigFont, Morris is extraBigFont, and so on).
#[derive(Clone, Debug, Default, Resource)]
pub struct T6HudFonts {
    pub sheet: Option<Arc<Image>>,
    pub fonts: Vec<T6HudFont>,
}

impl PartialEq for T6HudFonts {
    fn eq(&self, rhs: &Self) -> bool {
        self.fonts == rhs.fonts && self.sheet.is_some() == rhs.sheet.is_some()
    }
}

/// HUD icons by material name. Each is the colour map as the packs store it,
/// block-compressed the way the GPU reads it.
#[derive(Clone, Debug, Default, Resource)]
pub struct T6HudIcons(pub Vec<(String, Arc<Image>)>);

impl PartialEq for T6HudIcons {
    fn eq(&self, rhs_set: &Self) -> bool {
        self.0.len() == rhs_set.0.len() && self.0.iter().zip(&rhs_set.0).all(|(a, b)| a.0 == b.0)
    }
}

/// The UI scripts (the HavokScript `.lua` rawfiles under `ui/lui`, `ui/t6`
/// and `ui_mp/t6`, in zone load order, a later one winning a name), the
/// English strings they localize and the string tables they look up. Their
/// pictures are in [`T6HudIcons`].
#[derive(Clone, Debug, Default, Resource)]
pub struct T6Ui {
    pub scripts: Vec<(String, Arc<[u8]>)>,
    pub strings: Arc<Vec<(String, String)>>,
    /// String tables as (name, columns, rows, cells row by row).
    pub tables: Arc<Vec<(String, usize, usize, Vec<String>)>>,
    /// The zones' config files (`default_private.cfg` and the like), by name,
    /// for the menus' `exec <file>`.
    pub configs: Arc<Vec<(String, String)>>,
    /// The game settings' defaults (`gametype_setting <name> <value>` in
    /// `zm_gamesettings_default.cfg`, then `_zstandard.cfg`): what Custom
    /// Games shows before he changes anything.
    pub game_settings: Arc<Vec<(String, f32)>>,
}

impl PartialEq for T6Ui {
    fn eq(&self, rhs: &Self) -> bool {
        let same_scripts = self.scripts.len() == rhs.scripts.len()
            && self
                .scripts
                .iter()
                .zip(&rhs.scripts)
                .all(|((a_name, a), (b_name, b))| a_name == b_name && a.len() == b.len());
        same_scripts
            && self.strings.len() == rhs.strings.len()
            && self.tables.len() == rhs.tables.len()
    }
}

/// A Black Ops II full-screen filter (a `generic_filter_*` material; Mob of
/// the Dead's Afterlife is `generic_filter_afterlife_alcatraz`): its pictures
/// by image name, read from the packs, and its constants by name (the zones
/// keep the first 12 letters: `VignetteColo`).
#[derive(Clone, Debug, Default)]
pub struct T6ScreenFilter {
    pub material: String,
    pub tex_list: Vec<(String, Arc<Image>)>,
    pub consts: Vec<(String, [f32; 4])>,
}

impl T6ScreenFilter {
    pub fn constant(&self, name: &str) -> Option<[f32; 4]> {
        self.consts
            .iter()
            .find_map(|(n, v)| (n == name).then_some(*v))
    }

    /// The first picture whose image name holds `part`.
    pub fn image(&self, part: &str) -> Option<&Arc<Image>> {
        self.tex_list
            .iter()
            .find(|(n, _)| n.contains(part))
            .map(|(_, i)| i)
    }
}

/// The map's full-screen filters, by material name.
#[derive(Clone, Debug, Default, Resource)]
pub struct T6ScreenFilters(pub Vec<T6ScreenFilter>);

impl PartialEq for T6ScreenFilters {
    fn eq(&self, rhs: &Self) -> bool {
        self.0.len() == rhs.0.len()
            && self.0.iter().zip(&rhs.0).all(|(a, b)| {
                a.material == b.material
                    && a.consts == b.consts
                    && a.tex_list.len() == b.tex_list.len()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui_with(scripts: &[(&str, usize)]) -> T6Ui {
        T6Ui {
            scripts: scripts
                .iter()
                .map(|(name, len)| ((*name).to_owned(), Arc::from(vec![0u8; *len])))
                .collect(),
            ..T6Ui::default()
        }
    }

    #[test]
    fn ui_content_is_compared_by_script_name_and_size() {
        assert_eq!(ui_with(&[("a.lua", 3)]), ui_with(&[("a.lua", 3)]));
        assert_ne!(ui_with(&[("a.lua", 3)]), ui_with(&[("a.lua", 4)]));
        assert_ne!(ui_with(&[("a.lua", 3)]), ui_with(&[("b.lua", 3)]));
        assert_ne!(ui_with(&[("a.lua", 3)]), ui_with(&[]));
    }

    #[test]
    fn icons_are_compared_by_name_only() {
        let named = |names: &[&str]| {
            T6HudIcons(
                names
                    .iter()
                    .map(|n| ((*n).to_owned(), Arc::new(Image::default())))
                    .collect(),
            )
        };
        assert_eq!(named(&["perk_jugg"]), named(&["perk_jugg"]));
        assert_ne!(named(&["perk_jugg"]), named(&["perk_speed"]));
    }

    #[test]
    fn fonts_compare_by_their_glyphs_and_by_whether_a_sheet_is_loaded() {
        let bare = T6HudFonts::default();
        let sheeted = T6HudFonts {
            sheet: Some(Arc::new(Image::default())),
            ..T6HudFonts::default()
        };
        assert_eq!(bare, T6HudFonts::default());
        assert_ne!(bare, sheeted);
    }
}
