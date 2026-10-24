//! What the simulation is told about the game's content: every weapon's
//! numbers, the animations scripts can play, the shocks a map defines, and
//! which weapons the class editor offers.

use std::collections::BTreeMap;
use std::sync::Arc;

use asset_core::AssetNamespace;
use asset_game::{FamilySlot, WeaponFamilies, WeaponRegistry};
use assets::PreparedXAnims;
use frame::CacWeaponOffer;
use weapon_t6::facts::FactsTable;
use weapon_t6::viewmodel::VmTable;

use super::bootstrap::player_kit_collision;
use crate::combat_table;

/// What the content is built from.
pub(super) struct ContentInputs<'a> {
    pub weapons: &'a WeaponRegistry,
    pub bodies: &'a asset_model::BodyMeshCatalog,
    pub xanims: &'a PreparedXAnims,
    /// The namespace the map's animations are filed under.
    pub namespace: AssetNamespace,
    /// BO2's per-weapon facts, as text, named by script name.
    pub gun_facts_dump: &'a str,
    pub pen_table: weapon_t6::penetration::PenetrationDepthTable,
    pub pen_table_loaded: bool,
    pub hit_locations: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub shocks: &'a [(String, String)],
}

/// The content, and the tables other parts of the match need alongside it.
pub(super) struct Configured {
    pub content: sim::SimContentBuilder,
    pub combat: Vec<weapon_t6::combat_facts::WeaponCombatFacts>,
    pub equipment: Vec<sim::EquipmentRuntimeFacts>,
    /// The client reads the same facts for its view kick.
    pub gun_facts: Arc<FactsTable>,
    pub viewmodel: Arc<VmTable>,
}

/// The game's zombies guns keep the name they have; every other weapon is
/// known to the scripts under its `_mp` name.
fn keeps_own_name(bare: &str) -> bool {
    bare.ends_with("_zm") || bare.contains("_zm+")
}

/// The name BO2's scripts and fact dump know each weapon by, in weapon index
/// order (the first weapon is index 1).
fn script_names(weapons: &WeaponRegistry) -> Vec<String> {
    (1..=weapons.len() as u32)
        .map(|index| {
            let bare = weapons.name_of(index);
            if keeps_own_name(bare) {
                bare.to_owned()
            } else {
                weapons.script_name_of(index)
            }
        })
        .collect()
}

/// A cycle's rate: how often an animation of this many frames, played at this
/// frame rate, comes round. Nothing to play means no cycle.
fn cycles_per_second(numframes: u16, framerate: f32) -> f32 {
    if numframes > 0 && framerate > 0.0 {
        framerate / f32::from(numframes)
    } else {
        0.0
    }
}

/// How scripts can play each animation of the map's model set.
fn script_model_anims(
    xanims: &PreparedXAnims,
    namespace: AssetNamespace,
) -> impl Iterator<Item = (String, sim::ScriptModelPlayAnim)> + '_ {
    xanims.0.names().filter_map(move |name| {
        let parts = &xanims.0.get(namespace, name)?.parts;
        Some((
            name.to_owned(),
            sim::ScriptModelPlayAnim {
                looping: parts.flags & 1 != 0,
                frequency: cycles_per_second(parts.numframes, parts.framerate),
            },
        ))
    })
}

/// Black Ops II climbs with its own clips; any it lacks stay unbound.
fn mantle_binding(xanims: &PreparedXAnims) -> sim::MantleXAnimBind {
    let mut found = 0usize;
    let bind = sim::MantleXAnimBind::from_t6_clips(
        |index| {
            let name = sim::MantleXAnimBind::clip_name_t6(index)?;
            let clip = xanims.0.clip_with_root_motion(AssetNamespace::T6, name);
            found += usize::from(clip.is_some());
            clip
        },
        sim::PERK_MANTLE_REDUCTION_BO2MP,
    );
    diag::info!(
        World,
        "mantle: {found} of {} Black Ops II climb animations",
        sim::MANTLE_XANIM_NAMES_T6.len()
    );
    bind
}

