//! The per-frame look under the player's feet: is there ground, may it be
//! stood on, and what changes when it appears or vanishes.
//!
//! Three cases are handled here:
//! - The box is buried in solid. The 26 one-unit steps around the player are
//!   tried in turn; from the first clear one the box drops up to 1.25 units
//!   and rests there, even if that drop starts inside something. When no
//!   step is clear the player stays where he is, standing on whatever holds
//!   him, and is not on walkable ground.
//! - Nothing is under the feet. A longer look down decides whether the
//!   ground is still close enough to count as almost touching.
//! - Something is under the feet. It is walkable ground unless it is another
//!   player; that is decided by the entity number, not by contents.
//!
//! Landings themselves live in crate::crash.
//!
//! Leg animations requested come back as `BodyAnims`: stepping off ground
//! with nothing within a 64th of a 64-unit look down plays the jump (unless
//! diving); touching down after a fall of over 12 units plays the landing.
//!
//! Slick ground sets `PMF_SLIDING` while it is under the feet and records
//! the moment it was first stood on in `slide_time`. Stepping onto it asks
//! for a crouch and stepping off it (into the air too) for a stand, both as
//! predicted stance events. On the server only, each is followed by a slide
//! start or stop event carrying the ground's slide sound class.
//!
//! Still missing: the ground's surface type for footstep sounds (there is no
//! field for it yet), and riot shields and dogs as unwalkable ground.

use crate::bridge::{
    Bo2Feel, SweepAsk, HullExtents, TickLocals, UNSTICK_NUDGES, leap,
    queue_forecast_happening,
};
use playerstate_t6::game::{AvatarState, BODY_ENTITY_FIRST_SLOT, NO_ENTITY_SLOT};
use trace_t6::Trace;

use crate::crash::crash_land;
use crate::dive;
use playerstate_t6::game::entity_flag_bits::TURRET_MASK;
use crate::gait_cycle::ground_surface_type;
use crate::frame::BodyAnims;
use crate::gjk::Bo2Collision;

/// Reach of the ground look above and below the feet. Mounted on a turret
/// the look starts at the feet and reaches one unit down.
const PROBE: f32 = 0.25;
const TURRET_PROBE: f32 = 1.0;

/// Movement state bit: the ground under the player is slick. (Last stand is
/// carried by the move type, so this bit is free for sliding.)
pub const PMF_SLIDING: u32 = 0x40_0000;

/// Surface flag marking slick ground.
const SURF_SLICK: u32 = 0x2;

/// Predicted stance events asked for when slick ground starts or ends.
const EV_STANCE_FORCE_STAND: i32 = entity_t6::event::STANCE_FORCE_STAND as i32;
const EV_STANCE_FORCE_CROUCH: i32 = entity_t6::event::STANCE_FORCE_CROUCH as i32;

/// Slide start and stop events, sent only by the server's move.
const EV_SLIDE_START: i32 = entity_t6::event::SLIDE_START as i32;
const EV_SLIDE_STOP: i32 = entity_t6::event::SLIDE_STOP as i32;

/// Slide sound class for each of the 32 surface types (0 default, 1 metal,
/// 2 concrete, 3 wood, 4 snow, 5 dirt, 6 gravel, 7 grass, 8 mud).
const SLIDE_SOUND_CLASS: [i32; 32] = [
    0, 3, 2, 4, 4, 2, 5, 2, 7, 5, 7, 6, 4, 1, 8, 2, 2, 2, 5, 4, 8, 3, 2, 2, 2, 2, 5, 5, 1, 5, 7, 0,
];

/// When the look starts inside the floor, it is retried from this far below
/// the feet.
const STARTSOLID_RETRY: f32 = 0.001;

/// How far a box freed from solid drops looking for something to rest on.
const CORRECT_SOLID_DROP: f32 = 1.25;

/// With nothing under the feet: the look down from the air, the look down
/// on stepping off, and the share of the latter that still counts as almost
/// on the ground.
const MISSED_AIR_PROBE: f32 = 1.0;
const MISSED_LEFT_PROBE: f32 = 64.0;
const MISSED_ALMOST_FRACTION: f32 = 1.0 / 64.0;

