use super::{CONTENT_DIGEST_SCHEME, ContentComponents, content_components, content_digest};
use crate::identities::ScriptModelId;
use crate::spawn::{AuthoredSpawnPoint, MatchBootstrap};
use crate::world::SimBrush;
use crate::{
    AuthorityDObjCollision, AuthorityDObjCollisionBone, AuthorityDObjState, AuthorityModelOwner, EntityCollisionCapabilities,
    EquipmentRuntimeFacts, LinkedBrushCollisionBrush,
};
use weapon_t6::combat_facts::WeaponCombatFacts;
use weapon_t6::damage::DamageSteps;
use weapon_t6::dual_mag::DualMagTimes;

// What each part hashes to when there is nothing to hash; worked out independently of this code.
const EMPTY_MAP: u64 = 0x9809_6551_263e_b548;
const EMPTY_MODELS: u64 = 0x1d37_67fa_e067_f313;
const EMPTY_WEAPONS: u64 = 0x9d52_c031_d43b_70e6;
const CLASS_CATALOG: u64 = 0xc94a_bf9c_aa04_e4d1;
const EMPTY_WHOLE: u64 = 0x7c30_6e41_1caa_ec3a;

/// All seven inputs of a digest, owned.
pub(super) struct Content {
    pub combat: Vec<WeaponCombatFacts>,
    pub runnable: Vec<bool>,
    pub transition_groups: Vec<u32>,
    pub equipment: Vec<EquipmentRuntimeFacts>,
    pub bootstrap: MatchBootstrap,
    pub brushes: Vec<SimBrush>,
    pub models: Vec<EntityCollisionCapabilities>,
}

impl Content {
    pub fn nothing() -> Self {
        Self {
            combat: Vec::new(),
            runnable: Vec::new(),
            transition_groups: Vec::new(),
            equipment: Vec::new(),
            bootstrap: MatchBootstrap::default(),
            brushes: Vec::new(),
            models: Vec::new(),
        }
    }

    pub fn whole(&self) -> u64 {
        content_digest(
            &self.combat,
            &self.runnable,
            &self.transition_groups,
            &self.equipment,
            &self.bootstrap,
            &self.brushes,
            &self.models,
        )
    }

    pub fn parts(&self) -> ContentComponents {
        content_components(
            &self.combat,
            &self.runnable,
            &self.transition_groups,
            &self.equipment,
            &self.bootstrap,
            &self.brushes,
            &self.models,
        )
    }
}

fn spawn(classname: &str, origin: [f32; 3], angles: [f32; 3]) -> AuthoredSpawnPoint {
    AuthoredSpawnPoint {
        classname: classname.to_owned(),
        origin,
        angles,
        script_linkto: String::new(),
        script_destructable_area: String::new(),
    }
}

fn brush(planes: Vec<[f32; 4]>, contents: u32, surface_flags: Vec<u32>, glass: u16) -> SimBrush {
    SimBrush {
        planes,
        contents,
        plane_surface_flags: surface_flags,
        glass_encoded: glass,
        verts: Vec::new(),
        hit_surface_flags: 0,
    }
}

fn bone(index: u16) -> AuthorityDObjCollisionBone {
    let n = f32::from(index);
    AuthorityDObjCollisionBone {
        bone: index,
        part_classification: (index % 200) as u8,
        center: [n, n + 0.25, n + 0.5],
        axes: [[1.0, n, 0.0], [n + 2.0, 1.0, 0.0], [0.0, n + 3.0, 1.0]],
        half_size: [n + 4.0, n + 5.0, n + 6.0],
    }
}

fn model_with(
    id: u32,
    brushes: Vec<LinkedBrushCollisionBrush>,
    dobj: Option<AuthorityDObjState>,
) -> EntityCollisionCapabilities {
    EntityCollisionCapabilities::current_tick(
        AuthorityModelOwner::ScriptModel(ScriptModelId(id)),
        dobj,
        brushes,
    )
}

