use super::{CONTENT_PRINT_REVISION, ContentPartDigests, content_part_fingerprints, content_fingerprint};
use crate::match_ids::ScriptedModel;
use crate::spawn_pick::{MapSpawnSpot, MatchStartSetup};
use crate::sim_parts::SolidBrush;
use crate::{
    ServerModelCollision, ServerModelState, ServerModelSource, EntityHitAbilities,
    GadgetLiveStats, AttachedSolidShape,
};
use weapon_t6::combat_facts::GunBattleProfile;
use weapon_t6::damage::DamageSteps;
use weapon_t6::dual_mag::DualMagTimes;

// What each part hashes to when there is nothing to hash; worked out independently of this code.
const EMPTY_MAP: u64 = 0x888a_f7f5_9111_18db;
const EMPTY_MODELS: u64 = 0xe4b7_a2bf_3bb2_cfd0;
const EMPTY_WEAPONS: u64 = 0x5bc5_a072_c10b_8139;
const CLASS_CATALOG: u64 = 0x816b_a00c_1731_2f55;
const EMPTY_WHOLE: u64 = 0xd504_4f42_bb5b_9474;

/// All seven inputs of a digest, owned.
pub(super) struct Content {
    pub combat: Vec<GunBattleProfile>,
    pub runnable: Vec<bool>,
    pub transition_groups: Vec<u32>,
    pub equipment: Vec<GadgetLiveStats>,
    pub opening: MatchStartSetup,
    pub brushes: Vec<SolidBrush>,
    pub hit_rows: Vec<EntityHitAbilities>,
}

impl Content {
    pub fn nothing() -> Self {
        Self {
            combat: Vec::new(),
            runnable: Vec::new(),
            transition_groups: Vec::new(),
            equipment: Vec::new(),
            opening: MatchStartSetup::default(),
            brushes: Vec::new(),
            hit_rows: Vec::new(),
        }
    }

    pub fn whole(&self) -> u64 {
        content_fingerprint(
            &self.combat,
            &self.runnable,
            &self.transition_groups,
            &self.equipment,
            &self.opening,
            &self.brushes,
            &self.hit_rows,
        )
    }

    pub fn parts(&self) -> ContentPartDigests {
        content_part_fingerprints(
            &self.combat,
            &self.runnable,
            &self.transition_groups,
            &self.equipment,
            &self.opening,
            &self.brushes,
            &self.hit_rows,
        )
    }
}

fn spawn(kind_label: &str, spot: [f32; 3], aim: [f32; 3]) -> MapSpawnSpot {
    MapSpawnSpot {
        kind_name: kind_label.to_owned(),
        origin: spot,
        turn: aim,
        script_tied: String::new(),
        script_destructable_area: String::new(),
    }
}

fn brush(planes: Vec<[f32; 4]>, stuff_bits: u32, surface_flags: Vec<u32>, glass: u16) -> SolidBrush {
    SolidBrush {
        sides: planes,
        solid_bits: stuff_bits,
        plane_material_flags: surface_flags,
        glass_hit_code: glass,
        vertex_points: Vec::new(),
        hit_surface_flags: 0,
    }
}

fn bone(index: u16) -> xmodel_runtime::HitShapeBone {
    let n = f32::from(index);
    xmodel_runtime::HitShapeBone {
        bone: index,
        body_part_category: (index % 200) as u8,
        center: [n, n + 0.25, n + 0.5],
        axes: [[1.0, n, 0.0], [n + 2.0, 1.0, 0.0], [0.0, n + 3.0, 1.0]],
        half_ext: [n + 4.0, n + 5.0, n + 6.0],
    }
}

fn model_with(
    id: u32,
    brushes: Vec<AttachedSolidShape>,
    dobj: Option<ServerModelState>,
) -> EntityHitAbilities {
    EntityHitAbilities::now_tick(
        ServerModelSource::ScriptModel(ScriptedModel(id)),
        dobj,
        brushes,
    )
}

fn posed_model(name: &str, bones: Vec<xmodel_runtime::HitShapeBone>) -> ServerModelState {
    let mut dobj = ServerModelState::new_dirty(name.to_owned(), None, glam::Mat4::IDENTITY);
    dobj.live_hitbox = Some(ServerModelCollision { bones, coll: None, face_bits: 0 });
    dobj
}

