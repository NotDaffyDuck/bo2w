//! The movement rules the server plays by (jump height, gravity, sprint and fall tuning, perks),
//! sent with every snapshot so a second PC moves the way the host does.

use movement_t6::bridge::Bo2Feel;

use super::codec::{Decoded, Wire};
use crate::transport::wire::{WireReader, WireWriter};

/// Bit 0 says rules follow; bit 1 is always on (it was the "rules active" switch of the first
/// version of this message and old peers still expect it).
const RULES_FOLLOW: u8 = 1 << 0;
const ACTIVE: u8 = 1 << 1;
const JUMP_SLOWDOWN: u8 = 1 << 2;
const ZOMBIES: u8 = 1 << 3;
const DIVE: u8 = 1 << 4;
const OMNI: u8 = 1 << 5;

pub(super) fn put_feel(out: &mut WireWriter, feel: &Option<Bo2Feel>) {
    let Some(rules) = feel else {
        out.put_u8(0);
        return;
    };
    let mut flags = RULES_FOLLOW | ACTIVE;
    let switches = [
        (JUMP_SLOWDOWN, rules.jump_slowdown),
        (ZOMBIES, rules.zombies),
        (DIVE, rules.dive),
        (OMNI, rules.omni),
    ];
    for (bit, on) in switches {
        if on {
            flags |= bit;
        }
    }
    out.put_u8(flags);
    let scales: [f32; 13] = [
        rules.sprint_strafe_speed_scale,
        rules.fall_damage_min_height,
        rules.fall_damage_max_height,
        rules.fast_ads_multiplier,
        rules.sprint_multiplier,
        rules.speed_multiplier,
        rules.g_speed,
        rules.sprint_cycle_scale,
        rules.ducked_sprint_cycle_scale,
        rules.dtp_cycle_scale,
        rules.mantle_weapon_height,
        rules.mantle_weapon_anim_height,
        rules.jump_height,
    ];
    scales.put(out);
    rules.gravity.put(out);
    let perks: [u32; 5] = [
        rules.fall_damage_perk,
        rules.flak_jacket_perk,
        rules.fast_ads_perk,
        rules.longer_sprint_perk,
        rules.move_faster_perk,
    ];
    perks.put(out);
}

pub(super) fn take_feel(input: &mut WireReader<'_>) -> Decoded<Option<Bo2Feel>> {
    let flags = input.get_u8()?;
    if flags & RULES_FOLLOW == 0 {
        return Ok(None);
    }
    let [
        sprint_strafe_speed_scale,
        fall_damage_min_height,
        fall_damage_max_height,
        fast_ads_multiplier,
        sprint_multiplier,
        speed_multiplier,
        g_speed,
        sprint_cycle_scale,
        ducked_sprint_cycle_scale,
        dtp_cycle_scale,
        mantle_weapon_height,
        mantle_weapon_anim_height,
        jump_height,
    ] = <[f32; 13]>::take(input)?;
    let gravity = i32::take(input)?;
    let [
        fall_damage_perk,
        flak_jacket_perk,
        fast_ads_perk,
        longer_sprint_perk,
        move_faster_perk,
    ] = <[u32; 5]>::take(input)?;
    let has = |bit: u8| flags & bit != 0;
    Ok(Some(Bo2Feel {
        sprint_strafe_speed_scale,
        jump_slowdown: has(JUMP_SLOWDOWN),
        fall_damage_min_height,
        fall_damage_max_height,
        zombies: has(ZOMBIES),
        gravity,
        fall_damage_perk,
        flak_jacket_perk,
        fast_ads_perk,
        fast_ads_multiplier,
        longer_sprint_perk,
        sprint_multiplier,
        move_faster_perk,
        speed_multiplier,
        g_speed,
        dive: has(DIVE),
        sprint_cycle_scale,
        ducked_sprint_cycle_scale,
        dtp_cycle_scale,
        omni: has(OMNI),
        mantle_weapon_height,
        mantle_weapon_anim_height,
        jump_height,
    }))
}