fn posed_model(name: &str, bones: Vec<AuthorityDObjCollisionBone>) -> AuthorityDObjState {
    let mut dobj = AuthorityDObjState::new_dirty(name.to_owned(), None, glam::Mat4::IDENTITY);
    dobj.current_collision = Some(AuthorityDObjCollision { bones, coll: None });
    dobj
}

/// Every field in these rows holds a value of its own, so a pair of swapped fields shows up.
pub(super) fn populated() -> Content {
    let mut next = 0_i32;
    let mut int = move || {
        next += 7;
        next
    };

    let mut combat = WeaponCombatFacts::none();
    combat.fire_time_ms = int();
    combat.intro_fire_time_ms = int();
    combat.intro_fire_length = int();
    combat.fire_delay_ms = int();
    combat.raise_time_ms = int();
    combat.drop_time_ms = int();
    combat.alternate_weapon = int() as u32;
    combat.alternate_raise_time_ms = int();
    combat.alternate_drop_time_ms = int();
    combat.quick_drop_time_ms = int();
    combat.inventory_type = int();
    combat.weap_class = int();
    combat.player_anim_type = int();
    combat.select_requires_ammo = Some(true);
    combat.offhand_hold_is_cancelable = Some(false);
    combat.reload_time_ms = int();
    combat.reload_empty_time_ms = int();
    combat.clip_size = int();
    combat.start_ammo = int();
    combat.max_ammo = int();
    combat.ammo_index = int();
    combat.clip_index = int();
    combat.fire_type = int();
    combat.impact_type = int();
    combat.shots_per_fire = int();
    combat.burst_cooldown_ms = int();
    combat.bolt_action = true;
    combat.rechamber_time_ms = int();
    combat.rechamber_bolt_time_ms = int();
    combat.rechamber_bolt_delay_ms = int();
    combat.segmented_reload = true;
    combat.reload_start_time_ms = int();
    combat.reload_end_time_ms = int();
    combat.reload_ammo_add = int();
    combat.reload_add_time_ms = int();
    combat.reload_empty_add_time_ms = int();
    combat.reload_start_add_time_ms = int();
    combat.reload_start_add = int();
    combat.no_partial_reload = true;
    combat.dual_mag = Some(DualMagTimes {
        reload_ms: int(),
        reload_empty_ms: int(),
        add_ms: int(),
        empty_add_ms: int(),
    });
    combat.inherits_perks = true;
    combat.spread_before_fire_add = true;
    combat.sprint_raise_time_ms = int();
    combat.sprint_drop_time_ms = int();
    combat.damage = int();
    combat.min_damage = int();
    combat.max_damage_range = int() as f32 + 0.5;
    combat.min_damage_range = int() as f32 + 0.5;
    combat.damage_steps = Some(DamageSteps {
        damage: [int(), int(), int(), int(), int(), int()],
        range: [1.25, 2.25, 3.25, 4.25, 5.25, 6.25].map(|r| r + int() as f32),
        range_scale: int() as f32 + 0.75,
    });
    combat.hip_spread_stand_min = int() as f32 + 0.1;
    combat.hip_spread_ducked_min = int() as f32 + 0.1;
    combat.hip_spread_prone_min = int() as f32 + 0.1;
    combat.hip_spread_stand_max = int() as f32 + 0.1;
    combat.hip_spread_ducked_max = int() as f32 + 0.1;
    combat.hip_spread_prone_max = int() as f32 + 0.1;
    combat.hip_spread_decay_rate = int() as f32 + 0.1;
    combat.hip_spread_fire_add = int() as f32 + 0.1;
    combat.hip_spread_turn_add = int() as f32 + 0.1;
    combat.hip_spread_move_add = int() as f32 + 0.1;
    combat.hip_spread_ducked_decay = int() as f32 + 0.1;
    combat.hip_spread_prone_decay = int() as f32 + 0.1;
    combat.ads_spread = int() as f32 + 0.1;
    combat.aim_down_sight = true;
    combat.no_ads_when_mag_empty = true;
    combat.ads_in_rate = int() as f32 + 0.2;
    combat.ads_out_rate = int() as f32 + 0.2;
    combat.rechamber_while_ads = false;
    combat.reload_lowers_ads = true;
    combat.position_reload_trans_ms = int();
    combat.ads_fire_only = true;
    combat.melee_damage = int();
    combat.overlay_reticle = int();
    combat.melee_time_ms = int();
    combat.melee_delay_ms = int();
    combat.melee_charge_time_ms = int();
    combat.melee_charge_delay_ms = int();
    combat.knife_model = int() as u32;
    combat.quick_raise_time_ms = int();
    for slot in &mut combat.location_damage {
        *slot = int() as f32 + 0.3;
    }
    let plain = WeaponCombatFacts { fire_time_ms: 33, ..WeaponCombatFacts::none() };

    let mut grenade = EquipmentRuntimeFacts::default();
    grenade.offhand_class = int();
    grenade.start_ammo = int();
    grenade.clip_size = int();
    grenade.impact_damage = int();
    grenade.fuse_time_ms = int();
    grenade.hold_fire_time_ms = int();
    grenade.cook_off_hold = true;
    grenade.has_detonator = true;
    grenade.detonate_delay_ms = int();
    grenade.detonate_time_ms = int();
    grenade.projectile_rotates = true;
    grenade.projectile_rotate_type = int();
    grenade.stickiness = int();
    grenade.timed_detonation = true;
    grenade.explode_on_ground = true;
    grenade.proj_impact_explode = true;
    grenade.laser_sensor = true;
    grenade.stick_to_players = true;
    grenade.explosion_radius = int();
    grenade.explosion_radius_min = int();
    grenade.explosion_inner_damage = int();
    grenade.explosion_outer_damage = int();
    grenade.damage_cone_angle = int() as f32 + 0.4;
    grenade.missile_guidance = int();
    grenade.max_steering_accel = int() as f32 + 0.4;
    grenade.ignition_delay_ms = int();
    grenade.require_lock_to_fire = true;
    grenade.projectile_speed = int();
    grenade.projectile_speed_up = int();
    grenade.projectile_speed_forward = int();
    grenade.projectile_speed_relative_up = int();
    grenade.projectile_parent_velocity = Some(int() as f32 + 0.6);
    grenade.rolling_radius = Some(int() as f32 + 0.6);
    grenade.projectile_activate_dist = int();
    grenade.projectile_explosion_type = int();
    grenade.spawned_grenade_weapon = int() as u32;
    grenade.weap_type = int();
    grenade.weap_class = int();
    grenade.parallel_bounce = Some(core::array::from_fn(|i| i as f32 + 0.5));
    grenade.perpendicular_bounce = Some(core::array::from_fn(|i| i as f32 * 2.0 + 0.25));

    let mut bootstrap = MatchBootstrap::default();
    bootstrap.spawns = vec![
        spawn("mp_dm_spawn", [10.0, 20.0, 30.0], [0.0, 90.0, 0.0]),
        spawn("mp_tdm_spawn_axis_start", [-5.0, 6.5, 7.0], [1.0, 2.0, 3.0]),
    ];

    let brushes = vec![
        brush(
            vec![[1.0, 0.0, 0.0, 64.0], [0.0, 1.0, 0.0, -32.0], [0.0, 0.0, 1.0, 8.0]],
            0x1,
            vec![0x10, 0x20, 0x30],
            0,
        ),
        brush(vec![[0.0, 0.0, -1.0, 0.0]], 0x8001, vec![0x4], 0x1234),
    ];

    let models = vec![
        model_with(
            3,
            vec![LinkedBrushCollisionBrush {
                cmodel_handle: 9,
                origin: [1.0, 2.0, 3.0],
                angles: [4.0, 5.0, 6.0],
            }],
            Some(posed_model("com_plasticcase_beige_big", vec![bone(1), bone(2)])),
        ),
        model_with(4, Vec::new(), None),
    ];

    Content {
        combat: vec![combat, plain],
        runnable: vec![true, false, true],
        transition_groups: vec![5, 0, 11],
        equipment: vec![grenade, EquipmentRuntimeFacts::default()],
        bootstrap,
        brushes,
        models,
    }
}

