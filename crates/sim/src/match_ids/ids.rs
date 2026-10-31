//! Numbered names for lives, published events, sent actions, trigger pulls, thrown things, pellets, scripted models, and harm.

/// Declares a name made of one number. Stepping it on gives the following name, and the largest number is followed
/// by zero instead of overflowing.
macro_rules! counted_name {
    ($(#[$doc:meta])* $name:ident($inner:ty)) => {
        $(#[$doc])*
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $inner);

        impl $name {
            /// The name after this one.
            pub fn next(self) -> Self {
                Self(self.0.wrapping_add(1))
            }
        }
    };
    (@ordered $name:ident) => {
        impl $name {
            /// Whether this one came after `earlier`. The numbers wrap, so "after" means less than half the number
            /// range ahead.
            pub const fn is_newer_than(self, earlier: Self) -> bool {
                let gap = self.0.wrapping_sub(earlier.0);
                gap != 0 && gap <= u32::MAX / 2
            }
        }
    };
}

counted_name!(
    /// Counts a client's lives; each spawn gets the next one.
    LifeCount(u32)
);
counted_name!(
    /// Numbers the events the match publishes, in the order they happened.
    EventStamp(u32)
);
counted_name!(
    /// Numbers the actions a client has sent.
    ActionStamp(u32)
);
counted_name!(
    /// Names one trigger pull.
    BulletNo(u32)
);
counted_name!(
    /// Names one thrown or launched projectile.
    ThrownId(u32)
);

counted_name!(@ordered EventStamp);

/// Names one pellet of a shot (a shotgun fires several).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuckshotId(pub u16);

/// Names a script-spawned model by its place in the order the map's scripts created them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScriptedModel(pub(crate) u32);

impl self::ScriptedModel {
    /// The name of the model with this number on the wire.
    pub const fn decode_net(number: u32) -> Self {
        Self(number)
    }

    /// The number the name goes by on the wire.
    pub const fn encode_for_net(self) -> u32 {
        self.0
    }

    /// The name of the n-th model the map's scripts create. It is the same number as on the wire.
    pub const fn from_authored_source_ordinal(index: u32) -> Self {
        Self(index)
    }
}

/// What dealt a damage: a shot, a projectile, an explosion (named by the model that went off) or a melee blow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HarmOrigin {
    Shot(BulletNo),
    Projectile(ThrownId),
    Radius(ScriptedModel),
    Melee,
}

/// Which part of its life the match is in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GameRoundStage {
    #[default]
    Warmup = 0,
    Playing = 1,
    Intermission = 2,
    PostGame = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_step_on_and_wrap_to_zero() {
        assert_eq!(LifeCount(4).next(), LifeCount(5));
        assert_eq!(BulletNo(u32::MAX).next(), BulletNo(0));
        assert_eq!(ThrownId(u32::MAX).next(), ThrownId(0));
        assert_eq!(ActionStamp(u32::MAX).next(), ActionStamp(0));
        assert_eq!(EventStamp(u32::MAX).next(), EventStamp(0));
    }

    #[test]
    fn a_later_event_is_newer_even_across_the_wrap() {
        assert!(EventStamp(6).is_newer_than(EventStamp(5)));
        assert!(!EventStamp(5).is_newer_than(EventStamp(6)));
        assert!(!EventStamp(5).is_newer_than(EventStamp(5)));
        assert!(EventStamp(2).is_newer_than(EventStamp(u32::MAX - 1)));
        assert!(!EventStamp(u32::MAX - 1).is_newer_than(EventStamp(2)));
    }

    #[test]
    fn newer_stops_at_half_the_range() {
        assert!(EventStamp(0x7fff_ffff).is_newer_than(EventStamp(0)));
        assert!(!EventStamp(0x8000_0000).is_newer_than(EventStamp(0)));
    }

    #[test]
    fn a_script_model_name_keeps_its_ordinal_either_way_it_is_made() {
        assert_eq!(ScriptedModel::decode_net(9).encode_for_net(), 9);
        assert_eq!(ScriptedModel::from_authored_source_ordinal(9), ScriptedModel::decode_net(9));
    }

    #[test]
    fn damage_sources_sort_by_kind_then_by_name() {
        let shot = HarmOrigin::Shot(BulletNo(9));
        let projectile = HarmOrigin::Projectile(ThrownId(0));
        let radius = HarmOrigin::Radius(ScriptedModel::decode_net(0));
        assert!(shot < projectile && projectile < radius && radius < HarmOrigin::Melee);
        assert!(HarmOrigin::Shot(BulletNo(1)) < HarmOrigin::Shot(BulletNo(2)));
    }

    #[test]
    fn a_match_starts_in_warmup() {
        assert_eq!(GameRoundStage::default(), GameRoundStage::Warmup);
    }
}
