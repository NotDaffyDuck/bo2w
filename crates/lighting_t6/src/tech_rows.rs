//! Which technique a lit surface draws with.
//!
//! A material's technique set has 36 slots (0..=35). The renderer's own pass
//! keys start after them, and the lit passes take the first run of those:
//! one key per surface family, light column and fog mode.

use crate::stage_light::{LAMP_KIND_PARALLEL, POINT_LAMP, is_cone_lamp};

/// First pass key that is the renderer's own rather than a material slot.
const RENDERER_KEY_FIRST: u8 = 36;

/// The material slot for emissive drawing.
pub const TECHNIQUE_EMISSIVE: u8 = 3;

const fn renderer_key(n: u8) -> u8 {
    RENDERER_KEY_FIRST + n
}

/// Keys of the lit band come first: a base technique equal to this asks for
/// "whichever lit technique fits the surface and its light".
pub const LIT_DRAW_PATH_FIRST: u8 = renderer_key(0);

/// Drawn with nothing: the surface or light has no lit technique.
const TECHNIQUE_UNSET_KEY: u8 = renderer_key(0x29);
pub const TECHNIQUE_LIGHT_SPOT: u8 = renderer_key(0x1c);
pub const TECHNIQUE_LIGHT_OMNI: u8 = renderer_key(0x1d);
pub const TECHNIQUE_LIGHT_SPOT_SHADOW: u8 = renderer_key(0x1e);
pub const TECHNIQUE_SUN_SHADOW_CASTER: u8 = renderer_key(0x1f);
pub const TECHNIQUE_UNLIT: u8 = renderer_key(0x20);
pub const TECHNIQUE_DEBUG: u8 = renderer_key(0x25);

// The lit techniques are laid out per surface family. Inside a family the
// light columns are in this order, and each column has a plain key followed
// by a distance-fog key.
//   0 none, 1 sun, 2 spot, 3 omni, 4 sun shadowed, 5 spot shadowed, 6 omni shadowed
const LIGHT_COLUMNS: usize = 7;
const COLUMN_OFFSET: [u8; LIGHT_COLUMNS] = [0, 2, 6, 10, 4, 8, 12];
const COLUMN_SPOT: usize = 2;
const COLUMN_OMNI: usize = 3;
const COLUMN_SUN_SHADOWED: usize = 4;
const COLUMN_SPOT_SHADOWED: usize = 5;
/// A shadowed light uses the column this many places after its plain one.
const SHADOW_COLUMN_STEP: usize = 3;

/// Where each family's keys start inside the lit band: ordinary surfaces,
/// then instanced ones.
const FAMILY_START: [u8; 2] = [0, 14];
const FAMILY_ORDINARY: u8 = FAMILY_START[0];
const FAMILY_INSTANCED: u8 = FAMILY_START[1];

const fn lit_key(family_start: u8, column: usize, depth_haze: bool) -> u8 {
    renderer_key(family_start + COLUMN_OFFSET[column] + depth_haze as u8)
}

const fn family_row(family_start: u8, depth_haze: bool) -> [u8; LIGHT_COLUMNS] {
    let mut row_no = [0; LIGHT_COLUMNS];
    let mut column = 0;
    while column < LIGHT_COLUMNS {
        row_no[column] = lit_key(family_start, column, depth_haze);
        column += 1;
    }
    row_no
}

/// The keys for the given columns, family by family, each column's plain key
/// then its fog key. `N` must be four per column.
const fn column_slots<const N: usize>(columns: &[usize]) -> [u8; N] {
    assert!(N == 2 * FAMILY_START.len() * columns.len());
    let mut slots = [0; N];
    let mut at = 0;
    let mut family = 0;
    while family < FAMILY_START.len() {
        let mut c = 0;
        while c < columns.len() {
            slots[at] = lit_key(FAMILY_START[family], columns[c], false);
            slots[at + 1] = lit_key(FAMILY_START[family], columns[c], true);
            at += 2;
            c += 1;
        }
        family += 1;
    }
    slots
}

/// One surface family's lit keys, one per light column, without fog.
pub const PLAIN_LIT_ROW: [u8; LIGHT_COLUMNS] = family_row(FAMILY_ORDINARY, false);