// ---- the recorded values ----

#[test]
fn the_scheme_is_the_one_the_goldens_were_made_for() {
    assert_eq!(CONTENT_DIGEST_SCHEME, 16);
}

#[test]
fn empty_content_hashes_to_the_recorded_values() {
    let nothing = Content::nothing();
    assert_eq!(
        nothing.parts(),
        ContentComponents {
            map: EMPTY_MAP,
            models: EMPTY_MODELS,
            weapons: EMPTY_WEAPONS,
            classes: CLASS_CATALOG,
        }
    );
    assert_eq!(nothing.whole(), EMPTY_WHOLE);
}

#[test]
fn the_default_components_are_all_zero() {
    let zero = ContentComponents::default();
    assert_eq!((zero.map, zero.models, zero.weapons, zero.classes), (0, 0, 0, 0));
}

// ---- which input feeds which part ----

fn assert_only(changed: &Content, base: &Content, expect: [bool; 4]) {
    let (before, after) = (base.parts(), changed.parts());
    assert_eq!(
        [
            before.map != after.map,
            before.models != after.models,
            before.weapons != after.weapons,
            before.classes != after.classes,
        ],
        expect,
        "[map, models, weapons, classes] that moved"
    );
    assert_ne!(base.whole(), changed.whole(), "the whole digest must move too");
}

