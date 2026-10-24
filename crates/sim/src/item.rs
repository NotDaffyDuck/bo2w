use crate::frame::FrameWorld;
use anim_t6::bg_rand;
use entity_t6::{
    Trajectory, evaluate_trajectory, trajectory::tr_type::GRAVITY as TR_GRAVITY,
    trajectory::tr_type::STATIONARY as TR_STATIONARY,
};
use crate::entity_local::truncated_tr_delta;
use game_math::angle_vectors;
use playerstate_t6::game::{ENTITYNUM_NONE, PERK_SCAVENGER, PM_TYPE_DEAD, PlayerState};
use weapon_t6::ammo_table::{ammo_table_key, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, player_weapons_find_slot, set_ammo_not_in_clip, set_clip_for_hand};

use crate::bullet_collision::{MASK_PLAYER_SOLID, PLAYER_MAXS, PLAYER_MINS};
use crate::gentity::init_item_state;
use crate::world::{ClientId, Tick, give_weapon_to_ps_akimbo, gsc_give_weapon_is_akimbo};
use crate::{ClientLifecycle, ItemPickupRecord};

pub const G_MAX_DROPPED_WEAPONS: usize = 16;

pub const G_DROP_FORWARD_SPEED: f32 = 10.0;
pub const G_DROP_UP_SPEED_BASE: f32 = 10.0;

pub const G_DROP_UP_SPEED_RAND: f32 = 5.0;

pub const G_DROP_HORZ_SPEED_RAND: f32 = 100.0;

pub const WEAP_INVENTORY_PRIMARY: i32 = 0;

pub const ITEM_MINS: [f32; 3] = [0.0, 0.0, 0.0];
pub const ITEM_MAXS: [f32; 3] = [1.0, 1.0, 1.0];

/// The top of his box, which in BO2 follows his stance (ClientThink_real
/// copies the move's box onto him): 70 standing, 50 crouched or in last
/// stand, 30 prone.
pub(crate) fn stance_maxs_z(ps: &PlayerState) -> f32 {
    use movement_t6::bridge::{CROUCH_MAXS_Z, PRONE_MAXS_Z, STAND_MAXS_Z, StanceSurface};
    match movement_t6::bridge::stance_surface_type(ps) {
        StanceSurface::Stand => STAND_MAXS_Z,
        StanceSurface::Crouch | StanceSurface::LastStand => CROUCH_MAXS_Z,
        StanceSurface::Prone => PRONE_MAXS_Z,
    }
}

/// Drop_Item: a dropped gun starts halfway up his box: 35 standing, 25
/// crouched or in last stand, 15 prone.
fn drop_height(ps: &PlayerState) -> f32 {
    (stance_maxs_z(ps) - PLAYER_MINS[2]) * 0.5
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroppedItem {
    pub state: entity_t6::EntityState,
    pub origin: [f32; 3],
    pub falling: bool,
    pub clip_r: i32,
    pub clip_l: i32,
    pub stock: i32,
    /// BO2: the second slot's stock (an alt mode's or left hand's pool).
    pub stock2: i32,
    pub scavenger: bool,
    /// BO2 `LaunchItem`: the dropper cannot touch or use it before this
    /// level time (`DroppedItemClearOwner`, 1000 ms on); 0 on legacy maps.
    pub owner_clear_ms: i32,
}

pub fn random_unit(seed: &mut u32) -> f32 {
    bg_rand(seed) as f32 * (1.0 / 32768.0)
}

pub fn random_signed(seed: &mut u32) -> f32 {
    let bits = bg_rand(seed) as f32;
    let unit = bits * (1.0 / 32768.0);
    unit + unit - 1.0
}

pub fn drop_item_velocity(yaw_deg: f32, seed: &mut u32) -> [f32; 3] {
    let (forward, _, _) = angle_vectors([0.0, yaw_deg, 0.0]);
    let mut velocity = [
        forward[0] * G_DROP_FORWARD_SPEED,
        forward[1] * G_DROP_FORWARD_SPEED,
        forward[2] * G_DROP_FORWARD_SPEED,
    ];
    velocity[2] += random_signed(seed) * G_DROP_UP_SPEED_RAND + G_DROP_UP_SPEED_BASE;
    velocity[0] += random_signed(seed) * G_DROP_HORZ_SPEED_RAND;
    velocity[1] += random_signed(seed) * G_DROP_HORZ_SPEED_RAND;
    velocity
}

fn may_drop_weapon(world: &FrameWorld, ps: &PlayerState, weapon: u32) -> bool {
    if weapon == 0 {
        return false;
    }
    if player_weapons_find_slot(&ps.weapons, weapon as i32) < 0 {
        return false;
    }
    let name = world.weapon_script_name(weapon);
    if name.contains("ac130") {
        return false;
    }
    let Some(facts) = world.combat_facts_for(weapon) else {
        return false;
    };
    if facts.inventory_type != WEAP_INVENTORY_PRIMARY {
        return false;
    }
    if name.contains("riotshield") {
        return true;
    }
    let clip_key = clip_table_key(facts.clip_index, weapon);
    let clip_r = get_clip_for_hand(&ps.ammoclip, clip_key, 0);
    let clip_l = get_clip_for_hand(&ps.ammoclip, clip_key, 1);
    if clip_r == 0 && clip_l == 0 {
        return false;
    }
    let ammo_key = ammo_table_key(facts.ammo_index, weapon);
    let stock = get_ammo_not_in_clip(&ps.ammo, ammo_key);
    clip_r != 0 || clip_l != 0 || stock != 0
}

fn take_player_weapon(ps: &mut PlayerState, weapon: u32) {
    let want = weapon as i32;
    for slot in &mut ps.weapons {
        if *slot == want {
            *slot = 0;
        }
    }
    if ps.weapon == weapon || ps.weapon_primary == weapon {
        ps.weapon = 0;
        ps.weapon_primary = 0;
    }
}

fn ammo_from_ps(world: &FrameWorld, ps: &PlayerState, weapon: u32) -> (i32, i32, i32) {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return (0, 0, 0);
    };
    let clip_key = clip_table_key(facts.clip_index, weapon);
    let ammo_key = ammo_table_key(facts.ammo_index, weapon);
    (
        get_clip_for_hand(&ps.ammoclip, clip_key, 0),
        get_clip_for_hand(&ps.ammoclip, clip_key, 1),
        get_ammo_not_in_clip(&ps.ammo, ammo_key),
    )
}