/// What each weapon configuration is made of, by weapon index; `None` for an
/// index that is no configuration.
fn weapon_setups(weapons: &WeaponRegistry) -> Vec<Option<sim::WeaponSetup>> {
    (0..weapons.script_names_table().len() as u32)
        .map(|id| {
            let selection = weapons.describe_configuration(id)?;
            let family = selection.family.as_ref()?;
            let realm = if family.namespace == AssetNamespace::T5 {
                sim::script::Realm::T5
            } else {
                sim::script::Realm::T6
            };
            Some(sim::WeaponSetup {
                realm,
                base: family.base.clone(),
                attachments: selection.attachments.clone(),
            })
        })
        .collect()
}

/// A debugging aid: write BO2's fact dump where `BO2MP_GUN_FACTS_DUMP` says.
fn dump_gun_facts(dump: &str) {
    if let Some(path) = std::env::var_os("BO2MP_GUN_FACTS_DUMP") {
        let _ = std::fs::write(path, dump);
    }
}

pub(super) fn configure(with: ContentInputs) -> Configured {
    let ContentInputs {
        weapons,
        bodies,
        xanims,
        namespace,
        gun_facts_dump,
        pen_table,
        pen_table_loaded,
        hit_locations,
        shocks,
    } = with;
    let mut content = sim::SimContentBuilder::default();

    content.set_weapon_def_scales(weapons.scales_table());
    let combat = combat_table::from_registry(weapons, hit_locations);
    content.set_weapon_combat_table(combat.clone());
    content.set_weapon_runnable_table(weapons.runnable_table());
    content.set_weapon_transition_groups(weapons.configuration_transition_groups());
    content.set_bullet_pen_facts(combat_table::pen_from_registry(weapons));
    content.set_penetration_table(pen_table);
    content.set_pen_table_loaded(pen_table_loaded);
    let equipment = combat_table::equipment_from_registry(weapons);
    content.set_equipment_runtime_table(equipment.clone());
    content.set_weapon_melee_only(combat_table::melee_only_from_registry(weapons));
    content.set_weapon_script_sounds(combat_table::script_sounds_from_registry(weapons));

    dump_gun_facts(gun_facts_dump);
    let names = script_names(weapons);
    let rows = || {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| (index as u32 + 1, name.as_str()))
    };
    let gun_facts = Arc::new(FactsTable::from_dump(gun_facts_dump).reindexed(rows()));
    let viewmodel = Arc::new(VmTable::from_dump_reindexed(gun_facts_dump, rows()));
    content.set_gun_facts(FactsTable::clone(&gun_facts));

    content.set_player_kit_collisions(
        player_kit_collision(bodies, false),
        player_kit_collision(bodies, true),
    );
    content.set_script_model_anims(script_model_anims(xanims, namespace));
    content.set_mantle_xanims(mantle_binding(xanims));

    content.set_weapon_script_names(weapons.script_names_table());
    content.set_weapon_script_aliases(Vec::new());
    content.set_weapon_setups(weapon_setups(weapons));
    content.set_weapon_world_models(weapons.world_models_table());
    content.set_weapon_projectile_models(weapons.projectile_models_table());
    content.set_vehicle_turrets(weapons.vehicle_turrets());
    content.set_vehicle_compass(
        weapons
            .vehicle_compass()
            .map(|(name, icons, size)| (name.to_owned(), (icons.clone(), size))),
    );
    content.set_shocks(parse_shocks(shocks));

    Configured {
        content,
        combat,
        equipment,
        gun_facts,
        viewmodel,
    }
}

/// The map's shellshock definitions that read; the rest are reported and left
/// out.
fn parse_shocks(
    defined: &[(String, String)],
) -> BTreeMap<String, hud_t6::flashbang::ShellshockParms> {
    let mut shocks = BTreeMap::new();
    for (name, text) in defined {
        match hud_t6::flashbang::ShellshockParms::parse(text) {
            Ok(parms) => {
                shocks.insert(name.clone(), parms);
            }
            Err(error) => diag::warn!(Zone, "shock/{name}.shock: {error}"),
        }
    }
    diag::info!(Zone, "shellshocks: {:?}", shocks.keys().collect::<Vec<_>>());
    shocks
}

/// The weapons the class editor offers, by the slot they go in.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Offers {
    pub primary: Vec<CacWeaponOffer>,
    pub secondary: Vec<CacWeaponOffer>,
    pub lethal: Vec<CacWeaponOffer>,
    pub tactical: Vec<CacWeaponOffer>,
}