#[test]
fn weapon_inputs_move_only_the_weapons_part() {
    let base = populated();

    let mut changed = populated();
    changed.combat[1].fire_time_ms += 1;
    assert_only(&changed, &base, [false, false, true, false]);

    let mut changed = populated();
    changed.runnable[1] = true;
    assert_only(&changed, &base, [false, false, true, false]);

    let mut changed = populated();
    changed.transition_groups.push(2);
    assert_only(&changed, &base, [false, false, true, false]);

    let mut changed = populated();
    changed.equipment[1].clip_size += 1;
    assert_only(&changed, &base, [false, false, true, false]);
}

#[test]
fn map_inputs_move_only_the_map_part() {
    let base = populated();

    let mut changed = populated();
    changed.bootstrap.spawns[0].origin[2] += 1.0;
    assert_only(&changed, &base, [true, false, false, false]);

    let mut changed = populated();
    changed.brushes[1].contents ^= 0x2;
    assert_only(&changed, &base, [true, false, false, false]);
}

#[test]
fn script_models_move_only_the_models_part() {
    let base = populated();
    let mut changed = populated();
    changed.models.pop();
    assert_only(&changed, &base, [false, true, false, false]);
}

#[test]
fn what_the_digest_leaves_out_does_not_move_it() {
    let base = populated();
    let mut changed = populated();
    changed.bootstrap.spawns[0].script_linkto = "somewhere".to_owned();
    changed.bootstrap.seed = 99;
    changed.brushes[0].hit_surface_flags = 0xffff;
    changed.brushes[0].verts.push([1.0, 2.0, 3.0]);
    changed.equipment[0].ballistic_blade = !changed.equipment[0].ballistic_blade;
    changed.equipment[0].limit_grenade_impacts = !changed.equipment[0].limit_grenade_impacts;
    changed.combat[0].quick_switch = !changed.combat[0].quick_switch;
    changed.combat[0].optic_switch = !changed.combat[0].optic_switch;
    assert_eq!(changed.parts(), base.parts());
    assert_eq!(changed.whole(), base.whole());
}

#[test]
fn the_same_content_hashes_the_same_twice() {
    assert_eq!(populated().whole(), populated().whole());
    assert_eq!(populated().parts(), populated().parts());
}

// ---- every hashed field takes part ----

trait Nudge {
    fn nudged(self) -> Self;
}

impl Nudge for i32 {
    fn nudged(self) -> Self {
        self + 1
    }
}

impl Nudge for u32 {
    fn nudged(self) -> Self {
        self + 1
    }
}

impl Nudge for bool {
    fn nudged(self) -> Self {
        !self
    }
}

impl Nudge for f32 {
    fn nudged(self) -> Self {
        self + 1.0
    }
}

