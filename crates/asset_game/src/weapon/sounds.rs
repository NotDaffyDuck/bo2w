//! The sound aliases a weapon plays, one per event.
//!
//! Every event is an `Option<String>` field. The ones the mixer asks for by
//! name also get a [`WeaponSoundSlot`]; `fire_stop` and its player form are
//! only ever reached through the alias list.

use asset_t6::m2::SURF_TYPE_COUNT;

macro_rules! weapon_sounds {
    (
        slots { $($variant:ident => $field:ident),+ $(,)? }
        unslotted { $($extra:ident),+ $(,)? }
    ) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct WeaponSoundAliases {
            $(pub $field: Option<String>,)+
            $(pub $extra: Option<String>,)+
            /// One alias per surface a thrown weapon can land on.
            pub bounce: [Option<String>; SURF_TYPE_COUNT],
            /// Animation notetrack name to alias, for guns that name theirs.
            pub notetrack_sound_map: Vec<(String, String)>,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub enum WeaponSoundSlot {
            $($variant),+
        }

        impl WeaponSoundSlot {
            pub const ALL: [Self; [$(Self::$variant),+].len()] = [$(Self::$variant),+];
        }

        impl WeaponSoundAliases {
            pub(super) fn hint(&self, slot: WeaponSoundSlot) -> Option<&str> {
                match slot {
                    $(WeaponSoundSlot::$variant => self.$field.as_deref()),+
                }
            }

            fn named(&self) -> impl Iterator<Item = Option<&str>> {
                [$(self.$field.as_deref(),)+ $(self.$extra.as_deref(),)+].into_iter()
            }

            /// Fill every event this weapon leaves unset from `src`.
            pub(super) fn fill_gaps_from(&mut self, src: &Self) {
                $(if self.$field.is_none() {
                    self.$field.clone_from(&src.$field);
                })+
                $(if self.$extra.is_none() {
                    self.$extra.clone_from(&src.$extra);
                })+
                for (mine, theirs) in self.bounce.iter_mut().zip(&src.bounce) {
                    if mine.is_none() {
                        mine.clone_from(theirs);
                    }
                }
                if self.notetrack_sound_map.is_empty() {
                    self.notetrack_sound_map.clone_from(&src.notetrack_sound_map);
                }
            }
        }
    };
}

weapon_sounds! {
    slots {
        Fire => fire,
        FirePlayer => fire_player,
        EmptyFire => empty_fire,
        EmptyFirePlayer => empty_fire_player,
        MeleeSwipe => melee_swipe,
        MeleeSwipePlayer => melee_swipe_player,
        MeleeHit => melee_hit,
        MeleeMiss => melee_miss,
        Pickup => pickup,
        PickupPlayer => pickup_player,
        AmmoPickup => ammo_pickup,
        AmmoPickupPlayer => ammo_pickup_player,
        Pullback => pullback,
        PullbackPlayer => pullback_player,
        Reload => reload,
        ReloadPlayer => reload_player,
        ReloadEmpty => reload_empty,
        ReloadEmptyPlayer => reload_empty_player,
        ReloadStart => reload_start,
        ReloadStartPlayer => reload_start_player,
        ReloadEnd => reload_end,
        ReloadEndPlayer => reload_end_player,
        Rechamber => rechamber,
        RechamberPlayer => rechamber_player,
        AltSwitch => alt_switch,
        AltSwitchPlayer => alt_switch_player,
        Raise => raise,
        RaisePlayer => raise_player,
        FirstRaise => first_raise,
        FirstRaisePlayer => first_raise_player,
        Putaway => putaway,
        PutawayPlayer => putaway_player,
        ProjectileExplosion => proj_explosion,
        Projectile => projectile,
        ProjIgnition => proj_ignition_sound,
        FireLast => fire_last,
        FireLastPlayer => fire_last_player,
        AdsRaisePlayer => ads_raise_player,
        AdsLowerPlayer => ads_lower_player,
        AdsZoom => ads_zoom,
        FireLoop => fire_loop,
        FireLoopPlayer => fire_loop_player,
        SpinLoop => spin_loop,
        SpinLoopPlayer => spin_loop_player,
        StartSpin => start_spin,
        StartSpinPlayer => start_spin_player,
        StopSpin => stop_spin,
        StopSpinPlayer => stop_spin_player,
    }
    unslotted { fire_player_akimbo, fire_stop, fire_stop_player }
}

impl WeaponSoundAliases {
    /// Every non-empty alias this weapon can play, bounce and notetrack ones
    /// included. The same alias can appear more than once.
    pub fn reachable_aliases(&self) -> Vec<&str> {
        self.named()
            .flatten()
            .chain(self.bounce.iter().filter_map(Option::as_deref))
            .chain(self.notetrack_sound_map.iter().map(|(_, alias)| alias.as_str()))
            .filter(|alias| !alias.is_empty())
            .collect()
    }
}