fn set_ammo_on_ps(
    world: &FrameWorld,
    ps: &mut PlayerState,
    weapon: u32,
    clip_r: i32,
    clip_l: i32,
    stock: i32,
) {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return;
    };
    let clip_key = clip_table_key(facts.clip_index, weapon);
    let ammo_key = ammo_table_key(facts.ammo_index, weapon);
    if clip_key != 0 {
        let _ = set_clip_for_hand(&mut ps.ammoclip, clip_key, 0, clip_r);
        let _ = set_clip_for_hand(&mut ps.ammoclip, clip_key, 1, clip_l);
    }
    if ammo_key != 0 {
        let _ = set_ammo_not_in_clip(&mut ps.ammo, ammo_key, stock);
    }
}

fn add_ammo_on_ps(
    world: &FrameWorld,
    ps: &mut PlayerState,
    weapon: u32,
    clip_r: i32,
    clip_l: i32,
    stock: i32,
) {
    let (have_r, have_l, have_stock) = ammo_from_ps(world, ps, weapon);

    let Some(facts) = world.combat_facts_for(weapon) else {
        return;
    };
    set_ammo_on_ps(
        world,
        ps,
        weapon,
        have_r,
        have_l,
        have_stock
            .saturating_add(stock)
            .saturating_add(clip_r)
            .saturating_add(clip_l)
            .min(facts.max_ammo),
    );
}

fn current_primary_weapon(world: &FrameWorld, ps: &PlayerState) -> u32 {
    let weapon = if world
        .combat_facts_for(ps.weapon)
        .is_some_and(|f| f.inventory_type == 3)
    {
        ps.weapon_primary
    } else {
        ps.weapon
    };
    if weapon == 0 {
        return 0;
    }
    if player_weapons_find_slot(&ps.weapons, weapon as i32) < 0 {
        return 0;
    }
    match world.combat_facts_for(weapon) {
        Some(facts) if facts.inventory_type == WEAP_INVENTORY_PRIMARY => weapon,
        _ => 0,
    }
}

pub(crate) fn aabb_overlap(
    a_origin: [f32; 3],
    a_mins: [f32; 3],
    a_maxs: [f32; 3],
    b_origin: [f32; 3],
    b_mins: [f32; 3],
    b_maxs: [f32; 3],
) -> bool {
    for i in 0..3 {
        let a_min = a_origin[i] + a_mins[i];
        let a_max = a_origin[i] + a_maxs[i];
        let b_min = b_origin[i] + b_mins[i];
        let b_max = b_origin[i] + b_maxs[i];
        if a_min > b_max || a_max < b_min {
            return false;
        }
    }
    true
}