/// Write how the feet relate to the floor this frame.
fn set_contact(tick_vars: &mut TickLocals, on_floor: bool, near_floor: bool, walking: bool) {
    tick_vars.standing_on_floor = u32::from(on_floor);
    tick_vars.near_floor_contact = u32::from(near_floor);
    tick_vars.walking = u32::from(walking);
}

/// Look under the feet, settle the ground state for the frame, and return
/// the leg animations that asks for.
pub(crate) fn floor_probe<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    bounds: HullExtents,
    solids: &C,
    feel: Bo2Feel,
    authority_clock_ms: i32,
    server: bool,
) -> BodyAnims {
    let [x, y, z] = avatar.origin;
    let mounted = avatar.e_flags & TURRET_MASK != 0;
    let (lift, reach) = if mounted {
        (0.0, TURRET_PROBE)
    } else {
        (PROBE, PROBE)
    };
    let below_feet = [x, y, z - reach];
    // The sweep the kept result came from; its hit point lies along it.
    let (mut from, mut to) = ([x, y, z + lift], below_feet);
    let mut strike = ground_probe(solids, from, to, bounds);
    record_floor(tick_vars, &strike);
    slide_ground(avatar, strike.sflags, authority_clock_ms, server);

    if strike.fully_embedded {
        let Some((freed, path)) = escape_solid(avatar, tick_vars, bounds, solids) else {
            return BodyAnims::default();
        };
        strike = freed;
        [from, to] = path;
    }

    let mut walkable = strike.walkable && ent_walkable(entity_hit(&strike));
    if strike.starts_inside {
        let [x, y, z] = avatar.origin;
        from = [x, y, z - STARTSOLID_RETRY];
        to = below_feet;
        strike = ground_probe(solids, from, to, bounds);
        if strike.starts_inside {
            // Still inside: rest on whatever holds the box.
            avatar.standing_on = entity_hit(&strike);
            set_contact(tick_vars, true, true, strike.walkable);
            return BodyAnims::default();
        }
        record_floor(tick_vars, &strike);
        walkable = strike.walkable;
    }

    if strike.fraction == 1.0 {
        return BodyAnims {
            jump: nothing_underfoot(avatar, tick_vars, bounds, solids),
            ..BodyAnims::default()
        };
    }

    // There is no kick-off from steep slopes: that only exists in a
    // collision mode players never use.

    if !walkable {
        avatar.standing_on = NO_ENTITY_SLOT;
        set_contact(tick_vars, true, true, false);
        leap::clear_state(avatar);
        return BodyAnims::default();
    }
    set_contact(tick_vars, true, true, true);
    let was_airborne = avatar.standing_on == NO_ENTITY_SLOT;
    let land = was_airborne && crash_land(avatar, tick_vars, feel);
    avatar.standing_on = entity_hit(&strike);
    solids.brush_against(&strike, strike.finish_location(from, to));
    BodyAnims {
        land,
        ..BodyAnims::default()
    }
}

/// The box is buried. Step to the first clear neighbouring spot, drop from
/// it, record that drop as the floor and move the player to where it
/// stopped; give back the drop and its path. With every spot blocked the
/// player keeps his place on what the last blocked look hit, off the
/// ground, and any jump ends.
fn escape_solid<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    bounds: HullExtents,
    solids: &C,
) -> Option<(Trace, [[f32; 3]; 2])> {
    let mut last_blocked = Trace::default();
    for step in UNSTICK_NUDGES {
        let spot: [f32; 3] = core::array::from_fn(|axis| avatar.origin[axis] + step[axis]);
        let probe = player_trace(solids, spot, spot, bounds);
        if probe.starts_inside {
            last_blocked = probe;
            continue;
        }
        let [sx, sy, sz] = spot;
        let floor = [sx, sy, sz - CORRECT_SOLID_DROP];
        let drop = player_trace(solids, spot, floor, bounds);
        record_floor(tick_vars, &drop);
        avatar.origin = [sx, sy, sz + (floor[2] - sz) * drop.fraction];
        return Some((drop, [spot, floor]));
    }
    avatar.standing_on = entity_hit(&last_blocked);
    set_contact(tick_vars, false, false, false);
    leap::clear_state(avatar);
    None
}