/// Changes each named field of `$base` in turn and checks the digest of `$rows` moves.
macro_rules! every_field_counts {
    ($base:expr, $digest:expr, [$($field:ident),+ $(,)?]) => {{
        let base = $base;
        let before = $digest(&base);
        $(
            let mut changed = base;
            changed.$field = changed.$field.nudged();
            assert_ne!($digest(&changed), before, "`{}` does not reach the digest", stringify!($field));
        )+
    }};
}

fn weapons_of_one_combat_row(row: &WeaponCombatFacts) -> u64 {
    let mut content = Content::nothing();
    content.combat.push(*row);
    content.parts().weapons
}

fn weapons_of_one_equipment_row(row: &EquipmentRuntimeFacts) -> u64 {
    let mut content = Content::nothing();
    content.equipment.push(*row);
    content.parts().weapons
}

#[test]
fn every_scalar_combat_field_reaches_the_digest() {
    every_field_counts!(
        populated().combat[0],
        weapons_of_one_combat_row,
        [
            fire_time_ms,
            intro_fire_time_ms,
            intro_fire_length,
            fire_delay_ms,
            raise_time_ms,
            drop_time_ms,
            alternate_weapon,
            alternate_raise_time_ms,
            alternate_drop_time_ms,
            quick_drop_time_ms,
            inventory_type,
            weap_class,
            player_anim_type,
            reload_time_ms,
            reload_empty_time_ms,
            clip_size,
            start_ammo,
            max_ammo,
            ammo_index,
            clip_index,
            fire_type,
            impact_type,
            shots_per_fire,
            burst_cooldown_ms,
            bolt_action,
            rechamber_time_ms,
            rechamber_bolt_time_ms,
            rechamber_bolt_delay_ms,
            segmented_reload,
            reload_start_time_ms,
            reload_end_time_ms,
            reload_ammo_add,
            reload_add_time_ms,
            reload_empty_add_time_ms,
            reload_start_add_time_ms,
            reload_start_add,
            no_partial_reload,
            inherits_perks,
            spread_before_fire_add,
            sprint_raise_time_ms,
            sprint_drop_time_ms,
            damage,
            min_damage,
            max_damage_range,
            min_damage_range,
            hip_spread_stand_min,
            hip_spread_ducked_min,
            hip_spread_prone_min,
            hip_spread_stand_max,
            hip_spread_ducked_max,
            hip_spread_prone_max,
            hip_spread_decay_rate,
            hip_spread_fire_add,
            hip_spread_turn_add,
            hip_spread_move_add,
            hip_spread_ducked_decay,
            hip_spread_prone_decay,
            ads_spread,
            aim_down_sight,
            no_ads_when_mag_empty,
            ads_in_rate,
            ads_out_rate,
            rechamber_while_ads,
            reload_lowers_ads,
            position_reload_trans_ms,
            ads_fire_only,
            melee_damage,
            overlay_reticle,
            melee_time_ms,
            melee_delay_ms,
            melee_charge_time_ms,
            melee_charge_delay_ms,
            knife_model,
            quick_raise_time_ms,
        ]
    );
}

#[test]
fn the_optional_combat_fields_reach_the_digest() {
    let base = populated().combat[0];
    let before = weapons_of_one_combat_row(&base);

    // The three states of an optional flag all differ from each other.
    let flag_states = [None, Some(false), Some(true)].map(|state| {
        let mut row = base;
        row.select_requires_ammo = state;
        weapons_of_one_combat_row(&row)
    });
    assert_ne!(flag_states[0], flag_states[1]);
    assert_ne!(flag_states[1], flag_states[2]);
    assert_ne!(flag_states[0], flag_states[2]);

    let flag_states = [None, Some(false), Some(true)].map(|state| {
        let mut row = base;
        row.offhand_hold_is_cancelable = state;
        weapons_of_one_combat_row(&row)
    });
    assert_ne!(flag_states[0], flag_states[1]);
    assert_ne!(flag_states[1], flag_states[2]);
    assert_ne!(flag_states[0], flag_states[2]);

    let mut row = base;
    row.dual_mag = None;
    assert_ne!(weapons_of_one_combat_row(&row), before);
    for field in 0..4 {
        let mut row = base;
        let times = row.dual_mag.as_mut().unwrap();
        *[&mut times.reload_ms, &mut times.reload_empty_ms, &mut times.add_ms, &mut times.empty_add_ms][field] += 1;
        assert_ne!(weapons_of_one_combat_row(&row), before, "dual mag field {field}");
    }

    let mut row = base;
    row.damage_steps = None;
    assert_ne!(weapons_of_one_combat_row(&row), before);
    for step in 0..6 {
        let mut row = base;
        row.damage_steps.as_mut().unwrap().damage[step] += 1;
        assert_ne!(weapons_of_one_combat_row(&row), before, "damage step {step}");
        let mut row = base;
        row.damage_steps.as_mut().unwrap().range[step] += 1.0;
        assert_ne!(weapons_of_one_combat_row(&row), before, "range step {step}");
    }
    let mut row = base;
    row.damage_steps.as_mut().unwrap().range_scale += 1.0;
    assert_ne!(weapons_of_one_combat_row(&row), before);

    for slot in 0..base.location_damage.len() {
        let mut row = base;
        row.location_damage[slot] += 1.0;
        assert_ne!(weapons_of_one_combat_row(&row), before, "location damage {slot}");
    }
}