pub(crate) fn walker_can_touch(world: &FrameWorld, id: ClientId, ps: &PlayerState) -> bool {
    if !world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        return false;
    }
    if ps.health < 1 {
        return false;
    }
    if ps.pm_type >= PM_TYPE_DEAD {
        return false;
    }
    true
}

pub(crate) fn has_scavenger_perk(ps: &PlayerState) -> bool {
    (ps.perks[0] & PERK_SCAVENGER) != 0
}

pub(crate) fn push_dropped_item(
    world: &mut FrameWorld,
    weapon: u32,
    origin: [f32; 3],
    pos: Trajectory,
    apos: Trajectory,
    owner: i32,
    clip_r: i32,
    clip_l: i32,
    stock: i32,
    falling: bool,
    scavenger: bool,
) -> i32 {
    if world.dropped_item_count() >= G_MAX_DROPPED_WEAPONS {
        let evicted_number = world
            .dropped_item_numbers_sorted()
            .into_iter()
            .next()
            .expect("cap eviction requires an occupied dropped item");
        let evicted = world
            .remove_dropped_item_by_number(evicted_number)
            .expect("cap eviction number vanished");
        world.free_dynamic_entity_number(evicted.state.number);
    }
    let entnum = match world.allocate_dynamic_entity(crate::gentity::EntityRunKind::Item) {
        Ok(entity) => entity.number(),
        Err(error) => {
            diag::warn!(Sim, "dropped item not spawned: {error:?}");
            return ENTITYNUM_NONE;
        }
    };
    let state = init_item_state(entnum, weapon, pos, apos, owner);
    world.push_dropped_item(DroppedItem {
        state,
        origin,
        falling,
        clip_r,
        clip_l,
        stock,
        stock2: 0,
        scavenger,
        owner_clear_ms: 0,
    });
    entnum
}

pub(crate) fn launch_dropped_from_ps(
    world: &mut FrameWorld,
    tick: Tick,
    ps: &PlayerState,
    owner: i32,
    weapon: u32,
    clip_r: i32,
    clip_l: i32,
    stock: i32,
    scavenger: bool,
) -> i32 {
    let mut seed = world.anim_event_seed();
    // LaunchItem keeps only the whole units of the throw's speed.
    let velocity = truncated_tr_delta(drop_item_velocity(ps.viewangles[1], &mut seed));
    world.set_anim_event_seed(seed);

    let origin = [ps.origin[0], ps.origin[1], ps.origin[2] + drop_height(ps)];
    let time_ms = crate::corpse::level_time_ms(tick);
    let pos = Trajectory {
        tr_type: TR_GRAVITY,
        tr_time: time_ms,
        tr_duration: 0,
        tr_delta: velocity,
        tr_base: origin,
    };
    let apos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: 0,
        tr_duration: 0,
        tr_delta: [0.0; 3],
        tr_base: [0.0, ps.viewangles[1], 0.0],
    };
    push_dropped_item(
        world, weapon, origin, pos, apos, owner, clip_r, clip_l, stock, true, scavenger,
    )
}

pub(crate) fn drop_weapon(
    world: &mut FrameWorld,
    tick: Tick,
    player: ClientId,
    weapon: u32,
) -> Option<i32> {
    let ps = world.player(player).copied()?;
    if !ps.weapons.contains(&(weapon as i32)) || !may_drop_weapon(world, &ps, weapon) {
        return None;
    }
    let (clip_r, clip_l, stock) = ammo_from_ps(world, &ps, weapon);
    let number = launch_dropped_from_ps(
        world,
        tick,
        &ps,
        player.0 as i32,
        weapon,
        clip_r,
        clip_l,
        stock,
        false,
    );
    if number == ENTITYNUM_NONE {
        return None;
    }
    if let Some(ps) = world.player_mut(player) {
        take_player_weapon(ps, weapon);
    }
    Some(number)
}

pub(crate) fn drop_scavenger_item(
    world: &mut FrameWorld,
    tick: Tick,
    player: ClientId,
    weapon: u32,
) -> Option<i32> {
    let ps = world.player(player).copied()?;
    let number = launch_dropped_from_ps(world, tick, &ps, player.0 as i32, weapon, 0, 0, 0, true);
    (number != ENTITYNUM_NONE).then_some(number)
}