/// Keys for unshadowed sun light (no light, then sun), ordinary then
/// instanced surfaces, each with and without fog.
pub const UNSHADOWED_SUN_ROWS: [u8; 8] = column_slots(&[0, 1]);
/// Keys for unshadowed spot and omni lights.
pub const UNSHADOWED_LOCAL_ROWS: [u8; 8] = column_slots(&[COLUMN_SPOT, COLUMN_OMNI]);
/// Keys for the shadowed sun.
pub const SHADOWED_SUN_ROWS: [u8; 4] = column_slots(&[COLUMN_SUN_SHADOWED]);
/// Keys for shadowed spot lights.
pub const SHADOWED_CONE_ROWS: [u8; 4] = column_slots(&[COLUMN_SPOT_SHADOWED]);

/// The extra pass a light adds on top of the lit surface.
pub const fn additional_light_tech_type(light_kind_code: u8, shadow_map_present: bool) -> u8 {
    if is_cone_lamp(light_kind_code) {
        if shadow_map_present {
            TECHNIQUE_LIGHT_SPOT_SHADOW
        } else {
            TECHNIQUE_LIGHT_SPOT
        }
    } else if light_kind_code == POINT_LAMP {
        TECHNIQUE_LIGHT_OMNI
    } else {
        TECHNIQUE_UNSET_KEY
    }
}

/// The light column for a light type. Every spot shape shares one column and
/// a shadow map moves a light three columns along. `None` for a type with no
/// column.
const fn light_column(light_kind_code: u8, shadow_map_present: bool) -> Option<usize> {
    let plain = match light_kind_code {
        0 => 0,
        LAMP_KIND_PARALLEL => 1,
        POINT_LAMP => COLUMN_OMNI,
        t if is_cone_lamp(t) => COLUMN_SPOT,
        _ => return None,
    };
    let column = if shadow_map_present {
        plain + SHADOW_COLUMN_STEP
    } else {
        plain
    };
    if column < LIGHT_COLUMNS {
        Some(column)
    } else {
        None
    }
}

/// Which family a surface type belongs to; `None` for surface types that
/// have no lit technique.
const fn surface_family(surf_type: u8) -> Option<u8> {
    match surf_type {
        3 | 4 => Some(FAMILY_INSTANCED),
        0..=2 | 5..=12 | 15 => Some(FAMILY_ORDINARY),
        _ => None,
    }
}

/// The technique to draw a surface with. Anything other than the lit marker
/// passes through as it is; the marker resolves to the key for the surface
/// type, light and fog mode, or to the none technique when there is none.
pub fn lit_technique_kind(
    first_tree: u8,
    facet_kind: u8,
    light_kind_code: u8,
    depth_haze: bool,
    shadow_map_present: bool,
) -> u8 {
    if first_tree != LIT_DRAW_PATH_FIRST {
        return first_tree;
    }
    let (Some(column), Some(family)) = (
        light_column(light_kind_code, shadow_map_present),
        surface_family(facet_kind),
    ) else {
        return TECHNIQUE_UNSET_KEY;
    };
    lit_key(family, column, depth_haze)
}