#[test]
fn every_scalar_equipment_field_reaches_the_digest() {
    every_field_counts!(
        populated().equipment[0],
        weapons_of_one_equipment_row,
        [
            offhand_class,
            start_ammo,
            clip_size,
            impact_damage,
            fuse_time_ms,
            hold_fire_time_ms,
            cook_off_hold,
            has_detonator,
            detonate_delay_ms,
            detonate_time_ms,
            projectile_rotates,
            projectile_rotate_type,
            stickiness,
            timed_detonation,
            explode_on_ground,
            proj_impact_explode,
            laser_sensor,
            stick_to_players,
            explosion_radius,
            explosion_radius_min,
            explosion_inner_damage,
            explosion_outer_damage,
            damage_cone_angle,
            missile_guidance,
            max_steering_accel,
            ignition_delay_ms,
            require_lock_to_fire,
            projectile_speed,
            projectile_speed_up,
            projectile_speed_forward,
            projectile_speed_relative_up,
            projectile_activate_dist,
            projectile_explosion_type,
            spawned_grenade_weapon,
            weap_type,
            weap_class,
        ]
    );
}

#[test]
fn the_optional_equipment_fields_reach_the_digest() {
    let base = populated().equipment[0];
    let before = weapons_of_one_equipment_row(&base);

    let mut row = base;
    row.projectile_parent_velocity = None;
    assert_ne!(weapons_of_one_equipment_row(&row), before);
    let mut row = base;
    row.projectile_parent_velocity = Some(0.0);
    let zero = weapons_of_one_equipment_row(&row);
    row.projectile_parent_velocity = None;
    assert_ne!(weapons_of_one_equipment_row(&row), zero, "Some(0.0) is not the same as None");

    let mut row = base;
    row.rolling_radius = None;
    assert_ne!(weapons_of_one_equipment_row(&row), before);
    let mut row = base;
    row.rolling_radius = Some(row.rolling_radius.unwrap() + 1.0);
    assert_ne!(weapons_of_one_equipment_row(&row), before);

    let mut row = base;
    row.parallel_bounce = None;
    assert_ne!(weapons_of_one_equipment_row(&row), before);
    let mut row = base;
    row.perpendicular_bounce = None;
    assert_ne!(weapons_of_one_equipment_row(&row), before);
    for slot in [0, 15, 30] {
        let mut row = base;
        row.parallel_bounce.as_mut().unwrap()[slot] += 1.0;
        assert_ne!(weapons_of_one_equipment_row(&row), before, "parallel bounce {slot}");
        let mut row = base;
        row.perpendicular_bounce.as_mut().unwrap()[slot] += 1.0;
        assert_ne!(weapons_of_one_equipment_row(&row), before, "perpendicular bounce {slot}");
    }
}