pub(crate) fn think_item_move(world: &mut FrameWorld, time_ms: i32, number: i32) {
    if let Ok(entity) = world.entity_kernel().current_ref(number) {
        if world
            .entity_kernel()
            .resolve(entity)
            .is_ok_and(|view| view.relations.parent.is_some())
        {
            return;
        }
    }
    // bo2mp: with BO2's weapon facts a gun falls and lands as BO2's G_RunItem has it.
    if !world.gun_facts().is_empty() {
        crate::item_t6::run_item(world, time_ms, number);
        return;
    }
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    if !item.falling {
        return;
    }
    let traj = Trajectory {
        tr_time: item.state.lerp.pos.tr_time,
        tr_type: item.state.lerp.pos.tr_type,
        tr_duration: item.state.lerp.pos.tr_duration,
        tr_delta: item.state.lerp.pos.tr_delta,
        tr_base: item.state.lerp.pos.tr_base,
    };
    let desired = evaluate_trajectory(&traj, time_ms);
    let hit = world.trace_clip(
        item.origin,
        desired,
        ITEM_MINS,
        ITEM_MAXS,
        MASK_PLAYER_SOLID,
    );
    let mut fraction = hit.fraction;
    if hit.startsolid {
        fraction = 0.0;
    }
    let endpos = if fraction >= 1.0 { desired } else { hit.end_pos(item.origin, desired) };
    let Some(item) = world.dropped_item_mut_by_number(number) else {
        return;
    };
    item.origin = endpos;
    if fraction >= 1.0 {
        return;
    }
    // bo2mp lane F: an item dropped where its box already touches the world
    // (startsolid) lies where it is, and one that meets a wall drops
    // straight down from there; both used to stay put on the server while
    // their trajectory fell on through the floor on every client.
    if hit.allsolid || hit.startsolid || hit.normal[2] > 0.0 {
        item.falling = false;
        item.state.lerp.pos.tr_type = TR_STATIONARY;
        item.state.lerp.pos.tr_base = endpos;
        item.state.lerp.pos.tr_delta = [0.0; 3];
        item.state.lerp.pos.tr_time = 0;
        item.state.lerp.pos.tr_duration = 0;
    } else {
        item.state.lerp.pos.tr_base = endpos;
        item.state.lerp.pos.tr_delta = [0.0; 3];
        item.state.lerp.pos.tr_time = time_ms;
    }
}

pub(crate) fn phase_touch_items(world: &mut FrameWorld, _tick: Tick) {
    // bo2mp: with BO2's weapon facts the touch follows BO2 (`crate::item_t6`).
    if !world.gun_facts().is_empty() {
        crate::item_t6::phase_touch(world);
        return;
    }
    let mut walkers = world.client_ids_sorted();
    walkers.sort_unstable_by_key(|id| id.0);
    for walker in walkers {
        try_touch_one(world, walker);
    }
}

fn try_touch_one(world: &mut FrameWorld, walker: ClientId) {
    let Some(ps) = world.player(walker).copied() else {
        return;
    };
    if !walker_can_touch(world, walker, &ps) {
        return;
    }
    let candidates = world.dropped_item_numbers_sorted();
    let mut hit = None;
    for number in candidates {
        let Some(item) = world.dropped_item_by_number(number) else {
            continue;
        };
        if !aabb_overlap(
            ps.origin,
            PLAYER_MINS,
            PLAYER_MAXS,
            item.origin,
            ITEM_MINS,
            ITEM_MAXS,
        ) {
            continue;
        }

        if !item.scavenger && !ps.weapons.contains(&(item.state.weapon.0 as i32)) {
            continue;
        }
        if !item.scavenger {
            let weapon = item.state.weapon.0;
            let Some(facts) = world.combat_facts_for(weapon) else {
                continue;
            };
            let (_, _, stock) = ammo_from_ps(world, &ps, weapon);
            if stock >= facts.max_ammo || item.stock + item.clip_r + item.clip_l <= 0 {
                continue;
            }
        }
        if item.scavenger && !has_scavenger_perk(&ps) {
            continue;
        }
        hit = Some(number);
        break;
    }
    let Some(number) = hit else {
        return;
    };
    grab_number(world, walker, number);
}

