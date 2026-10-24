//! Where a model's material slots stand against the material definitions.

use asset_core::{AssetEdge, AssetEdgeReason, MaterialKey, MaterialSpace, WalkLocalMaterialIndex};
use asset_material::MaterialDefinitions;

/// A slot is in use when the model's own data points at a material for it, or
/// a material name has been put there.
fn slot_in_use(
    authored: &[Option<WalkLocalMaterialIndex>],
    keys: &[Option<MaterialKey>],
    slot: usize,
) -> bool {
    let pointed_at = authored.get(slot).is_some_and(Option::is_some);
    let named = keys
        .get(slot)
        .and_then(Option::as_ref)
        .is_some_and(|key| !key.name.is_empty());
    pointed_at || named
}

/// The material key of each slot the model points at, read from the
/// definitions it was walked with, and an edge for each slot: absent when the
/// slot is unused, otherwise not yet resolved. Slots stay unnamed when there
/// are no definitions to read.
pub fn capture_xmodel_material_slots(
    authored: &[Option<WalkLocalMaterialIndex>],
    materials: Option<&MaterialDefinitions>,
) -> (Vec<Option<MaterialKey>>, Vec<AssetEdge<MaterialSpace>>) {
    let keys: Vec<Option<MaterialKey>> = authored
        .iter()
        .map(|slot| {
            let material = materials?.materials.get(slot.as_ref()?.get())?;
            Some(MaterialKey {
                namespace: material.namespace,
                name: material.name.to_string(),
            })
        })
        .collect();
    let edges = (0..authored.len())
        .map(|slot| {
            if slot_in_use(authored, &keys, slot) {
                AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss)
            } else {
                AssetEdge::Absent
            }
        })
        .collect();
    (keys, edges)
}

/// Finds the definition row of every slot's key. A slot that is unused gets
/// no edge; a used one whose key matches no real definition stays unresolved.
pub fn stamp_xmodel_material_edges(
    keys: &[Option<MaterialKey>],
    authored: &[Option<WalkLocalMaterialIndex>],
    materials: &MaterialDefinitions,
) -> Vec<AssetEdge<MaterialSpace>> {
    (0..keys.len().max(authored.len()))
        .map(|slot| {
            if !slot_in_use(authored, keys, slot) {
                return AssetEdge::Absent;
            }
            let row = keys
                .get(slot)
                .and_then(Option::as_ref)
                .and_then(|key| materials.material_index_by_key(key));
            match row {
                Some(row) => AssetEdge::bind(row, materials.zone_of(row.order())),
                None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_definitions_slots_are_unnamed_and_unresolved() {
        let authored = [Some(WalkLocalMaterialIndex::from_walk(0)), None];
        let (keys, edges) = capture_xmodel_material_slots(&authored, None);
        assert!(keys.iter().all(Option::is_none));
        assert_eq!(edges.len(), 2);
        assert!(matches!(edges[0], AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss)));
        assert!(matches!(edges[1], AssetEdge::Absent));
    }

    #[test]
    fn a_slot_that_nothing_names_or_points_at_has_no_edge() {
        let edges = stamp_xmodel_material_edges(&[None, None], &[None], &MaterialDefinitions::default());
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|edge| matches!(edge, AssetEdge::Absent)));
    }
}
