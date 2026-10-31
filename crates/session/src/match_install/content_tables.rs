//! What the simulation is told about the game's content: every weapon's
//! numbers, the animations scripts can play, the shocks a map defines, and
//! which weapons the class editor offers.

use std::collections::BTreeMap;
use std::sync::Arc;

use asset_core::AssetCategoryScope;
use asset_game::{FamilySlot, WeaponFamilies, WeaponLedger};
use assets::StagedXAnims;
use frame::CustomClassGunChoice;
use weapon_t6::facts::FactsTable;
use weapon_t6::viewmodel::VmTable;

use super::bootstrap::player_kit_collision;
use crate::fight_table;

/// What the content is built from.
pub(super) struct ContentInputs<'a> {
    pub weapons: &'a WeaponLedger,
    pub bodies: &'a asset_model::CharacterMeshRoster,
    pub animation_refs: &'a StagedXAnims,
    /// The namespace the map's animations are filed under.
    pub namespace: AssetCategoryScope,
    /// BO2's per-weapon facts, as text, named by script name.
    pub gun_facts_dump: &'a str,
    pub pen_table: weapon_t6::penetration::BulletPierceDepthChart,
    pub wall_pierce_table_ready: bool,
    pub hit_locations: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub shocks: &'a [(String, String)],
}

/// The content, and the tables other parts of the match need alongside it.
pub(super) struct Configured {
    pub content: sim::SimContentBuilder,
    pub combat: Vec<weapon_t6::combat_facts::GunBattleProfile>,
    pub equipment: Vec<sim::GadgetLiveStats>,
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
fn script_names(weapons: &WeaponLedger) -> Vec<String> {
    (1..=weapons.len() as u32)
        .map(|index| {
            let bare = weapons.label_for(index);
            if keeps_own_name(bare) {
                bare.to_owned()
            } else {
                weapons.gsc_name_for(index)
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
    animation_refs: &StagedXAnims,
    namespace: AssetCategoryScope,
) -> impl Iterator<Item = (String, sim::ScriptedModelAnimPlay)> + '_ {
    animation_refs.0.names().filter_map(move |name| {
        let parts = &animation_refs.0.get(namespace, name)?.pieces;
        Some((
            name.to_owned(),
            sim::ScriptedModelAnimPlay {
                loops: parts.flags & 1 != 0,
                hz: cycles_per_second(parts.frame_total, parts.fps),
            },
        ))
    })
}

/// Black Ops II climbs with its own clips; any it lacks stay unbound.
fn mantle_binding(animation_refs: &StagedXAnims) -> sim::LedgeClimbAnimSet {
    let mut found = 0usize;
    let bind = sim::LedgeClimbAnimSet::from_t6_clips(
        |index| {
            let name = sim::LedgeClimbAnimSet::clip_name_t6(index)?;
            let clip = animation_refs.0.clip_with_root_motion(AssetCategoryScope::T6, name);
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
fn weapon_setups(weapons: &WeaponLedger) -> Vec<Option<sim::WeaponSetup>> {
    (0..weapons.script_label_sheet().len() as u32)
        .map(|id| {
            let selection = weapons.describe_configuration(id)?;
            let family = selection.family.as_ref()?;
            let realm = if family.game == AssetCategoryScope::T5 {
                sim::gsc_host::Realm::T5
            } else {
                sim::gsc_host::Realm::T6
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
        animation_refs,
        namespace,
        gun_facts_dump,
        pen_table,
        wall_pierce_table_ready,
        hit_locations,
        shocks,
    } = with;
    let mut content = sim::SimContentBuilder::default();

    content.with_gun_scales(weapons.multiplier_sheet());
    let combat = fight_table::from_registry(weapons, hit_locations);
    content.with_gun_fight_table(combat.clone());
    content.set_weapon_runnable_table(weapons.runnable_table());
    content.set_weapon_transition_groups(weapons.configuration_transition_groups());
    content.with_pierce_facts(fight_table::pen_from_registry(weapons));
    content.with_pierce_table(pen_table);
    content.mark_pierce_table_ready(wall_pierce_table_ready);
    let equipment = fight_table::equipment_from_registry(weapons);
    content.with_gear_table(equipment.clone());
    content.set_weapon_melee_only(fight_table::melee_only_from_registry(weapons));
    content.set_weapon_raw_kinds(fight_table::raw_kinds_from_registry(weapons));
    content.set_weapon_script_sounds(fight_table::script_sounds_from_registry(weapons));

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
    content.set_script_model_anims(script_model_anims(animation_refs, namespace));
    content.with_climb_clips(mantle_binding(animation_refs));

    content.store_weapon_script_labels(weapons.script_label_sheet());
    content.set_weapon_script_aliases(Vec::new());
    content.set_weapon_setups(weapon_setups(weapons));
    content.set_weapon_world_models(weapons.world_models_table());
    content.set_weapon_world_parts(
        weapons
            .world_parts_table()
            .into_iter()
            .map(|parts| {
                parts
                    .into_iter()
                    .map(|(part_model, tag, offset, rotation)| sim::WeaponPart {
                        mesh: part_model,
                        joint: tag,
                        offset,
                        rotation,
                    })
                    .collect()
            })
            .collect(),
    );
    content.set_weapon_display_names(weapons.display_names_table());
    content.set_weapon_projectile_models(weapons.projectile_models_table());
    content.set_vehicle_turrets(weapons.vehicle_turrets());
    content.set_vehicle_compass(
        weapons
            .vehicle_compass()
            .map(|(label, icons, size)| (label.to_owned(), (icons.clone(), size))),
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
    pub primary: Vec<CustomClassGunChoice>,
    pub secondary: Vec<CustomClassGunChoice>,
    pub lethal: Vec<CustomClassGunChoice>,
    pub tactical: Vec<CustomClassGunChoice>,
}

impl Offers {
    fn file(&mut self, slot: FamilySlot, choice: CustomClassGunChoice) {
        match slot {
            FamilySlot::Primary => self.primary.push(choice),
            FamilySlot::Secondary => self.secondary.push(choice),
            FamilySlot::Lethal => self.lethal.push(choice),
            FamilySlot::Tactical => self.tactical.push(choice),
            FamilySlot::Other => {}
        }
    }
}

fn sorted_into_slots(offers: impl IntoIterator<Item = (FamilySlot, CustomClassGunChoice)>) -> Offers {
    let mut slots = Offers::default();
    for (slot, offer) in offers {
        slots.file(slot, offer);
    }
    slots
}

/// What the class editor can offer, and what it was told to leave out (each
/// with its reason).
pub(super) fn class_offers(families: &WeaponFamilies) -> (Offers, Vec<(String, String)>) {
    let offered = families.offered().map(|family| {
        let offer = CustomClassGunChoice {
            key: family.key.asset_key(),
            loot_bundle: Some(family.loot_bundle.clone()),
            attachments: family
                .attachments
                .iter()
                .map(|choice| choice.name.clone())
                .collect(),
        };
        (family.bay, offer)
    });
    (sorted_into_slots(offered), families.excluded().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(key: &str) -> CustomClassGunChoice {
        CustomClassGunChoice {
            key: key.to_owned(),
            loot_bundle: None,
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
        let keys = |offers: &[CustomClassGunChoice]| -> Vec<String> {
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
            ("good".to_owned(), String::default()),
        ]);
        assert_eq!(shocks.keys().collect::<Vec<_>>(), ["good"]);
    }

    #[test]
    fn no_shocks_defined_means_none_installed() {
        assert!(parse_shocks(&[]).is_empty());
    }
}