pub(crate) fn grab_number(world: &mut FrameWorld, walker: ClientId, number: i32) {
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    if item.scavenger {
        grab_scavenger(world, walker, number);
        return;
    }
    let weapon = u32::try_from(item.state.index).unwrap_or(0);
    if weapon == 0 {
        return;
    }
    let Some(ps) = world.player(walker).copied() else {
        return;
    };
    let picker_pm_type = ps.pm_type;
    let already_has = player_weapons_find_slot(&ps.weapons, weapon as i32) >= 0;
    world.remove_dropped_item_by_number(number);
    world.free_dynamic_entity_number(item.state.number);
    let mut swapped_entnum = ENTITYNUM_NONE;
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(weapon));
    if already_has {
        let mut next = ps;
        weapon_t6::ammo_table::latch_weapon_dual_wield(&next.weapons, &mut next.weapon_data, weapon, akimbo);
        if next.weapon == weapon {
            next.last_weapon_hand =
                weapon_t6::ammo_table::num_hands_for_held(&next.weapons, &next.weapon_data, weapon);
        }
        add_ammo_on_ps(
            world,
            &mut next,
            weapon,
            item.clip_r,
            item.clip_l,
            item.stock,
        );
        if let Some(slot) = world.player_mut(walker) {
            *slot = next;
        }
    } else {
        let current = current_primary_weapon(world, &ps);
        if current != 0 && primary_count(world, &ps) >= 2 {
            swapped_entnum = drop_current_primary_at(
                world,
                walker,
                current,
                item.origin,
                item.state.lerp.apos.tr_base,
            );
        }
        if let Some(mut next) = world.player(walker).copied() {
            give_weapon_to_ps_akimbo(&mut next, weapon, akimbo);

            if let Some(facts) = world.combat_facts_for(weapon) {
                let hand = weapon_t6::combat_facts::spawn_weapon_hand(weapon, &facts);
                next.weaponstate_primary = hand.weaponstate;
                next.weapon_time = hand.weapon_time;
                next.weapon_delay = hand.weapon_delay;
                next.weap_anim = hand.weap_anim;
                next.weaponstate_secondary = hand.weaponstate;
                next.weapon_time_secondary = hand.weapon_time;
                next.weapon_delay_secondary = hand.weapon_delay;
                next.weap_anim_secondary = hand.weap_anim;
            }
            set_ammo_on_ps(
                world,
                &mut next,
                weapon,
                item.clip_r,
                item.clip_l,
                item.stock,
            );
            if let Some(slot) = world.player_mut(walker) {
                *slot = next;
            }
        }
    }
    let next = world.player(walker).copied().expect("picker exists");
    let (clip, _, stock) = ammo_from_ps(world, &next, weapon);
    {
        let meta = world.client_meta_mut(walker);
        meta.ammo_by_weapon
            .retain(|(id, _, _)| next.weapons.contains(&(*id as i32)));
        meta.taped_mag_spent
            .retain(|id| next.weapons.contains(&(*id as i32)));
        meta.set_ammo(weapon, clip, stock);
        meta.mirror_held_ammo(next.weapon);
    }
    if !already_has {
        let meta = world.client_meta_mut(walker);
        meta.set_quick_reload_ready(weapon, true);
        meta.weapon_shot_count = 0;
        meta.burst_latch = false;
        meta.burst_latch_secondary = false;
        meta.rechamber_pending = false;
        meta.rechamber_pending_secondary = false;
        meta.t6_gun = None;
    }
    world.item_pickups_mut().push(ItemPickupRecord {
        picker: walker.0 as i32,
        weapon,
        from_entnum: item.state.number,
        clip_r: item.clip_r,
        clip_l: item.clip_l,
        stock: item.stock,
        swapped_entnum,
        picker_pm_type,
        ammo_only: false,
    });
    let tick = Tick((world.entity_kernel().level_time_ms() / 50) as u32);
    world.push_entity_event(
        tick,
        crate::EventAudience::All,
        entity_t6::EntityEventKind::ITEM_PICKUP,
        crate::EntityEventPayload {
            number: walker.0 as i32,
            event_parm: weapon as i32,
            weapon,
            origin: ps.origin,
            ..Default::default()
        },
    );
    perf::pickup(picker_pm_type);
}

pub(crate) fn grab_scavenger(world: &mut FrameWorld, walker: ClientId, number: i32) {
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    let Some(ps) = world.player(walker).copied() else {
        return;
    };
    if !has_scavenger_perk(&ps) {
        return;
    }
    let picker_pm_type = ps.pm_type;
    let weapon = u32::try_from(item.state.index).unwrap_or(0);
    world.remove_dropped_item_by_number(number);
    world.free_dynamic_entity_number(item.state.number);
    world.item_pickups_mut().push(ItemPickupRecord {
        picker: walker.0 as i32,
        weapon,
        from_entnum: item.state.number,
        clip_r: 0,
        clip_l: 0,
        stock: 0,
        swapped_entnum: ENTITYNUM_NONE,
        picker_pm_type,
        ammo_only: false,
    });
    perf::pickup(picker_pm_type);
}

