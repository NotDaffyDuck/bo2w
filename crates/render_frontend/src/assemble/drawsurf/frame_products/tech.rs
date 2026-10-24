//! Which technique each row of a product list is drawn with.
//!
//! A list is not always drawn with one technique: the lit colour list asks
//! the lit-method table per row, the dynamic-light list derives it from the
//! light a row was made for, and every other list uses one technique for all
//! rows. [`TechRule`] names those three cases so the list builders do not
//! pass the same five loose parameters around.

use render_material::{MaterialDrawKey, RuntimeMaterialCatalog};

use super::super::lit_draw_method::colour_lit_technique;
use super::super::material_runtime::resolve_material_technique;
use super::super::retained::RetainedDrawItem;
use super::super::setup::TechType;

/// Bit 0 of a technique's flags. A row whose technique carries it belongs to
/// the distortion pass, so the colour passes leave it out once distortion is
/// switched off.
const TECHNIQUE_FLAG_DISTORTION_ONLY: u16 = 1;

#[derive(Clone, Copy)]
pub(super) enum TechRule<'a> {
    /// Every row uses this technique.
    Single(TechType),
    /// The lit colour list: `base` refined per row by the lit-method table.
    Lit {
        base: TechType,
        light_types: &'a [u8],
        dfog: bool,
        sun_shadow_map: bool,
        spot_shadowed: &'a [u8],
    },
    /// The dynamic-light list: the technique follows the light the row was
    /// made for (its scene light index) and whether that light casts a spot
    /// shadow.
    DynamicLight {
        light_types: &'a [u8],
        spot_shadowed: &'a [u8],
    },
}

impl TechRule<'_> {
    pub(super) fn tech_for(&self, row: &RetainedDrawItem) -> TechType {
        match *self {
            Self::Single(tech) => tech,
            Self::Lit {
                base,
                light_types,
                dfog,
                sun_shadow_map,
                spot_shadowed,
            } => colour_lit_technique(base, row.key, light_types, dfog, sun_shadow_map, spot_shadowed),
            Self::DynamicLight {
                light_types,
                spot_shadowed,
            } => {
                let light = dpvs_t6::GfxDrawSurf { packed: row.key }.primary_light_index();
                let light_type = light_types.get(usize::from(light)).copied().unwrap_or(0);
                TechType(lighting_t6::additional_light_tech_type(
                    light_type,
                    spot_shadowed.contains(&light),
                ))
            }
        }
    }
}

/// The key a row is looked up under in the material catalog.
pub(super) fn material_key(row: &RetainedDrawItem) -> MaterialDrawKey {
    MaterialDrawKey::new(row.key, row.material_rank).with_material_id(row.material_id)
}

/// Whether the colour pass keeps `row` when the distortion pass is off.
pub(super) fn kept_without_distortion(
    catalog: &RuntimeMaterialCatalog,
    rule: &TechRule<'_>,
    row: &RetainedDrawItem,
) -> bool {
    let tech = rule.tech_for(row);
    let distortion_only = resolve_material_technique(catalog, material_key(row), tech)
        .is_ok_and(|(_, technique)| technique.flags & TECHNIQUE_FLAG_DISTORTION_ONLY != 0);
    !distortion_only
}

#[cfg(test)]
mod tests {
    use super::super::lanes::tests::row;
    use super::*;

    #[test]
    fn a_single_rule_ignores_the_row() {
        let rule = TechRule::Single(TechType(9));
        assert_eq!(rule.tech_for(&row(1, 1)), TechType(9));
        assert_eq!(rule.tech_for(&row(u64::MAX, 7)), TechType(9));
    }

    #[test]
    fn the_light_rule_follows_the_light_of_the_row() {
        let light_types = [0u8, 1, 2];
        let rule = TechRule::DynamicLight {
            light_types: &light_types,
            spot_shadowed: &[],
        };
        for light in 0..4u8 {
            let key = super::super::super::with_scene_light_index(0, light);
            let mut r = row(key, 1);
            r.key = key;
            let expected = TechType(lighting_t6::additional_light_tech_type(
                light_types.get(usize::from(light)).copied().unwrap_or(0),
                false,
            ));
            assert_eq!(rule.tech_for(&r), expected, "light {light}");
        }
    }

    #[test]
    fn a_shadowed_light_picks_its_own_technique() {
        let light_types = [2u8];
        let key = super::super::super::with_scene_light_index(0, 0);
        let mut r = row(key, 1);
        r.key = key;
        let plain = TechRule::DynamicLight {
            light_types: &light_types,
            spot_shadowed: &[],
        };
        let shadowed = TechRule::DynamicLight {
            light_types: &light_types,
            spot_shadowed: &[0],
        };
        assert_eq!(
            shadowed.tech_for(&r),
            TechType(lighting_t6::additional_light_tech_type(2, true))
        );
        assert_eq!(
            plain.tech_for(&r),
            TechType(lighting_t6::additional_light_tech_type(2, false))
        );
    }

    #[test]
    fn an_unknown_light_counts_as_type_zero() {
        let key = super::super::super::with_scene_light_index(0, 5);
        let mut r = row(key, 1);
        r.key = key;
        let rule = TechRule::DynamicLight {
            light_types: &[],
            spot_shadowed: &[],
        };
        assert_eq!(
            rule.tech_for(&r),
            TechType(lighting_t6::additional_light_tech_type(0, false))
        );
    }
}
