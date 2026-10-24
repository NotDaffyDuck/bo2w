//! Building a material population: rows come in from each zone, and a name
//! that two zones both define ends up as one row.

use crate::{
    AssetNamespace, AssetRef, AuthoredImage, AuthoredMaterial, MaterialDefinitions,
};

/// A population while it is still being assembled. `publish` hands the finished
/// rows on and drops the rest, so nothing downstream can keep linking.
#[derive(Clone, Debug, Default)]
pub struct MaterialCatalog {
    defs: MaterialDefinitions,
    capture_ns: AssetNamespace,
}

impl std::ops::Deref for MaterialCatalog {
    type Target = MaterialDefinitions;

    fn deref(&self) -> &Self::Target {
        &self.defs
    }
}

impl std::ops::DerefMut for MaterialCatalog {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.defs
    }
}

fn push_row<T>(rows: &mut Vec<T>, row: T) -> usize {
    rows.push(row);
    rows.len() - 1
}

impl MaterialCatalog {
    pub fn mark_images_common_owned(&mut self) {
        self.images.iter_mut().for_each(|image| image.common_owned = true);
    }

    /// Which game the rows now being linked belong to.
    pub fn set_capture_ns(&mut self, ns: AssetNamespace) {
        self.capture_ns = ns;
    }

    pub fn capture_ns(&self) -> AssetNamespace {
        self.capture_ns
    }

    /// Adds an image, or folds it into the row already holding its name.
    /// Returns the row either way.
    ///
    /// A real name is never replaced by a reference to it. A body is never
    /// replaced by an empty one. Decoded pixels carry over to whichever row
    /// survives. Two real images of one name that read in different colour
    /// spaces stay two rows.
    pub fn link_image(&mut self, incoming: AuthoredImage) -> usize {
        let held = self.images.iter().position(|row| {
            row.namespace == incoming.namespace && row.name.same_name(&incoming.name)
        });
        let Some(at) = held else {
            return push_row(&mut self.images, incoming);
        };
        let row = &mut self.images[at];
        let colour_spaces_differ = row.name.is_real()
            && incoming.name.is_real()
            && row.use_srgb_reads != incoming.use_srgb_reads;
        if colour_spaces_differ {
            return push_row(&mut self.images, incoming);
        }
        let loses_body = incoming.payload.is_empty() && !row.payload.is_empty();
        if AssetRef::incoming_owns_slot(&row.name, &incoming.name) && !loses_body {
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
    pub fn link_material(&mut self, incoming: AuthoredMaterial) -> usize {
        let held = self.materials.iter().position(|row| {
            row.namespace == incoming.namespace && row.name.same_name(&incoming.name)
        });
        let Some(at) = held else {
            return push_row(&mut self.materials, incoming);
        };
        if AssetRef::incoming_owns_slot(&self.materials[at].name, &incoming.name) {
            self.materials[at] = incoming;
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
            if material.name.is_reference() {
                remap.push(None);
            } else {
                remap.push(Some(self.defs.materials.len()));
                self.defs.materials.push(material);
            }
        }
        (self.defs, remap)
    }
}