fn drop_current_primary_at(
    world: &mut FrameWorld,
    walker: ClientId,
    weapon: u32,
    origin: [f32; 3],
    angles: [f32; 3],
) -> i32 {
    let Some(ps) = world.player(walker).copied() else {
        return ENTITYNUM_NONE;
    };
    let (clip_r, clip_l, stock) = ammo_from_ps(world, &ps, weapon);
    let pos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: 0,
        tr_duration: 0,
        tr_delta: [0.0; 3],
        tr_base: origin,
    };
    let apos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: 0,
        tr_duration: 0,
        tr_delta: [0.0; 3],
        tr_base: angles,
    };
    let entnum = push_dropped_item(
        world,
        weapon,
        origin,
        pos,
        apos,
        walker.0 as i32,
        clip_r,
        clip_l,
        stock,
        false,
        false,
    );
    if entnum == ENTITYNUM_NONE {
        return ENTITYNUM_NONE;
    }
    if let Some(ps_mut) = world.player_mut(walker) {
        take_player_weapon(ps_mut, weapon);
    }
    entnum
}

fn primary_count(world: &FrameWorld, ps: &PlayerState) -> usize {
    ps.weapons
        .iter()
        .filter(|&&w| {
            w > 0
                && world
                    .combat_facts_for(w as u32)
                    .is_some_and(|f| f.inventory_type == WEAP_INVENTORY_PRIMARY)
        })
        .count()
}

#[derive(Clone, Copy)]
struct UseItem {
    number: i32,
    weapon: u32,
    projectile: bool,
}

fn projectile_pickup_ammo(
    world: &FrameWorld,
    walker: ClientId,
    ps: &PlayerState,
    projectile: &crate::ProjectileState,
) -> Option<(i32, i32, i32)> {
    let weapon = projectile.weapon;
    if !ps.weapons.contains(&(weapon as i32)) {
        return None;
    }
    let facts = world.equipment_facts_for(weapon)?;
    if !facts.is_retrievable_knife()
        && (!facts.is_offhand()
            || facts.stickiness == 0
            || facts.timed_detonation
            || facts.proj_impact_explode
            || projectile.detonate_at_ms.is_some()
            || projectile.owner != walker
            || world.client_meta(walker)?.life_sequence != projectile.owner_life)
    {
        return None;
    }
    let (clip, left, stock) = ammo_from_ps(world, ps, weapon);
    if facts.ballistic_blade {
        if ps.weapon != weapon && clip == 0 {
            return Some((1, left, stock));
        }
        let combat = world.combat_facts_for(weapon)?;
        (stock < combat.max_ammo).then_some((clip, left, stock + 1))
    } else {
        (clip < facts.clip_size.max(1)).then_some((clip + 1, left, stock))
    }
}

fn grab_projectile(world: &mut FrameWorld, walker: ClientId, number: i32, tick: Tick) {
    let Some(projectile) = world.projectile_by_number(number) else {
        return;
    };
    let Some(mut ps) = world.player(walker).copied() else {
        return;
    };
    if projectile.pos.tr_type != TR_STATIONARY {
        return;
    }
    let weapon = projectile.weapon;
    let Some((clip, left, stock)) = projectile_pickup_ammo(world, walker, &ps, &projectile) else {
        return;
    };
    let (old_clip, _, old_stock) = ammo_from_ps(world, &ps, weapon);
    set_ammo_on_ps(world, &mut ps, weapon, clip, left, stock);
    *world.player_mut(walker).expect("picker exists") = ps;
    let meta = world.client_meta_mut(walker);
    meta.set_ammo(weapon, clip, stock);
    meta.mirror_held_ammo(ps.weapon);
    world.remove_projectile_by_number(number);
    world.free_dynamic_entity_number(number);
    world.item_pickups_mut().push(ItemPickupRecord {
        picker: walker.0 as i32,
        weapon,
        from_entnum: number,
        clip_r: clip - old_clip,
        clip_l: 0,
        stock: stock - old_stock,
        swapped_entnum: ENTITYNUM_NONE,
        picker_pm_type: ps.pm_type,
        ammo_only: true,
    });
    world.push_entity_event(
        tick,
        crate::EventAudience::All,
        entity_t6::EntityEventKind::AMMO_PICKUP,
        crate::EntityEventPayload {
            number: walker.0 as i32,
            event_parm: weapon as i32,
            weapon,
            origin: ps.origin,
            ..Default::default()
        },
    );
    perf::pickup(ps.pm_type);
}