/// No ground right under the feet: look further down to see if it is still
/// close. True when the player has just stepped off and the ground is not
/// close, which plays the jump on the legs (unless diving).
fn nothing_underfoot<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    bounds: HullExtents,
    solids: &C,
) -> bool {
    let [x, y, z] = avatar.origin;
    let was_airborne = avatar.standing_on == NO_ENTITY_SLOT;
    let look = if was_airborne {
        MISSED_AIR_PROBE
    } else {
        MISSED_LEFT_PROBE
    };
    let below = player_trace(solids, [x, y, z], [x, y, z - look], bounds);
    let close = match was_airborne {
        true => below.fraction != 1.0,
        false => below.fraction < MISSED_ALMOST_FRACTION,
    };
    avatar.standing_on = NO_ENTITY_SLOT;
    set_contact(tick_vars, false, close, false);
    let diving = avatar.movement_state_bits & dive::PMF_DIVE != 0;
    !(was_airborne || close || diving)
}

/// Whether an entity may be stood on: every one but a player.
fn ent_walkable(entity: i32) -> bool {
    !is_player(entity)
}

/// The first `BODY_ENTITY_FIRST_SLOT` entity numbers are players.
pub(crate) fn is_player(entity: i32) -> bool {
    entity != NO_ENTITY_SLOT && (0..BODY_ENTITY_FIRST_SLOT).contains(&entity)
}

/// Sweep the player's box: by the hull solver when it is on, otherwise by
/// the plain box trace.
fn player_trace<C: Bo2Collision>(
    solids: &C,
    from_pt: [f32; 3],
    end: [f32; 3],
    bounds: HullExtents,
) -> Trace {
    let HullExtents {
        lo_corner: box_lo,
        hi_corner: box_hi,
        collide_filter,
    } = bounds;
    if solids.gjk_on() {
        crate::gjk::trace::player_trace(solids, from_pt, end, box_lo, box_hi, collide_filter)
    } else {
        solids.trace(SweepAsk {
            start: from_pt,
            end,
            lo_corner: box_lo,
            hi_corner: box_hi,
            collide_filter,
        })
    }
}

/// The look under the feet: the hull solver's own ground sweep when it is
/// on, otherwise the ordinary box sweep.
fn ground_probe<C: Bo2Collision>(
    solids: &C,
    from_pt: [f32; 3],
    end: [f32; 3],
    bounds: HullExtents,
) -> Trace {
    if !solids.gjk_on() {
        return player_trace(solids, from_pt, end, bounds);
    }
    let HullExtents {
        lo_corner: box_lo,
        hi_corner: box_hi,
        collide_filter,
    } = bounds;
    crate::gjk::trace::floor_probe(solids, from_pt, end, box_lo, box_hi, collide_filter)
}

fn entity_hit(trace: &Trace) -> i32 {
    trace_t6::trace_get_entity_hit_id(trace).into()
}

/// Slick ground under the feet sets the sliding bit and, on the first such
/// frame, stamps the time and asks for a crouch. Anything else (the air
/// included) clears the bit and, on the first such frame, clears the time
/// and asks for a stand. The server follows either with a slide start or
/// stop carrying the ground's slide sound class.
fn slide_ground(avatar: &mut AvatarState, surf: u32, authority_clock_ms: i32, server: bool) {
    let slick = surf & SURF_SLICK != 0;
    let was_sliding = avatar.slide_time != 0;
    if slick {
        avatar.movement_state_bits |= PMF_SLIDING;
    } else {
        avatar.movement_state_bits &= !PMF_SLIDING;
    }
    if slick == was_sliding {
        return;
    }
    let (stance, slide_event) = if slick {
        avatar.slide_time = authority_clock_ms;
        (EV_STANCE_FORCE_CROUCH, EV_SLIDE_START)
    } else {
        avatar.slide_time = 0;
        (EV_STANCE_FORCE_STAND, EV_SLIDE_STOP)
    };
    queue_forecast_happening(avatar, stance, 0);
    if server {
        let class = slide_sound_class(avatar, surf);
        queue_forecast_happening(avatar, slide_event, class);
    }
}

