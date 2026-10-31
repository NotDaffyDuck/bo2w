//! How a model's material slots line up with the loaded material definitions.

use asset_core::AssetDependencyLink;
use asset_core::DependencyCause;
use asset_core::MaterialKey;
use asset_core::SurfaceShaderSpace;
use asset_core::TraversalMaterialSlot;
use asset_material::MaterialDefinitions;

/// True when the model's data points the slot at a material, or a non-empty
/// material name has been put in it.
fn slot_is_used(pointer: Option<&Option<TraversalMaterialSlot>>, key: Option<&Option<MaterialKey>>) -> bool {
    if pointer.is_some_and(Option::is_some) {
        return true;
    }
    matches!(key, Some(Some(named)) if !named.name.is_empty())
}

/// An unused slot links to nothing; a used one links to what `lookup` finds,
/// or stays unbound when it finds nothing.
fn link_for(used: bool, lookup: impl FnOnce() -> Option<AssetDependencyLink<SurfaceShaderSpace>>) -> AssetDependencyLink<SurfaceShaderSpace> {
    match used.then(lookup) {
        None => AssetDependencyLink::NothingThere,
        Some(found) => found.unwrap_or(AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure)),
    }
}

/// Reads each slot's material key from `defs` (slots stay unnamed without
/// them) and gives every slot a link: nothing for an unused slot, unbound
/// for a used one.
pub fn read_slot_keys(pointers: &[Option<TraversalMaterialSlot>], defs: Option<&MaterialDefinitions>) -> (Vec<Option<MaterialKey>>, Vec<AssetDependencyLink<SurfaceShaderSpace>>) {
    let mut keys = Vec::with_capacity(pointers.len());
    for pointer in pointers {
        let row = defs.zip(pointer.as_ref()).and_then(|(table, at)| table.materials.get(at.get()));
        keys.push(row.map(|found| MaterialKey { namespace: found.namespace, name: found.name.to_string() }));
    }
    let links = pointers
        .iter()
        .zip(&keys)
        .map(|(pointer, key)| link_for(slot_is_used(Some(pointer), Some(key)), || None))
        .collect();
    (keys, links)
}

/// Looks up the definition row of every slot's key. Unused slots link to
/// nothing; a used slot whose key names no definition stays unbound.
pub fn bind_slot_keys(keys: &[Option<MaterialKey>], pointers: &[Option<TraversalMaterialSlot>], defs: &MaterialDefinitions) -> Vec<AssetDependencyLink<SurfaceShaderSpace>> {
    let slots = pointers.len().max(keys.len());
    let mut links = Vec::with_capacity(slots);
    for at in 0..slots {
        let key = keys.get(at);
        let lookup = || {
            let row = defs.material_index_by_key(key?.as_ref()?)?;
            Some(AssetDependencyLink::bind(row, defs.owning_zone(row.order())))
        };
        links.push(link_for(slot_is_used(pointers.get(at), key), lookup));
    }
    links
}

/// Older spellings other crates still call; the rename sweep moves them over.
pub fn snapshot_mesh_surface_slots(
    pointers: &[Option<TraversalMaterialSlot>],
    defs: Option<&MaterialDefinitions>,
) -> (Vec<Option<MaterialKey>>, Vec<AssetDependencyLink<SurfaceShaderSpace>>) {
    read_slot_keys(pointers, defs)
}

/// Older spellings other crates still call; the rename sweep moves them over.
pub fn write_mesh_surface_links(keys: &[Option<MaterialKey>], pointers: &[Option<TraversalMaterialSlot>], defs: &MaterialDefinitions) -> Vec<AssetDependencyLink<SurfaceShaderSpace>> {
    bind_slot_keys(keys, pointers, defs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_core::AssetCategoryScope;

    fn blank<T: Default>() -> T {
        T::default()
    }

    fn walked(at: usize) -> Option<TraversalMaterialSlot> {
        Some(TraversalMaterialSlot::from_walk(at))
    }

    fn named(text: &str) -> Option<MaterialKey> {
        Some(MaterialKey { namespace: AssetCategoryScope::T6, name: text.into() })
    }

    fn one_material(label: &str) -> MaterialDefinitions {
        let row = asset_material::DesignerMaterial {
            name: label.into(),
            namespace: AssetCategoryScope::T6,
            method_set: blank(),
            draw_order: blank(),
            game_flag_info: blank(),
            condition_bits: blank(),
            camera_zone: blank(),
            facet_type_bits: blank(),
            raster_bits: blank(),
            raster_slot: blank(),
            method_table: blank(),
            route: blank(),
            textures: blank(),
            constants: blank(),
            zone: asset_core::ZoneCustodian::TEST,
            t6_draw: blank(),
        };
        MaterialDefinitions { materials: vec![row], ..blank() }
    }

    #[test]
    fn without_definitions_used_slots_wait_and_empty_ones_link_nowhere() {
        let (keys, links) = read_slot_keys(&[walked(0), None], None);
        assert!(keys.iter().all(Option::is_none));
        assert_eq!(links.len(), 2);
        assert!(matches!(links[0], AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure)));
        assert!(matches!(links[1], AssetDependencyLink::NothingThere));
    }

    #[test]
    fn a_used_slot_binds_only_when_its_key_names_a_definition() {
        let defs = one_material("wall");
        let keys = [named("wall"), named("floor"), None, named("")];
        let pointers = [None, None, walked(0), None];
        let links = bind_slot_keys(&keys, &pointers, &defs);
        let shape: Vec<(bool, bool)> = links
            .iter()
            .map(|link| (link.is_attached(), matches!(link, AssetDependencyLink::NothingThere)))
            .collect();
        assert_eq!(shape, [(true, false), (false, false), (false, false), (false, true)]);
        assert_eq!(links[0].bound().map(|row| row.order()), Some(0));
    }

    #[test]
    fn read_slots_carry_their_material_name_and_stay_unbound() {
        let defs = one_material("wall");
        let (keys, links) = read_slot_keys(&[walked(0), walked(5), None], Some(&defs));
        assert_eq!(keys[0].as_ref().map(|key| key.name.as_str()), Some("wall"));
        assert!(keys[1].is_none() && keys[2].is_none());
        let waiting: Vec<bool> = links.iter().map(|link| matches!(link, AssetDependencyLink::Unresolved(_))).collect();
        assert_eq!(waiting, [true, true, false]);
    }

    #[test]
    fn a_slot_nothing_names_or_points_at_links_nowhere() {
        let links = bind_slot_keys(&[None, None], &[None], &MaterialDefinitions::default());
        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|link| matches!(link, AssetDependencyLink::NothingThere)));
    }
}
