//! The weapon sounds an effect event triggers: finding the alias a weapon
//! authored for a slot, and queueing it for the audio side.

use asset_core::AssetNamespace;
use asset_game::WeaponSoundSlot;
use assets::PreparedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

#[derive(SystemParam)]
pub(super) struct WeaponVoices<'w> {
    weapons: Option<Res<'w, PreparedWeapons>>,
    bank: Option<Res<'w, audio::SoundBank>>,
    out: Option<ResMut<'w, Messages<audio::WeaponSound>>>,
}

impl WeaponVoices<'_> {
    /// The alias the sound bank holds for one of the weapon's sound slots.
    pub(super) fn alias(&self, weapon: u32, slot: WeaponSoundSlot) -> Option<String> {
        let weapons = self.weapons.as_deref()?;
        let bank = self.bank.as_deref()?;
        weapons
            .0
            .weapon_sound_alias(weapon, slot, &bank.0)
            .map(str::to_owned)
    }

    /// Whether the weapon's own data names a sound for the slot.
    pub(super) fn authors(&self, weapon: u32, slot: WeaponSoundSlot) -> bool {
        self.weapons
            .as_deref()
            .is_some_and(|weapons| weapons.0.authored_weapon_sound(weapon, slot).is_some())
    }

    /// The report of one shot, chosen the way the client chooses it: the
    /// last round and the shooter's own gun each have a sound of their own.
    pub(super) fn fire_alias(&self, weapon: u32, last_shot: bool, own_gun: bool) -> Option<String> {
        let weapons = self.weapons.as_deref()?;
        let bank = self.bank.as_deref()?;
        let find = |slot: WeaponSoundSlot| weapons.0.weapon_sound_alias(weapon, slot, &bank.0);
        audio::select_cg_fire_alias(
            last_shot,
            own_gun,
            find(WeaponSoundSlot::Fire),
            find(WeaponSoundSlot::FirePlayer),
            find(WeaponSoundSlot::FireLast),
            find(WeaponSoundSlot::FireLastPlayer),
        )
        .map(str::to_owned)
    }

    /// Queues the alias to be heard. Answers whether the audio side exists
    /// to hear it.
    pub(super) fn say(
        &mut self,
        weapon: u32,
        alias: String,
        origin_inches: Option<[f32; 3]>,
        snd_ent: Option<u32>,
    ) -> bool {
        let namespace = self
            .weapons
            .as_deref()
            .and_then(|weapons| weapons.0.namespace_of(weapon))
            .unwrap_or(AssetNamespace::T6);
        let Some(out) = self.out.as_deref_mut() else {
            return false;
        };
        out.write(audio::WeaponSound {
            namespace,
            alias,
            origin_inches,
            snd_ent,
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
        world.init_resource::<Messages<audio::WeaponSound>>();
        let mut state = SystemState::<WeaponVoices>::new(&mut world);
        {
            let mut voices = state.get_mut(&mut world).unwrap();
            assert!(voices.say(4, "boom".into(), Some([1.0, 2.0, 3.0]), Some(9)));
        }
        let queued: Vec<_> = world
            .resource_mut::<Messages<audio::WeaponSound>>()
            .drain()
            .collect();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].alias, "boom");
        assert_eq!(queued[0].origin_inches, Some([1.0, 2.0, 3.0]));
        assert_eq!(queued[0].snd_ent, Some(9));
        // No weapon table to name a game: the sound is a Black Ops II one.
        assert_eq!(queued[0].namespace, AssetNamespace::T6);
    }

    #[test]
    fn without_a_weapon_table_no_alias_is_found() {
        let mut world = World::new();
        let mut state = SystemState::<WeaponVoices>::new(&mut world);
        let voices = state.get_mut(&mut world).unwrap();
        assert_eq!(voices.alias(4, WeaponSoundSlot::Fire), None);
        assert_eq!(voices.fire_alias(4, true, true), None);
        assert!(!voices.authors(4, WeaponSoundSlot::Fire));
    }
}