/// Whether a technique is one of the lit keys, which the material loader
/// treats as needing the lit colour-space remap.
pub const fn slot_is_lit_substitute(tech: u8) -> bool {
    let mut family = 0;
    while family < FAMILY_START.len() {
        let mut column = 0;
        while column < LIGHT_COLUMNS {
            let plain = lit_key(FAMILY_START[family], column, false);
            if tech == plain || tech == lit_key(FAMILY_START[family], column, true) {
                return true;
            }
            column += 1;
        }
        family += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_light::{
        CONE_LAMP, GFX_LIGHT_TYPE_SPOT_ROUND, GFX_LIGHT_TYPE_SPOT_SQUARE,
    };

    const SPOTS: [u8; 3] = [
        CONE_LAMP,
        GFX_LIGHT_TYPE_SPOT_SQUARE,
        GFX_LIGHT_TYPE_SPOT_ROUND,
    ];

    #[test]
    fn the_lit_band_starts_after_the_material_slots() {
        assert_eq!(LIT_DRAW_PATH_FIRST, 36);
        assert_eq!(PLAIN_LIT_ROW[0], LIT_DRAW_PATH_FIRST);
    }

    #[test]
    fn the_standard_row_lists_each_light_column() {
        assert_eq!(PLAIN_LIT_ROW, [36, 38, 42, 46, 40, 44, 48]);
    }

    #[test]
    fn the_fog_row_sits_one_key_after_the_plain_one() {
        let plain = family_row(FAMILY_ORDINARY, false);
        let fog = family_row(FAMILY_ORDINARY, true);
        for (p, f) in plain.iter().zip(fog) {
            assert_eq!(*p + 1, f);
        }
    }

    #[test]
    fn instanced_keys_follow_the_ordinary_ones() {
        assert_eq!(
            family_row(FAMILY_INSTANCED, false),
            [50, 52, 56, 60, 54, 58, 62]
        );
        assert_eq!(
            family_row(FAMILY_INSTANCED, true),
            [51, 53, 57, 61, 55, 59, 63]
        );
    }

    #[test]
    fn the_slot_lists_hold_both_families_with_and_without_fog() {
        assert_eq!(
            UNSHADOWED_SUN_ROWS,
            [36, 37, 38, 39, 50, 51, 52, 53]
        );
        assert_eq!(
            UNSHADOWED_LOCAL_ROWS,
            [42, 43, 46, 47, 56, 57, 60, 61]
        );
        assert_eq!(SHADOWED_SUN_ROWS, [40, 41, 54, 55]);
        assert_eq!(SHADOWED_CONE_ROWS, [44, 45, 58, 59]);
    }

    #[test]
    fn the_slot_lists_do_not_overlap() {
        let mut all: Vec<u8> = Vec::new();
        all.extend(UNSHADOWED_SUN_ROWS);
        all.extend(UNSHADOWED_LOCAL_ROWS);
        all.extend(SHADOWED_SUN_ROWS);
        all.extend(SHADOWED_CONE_ROWS);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 24);
        // The omni-shadowed column is not part of any list.
        assert!(!all.contains(&48));
    }

    #[test]
    fn every_row_key_is_a_remap_slot_and_nothing_else_in_the_band_is_missed() {
        let rows = [
            family_row(FAMILY_ORDINARY, false),
            family_row(FAMILY_ORDINARY, true),
            family_row(FAMILY_INSTANCED, false),
            family_row(FAMILY_INSTANCED, true),
        ];
        for tech in 0..=u8::MAX {
            let in_a_row = rows.iter().any(|row| row.contains(&tech));
            assert_eq!(slot_is_lit_substitute(tech), in_a_row, "tech {tech}");
        }
    }

    #[test]
    fn the_extra_light_passes_are_not_remap_slots() {
        assert!(!slot_is_lit_substitute(TECHNIQUE_LIGHT_SPOT));
        assert!(!slot_is_lit_substitute(TECHNIQUE_UNLIT));
        assert!(!slot_is_lit_substitute(TECHNIQUE_EMISSIVE));
    }

    #[test]
    fn spots_and_omnis_pick_their_extra_pass() {
        for spot in SPOTS {
            assert_eq!(
                additional_light_tech_type(spot, false),
                TECHNIQUE_LIGHT_SPOT
            );
            assert_eq!(
                additional_light_tech_type(spot, true),
                TECHNIQUE_LIGHT_SPOT_SHADOW
            );
        }
        assert_eq!(
            additional_light_tech_type(POINT_LAMP, false),
            TECHNIQUE_LIGHT_OMNI
        );
        assert_eq!(
            additional_light_tech_type(POINT_LAMP, true),
            TECHNIQUE_LIGHT_OMNI
        );
    }

    #[test]
    fn the_sun_and_unknown_types_add_no_extra_pass() {
        assert_eq!(
            additional_light_tech_type(LAMP_KIND_PARALLEL, false),
            TECHNIQUE_UNSET_KEY
        );
        assert_eq!(additional_light_tech_type(0, true), TECHNIQUE_UNSET_KEY);
        assert_eq!(additional_light_tech_type(6, false), TECHNIQUE_UNSET_KEY);
    }

    #[test]
    fn light_types_land_in_their_columns() {
        assert_eq!(light_column(0, false), Some(0));
        assert_eq!(light_column(LAMP_KIND_PARALLEL, false), Some(1));
        assert_eq!(light_column(POINT_LAMP, false), Some(3));
        for spot in SPOTS {
            assert_eq!(light_column(spot, false), Some(2));
        }
    }

    #[test]
    fn a_shadow_map_moves_a_light_three_columns_on() {
        assert_eq!(light_column(LAMP_KIND_PARALLEL, true), Some(4));
        assert_eq!(light_column(POINT_LAMP, true), Some(6));
        for spot in SPOTS {
            assert_eq!(light_column(spot, true), Some(5));
        }
    }

    #[test]
    fn a_type_with_no_column_has_none() {
        assert_eq!(light_column(6, false), None);
        assert_eq!(light_column(u8::MAX, true), None);
    }

    #[test]
    fn only_the_lit_marker_is_resolved() {
        assert_eq!(
            lit_technique_kind(TECHNIQUE_EMISSIVE, 0, LAMP_KIND_PARALLEL, true, true),
            TECHNIQUE_EMISSIVE
        );
        assert_eq!(
            lit_technique_kind(TECHNIQUE_UNLIT, 99, 77, false, false),
            TECHNIQUE_UNLIT
        );
    }

    #[test]
    fn an_ordinary_surface_in_sun_uses_the_standard_row() {
        let lit = lit_technique_kind(
            LIT_DRAW_PATH_FIRST,
            0,
            LAMP_KIND_PARALLEL,
            false,
            false,
        );
        assert_eq!(lit, PLAIN_LIT_ROW[1]);
        let fogged = lit_technique_kind(
            LIT_DRAW_PATH_FIRST,
            0,
            LAMP_KIND_PARALLEL,
            true,
            false,
        );
        assert_eq!(fogged, PLAIN_LIT_ROW[1] + 1);
    }

    #[test]
    fn an_instanced_surface_uses_the_instanced_row() {
        for surf in [3, 4] {
            let tech = lit_technique_kind(
                LIT_DRAW_PATH_FIRST,
                surf,
                POINT_LAMP,
                false,
                false,
            );
            assert_eq!(tech, 60);
            let shadowed = lit_technique_kind(
                LIT_DRAW_PATH_FIRST,
                surf,
                POINT_LAMP,
                true,
                true,
            );
            assert_eq!(shadowed, 63);
        }
    }

    #[test]
    fn glass_uses_the_ordinary_family() {
        let glass = lit_technique_kind(
            LIT_DRAW_PATH_FIRST,
            15,
            CONE_LAMP,
            false,
            false,
        );
        assert_eq!(glass, PLAIN_LIT_ROW[2]);
    }

    #[test]
    fn surface_types_without_a_lit_technique_draw_nothing() {
        for surf in [13, 14, 16, u8::MAX] {
            let tech = lit_technique_kind(
                LIT_DRAW_PATH_FIRST,
                surf,
                LAMP_KIND_PARALLEL,
                false,
                false,
            );
            assert_eq!(tech, TECHNIQUE_UNSET_KEY, "surface type {surf}");
        }
    }

    #[test]
    fn a_light_without_a_column_draws_nothing() {
        assert_eq!(
            lit_technique_kind(LIT_DRAW_PATH_FIRST, 0, 6, false, false),
            TECHNIQUE_UNSET_KEY
        );
    }

    #[test]
    fn every_surface_family_covers_the_whole_resolver() {
        // Each key the resolver can produce is a remap slot.
        for surf in 0..=u8::MAX {
            for light in 0..=8 {
                for (depth_haze, shadow) in [(false, false), (false, true), (true, false), (true, true)] {
                    let tech = lit_technique_kind(LIT_DRAW_PATH_FIRST, surf, light, depth_haze, shadow);
                    assert!(tech == TECHNIQUE_UNSET_KEY || slot_is_lit_substitute(tech));
                }
            }
        }
    }
}