/// The T6 gun state the use button reads, with BO2's weapon facts on.
fn t6_copy(world: &FrameWorld, walker: ClientId) -> Option<weapon_t6::player::T6WeaponPs> {
    if world.gun_facts().is_empty() {
        None
    } else {
        crate::t6_gun::synced_copy(world, walker)
    }
}

fn selected_item(world: &FrameWorld, walker: ClientId, ps: &PlayerState) -> Option<UseItem> {
    // bo2mp: with BO2's weapon facts the use button offers what WeaponPickup_Grab would take,
    // behind Player_UpdateCursorHints' gates.
    let t6 = t6_copy(world, walker);
    let blocked = match &t6 {
        Some(t6) => crate::item_t6::hint_blocked(t6, ps),
        None => ps.pm_flags & (4 | 0x8000) != 0 || matches!(ps.weapon_state_t6, 22..=24 | 26..=28),
    };
    if !walker_can_touch(world, walker, ps) || blocked {
        return None;
    }

    let eye = [
        ps.origin[0],
        ps.origin[1],
        ps.origin[2] + ps.view_height_current,
    ];
    let (forward, _, _) = angle_vectors(ps.viewangles);
    let now = world.entity_kernel().level_time_ms();
    let mut best: Option<(f32, UseItem)> = None;
    for number in world.dropped_item_numbers_sorted() {
        let Some(item) = world.dropped_item_by_number(number) else {
            continue;
        };
        if let Some(t6) = &t6 {
            // Player_GetUseList only looks 96 units above and below his eye.
            let below = item.origin[2] + ITEM_MAXS[2] < eye[2] - 96.0;
            let above = item.origin[2] + ITEM_MINS[2] > eye[2] + 96.0;
            if below
                || above
                || !crate::item_t6::use_candidate(t6, world.gun_facts(), &item, walker, now)
            {
                continue;
            }
        } else {
            if item.scavenger
                || item.state.weapon.0 == 0
                || ps.weapons.contains(&(item.state.weapon.0 as i32))
            {
                continue;
            }
            if primary_count(world, ps) >= 2 && current_primary_weapon(world, ps) == 0 {
                continue;
            }
        }
        let center: [f32; 3] =
            core::array::from_fn(|i| item.origin[i] + (ITEM_MINS[i] + ITEM_MAXS[i]) * 0.5);
        let delta: [f32; 3] = core::array::from_fn(|i| center[i] - eye[i]);
        let distance = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
        if distance > 128.0 {
            continue;
        }
        let dot = if distance > 0.0 {
            (0..3).map(|i| forward[i] * delta[i] / distance).sum()
        } else {
            0.0
        };
        if world
            .trace_world(eye, center, [0.0; 3], [0.0; 3], 0x11)
            .fraction
            < 1.0
        {
            continue;
        }
        let score = distance + (1.0 - (dot + 1.0) * 0.5) * 256.0;
        if best.as_ref().is_none_or(|(old, _)| score < *old) {
            best = Some((
                score,
                UseItem {
                    number: item.state.number,
                    weapon: item.state.index as u32,
                    projectile: false,
                },
            ));
        }
    }
    world.visit_projectiles(|projectile| {
        if projectile.pos.tr_type != TR_STATIONARY
            || projectile_pickup_ammo(world, walker, ps, projectile).is_none()
        {
            return;
        }
        let from_player: [f32; 3] = core::array::from_fn(|i| projectile.origin[i] - ps.origin[i]);
        if from_player.iter().map(|v| v * v).sum::<f32>() > 90.0 * 90.0 {
            return;
        }
        let delta: [f32; 3] = core::array::from_fn(|i| projectile.origin[i] - eye[i]);
        let distance = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
        if distance > 160.0
            || world
                .trace_world(eye, projectile.origin, [0.0; 3], [0.0; 3], 0x11)
                .fraction
                < 1.0
        {
            return;
        }
        let dot = if distance > 0.0 {
            (0..3).map(|i| forward[i] * delta[i] / distance).sum()
        } else {
            0.0
        };
        let score = distance + (1.0 - (dot + 1.0) * 0.5) * 256.0 - 512.0;
        if best.as_ref().is_none_or(|(old, _)| score < *old) {
            best = Some((
                score,
                UseItem {
                    number: projectile.entnum,
                    weapon: projectile.weapon,
                    projectile: true,
                },
            ));
        }
    });
    best.map(|(_, item)| item)
}

