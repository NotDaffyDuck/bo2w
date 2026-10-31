//! Building a material population: rows come in from each zone, and a name
//! that two zones both define ends up as one row.

use crate::{
    AssetCategoryScope, AssetPointer, HandBuiltTexture, DesignerMaterial, MaterialDefinitions,
};

/// A population while it is still being assembled. `publish` hands the finished
/// rows on and drops the rest, so nothing downstream can keep linking.
#[derive(Clone, Debug, Default)]
pub struct SurfaceShaderRoster {
    defs: MaterialDefinitions,
    snapshot_nanos: AssetCategoryScope,
}

impl std::ops::Deref for SurfaceShaderRoster {
    type Target = MaterialDefinitions;

    fn deref(&self) -> &Self::Target {
        &self.defs
    }
}

impl std::ops::DerefMut for SurfaceShaderRoster {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.defs
    }
}

fn push_row<T>(rows: &mut Vec<T>, row: T) -> usize {
    rows.extend([row]);
    rows.len() - 1
}

impl SurfaceShaderRoster {
    pub fn mark_images_common_owned(&mut self) {
        self.images.iter_mut().for_each(|image| image.common_owned = true);
    }

    /// Which game the rows now being linked belong to.
    pub fn choose_scope(&mut self, ns: AssetCategoryScope) {
        let _previous = std::mem::replace(&mut self.snapshot_nanos, ns);
    }

    pub fn snapshot_nanos(&self) -> AssetCategoryScope {
        self.snapshot_nanos
    }

    /// Adds an image, or folds it into the row already holding its name.
    /// Returns the row either way.
    ///
    /// A real name is never replaced by a reference to it. A body is never
    /// replaced by an empty one. Decoded pixels carry over to whichever row
    /// survives. Two real images of one name that read in different colour
    /// spaces stay two rows.
    pub fn add_texture(&mut self, incoming: HandBuiltTexture) -> usize {
        let held = self.images.iter().position(|row| {
            row.namespace == incoming.namespace && row.name.spells_same(&incoming.name)
        });
        let Some(at) = held else {
            return push_row(&mut self.images, incoming);
        };
        let row = &mut self.images[at];
        let colour_spaces_differ = row.name.is_genuine()
            && incoming.name.is_genuine()
            && row.srgb_sampled != incoming.srgb_sampled;
        if colour_spaces_differ {
            return push_row(&mut self.images, incoming);
        }
        let loses_body = incoming.payload.is_empty() && !row.payload.is_empty();
        if AssetPointer::incoming_owns_slot(&row.name, &incoming.name) && !loses_body {
            let mut replacement = incoming;
            replacement.decoded = replacement.decoded.or_else(|| row.decoded.take());
            *row = replacement;
        } else if row.decoded.is_none() && incoming.decoded.is_some() {
            row.decoded = incoming.decoded;
            row.common_owned = incoming.common_owned;
        }
        at
    }

    /// Adds a material, or lets it take over the row already holding its name
    /// when it has the better claim. Returns the row either way.
    pub fn add_material(&mut self, arriving: DesignerMaterial) -> usize {
        let held = self.materials.iter().position(|row| {
            row.namespace == arriving.namespace && row.name.spells_same(&arriving.name)
        });
        let Some(at) = held else {
            return push_row(&mut self.materials, arriving);
        };
        if AssetPointer::incoming_owns_slot(&self.materials[at].name, &arriving.name) {
            self.materials[at] = arriving;
        }
        at
    }

    /// Ends the build. Material rows that only referred to a name some other
    /// zone defines are dropped; the result says where each provisional row
    /// went, `None` for the dropped ones.
    pub fn publish(mut self) -> (MaterialDefinitions, Vec<Option<usize>>) {
        let provisional = std::mem::take(&mut self.defs.materials);
        let mut remap = Vec::with_capacity(provisional.len());
        for material in provisional {
            // A kept row lands at the next free place; a reference is dropped.
            if material.name.is_genuine() {
                remap.push(Some(self.defs.materials.len()));
                self.defs.materials.push(material);
            } else {
                remap.push(None);
            }
        }
        (self.defs, remap)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ZoneCustodian;

    fn d<T: Default>() -> T {
        T::default()
    }

    fn material(name: &str, sort_key: u8) -> DesignerMaterial {
        DesignerMaterial {
            name: AssetPointer::decode(name),
            namespace: d(),
            method_set: d(),
            draw_order: sort_key,
            game_flag_info: d(),
            condition_bits: d(),
            camera_zone: d(),
            facet_type_bits: d(),
            raster_bits: d(),
            raster_slot: d(),
            method_table: d(),
            route: d(),
            textures: d(),
            constants: d(),
            zone: ZoneCustodian::TEST,
            t6_draw: d(),
        }
    }

    fn image(name: &str, payload: &[u8]) -> HandBuiltTexture {
        HandBuiltTexture {
            namespace: d(),
            name: AssetPointer::decode(name),
            texture_kind: d(),
            semantic: d(),
            category: d(),
            srgb_sampled: d(),
            width: 1,
            height: 1,
            depth: 1,
            tier_total: 1,
            format: d(),
            payload: Arc::new(payload.to_vec()),
            decoded: d(),
            common_owned: d(),
            decoded_variant: d(),
        }
    }

    #[test]
    fn publish_drops_references_and_remaps_the_rest() {
        let mut catalog = SurfaceShaderRoster::default();
        let rows = [("a", 1), (",b", 2), ("c", 3), (",d", 4), ("e", 5)];
        for (name, key) in rows {
            catalog.materials.push(material(name, key));
        }
        let (defs, remap) = catalog.publish();
        assert_eq!(remap, [Some(0), None, Some(1), None, Some(2)]);
        let kept: Vec<_> = defs.materials.iter().map(|m| m.draw_order).collect();
        assert_eq!(kept, [1, 3, 5]);
    }

    #[test]
    fn linking_a_material_by_name_keeps_the_better_claim() {
        let mut catalog = SurfaceShaderRoster::default();
        assert_eq!(catalog.add_material(material(",m", 1)), 0);
        assert_eq!(catalog.add_material(material("m", 2)), 0);
        assert_eq!(catalog.add_material(material(",m", 3)), 0);
        assert_eq!(catalog.add_material(material("n", 4)), 1);
        let keys: Vec<_> = catalog.materials.iter().map(|m| m.draw_order).collect();
        assert_eq!(keys, [2, 4]);
    }

    #[test]
    fn linking_an_image_never_loses_a_body_to_an_empty_one() {
        let mut catalog = SurfaceShaderRoster::default();
        assert_eq!(catalog.add_texture(image("i", &[1, 2])), 0);
        assert_eq!(catalog.add_texture(image("i", &[])), 0);
        assert_eq!(catalog.add_texture(image("j", &[9])), 1);
        assert_eq!(catalog.images[0].payload.as_slice(), [1, 2]);
        assert_eq!(catalog.images.len(), 2);
    }

    #[test]
    fn marking_images_common_owned_marks_every_row() {
        let mut catalog = SurfaceShaderRoster::default();
        catalog.add_texture(image("i", &[1]));
        catalog.add_texture(image("j", &[2]));
        catalog.mark_images_common_owned();
        assert!(catalog.images.iter().all(|row| row.common_owned));
    }
}