#[test]
fn the_order_of_rows_matters() {
    let base = populated();
    let mut swapped = populated();
    swapped.combat.swap(0, 1);
    assert_ne!(swapped.parts().weapons, base.parts().weapons);

    let mut swapped = populated();
    swapped.equipment.swap(0, 1);
    assert_ne!(swapped.parts().weapons, base.parts().weapons);

    let mut swapped = populated();
    swapped.bootstrap.spawns.swap(0, 1);
    assert_ne!(swapped.parts().map, base.parts().map);

    let mut swapped = populated();
    swapped.models.swap(0, 1);
    assert_ne!(swapped.parts().models, base.parts().models);
}

#[test]
fn admission_lists_are_told_apart_by_which_list_a_value_is_in() {
    let mut one = Content::nothing();
    one.runnable = vec![true];
    let mut other = Content::nothing();
    other.transition_groups = vec![1];
    assert_ne!(one.parts().weapons, other.parts().weapons);
}

// ---- the map ----

#[test]
fn every_spawn_and_brush_field_reaches_the_digest() {
    let base = populated();
    let before = base.parts().map;
    let moved = |edit: &dyn Fn(&mut Content)| {
        let mut changed = populated();
        edit(&mut changed);
        assert_ne!(changed.parts().map, before);
    };

    moved(&|c| c.bootstrap.spawns[1].classname.push('x'));
    for axis in 0..3 {
        moved(&|c| c.bootstrap.spawns[1].origin[axis] += 1.0);
        moved(&|c| c.bootstrap.spawns[1].angles[axis] += 1.0);
    }
    moved(&|c| {
        c.bootstrap.spawns.pop();
    });
    for component in 0..4 {
        moved(&|c| c.brushes[0].planes[2][component] += 1.0);
    }
    moved(&|c| c.brushes[0].planes.pop().map_or((), drop));
    moved(&|c| c.brushes[0].contents += 1);
    moved(&|c| c.brushes[0].plane_surface_flags[1] += 1);
    moved(&|c| c.brushes[0].plane_surface_flags.pop().map_or((), drop));
    moved(&|c| c.brushes[1].glass_encoded += 1);
    moved(&|c| {
        c.brushes.pop();
    });
}

// ---- script models ----

#[test]
fn every_script_model_field_reaches_the_digest() {
    let before = populated().parts().models;
    let moved = |edit: &dyn Fn(&mut Content)| {
        let mut changed = populated();
        edit(&mut changed);
        assert_ne!(changed.parts().models, before);
    };

    moved(&|c| c.models[0].owner = AuthorityModelOwner::ScriptModel(ScriptModelId(99)));
    moved(&|c| c.models[0].linked_brushes[0].cmodel_handle += 1);
    for axis in 0..3 {
        moved(&|c| c.models[0].linked_brushes[0].origin[axis] += 1.0);
        moved(&|c| c.models[0].linked_brushes[0].angles[axis] += 1.0);
    }
    moved(&|c| c.models[0].linked_brushes.clear());
    moved(&|c| c.models[0].dobj = None);
    moved(&|c| c.models[0].dobj.as_mut().unwrap().current_model.push('x'));
    moved(&|c| c.models[0].dobj.as_mut().unwrap().model_revision += 1);
    moved(&|c| c.models[0].dobj.as_mut().unwrap().pose_revision += 1);
    let on_bone = |c: &mut Content, edit: &dyn Fn(&mut AuthorityDObjCollisionBone)| {
        let collision = c.models[0].dobj.as_mut().unwrap().current_collision.as_mut().unwrap();
        edit(&mut collision.bones[1]);
    };
    moved(&|c| on_bone(c, &|b| b.bone += 1));
    moved(&|c| on_bone(c, &|b| b.part_classification += 1));
    for axis in 0..3 {
        moved(&|c| on_bone(c, &|b| b.center[axis] += 1.0));
        moved(&|c| on_bone(c, &|b| b.half_size[axis] += 1.0));
        for row in 0..3 {
            moved(&|c| on_bone(c, &|b| b.axes[row][axis] += 1.0));
        }
    }
    moved(&|c| {
        c.models[0].dobj.as_mut().unwrap().current_collision.as_mut().unwrap().bones.pop();
    });
    moved(&|c| c.models[0].dobj.as_mut().unwrap().current_collision = None);
}