/// BO2's g_useholdtime: how long the use/reload button is held to take an
/// item (ms). A shorter tap reloads.
const USE_HOLD_MS: i32 = 250;

pub(crate) fn phase_use_items(
    world: &mut FrameWorld,
    tick: Tick,
    presses: &[crate::UsePress],
    cmds: &[(u32, u32)],
) {
    let now = crate::corpse::level_time_ms(tick);
    for id in world.client_ids_sorted() {
        let Some(ps) = world.player(id).copied() else {
            continue;
        };
        let selected = selected_item(world, id, &ps);
        let t6 = t6_copy(world, id);
        let bits = cmds
            .iter()
            .filter(|(client, _)| *client == id.0)
            .fold(0u32, |all, (_, bits)| all | *bits);
        let held =
            bits & (playerstate_t6::game::buttons::USE | playerstate_t6::game::buttons::USE_RELOAD) != 0;
        let pressed = presses.iter().any(|p| p.client == id.0 && p.edge);
        let selected_ref = selected.map(|item| {
            world
                .entity_kernel()
                .current_ref(item.number)
                .expect("occupied item")
        });
        let meta = world.client_meta_mut(id);
        // BO2: the use/reload button let go before it took the item reloads
        // instead (a bot's reload is a 0.1 s tap of it).
        if !held
            && meta
                .item_use_hold
                .take()
                .is_some_and(|(_, by_reload)| by_reload)
        {
            meta.item_use_reload |= meta.item_use_entity.is_some();
        }
        if !held || selected.is_none() {
            meta.item_use_entity = None;
        }
        if pressed && t6.as_ref().is_none_or(crate::item_t6::press_picks) {
            meta.item_use_entity = selected_ref;
            meta.item_use_hold = Some((now, bits & playerstate_t6::game::buttons::USE == 0));
        }
        let pending = meta.item_use_entity;
        let ready = now - meta.item_use_spawn_ms >= 500;
        // BO2: the use/reload button takes the item once held 250 ms, and
        // never while his gun is still coming up or going away (so a swap
        // cannot swap straight back). The use button takes it at once. With
        // BO2's weapon facts Player_ActivateHoldCmdAllowed's gates hold for
        // both buttons.
        let long_enough = match &t6 {
            Some(t6) => {
                crate::item_t6::hold_takes(t6, &ps)
                    && meta
                        .item_use_hold
                        .is_none_or(|(since, by_reload)| !by_reload || now - since >= USE_HOLD_MS)
            }
            None => meta.item_use_hold.is_none_or(|(since, by_reload)| {
                !by_reload
                    // TODO(fact): the swim in and out hold it too, as our old MW2 mirror read them.
                    || (now - since >= USE_HOLD_MS
                        && !weapon_t6::state::is_raising_or_swim_out(ps.weapon_state_t6)
                        && !weapon_t6::state::is_dropping_or_swim_in(ps.weapon_state_t6))
            }),
        };
        if held && ready && long_enough && pending.is_some() && selected_ref == pending {
            let item = selected.expect("selected use item");
            if item.projectile {
                grab_projectile(world, id, item.number, tick);
            } else if !world.gun_facts().is_empty() {
                crate::item_t6::grab(world, id, item.number);
            } else {
                grab_number(world, id, item.number);
            }
            let meta = world.client_meta_mut(id);
            meta.item_use_entity = None;
            meta.item_use_hold = None;
        }
        let selected = selected_item(
            world,
            id,
            &world.player(id).copied().expect("client exists"),
        );
        let dual = selected
            .is_some_and(|item| gsc_give_weapon_is_akimbo(world.weapon_script_name(item.weapon)));
        if let Some(ps) = world.player_mut(id) {
            ps.cursor_hint = selected.map_or(0, |item| item.weapon as i32 + 4);
            ps.cursor_hint_ent_index = selected.map_or(ENTITYNUM_NONE, |item| item.number);
            ps.cursor_hint_string = -1;
            ps.cursor_hint_dual_wield = i32::from(dual);
        }
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;
    use movement_t6::bridge::view_height;

    #[test]
    fn a_gun_drops_from_half_his_height_in_each_stance() {
        let mut ps = PlayerState::ZERO;
        let mut at = |target| {
            ps.view_height_target = target;
            drop_height(&ps)
        };
        assert_eq!(at(view_height::STAND), 35.0);
        assert_eq!(at(view_height::CROUCH), 25.0);
        assert_eq!(at(view_height::LAST_STAND), 25.0);
        assert_eq!(at(view_height::PRONE), 15.0);
    }
}