impl Offers {
    fn add(&mut self, slot: FamilySlot, offer: CacWeaponOffer) {
        match slot {
            FamilySlot::Primary => self.primary.push(offer),
            FamilySlot::Secondary => self.secondary.push(offer),
            FamilySlot::Lethal => self.lethal.push(offer),
            FamilySlot::Tactical => self.tactical.push(offer),
            FamilySlot::Other => {}
        }
    }
}

fn sorted_into_slots(offers: impl IntoIterator<Item = (FamilySlot, CacWeaponOffer)>) -> Offers {
    let mut slots = Offers::default();
    for (slot, offer) in offers {
        slots.add(slot, offer);
    }
    slots
}

/// What the class editor can offer, and what it was told to leave out (each
/// with its reason).
pub(super) fn class_offers(families: &WeaponFamilies) -> (Offers, Vec<(String, String)>) {
    let offered = families.offered().map(|family| {
        let offer = CacWeaponOffer {
            key: family.key.asset_key(),
            item_group: Some(family.item_group.clone()),
            attachments: family
                .attachments
                .iter()
                .map(|choice| choice.name.clone())
                .collect(),
        };
        (family.slot, offer)
    });
    (sorted_into_slots(offered), families.excluded().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(key: &str) -> CacWeaponOffer {
        CacWeaponOffer {
            key: key.to_owned(),
            item_group: None,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn zombies_guns_keep_their_names() {
        assert!(keeps_own_name("ray_gun_zm"));
        assert!(keeps_own_name("m1911_zm+upgraded"));
        assert!(!keeps_own_name("an94"));
        assert!(!keeps_own_name("an94_mp"));
        assert!(!keeps_own_name("zm_gun"));
    }

    #[test]
    fn a_cycle_runs_at_its_frame_rate_over_its_length() {
        assert_eq!(cycles_per_second(30, 30.0), 1.0);
        assert_eq!(cycles_per_second(60, 30.0), 0.5);
    }

    #[test]
    fn an_animation_with_nothing_in_it_does_not_cycle() {
        assert_eq!(cycles_per_second(0, 30.0), 0.0);
        assert_eq!(cycles_per_second(30, 0.0), 0.0);
        assert_eq!(cycles_per_second(30, -5.0), 0.0);
    }

    #[test]
    fn offers_go_in_the_slot_of_their_family() {
        let slots = sorted_into_slots([
            (FamilySlot::Primary, offer("a")),
            (FamilySlot::Secondary, offer("b")),
            (FamilySlot::Lethal, offer("c")),
            (FamilySlot::Tactical, offer("d")),
            (FamilySlot::Primary, offer("e")),
        ]);
        let keys = |offers: &[CacWeaponOffer]| -> Vec<String> {
            offers.iter().map(|offer| offer.key.clone()).collect()
        };
        assert_eq!(keys(&slots.primary), ["a", "e"]);
        assert_eq!(keys(&slots.secondary), ["b"]);
        assert_eq!(keys(&slots.lethal), ["c"]);
        assert_eq!(keys(&slots.tactical), ["d"]);
    }

    #[test]
    fn a_weapon_that_goes_in_no_slot_is_not_offered() {
        let slots = sorted_into_slots([(FamilySlot::Other, offer("x"))]);
        assert_eq!(slots, Offers::default());
    }

    #[test]
    fn no_weapons_offer_nothing() {
        assert_eq!(sorted_into_slots([]), Offers::default());
    }

    #[test]
    fn a_shock_that_reads_is_kept_under_its_name() {
        let shocks = parse_shocks(&[("flashbang".to_owned(), "bg_shock_movement 0.5".to_owned())]);
        assert_eq!(shocks["flashbang"].movement_scale, 0.5);
    }

    #[test]
    fn a_shock_that_does_not_read_is_left_out() {
        let shocks = parse_shocks(&[
            ("bad".to_owned(), "bg_shock_movement fast".to_owned()),
            (
                "blur".to_owned(),
                "bg_shock_screenBlurBlendTime 0".to_owned(),
            ),
            ("good".to_owned(), String::new()),
        ]);
        assert_eq!(shocks.keys().collect::<Vec<_>>(), ["good"]);
    }

    #[test]
    fn no_shocks_defined_means_none_installed() {
        assert!(parse_shocks(&[]).is_empty());
    }
}
