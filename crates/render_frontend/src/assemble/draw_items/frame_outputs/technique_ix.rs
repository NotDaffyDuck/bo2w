//! Which technique each row of a product list is drawn with.
//!
//! A list is not always drawn with one technique: the lit colour list asks
//! the lit-method table per row, the dynamic-light list derives it from the
//! light a row was made for, and every other list uses one technique for all
//! rows. [`TechRule`] names those three cases so the list builders do not
//! pass the same five loose parameters around.

use render_material::{SurfaceSortKeyPair, LiveMaterialLedger};

use super::super::lighting_draw_method::colour_lit_method;
use super::super::material_live::select_material_technique;
use super::super::kept_lanes::HeldDrawItem;
use super::super::material_setup::MethodType;

/// Bit 0 of a technique's flags. A row whose technique carries it belongs to
/// the distortion pass, so the colour passes leave it out once distortion is
/// switched off.
const TECHNIQUE_FLAG_DISTORTION_ONLY: u16 = 1;

#[derive(Clone, Copy)]
pub(super) enum TechRule<'a> {
    /// Every row uses this technique.
    Single(MethodType),
    /// The lit colour list: `base` refined per row by the lit-method table.
    Lit {
        base: MethodType,
        light_types: &'a [u8],
        depth_haze: bool,
        daylight_shadow_map: bool,
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
    pub(super) fn tech_for(&self, row: &HeldDrawItem) -> MethodType {
        match *self {
            Self::Single(tech) => tech,
            Self::Lit {
                base,
                light_types: lamp_types,
                depth_haze,
                daylight_shadow_map: sun_map,
                spot_shadowed: spot_has_shadow,
            } => colour_lit_method(base, row.key, lamp_types, depth_haze, sun_map, spot_has_shadow),
            Self::DynamicLight {
                light_types: light_kinds,
                spot_shadowed: shadowed_spots,
            } => {
                let light = dpvs_t6::SortedDraw { packed: row.key }.key_light_index();
                let lamp_type = light_kinds.get(usize::from(light)).copied().unwrap_or(0);
                MethodType(lighting_t6::additional_light_tech_type(
                    lamp_type,
                    shadowed_spots.contains(&light),
                ))
            }
        }
    }
}

/// The key a row is looked up under in the material catalog.
pub(super) fn material_key(row: &HeldDrawItem) -> SurfaceSortKeyPair {
    SurfaceSortKeyPair::new(row.key, row.material_order_no).with_material_id(row.material_key)
}

/// Whether the colour pass keeps `row` when the distortion pass is off.
pub(super) fn kept_without_distortion(
    catalog: &LiveMaterialLedger,
    rule: &TechRule<'_>,
    row: &HeldDrawItem,
) -> bool {
    let tech = rule.tech_for(row);
    let distortion_only = select_material_technique(catalog, material_key(row), tech)
        .is_ok_and(|(_slot, chosen)| chosen.flags & TECHNIQUE_FLAG_DISTORTION_ONLY != 0);
    !distortion_only
}

#[cfg(test)]
mod tests {
    use super::super::lane_set::tests::row;
    use super::*;

    #[test]
    fn a_single_rule_ignores_the_row() {
        let rule = TechRule::Single(MethodType(9));
        assert_eq!(rule.tech_for(&row(1, 1)), MethodType(9));
        assert_eq!(rule.tech_for(&row(u64::MAX, 7)), MethodType(9));
    }

    #[test]
    fn the_light_rule_follows_the_light_of_the_row() {
        let light_types = [0u8, 1, 2];
        let rule = TechRule::DynamicLight {
            light_types: &light_types,
            spot_shadowed: &[],
        };
        for light in 0..4u8 {
            let key = super::super::super::scene_light_index_of(0, light);
            let mut r = row(key, 1);
            r.key = key;
            let expected = MethodType(lighting_t6::additional_light_tech_type(
                light_types.get(usize::from(light)).copied().unwrap_or(0),
                false,
            ));
            assert_eq!(rule.tech_for(&r), expected, "light {light}");
        }
    }

    #[test]
    fn a_shadowed_light_picks_its_own_technique() {
        let light_types = [2u8];
        let key = super::super::super::scene_light_index_of(0, 0);
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
            MethodType(lighting_t6::additional_light_tech_type(2, true))
        );
        assert_eq!(
            plain.tech_for(&r),
            MethodType(lighting_t6::additional_light_tech_type(2, false))
        );
    }

    #[test]
    fn an_unknown_light_counts_as_type_zero() {
        let key = super::super::super::scene_light_index_of(0, 5);
        let mut r = row(key, 1);
        r.key = key;
        let rule = TechRule::DynamicLight {
            light_types: &[],
            spot_shadowed: &[],
        };
        assert_eq!(
            rule.tech_for(&r),
            MethodType(lighting_t6::additional_light_tech_type(0, false))
        );
    }
}
