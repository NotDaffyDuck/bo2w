//! What a weapon file says about a thrown, launched or planted item, boiled down to the numbers the flight code reads.

/// `secondary_hand_category` value of smoke grenades.
pub(super) const OFFHAND_SMOKE: i32 = 2;
/// `weap_class` value of throwing knives.
const WEAP_CLASS_KNIFE: i32 = 9;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GadgetLiveStats {
    // --- loadout side
    pub secondary_hand_category: i32,
    /// bo2zm: Black Ops II's `offhandSlot` (1 lethal, 2 tactical), 0 = by class.
    pub offhand_slot: i32,
    pub opening_ammo: i32,
    pub magazine_capacity: i32,
    pub gun_family_code: i32,
    pub class_code: i32,

    // --- fuse and detonation
    pub detonation_delay_millis: i32,
    pub trigger_hold_millis: i32,
    pub grenade_cookoff_wait: bool,
    pub has_detonator: bool,
    pub detonate_delay_ms: i32,
    pub detonate_time_ms: i32,
    pub timed_detonation: bool,
    /// The fuse only counts once the item lies on a floor.
    pub explode_on_ground: bool,
    pub shell_detonate_on_hit: bool,
    /// bo2zm: Black Ops II's `bBulletImpactExplode`: its bullet explodes
    /// where it hits (`combat::bullet_impact_blast`).
    pub bullet_impact_explode: bool,
    /// bo2zm: Black Ops II's `bForceBounce`: a rocket (projectile) that
    /// bounces off what it hits; any other rocket that does not explode on
    /// impact hits and is gone (`G_FireRocket` sets the bounce flag only
    /// from this; `G_InitGrenadeEntity` always sets it for grenades).
    pub force_bounce: bool,
    /// The sensor grenade's laser.
    pub laser_sensor: bool,

    // --- contact
    pub collision_harm: i32,
    /// A second impact on the same target in a row does no damage.
    pub limit_grenade_impacts: bool,
    pub stickiness: i32,
    pub adheres_to_bodies: bool,
    pub ballistic_blade: bool,
    /// The weapon whose grenade a bolt leaves behind where it sticks (0: none).
    pub spawned_grenade_weapon: u32,
    pub along_rebound: Option<[f32; 31]>,
    pub across_rebound: Option<[f32; 31]>,
    /// Radius a rolling grenade lies and rolls on, instead of its centre.
    pub rolling_radius: Option<f32>,

    // --- blast
    pub blast_outer_reach: i32,
    pub blast_reach_floor: i32,
    pub blast_core_damage: i32,
    pub blast_rim_damage: i32,
    pub damage_cone_angle: f32,
    pub shell_blast_kind: i32,

    // --- launch
    pub missile_velocity: i32,
    pub missile_upward_velocity: i32,
    pub projectile_speed_forward: i32,
    pub projectile_speed_relative_up: i32,
    /// `Some` marks a row from this game's weapon files; the number itself is only read for vehicles.
    pub projectile_parent_velocity: Option<f32>,
    pub projectile_rotates: bool,
    /// 1: end-over-end spin of a blade.
    pub projectile_rotate_type: i32,
    pub shell_arming_range: i32,

    // --- guidance
    pub missile_guidance: i32,
    pub max_steering_accel: f32,
    pub ignition_delay_ms: i32,
    pub require_lock_to_fire: bool,
}

impl GadgetLiveStats {
    /// bo2zm: the class its offhand key looks for. Black Ops II gives a
    /// class-1 grenade in slot 2 (the Hell's Retriever) to the tactical key.
    pub fn select_class(&self) -> i32 {
        if self.offhand_slot == 2 && self.secondary_hand_category == 1 { 3 } else { self.secondary_hand_category }
    }

    /// bo2zm: Black Ops II's `MissileImpact` bounces a missile only when its
    /// bounce flag is set: always for a grenade (`G_InitGrenadeEntity`), and
    /// for a rocket only with `bForceBounce` (`G_FireRocket`). Any other
    /// rocket hits what it struck and is gone: the Ray Gun Mark II's bolt
    /// (no splash, no impact explode) kills one zombie and vanishes.
    pub(crate) fn rocket_ends_on_impact(self) -> bool {
        self.gun_family_code == i32::from(weapon_t6::facts::gun_family_code::PROJECTILE) && !self.force_bounce
    }

    /// Something that can actually be thrown or fired and then does something.
    pub fn ready_for_use(self) -> bool {
        let leaves_hand = self.missile_velocity > 0 || self.stickiness != 0;
        let does_something =
            self.detonation_delay_millis > 0 || self.collision_harm > 0 || self.blast_core_damage > 0;
        leaves_hand && does_something
    }

    pub(crate) fn is_throwing_knife(self) -> bool {
        self.class_code == WEAP_CLASS_KNIFE
    }

    /// Knives and blades come back out of the wall as pickups.
    pub(crate) fn is_retrievable_knife(self) -> bool {
        self.is_throwing_knife() || self.ballistic_blade
    }

    pub fn is_secondary_gear(self) -> bool {
        self.secondary_hand_category != 0
    }

    pub fn starting_magazine_count(self) -> i32 {
        self.opening_ammo.max(self.magazine_capacity).max(1)
    }
}
