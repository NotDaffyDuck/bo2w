//! The weapon sounds an effect event triggers: finding the alias a weapon
//! authored for a slot, and queueing it for the audio side.

use asset_core::AssetCategoryScope;
use asset_game::WeaponAudioRole;
use assets::StagedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

#[derive(SystemParam)]
pub(super) struct WeaponVoices<'w> {
    gun_table: Option<Res<'w, StagedWeapons>>,
    bank: Option<Res<'w, audio::AudioCatalogRes>>,
    out: Option<ResMut<'w, Messages<audio::GunNoiseMsg>>>,
}

impl WeaponVoices<'_> {
    /// The alias the sound bank holds for one of the weapon's sound slots.
    pub(super) fn alias(&self, gun_no: u32, slot: WeaponAudioRole) -> Option<String> {
        let weapons = self.gun_table.as_deref()?;
        let bank = self.bank.as_deref()?;
        weapons
            .0
            .weapon_sound_alias(gun_no, slot, &bank.0)
            .map(str::to_owned)
    }

    /// Whether the weapon's own data names a sound for the slot.
    pub(super) fn authors(&self, gun_no: u32, slot: WeaponAudioRole) -> bool {
        self.gun_table
            .as_deref()
            .is_some_and(|weapons| weapons.0.authored_weapon_sound(gun_no, slot).is_some())
    }

    /// The report of one shot, chosen the way the client chooses it: the
    /// last round and the shooter's own gun each have a sound of their own.
    pub(super) fn fire_alias(&self, gun_no: u32, prior_shot: bool, own_gun: bool) -> Option<String> {
        let weapons = self.gun_table.as_deref()?;
        let bank = self.bank.as_deref()?;
        let find = |slot: WeaponAudioRole| weapons.0.weapon_sound_alias(gun_no, slot, &bank.0);
        audio::pick_view_gunshot_cue(
            prior_shot,
            own_gun,
            find(WeaponAudioRole::Fire),
            find(WeaponAudioRole::FirePlayer),
            find(WeaponAudioRole::FireLast),
            find(WeaponAudioRole::FireLastPlayer),
        )
        .map(str::to_owned)
    }

    /// Queues the alias to be heard. Answers whether the audio side exists
    /// to hear it.
    pub(super) fn say(
        &mut self,
        gun_no: u32,
        cue: String,
        position_in_inches: Option<[f32; 3]>,
        source_entity_no: Option<u32>,
    ) -> bool {
        let namespace = self
            .gun_table
            .as_deref()
            .and_then(|weapons| weapons.0.scope_for(gun_no))
            .unwrap_or(AssetCategoryScope::T6);
        let Some(out) = self.out.as_deref_mut() else {
            return false;
        };
        out.write(audio::GunNoiseMsg {
            cue_ns: namespace,
            alias: cue,
            position_in_inches,
            source_entity_no,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    #[test]
    fn nothing_is_heard_without_an_audio_queue() {
        let mut world = World::new();
        let mut state = SystemState::<WeaponVoices>::new(&mut world);
        let mut voices = state.get_mut(&mut world).unwrap();
        assert!(!voices.say(4, "boom".into(), None, None));
    }

    #[test]
    fn a_sound_is_queued_with_its_place_and_source() {
        let mut world = World::new();
        world.init_resource::<Messages<audio::GunNoiseMsg>>();
        let mut state = SystemState::<WeaponVoices>::new(&mut world);
        {
            let mut voices = state.get_mut(&mut world).unwrap();
            assert!(voices.say(4, "boom".into(), Some([1.0, 2.0, 3.0]), Some(9)));
        }
        let queued: Vec<_> = world
            .resource_mut::<Messages<audio::GunNoiseMsg>>()
            .drain()
            .collect();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].alias, "boom");
        assert_eq!(queued[0].position_in_inches, Some([1.0, 2.0, 3.0]));
        assert_eq!(queued[0].source_entity_no, Some(9));
        // No weapon table to name a game: the sound is a Black Ops II one.
        assert_eq!(queued[0].cue_ns, AssetCategoryScope::T6);
    }

    #[test]
    fn without_a_weapon_table_no_alias_is_found() {
        let mut world = World::new();
        let mut state = SystemState::<WeaponVoices>::new(&mut world);
        let voices = state.get_mut(&mut world).unwrap();
        assert_eq!(voices.alias(4, WeaponAudioRole::Fire), None);
        assert_eq!(voices.fire_alias(4, true, true), None);
        assert!(!voices.authors(4, WeaponAudioRole::Fire));
    }
}