/// Every field in these rows holds a value of its own, so a pair of swapped fields shows up.
pub(super) fn populated() -> Content {
    let mut next = 0_i32;
    let mut int = move || {
        next += 7;
        next
    };

    let mut combat = GunBattleProfile::none();
    combat.shot_gap_ms = int();
    combat.intro_fire_time_ms = int();
    combat.intro_fire_length = int();
    combat.shot_lead_ms = int();
    combat.draw_ms = int();
    combat.holster_ms = int();
    combat.alternate_weapon = int() as u32;
    combat.alternate_raise_time_ms = int();
    combat.alternate_drop_time_ms = int();
    combat.fast_holster_ms = int();
    combat.carry_slot_kind = int();
    combat.gun_kind = int();
    combat.player_motion_kind = int();
    combat.select_requires_ammo = Some(true);
    combat.offhand_hold_is_cancelable = Some(false);
    combat.reload_duration_millis = int();
    combat.refill_dry_millis = int();
    combat.magazine_capacity = int();
    combat.spawn_rounds = int();
    combat.reserve_cap = int();
    combat.ammo_type_slot = int();
    combat.motion_clip_slot = int();
    combat.trigger_mode = int();
    combat.impact_kind_id = int();
    combat.rounds_each_trigger = int();
    combat.volley_rest_millis = int();
    combat.manual_cycle = true;
    combat.cycle_ms = int();
    combat.bolt_rack_duration_millis = int();
    combat.bolt_rack_wait_millis = int();
    combat.staged_reload = true;
    combat.refill_begin_millis = int();
    combat.refill_finish_millis = int();
    combat.reload_rounds_gained = int();
    combat.reload_refill_millis = int();
    combat.empty_reload_refill_millis = int();
    combat.reload_begin_refill_millis = int();
    combat.reload_begin_bonus = int();
    combat.forbid_topup_reload = true;
    combat.dual_mag = Some(DualMagTimes {
        reload_ms: int(),
        reload_empty_ms: int(),
        add_ms: int(),
        empty_add_ms: int(),
    });
    combat.carries_over_talents = true;
    combat.spread_before_fire_add = true;
    combat.run_lift_millis = int();
    combat.run_lower_millis = int();
    combat.damage = int();
    combat.hurt_floor = int();
    combat.full_damage_reach = int() as f32 + 0.5;
    combat.minimum_damage_reach = int() as f32 + 0.5;
    combat.damage_steps = Some(DamageSteps {
        damage: [int(), int(), int(), int(), int(), int()],
        range: [1.25, 2.25, 3.25, 4.25, 5.25, 6.25].map(|r| r + int() as f32),
        range_scale: int() as f32 + 0.75,
    });
    combat.hip_cone_up_lo = int() as f32 + 0.1;
    combat.hip_cone_crouch_lo = int() as f32 + 0.1;
    combat.hip_cone_lie_lo = int() as f32 + 0.1;
    combat.hip_cone_up_hi = int() as f32 + 0.1;
    combat.hip_cone_crouch_hi = int() as f32 + 0.1;
    combat.hip_cone_lie_hi = int() as f32 + 0.1;
    combat.hip_cone_settle = int() as f32 + 0.1;
    combat.hip_cone_per_shot = int() as f32 + 0.1;
    combat.hip_cone_per_turn = int() as f32 + 0.1;
    combat.hip_cone_per_step = int() as f32 + 0.1;
    combat.hip_cone_crouch_settle = int() as f32 + 0.1;
    combat.hip_cone_lie_settle = int() as f32 + 0.1;
    combat.aimed_cone = int() as f32 + 0.1;
    combat.scoped_aiming = true;
    combat.block_aim_if_clip_dry = true;
    combat.scope_raise_speed = int() as f32 + 0.2;
    combat.scope_lower_speed = int() as f32 + 0.2;
    combat.rack_bolt_when_scoped = false;
    combat.reload_lowers_ads = true;
    combat.position_reload_trans_ms = int();
    combat.fire_only_when_scoped = true;
    combat.bash_hurt = int();
    combat.hud_crosshair_layer = int();
    combat.bash_ms = int();
    combat.strike_wait_millis = int();
    combat.bash_windup_ms = int();
    combat.lunge_windup_wait_millis = int();
    combat.blade_mesh = int() as u32;
    combat.fast_draw_ms = int();
    for slot in &mut combat.location_damage {
        *slot = int() as f32 + 0.3;
    }
    let plain = GunBattleProfile { shot_gap_ms: 33, ..GunBattleProfile::none() };

    let mut grenade = GadgetLiveStats::default();
    grenade.secondary_hand_category = int();
    grenade.opening_ammo = int();
    grenade.magazine_capacity = int();
    grenade.collision_harm = int();
    grenade.detonation_delay_millis = int();
    grenade.trigger_hold_millis = int();
    grenade.grenade_cookoff_wait = true;
    grenade.has_detonator = true;
    grenade.detonate_delay_ms = int();
    grenade.detonate_time_ms = int();
    grenade.projectile_rotates = true;
    grenade.projectile_rotate_type = int();
    grenade.stickiness = int();
    grenade.timed_detonation = true;
    grenade.explode_on_ground = true;
    grenade.shell_detonate_on_hit = true;
    grenade.laser_sensor = true;
    grenade.adheres_to_bodies = true;
    grenade.blast_outer_reach = int();
    grenade.blast_reach_floor = int();
    grenade.blast_core_damage = int();
    grenade.blast_rim_damage = int();
    grenade.damage_cone_angle = int() as f32 + 0.4;
    grenade.missile_guidance = int();
    grenade.max_steering_accel = int() as f32 + 0.4;
    grenade.ignition_delay_ms = int();
    grenade.require_lock_to_fire = true;
    grenade.missile_velocity = int();
    grenade.missile_upward_velocity = int();
    grenade.projectile_speed_forward = int();
    grenade.projectile_speed_relative_up = int();
    grenade.projectile_parent_velocity = Some(int() as f32 + 0.6);
    grenade.rolling_radius = Some(int() as f32 + 0.6);
    grenade.shell_arming_range = int();
    grenade.shell_blast_kind = int();
    grenade.spawned_grenade_weapon = int() as u32;
    grenade.gun_family_code = int();
    grenade.class_code = int();
    grenade.along_rebound = Some(core::array::from_fn(|i| i as f32 + 0.5));
    grenade.across_rebound = Some(core::array::from_fn(|i| i as f32 * 2.0 + 0.25));

    let mut opening = MatchStartSetup::default();
    opening.spawns = vec![
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
            vec![AttachedSolidShape {
                collision_model_ref: 9,
                origin: [1.0, 2.0, 3.0],
                turn: [4.0, 5.0, 6.0],
            }],
            Some(posed_model("com_plasticcase_beige_big", vec![bone(1), bone(2)])),
        ),
        model_with(4, Vec::new(), None),
    ];

    Content {
        combat: vec![combat, plain],
        runnable: vec![true, false, true],
        transition_groups: vec![5, 0, 11],
        equipment: vec![grenade, GadgetLiveStats::default()],
        opening,
        brushes,
        hit_rows: models,
    }
}

