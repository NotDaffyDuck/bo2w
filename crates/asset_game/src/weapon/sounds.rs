//! The sound aliases a weapon plays, one per event.
//!
//! Every event is an `Option<String>` field. The ones the mixer asks for by
//! name also get a [`WeaponAudioRole`]; `fire_stop` and its player form are
//! only ever reached through the alias list.

use asset_t6::m2::SURF_TYPE_COUNT;

macro_rules! weapon_sounds {
    (
        slots { $($field:ident @ $variant:ident),+ $(,)? }
        unslotted { $($extra:ident),+ $(,)? }
    ) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct WeaponAudioNicknames {
            $(pub $field: Option<String>,)+
            $(pub $extra: Option<String>,)+
            /// One alias per surface a thrown weapon can land on.
            pub ricochet: [Option<String>; SURF_TYPE_COUNT],
            /// Animation notetrack name to alias, for guns that name theirs.
            pub cue_to_audio_lookup: Vec<(String, String)>,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub enum WeaponAudioRole {
            $($variant,)+
        }

        impl WeaponAudioRole {
            pub const ALL: [Self; [$(Self::$variant),+].len()] = [$(Self::$variant),+];
        }

        impl WeaponAudioNicknames {
            pub(super) fn hint(&self, wanted: WeaponAudioRole) -> Option<&str> {
                // The slots are numbered in field order, ahead of the unslotted ones.
                self.named().nth(wanted as usize).flatten()
            }

            fn named(&self) -> impl Iterator<Item = Option<&str>> {
                [$(self.$field.as_deref(),)+ $(self.$extra.as_deref(),)+].into_iter()
            }

            /// Fill every event this weapon leaves unset from `donor`.
            pub(super) fn fill_gaps_from(&mut self, donor: &Self) {
                $(if self.$field.is_none() {
                    self.$field = donor.$field.clone();
                })+
                $(if self.$extra.is_none() {
                    self.$extra = donor.$extra.clone();
                })+
                for (own, other) in self.ricochet.iter_mut().zip(&donor.ricochet) {
                    if own.is_none() {
                        *own = other.clone();
                    }
                }
                if self.cue_to_audio_lookup.is_empty() {
                    self.cue_to_audio_lookup = donor.cue_to_audio_lookup.clone();
                }
            }
        }
    };
}

weapon_sounds! {
    slots {
        fire @ Fire,
        trigger_sound_name @ FirePlayer,
        empty_fire @ EmptyFire,
        empty_fire_player @ EmptyFirePlayer,
        melee_swipe @ MeleeSwipe,
        melee_swipe_player @ MeleeSwipePlayer,
        melee_hit @ MeleeHit,
        melee_miss @ MeleeMiss,
        pickup @ Pickup,
        pickup_sound_name @ PickupPlayer,
        ammo_pickup @ AmmoPickup,
        ammo_pickup_player @ AmmoPickupPlayer,
        pullback @ Pullback,
        pullback_player @ PullbackPlayer,
        reload @ Reload,
        reload_player @ ReloadPlayer,
        reload_empty @ DryReload,
        reload_empty_player @ ReloadEmptyPlayer,
        reload_start @ ReloadKickoff,
        reload_start_player @ ReloadStartPlayer,
        reload_end @ ReloadEnd,
        reload_end_player @ ReloadEndPlayer,
        rechamber @ Rechamber,
        rechamber_player @ RechamberPlayer,
        alt_switch @ AltSwitch,
        alt_switch_player @ AltSwitchPlayer,
        raise @ Raise,
        raise_player @ RaisePlayer,
        first_raise @ FirstRaise,
        first_raise_player @ FirstRaisePlayer,
        putaway @ Putaway,
        putaway_player @ PutawayPlayer,
        projectile_blast_fx @ ProjectileExplosion,
        projectile @ Projectile,
        proj_ignition_sound @ ProjIgnition,
        final_round_fired @ FireLast,
        fire_last_player @ FireLastPlayer,
        ads_raise_player @ AdsRaisePlayer,
        ads_lower_player @ AdsLowerPlayer,
        ads_zoom @ AdsZoom,
        fire_loop @ FireLoop,
        fire_loop_player @ FireLoopPlayer,
        spin_loop @ SpinLoop,
        spin_loop_player @ SpinLoopPlayer,
        start_spin @ StartSpin,
        start_spin_player @ StartSpinPlayer,
        stop_spin @ StopSpin,
        stop_spin_player @ StopSpinPlayer,
    }
    unslotted { fire_player_akimbo, fire_stop, fire_stop_player }
}

impl WeaponAudioNicknames {
    /// Every non-empty alias this weapon can play, bounce and notetrack ones
    /// included. The same alias can appear more than once.
    pub fn attainable_entries(&self) -> Vec<&str> {
        let mut found: Vec<&str> = self.named().flatten().collect();
        found.extend(self.ricochet.iter().filter_map(Option::as_deref));
        found.extend(self.cue_to_audio_lookup.iter().map(|(_, played)| played.as_str()));
        found.retain(|text| !text.is_empty());
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_reads_its_own_field() {
        let aliases = WeaponAudioNicknames {
            fire: Some("fire_a".into()),
            ads_zoom: Some("zoom_a".into()),
            proj_ignition_sound: Some("ignite_a".into()),
            stop_spin_player: Some("stop_a".into()),
            fire_stop: Some("unslotted_a".into()),
            ..WeaponAudioNicknames::default()
        };
        let heard: Vec<(WeaponAudioRole, &str)> = WeaponAudioRole::ALL
            .into_iter()
            .filter_map(|slot| Some((slot, aliases.hint(slot)?)))
            .collect();
        assert_eq!(
            heard,
            [
                (WeaponAudioRole::Fire, "fire_a"),
                (WeaponAudioRole::ProjIgnition, "ignite_a"),
                (WeaponAudioRole::AdsZoom, "zoom_a"),
                (WeaponAudioRole::StopSpinPlayer, "stop_a"),
            ]
        );
    }

    #[test]
    fn gaps_fill_from_the_donor_and_reachable_skips_empty_ones() {
        let mut gun = WeaponAudioNicknames { fire: Some("mine".into()), reload: Some(String::new()), ..Default::default() };
        let mut donor = WeaponAudioNicknames { fire: Some("theirs".into()), pickup: Some("pick".into()), ..Default::default() };
        donor.ricochet[0] = Some("thud".into());
        donor.cue_to_audio_lookup = vec![("note".into(), "played".into())];
        gun.fill_gaps_from(&donor);
        assert_eq!(gun.hint(WeaponAudioRole::Fire), Some("mine"));
        assert_eq!(gun.hint(WeaponAudioRole::Pickup), Some("pick"));
        let mut reachable = gun.attainable_entries();
        reachable.sort_unstable();
        assert_eq!(reachable, ["mine", "pick", "played", "thud"]);
    }
}
