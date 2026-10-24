//! What a weapon file says about a thrown, launched or planted item, boiled down to the numbers the flight code reads.

/// `offhand_class` value of smoke grenades.
pub(super) const OFFHAND_SMOKE: i32 = 2;
/// `weap_class` value of throwing knives.
const WEAP_CLASS_KNIFE: i32 = 9;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EquipmentRuntimeFacts {
    // --- loadout side
    pub offhand_class: i32,
    pub start_ammo: i32,
    pub clip_size: i32,
    pub weap_type: i32,
    pub weap_class: i32,

    // --- fuse and detonation
    pub fuse_time_ms: i32,
    pub hold_fire_time_ms: i32,
    pub cook_off_hold: bool,
    pub has_detonator: bool,
    pub detonate_delay_ms: i32,
    pub detonate_time_ms: i32,
    pub timed_detonation: bool,
    /// The fuse only counts once the item lies on a floor.
    pub explode_on_ground: bool,
    pub proj_impact_explode: bool,
    /// The sensor grenade's laser.
    pub laser_sensor: bool,

    // --- contact
    pub impact_damage: i32,
    /// A second impact on the same target in a row does no damage.
    pub limit_grenade_impacts: bool,
    pub stickiness: i32,
    pub stick_to_players: bool,
    pub ballistic_blade: bool,
    /// The weapon whose grenade a bolt leaves behind where it sticks (0: none).
    pub spawned_grenade_weapon: u32,
    pub parallel_bounce: Option<[f32; 31]>,
    pub perpendicular_bounce: Option<[f32; 31]>,
    /// Radius a rolling grenade lies and rolls on, instead of its centre.
    pub rolling_radius: Option<f32>,

    // --- blast
    pub explosion_radius: i32,
    pub explosion_radius_min: i32,
    pub explosion_inner_damage: i32,
    pub explosion_outer_damage: i32,
    pub damage_cone_angle: f32,
    pub projectile_explosion_type: i32,

    // --- launch
    pub projectile_speed: i32,
    pub projectile_speed_up: i32,
    pub projectile_speed_forward: i32,
    pub projectile_speed_relative_up: i32,
    /// `Some` marks a row from this game's weapon files; the number itself is only read for vehicles.
    pub projectile_parent_velocity: Option<f32>,
    pub projectile_rotates: bool,
    /// 1: end-over-end spin of a blade.
    pub projectile_rotate_type: i32,
    pub projectile_activate_dist: i32,

    // --- guidance
    pub missile_guidance: i32,
    pub max_steering_accel: f32,
    pub ignition_delay_ms: i32,
    pub require_lock_to_fire: bool,
}

impl EquipmentRuntimeFacts {
    /// Something that can actually be thrown or fired and then does something.
    pub fn is_usable(self) -> bool {
        let leaves_hand = self.projectile_speed > 0 || self.stickiness != 0;
        let does_something =
            self.fuse_time_ms > 0 || self.impact_damage > 0 || self.explosion_inner_damage > 0;
        leaves_hand && does_something
    }

    pub(crate) fn is_throwing_knife(self) -> bool {
        self.weap_class == WEAP_CLASS_KNIFE
    }

    /// Knives and blades come back out of the wall as pickups.
    pub(crate) fn is_retrievable_knife(self) -> bool {
        self.is_throwing_knife() || self.ballistic_blade
    }

    pub fn is_offhand(self) -> bool {
        self.offhand_class != 0
    }

    pub fn spawn_clip_count(self) -> i32 {
        self.start_ammo.max(self.clip_size).max(1)
    }
}