// ---- the recorded values ----

#[test]
fn the_scheme_is_the_one_the_goldens_were_made_for() {
    assert_eq!(CONTENT_PRINT_REVISION, 17);
}

#[test]
fn empty_content_hashes_to_the_recorded_values() {
    let nothing = Content::nothing();
    assert_eq!(
        nothing.parts(),
        ContentPartDigests {
            map: EMPTY_MAP,
            meshes: EMPTY_MODELS,
            guns: EMPTY_WEAPONS,
            kits: CLASS_CATALOG,
        }
    );
    assert_eq!(nothing.whole(), EMPTY_WHOLE);
}

#[test]
fn the_default_components_are_all_zero() {
    let zero = ContentPartDigests::default();
    assert_eq!((zero.map, zero.meshes, zero.guns, zero.kits), (0, 0, 0, 0));
}

// ---- which input feeds which part ----

fn assert_only(changed: &Content, base: &Content, expect: [bool; 4]) {
    let (before, after) = (base.parts(), changed.parts());
    assert_eq!(
        [
            before.map != after.map,
            before.meshes != after.meshes,
            before.guns != after.guns,
            before.kits != after.kits,
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
    changed.combat[1].shot_gap_ms += 1;
    assert_only(&changed, &base, [false, false, true, false]);

    let mut changed = populated();
    changed.runnable[1] = true;
    assert_only(&changed, &base, [false, false, true, false]);

    let mut changed = populated();
    changed.transition_groups.push(2);
    assert_only(&changed, &base, [false, false, true, false]);

    let mut changed = populated();
    changed.equipment[1].magazine_capacity += 1;
    assert_only(&changed, &base, [false, false, true, false]);
}

#[test]
fn map_inputs_move_only_the_map_part() {
    let base = populated();

    let mut changed = populated();
    changed.opening.spawns[0].origin[2] += 1.0;
    assert_only(&changed, &base, [true, false, false, false]);

    let mut changed = populated();
    changed.brushes[1].solid_bits ^= 0x2;
    assert_only(&changed, &base, [true, false, false, false]);
}

#[test]
fn script_models_move_only_the_models_part() {
    let base = populated();
    let mut changed = populated();
    changed.hit_rows.pop();
    assert_only(&changed, &base, [false, true, false, false]);
}

#[test]
fn what_the_digest_leaves_out_does_not_move_it() {
    let base = populated();
    let mut changed = populated();
    changed.opening.spawns[0].script_tied = "somewhere".to_owned();
    changed.opening.root = 99;
    changed.brushes[0].hit_surface_flags = 0xffff;
    changed.brushes[0].vertex_points.push([1.0, 2.0, 3.0]);
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

fn weapons_of_one_combat_row(row: &GunBattleProfile) -> u64 {
    let mut content = Content::nothing();
    content.combat.push(*row);
    content.parts().guns
}

fn weapons_of_one_equipment_row(row: &GadgetLiveStats) -> u64 {
    let mut content = Content::nothing();
    content.equipment.push(*row);
    content.parts().guns
}

#[test]
fn every_scalar_combat_field_reaches_the_digest() {
    every_field_counts!(
        populated().combat[0],
        weapons_of_one_combat_row,
        [
            shot_gap_ms,
            intro_fire_time_ms,
            intro_fire_length,
            charge_shot,
            shot_lead_ms,
            draw_ms,
            holster_ms,
            alternate_weapon,
            alternate_raise_time_ms,
            alternate_drop_time_ms,
            fast_holster_ms,
            carry_slot_kind,
            gun_kind,
            player_motion_kind,
            reload_duration_millis,
            refill_dry_millis,
            magazine_capacity,
            spawn_rounds,
            reserve_cap,
            ammo_type_slot,
            motion_clip_slot,
            trigger_mode,
            impact_kind_id,
            rounds_each_trigger,
            volley_rest_millis,
            manual_cycle,
            cycle_ms,
            bolt_rack_duration_millis,
            bolt_rack_wait_millis,
            staged_reload,
            refill_begin_millis,
            refill_finish_millis,
            reload_rounds_gained,
            reload_refill_millis,
            empty_reload_refill_millis,
            reload_begin_refill_millis,
            reload_begin_bonus,
            forbid_topup_reload,
            unlimited_ammo,
            carries_over_talents,
            spread_before_fire_add,
            flat_spread,
            run_lift_millis,
            run_lower_millis,
            damage,
            hurt_floor,
            full_damage_reach,
            minimum_damage_reach,
            hip_cone_up_lo,
            hip_cone_crouch_lo,
            hip_cone_lie_lo,
            hip_cone_up_hi,
            hip_cone_crouch_hi,
            hip_cone_lie_hi,
            hip_cone_settle,
            hip_cone_per_shot,
            hip_cone_per_turn,
            hip_cone_per_step,
            hip_cone_crouch_settle,
            hip_cone_lie_settle,
            aimed_cone,
            scoped_aiming,
            block_aim_if_clip_dry,
            scope_raise_speed,
            scope_lower_speed,
            rack_bolt_when_scoped,
            reload_lowers_ads,
            position_reload_trans_ms,
            fire_only_when_scoped,
            bash_hurt,
            hud_crosshair_layer,
            bash_ms,
            strike_wait_millis,
            bash_windup_ms,
            lunge_windup_wait_millis,
            blade_mesh,
            fast_draw_ms,
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
            secondary_hand_category,
            opening_ammo,
            magazine_capacity,
            collision_harm,
            detonation_delay_millis,
            trigger_hold_millis,
            grenade_cookoff_wait,
            has_detonator,
            detonate_delay_ms,
            detonate_time_ms,
            projectile_rotates,
            projectile_rotate_type,
            stickiness,
            timed_detonation,
            explode_on_ground,
            shell_detonate_on_hit,
            bullet_impact_explode,
            force_bounce,
            laser_sensor,
            adheres_to_bodies,
            blast_outer_reach,
            blast_reach_floor,
            blast_core_damage,
            blast_rim_damage,
            damage_cone_angle,
            missile_guidance,
            max_steering_accel,
            ignition_delay_ms,
            require_lock_to_fire,
            missile_velocity,
            missile_upward_velocity,
            projectile_speed_forward,
            projectile_speed_relative_up,
            shell_arming_range,
            shell_blast_kind,
            spawned_grenade_weapon,
            gun_family_code,
            class_code,
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
    row.along_rebound = None;
    assert_ne!(weapons_of_one_equipment_row(&row), before);
    let mut row = base;
    row.across_rebound = None;
    assert_ne!(weapons_of_one_equipment_row(&row), before);
    for slot in [0, 15, 30] {
        let mut row = base;
        row.along_rebound.as_mut().unwrap()[slot] += 1.0;
        assert_ne!(weapons_of_one_equipment_row(&row), before, "parallel bounce {slot}");
        let mut row = base;
        row.across_rebound.as_mut().unwrap()[slot] += 1.0;
        assert_ne!(weapons_of_one_equipment_row(&row), before, "perpendicular bounce {slot}");
    }
}

#[test]
fn the_order_of_rows_matters() {
    let base = populated();
    let mut swapped = populated();
    swapped.combat.swap(0, 1);
    assert_ne!(swapped.parts().guns, base.parts().guns);

    let mut swapped = populated();
    swapped.equipment.swap(0, 1);
    assert_ne!(swapped.parts().guns, base.parts().guns);

    let mut swapped = populated();
    swapped.opening.spawns.swap(0, 1);
    assert_ne!(swapped.parts().map, base.parts().map);

    let mut swapped = populated();
    swapped.hit_rows.swap(0, 1);
    assert_ne!(swapped.parts().meshes, base.parts().meshes);
}

#[test]
fn admission_lists_are_told_apart_by_which_list_a_value_is_in() {
    let mut one = Content::nothing();
    one.runnable = vec![true];
    let mut other = Content::nothing();
    other.transition_groups = vec![1];
    assert_ne!(one.parts().guns, other.parts().guns);
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

    moved(&|c| c.opening.spawns[1].kind_name.push('x'));
    for axis in [0, 1, 2] {
        moved(&|c| c.opening.spawns[1].origin[axis] += 1.0);
        moved(&|c| c.opening.spawns[1].turn[axis] += 1.0);
    }
    moved(&|c| {
        c.opening.spawns.pop();
    });
    for piece in 0..4 {
        moved(&|c| c.brushes[0].sides[2][piece] += 1.0);
    }
    moved(&|c| c.brushes[0].sides.pop().map_or((), drop));
    moved(&|c| c.brushes[0].solid_bits += 1);
    moved(&|c| c.brushes[0].plane_material_flags[1] += 1);
    moved(&|c| c.brushes[0].plane_material_flags.pop().map_or((), drop));
    moved(&|c| c.brushes[1].glass_hit_code += 1);
    moved(&|c| {
        c.brushes.pop();
    });
}

// ---- script models ----

#[test]
fn every_script_model_field_reaches_the_digest() {
    let before = populated().parts().meshes;
    let moved = |edit: &dyn Fn(&mut Content)| {
        let mut changed = populated();
        edit(&mut changed);
        assert_ne!(changed.parts().meshes, before);
    };

    moved(&|c| c.hit_rows[0].owner = ServerModelSource::ScriptModel(ScriptedModel(99)));
    moved(&|c| c.hit_rows[0].attached_solids[0].collision_model_ref += 1);
    for axis in [0, 1, 2] {
        moved(&|c| c.hit_rows[0].attached_solids[0].origin[axis] += 1.0);
        moved(&|c| c.hit_rows[0].attached_solids[0].turn[axis] += 1.0);
    }
    moved(&|c| c.hit_rows[0].attached_solids.clear());
    moved(&|c| c.hit_rows[0].dobj = None);
    moved(&|c| c.hit_rows[0].dobj.as_mut().unwrap().active_mesh.push('x'));
    moved(&|c| c.hit_rows[0].dobj.as_mut().unwrap().model_version_stamp += 1);
    moved(&|c| c.hit_rows[0].dobj.as_mut().unwrap().pose_rev += 1);
    let on_bone = |c: &mut Content, edit: &dyn Fn(&mut xmodel_runtime::HitShapeBone)| {
        let collision = c.hit_rows[0].dobj.as_mut().unwrap().live_hitbox.as_mut().unwrap();
        edit(&mut collision.bones[1]);
    };
    moved(&|c| on_bone(c, &|b| b.bone += 1));
    moved(&|c| on_bone(c, &|b| b.body_part_category += 1));
    for axis in [0, 1, 2] {
        moved(&|c| on_bone(c, &|b| b.center[axis] += 1.0));
        moved(&|c| on_bone(c, &|b| b.half_ext[axis] += 1.0));
        for row in [0, 1, 2] {
            moved(&|c| on_bone(c, &|b| b.axes[row][axis] += 1.0));
        }
    }
    moved(&|c| {
        c.hit_rows[0].dobj.as_mut().unwrap().live_hitbox.as_mut().unwrap().bones.pop();
    });
    moved(&|c| c.hit_rows[0].dobj.as_mut().unwrap().live_hitbox = None);
}