/// The slide sound class of the ground's surface type (default for types
/// outside the table).
fn slide_sound_class(avatar: &AvatarState, surf: u32) -> i32 {
    let surface = ground_surface_type(avatar, surf);
    usize::try_from(surface)
        .ok()
        .and_then(|index| SLIDE_SOUND_CLASS.get(index).copied())
        .unwrap_or_default()
}

/// Keep the frame's floor result in the packed eleven-word form the other
/// moves read.
fn record_floor(tick_vars: &mut TickLocals, strike: &Trace) {
    let [nx, ny, nz] = strike.normal.map(f32::to_bits);
    let entity_and_model = u32::from(strike.struck_entity_num) | u32::from(strike.mesh_slot_number) << 16;
    let part_and_group = u32::from(strike.hit_part_label) | u32::from(strike.hit_body_group) << 16;
    let solidity = u32::from(strike.fully_embedded)
        | u32::from(strike.starts_inside) << 8
        | u32::from(strike.walkable) << 16;
    tick_vars.floor_probe = [
        strike.fraction.to_bits(),
        nx,
        ny,
        nz,
        strike.sflags,
        strike.cflags,
        0,
        strike.impact_category as u32,
        entity_and_model,
        part_and_group,
        solidity,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::SolidProbe;
    use playerstate_t6::game::movement_state_bits;
    use trace_t6::TraceHitType;

    const WORLD: u16 = trace_t6::ENTITYNUM_WORLD;
    const ON_WORLD: i32 = WORLD as i32;

    /// Solid floor at z = 0, optionally a solid block that traps any box
    /// starting inside it, and optionally a player body as what is hit.
    struct Floor {
        body: Option<u16>,
        block: Option<([f32; 3], [f32; 3])>,
    }

    const BARE: Floor = Floor {
        body: None,
        block: None,
    };

    fn within(lo: [f32; 3], hi: [f32; 3], p: [f32; 3], axes: usize) -> bool {
        (0..axes).all(|i| (lo[i]..=hi[i]).contains(&p[i]))
    }

    impl crate::gjk::Bo2Collision for Floor {}

    impl SolidProbe for Floor {
        fn trace(&self, input: SweepAsk) -> Trace {
            let mut out = Trace::new();
            out.impact_category = TraceHitType::Entity;
            out.struck_entity_num = self.body.unwrap_or(WORLD);
            let (start, end) = (input.start, input.end);
            if let Some((lo, hi)) = self.block {
                if within(lo, hi, start, 3) {
                    out.starts_inside = true;
                    out.fully_embedded = within(lo, hi, end, 3);
                    out.fraction = 0.0_f32;
                    return out;
                }
            }
            // The highest surface under the start: the block's top or the floor.
            let surface = match self.block {
                Some((lo, hi)) if within(lo, hi, start, 2) => hi[2].max(0.0),
                _ => 0.0,
            };
            if start[2] >= surface && end[2] < surface {
                out.fraction = (start[2] - surface) / (start[2] - end[2]);
                out.normal = [0.0, 0.0, 1.0];
                out.walkable = true;
            }
            out
        }
    }

    fn body_box() -> HullExtents {
        let half_width = 15.0;
        HullExtents {
            lo_corner: [-half_width, -half_width, 0.0],
            hi_corner: [half_width, half_width, 70.0],
            collide_filter: 1,
        }
    }

    fn standing(at: [f32; 3], on: i32) -> AvatarState {
        AvatarState {
            origin: at,
            standing_on: on,
            ..AvatarState::ZERO
        }
    }

    fn probe(world: &Floor, avatar: &mut AvatarState, tick_vars: &mut TickLocals) -> BodyAnims {
        floor_probe(avatar, tick_vars, body_box(), world, Bo2Feel::default(), 0, false)
    }

    fn run(world: &Floor, at: [f32; 3], on: i32) -> (AvatarState, TickLocals) {
        let mut avatar = standing(at, on);
        let mut tick_vars = crate::testing::tick_vars(false);
        probe(world, &mut avatar, &mut tick_vars);
        (avatar, tick_vars)
    }

    fn contact(tick_vars: &TickLocals) -> (u32, u32, u32) {
        (tick_vars.walking, tick_vars.standing_on_floor, tick_vars.near_floor_contact)
    }

    #[test]
    fn feet_on_the_floor_are_walking() {
        let (avatar, tick_vars) = run(&BARE, [0.0; 3], ON_WORLD);
        assert_eq!(avatar.standing_on, ON_WORLD);
        assert_eq!(contact(&tick_vars), (1, 1, 1));
    }

    #[test]
    fn players_are_not_ground_but_later_entities_are() {
        let on_player = Floor {
            body: Some(3),
            block: None,
        };
        let mut avatar = standing([0.0; 3], 3);
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        let mut tick_vars = crate::testing::tick_vars(false);
        probe(&on_player, &mut avatar, &mut tick_vars);
        assert_eq!(avatar.standing_on, NO_ENTITY_SLOT);
        assert_eq!((tick_vars.walking, tick_vars.standing_on_floor), (0, 1));
        assert_eq!(
            avatar.movement_state_bits & movement_state_bits::JUMPING,
            0,
            "jump ended"
        );

        let on_thing = Floor {
            body: Some(18),
            block: None,
        };
        let (avatar, tick_vars) = run(&on_thing, [0.0; 3], 18);
        assert_eq!(avatar.standing_on, 18);
        assert_eq!(tick_vars.walking, 1);
    }

    #[test]
    fn buried_box_steps_up_and_rests_on_the_block() {
        let world = Floor {
            body: None,
            block: Some(([-0.5, -0.5, -1.0], [0.5, 0.5, 0.5])),
        };
        // Up a unit to 1.2 is clear; the drop from there meets the top at 0.5.
        let (avatar, tick_vars) = run(&world, [0.0, 0.0, 0.2], ON_WORLD);
        assert!((avatar.origin[2] - 0.5).abs() < 1e-5, "{:?}", avatar.origin);
        assert_eq!(tick_vars.walking, 1);
    }

    #[test]
    fn fully_trapped_box_stays_put_off_the_ground() {
        let world = Floor {
            body: None,
            block: Some(([-5.0; 3], [5.0; 3])),
        };
        let mut avatar = standing([0.0; 3], NO_ENTITY_SLOT);
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        let mut tick_vars = crate::testing::tick_vars(true);
        probe(&world, &mut avatar, &mut tick_vars);
        assert_eq!(avatar.standing_on, ON_WORLD);
        assert_eq!(contact(&tick_vars), (0, 0, 0));
        assert_eq!(avatar.movement_state_bits & movement_state_bits::JUMPING, 0);
        assert_eq!(avatar.origin, [0.0; 3], "not moved");
    }

    #[test]
    fn stepping_off_looks_far_down_and_the_air_looks_one_unit() {
        // Half a unit up after stepping off: the short look misses, the long
        // one hits within a 64th, so still almost grounded.
        let (avatar, tick_vars) = run(&BARE, [0.0, 0.0, 0.5], ON_WORLD);
        assert_eq!(avatar.standing_on, NO_ENTITY_SLOT);
        assert_eq!(contact(&tick_vars), (0, 0, 1));
        let (_, tick_vars) = run(&BARE, [0.0, 0.0, 0.5], NO_ENTITY_SLOT);
        assert_eq!(tick_vars.near_floor_contact, 1);
        let (_, tick_vars) = run(&BARE, [0.0, 0.0, 2.0], NO_ENTITY_SLOT);
        assert_eq!(tick_vars.near_floor_contact, 0);
    }

    #[test]
    fn slick_ground_crouches_once_and_stands_once_on_leaving() {
        let mut avatar = AvatarState::ZERO;
        slide_ground(&mut avatar, SURF_SLICK, 5000, false);
        assert_ne!(avatar.movement_state_bits & PMF_SLIDING, 0);
        assert_eq!(avatar.slide_time, 5000);
        assert_eq!(
            (avatar.predictable_event_sequence, avatar.predictable_events[0]),
            (1, EV_STANCE_FORCE_CROUCH)
        );
        slide_ground(&mut avatar, SURF_SLICK | 0x10, 5050, false);
        assert_eq!((avatar.slide_time, avatar.predictable_event_sequence), (5000, 1));
        slide_ground(&mut avatar, 0, 5100, false);
        assert_eq!(avatar.movement_state_bits & PMF_SLIDING, 0);
        assert_eq!(avatar.slide_time, 0);
        assert_eq!(
            (avatar.predictable_event_sequence, avatar.predictable_events[1]),
            (2, EV_STANCE_FORCE_STAND)
        );
        slide_ground(&mut avatar, 0, 5150, false);
        assert_eq!(avatar.predictable_event_sequence, 2);
    }

    #[test]
    fn server_adds_slide_start_and_stop_with_the_sound_class() {
        let grass = SURF_SLICK | (8 << 20);
        let mut avatar = AvatarState::ZERO;
        slide_ground(&mut avatar, grass, 5000, true);
        assert_eq!(avatar.predictable_event_sequence, 2);
        assert_eq!(
            (
                avatar.predictable_events[0],
                avatar.predictable_events[1],
                avatar.predictable_event_parms[1]
            ),
            (EV_STANCE_FORCE_CROUCH, EV_SLIDE_START, 7)
        );
        slide_ground(&mut avatar, grass, 5050, true);
        assert_eq!(avatar.predictable_event_sequence, 2);
        slide_ground(&mut avatar, 0, 5100, true);
        assert_eq!(avatar.predictable_event_sequence, 4);
        assert_eq!(
            (
                avatar.predictable_events[2],
                avatar.predictable_events[3],
                avatar.predictable_event_parms[3]
            ),
            (EV_STANCE_FORCE_STAND, EV_SLIDE_STOP, 0)
        );
        // Wading, the class comes from the water (mud).
        let mut wading = AvatarState {
            waterlevel: 1,
            ..AvatarState::ZERO
        };
        slide_ground(&mut wading, SURF_SLICK, 5000, true);
        assert_eq!(wading.predictable_event_parms[1], 8);
    }

    fn falling_anims(world: &Floor, at: [f32; 3], on: i32, bits: u32) -> BodyAnims {
        let fall = [0.0, 0.0, -300.0];
        let mut avatar = AvatarState {
            movement_state_bits: bits,
            gravity: 800,
            velocity: fall,
            ..standing(at, on)
        };
        let mut tick_vars = crate::testing::tick_vars(false);
        tick_vars.prior_tick_speed_vec = fall;
        probe(world, &mut avatar, &mut tick_vars)
    }

    #[test]
    fn stepping_off_a_drop_plays_the_jump() {
        let jumped = BodyAnims {
            jump: true,
            ..BodyAnims::default()
        };
        assert_eq!(falling_anims(&BARE, [0.0, 0.0, 2.0], ON_WORLD, 0), jumped);
        assert!(falling_anims(&BARE, [0.0, 0.0, 100.0], ON_WORLD, 0).jump);
        assert!(!falling_anims(&BARE, [0.0, 0.0, 0.5], ON_WORLD, 0).jump);
        assert!(!falling_anims(&BARE, [0.0, 0.0, 2.0], NO_ENTITY_SLOT, 0).jump);
        assert!(!falling_anims(&BARE, [0.0, 0.0, 2.0], ON_WORLD, dive::PMF_DIVE).jump);
    }

    #[test]
    fn hard_touchdown_plays_the_landing() {
        // 300 units a second downward is a 56-unit fall.
        let landed = BodyAnims {
            land: true,
            ..BodyAnims::default()
        };
        assert_eq!(falling_anims(&BARE, [0.0; 3], NO_ENTITY_SLOT, 0), landed);
        assert!(!falling_anims(&BARE, [0.0; 3], ON_WORLD, 0).land);
        assert!(!falling_anims(&BARE, [0.0; 3], NO_ENTITY_SLOT, dive::PMF_DIVE).land);
    }
}
