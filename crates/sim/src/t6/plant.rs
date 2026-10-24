//! bo2mp: BO2's plant probe for claymores, Bouncing Betties and the tactical insertion. The
//! weapon code asks two questions before a plant (`T6Env::tac_insert_allowed` and
//! `T6Env::plant_spot_ok`); the game answers them here from the world's clip, the way
//! PM_AllowTacticalInsertion and PM_WeaponAllowPlant's traces do (pm.c). Numbers and dvar
//! defaults from the BO2 exe.
//!
//! Only the clip world is traced. BO2's traces also meet entities: other players can block a
//! tactical insertion, and an aim trace that ends on an entity whose contents carry 0x200
//! refuses the plant without raising the gun. Neither is seen here yet. An entity a script
//! barred tactical insertions from (`allowtacticalinsertion(0)`, Hardpoint's live hill) is:
//! `entity_blocks_tac_insert`.

use game_math::{add_lean_to_position, angle_vectors};
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState};
use weapon_t6::cmd::button;
use weapon_t6::env::pmf;
use weapon_t6::facts::{FactsTable, weap_type};
use weapon_t6::player::{T6WeaponPs, wf};
use weapon_t6::state;

use crate::world::SimState;

/// Standing on the world (not on a mover or a player).
const ENTITYNUM_WORLD: i32 = ENTITYNUM_NONE - 1;
/// PM_AllowTacticalInsertion: a standing player's box (`playerMins` / `playerMaxs`) must fit
/// where he stands, traced with this mask.
const PLAYER_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
const PLAYER_MAXS: [f32; 3] = [15.0, 15.0, 70.0];
const PLAYER_MASK: u32 = 0x0081_0011;
/// The movement bits that stop a tactical insertion: climbing a mantle or a ladder, in the air
/// from a dive. (BO2 also tests 0x800000, which nothing sets here, and the jump bit 0x400000
/// only with `tu6_player_jump_blocks_tac_insert`, off by default.)
const TAC_BLOCK_FLAGS: u32 = pmf::MANTLE | pmf::LADDER | pmf::DTP_AIR;
/// PM_WeaponAllowPlant: how far the aim trace looks (`trace_dist`) and its mask.
const TRACE_DIST: f32 = 1000.0;
const AIM_MASK: u32 = 0x0080_0a31;
/// `bg_disableWeaponPlantingInWater` and `bg_plantInWaterDepth`: aimed at water, the plant needs
/// a floor this close under the surface.
const DISABLE_PLANTING_IN_WATER: bool = true;
const PLANT_IN_WATER_DEPTH: f32 = 5.0;
/// The depth probe's mask.
const DEPTH_MASK: u32 = 0x811;
/// The surface type in `surfaceFlags`, and water.
const SURF_TYPE_MASK: u32 = 0x03f0_0000;
const SURF_TYPE_WATER: u32 = 0x0140_0000;
/// A contents bit on water that may be planted in all the same.
const CONTENTS_PLANT_OK_WATER: u32 = 0x1_0000;

/// Both answers for this command: (`plant_spot_ok`, `tac_insert_allowed`). The traces run only
/// while a plant can be asked for: a plantable carried and an offhand key down, the plantable in
/// hand on the trigger, or an offhand under way. Otherwise both stand at yes (nothing reads them).
pub(crate) fn answers(
    sim: &SimState,
    ps: &PlayerState,
    gun: &T6WeaponPs,
    facts: &FactsTable,
    buttons: u32,
    t6_pm_flags: u32,
) -> (bool, bool) {
    let Some(w) = plantable(gun, facts) else {
        return (true, true);
    };
    let asking = buttons & (button::FRAG | button::SMOKE) != 0
        || (gun.weapon == w && buttons & button::ATTACK != 0)
        || gun.weap_flags & wf::USING_OFFHAND != 0
        || (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&gun.weaponstate);
    if !asking {
        return (true, true);
    }
    let tac = !facts.get(w).offhand.tactical_insertion || tac_insert_allowed(sim, ps, gun, t6_pm_flags);
    (spot_ok(sim, ps, facts, w), tac)
}

/// BO2's ClientThink, mid-throw with a tactical insertion (BG_UsingTacticalInsertion: an
/// offhand state, the offhand a tactical insertion): when his box meets an entity a script
/// barred them from (Player_EntityBlocksTacticalInsertion, an overlap of world boxes), the
/// weapFlags bit PM_AllowTacticalInsertion refuses on is set. The offhand press clears it.
pub(crate) fn entity_blocks_tac_insert(
    sim: &SimState,
    ps: &PlayerState,
    gun: &T6WeaponPs,
    facts: &FactsTable,
) -> bool {
    let using = (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&gun.weaponstate)
        && gun.off_hand_weapon != 0
        && facts.get(gun.off_hand_weapon).offhand.tactical_insertion;
    using
        && sim.tac_insert_blockers().iter().any(|(lo, hi)| {
            (0..3).all(|i| ps.origin[i] + PLAYER_MINS[i] <= hi[i] && lo[i] <= ps.origin[i] + PLAYER_MAXS[i])
        })
}

/// PM_AllowTacticalInsertion: not blocked by script, standing on the world, not climbing or
/// diving, and a standing player fits where he is. `t6_pm_flags` = the gun's pm_flags.
fn tac_insert_allowed(sim: &SimState, ps: &PlayerState, gun: &T6WeaponPs, t6_pm_flags: u32) -> bool {
    if gun.weap_flags & wf::RIOTSHIELD_CANT_PLACE != 0
        || ps.ground_entity_num != ENTITYNUM_WORLD
        || t6_pm_flags & TAC_BLOCK_FLAGS != 0
    {
        return false;
    }
    let tr = sim.trace_world(ps.origin, ps.origin, PLAYER_MINS, PLAYER_MAXS, PLAYER_MASK);
    !tr.startsolid && !tr.allsolid && tr.fraction >= 1.0
}

/// The plant the next check is for: the offhand in use, the gun in hand, then the held ones.
/// (A player carries one piece of equipment, so the first is the one.)
fn plantable(gun: &T6WeaponPs, facts: &FactsTable) -> Option<u32> {
    let checked = |w: u32| {
        if w == 0 {
            return false;
        }
        let f = facts.get(w);
        f.core.weap_type == weap_type::GRENADE && f.core.plantable && f.offhand.projectile_speed_up <= 0
    };
    [gun.off_hand_weapon, gun.weapon]
        .into_iter()
        .chain(gun.held.iter().map(|h| h.weapon))
        .find(|&w| checked(w))
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        v
    }
}

/// PM_WeaponAllowPlant's traces for plantable `w`: from the eye (with lean) 1000 units along the
/// throw; on water, down 5 units from the surface for a floor. `false` = aimed into deep water.
fn spot_ok(sim: &SimState, ps: &PlayerState, facts: &FactsTable, w: u32) -> bool {
    let o = facts.get(w).offhand;
    let (fwd, _, _) = angle_vectors(ps.viewangles);
    let speed = o.projectile_speed as f32;
    let mut dir = [fwd[0] * speed, fwd[1] * speed, o.projectile_speed_up as f32 + fwd[2] * speed];
    if o.projectile_speed_forward != 0 {
        let flat = (fwd[0] * fwd[0] + fwd[1] * fwd[1]).sqrt();
        if flat > 0.0 {
            let s = o.projectile_speed_forward as f32 / flat;
            dir[0] += fwd[0] * s;
            dir[1] += fwd[1] * s;
        }
    }
    let dir = normalize(dir);
    let eye = add_lean_to_position(
        [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current],
        ps.viewangles[1],
        ps.leanf,
        16.0,
        20.0,
    );
    let end = [
        eye[0] + dir[0] * TRACE_DIST,
        eye[1] + dir[1] * TRACE_DIST,
        eye[2] + dir[2] * TRACE_DIST,
    ];
    let tr = sim.trace_world(eye, end, [0.0; 3], [0.0; 3], AIM_MASK);
    if tr.sflags & SURF_TYPE_MASK != SURF_TYPE_WATER
        || !DISABLE_PLANTING_IN_WATER
        || tr.cflags & CONTENTS_PLANT_OK_WATER != 0
    {
        return true;
    }
    let hit = [
        eye[0] + (end[0] - eye[0]) * tr.fraction,
        eye[1] + (end[1] - eye[1]) * tr.fraction,
        eye[2] + (end[2] - eye[2]) * tr.fraction,
    ];
    let down = [hit[0], hit[1], hit[2] - PLANT_IN_WATER_DEPTH];
    let floor = sim.trace_world(hit, down, [0.0; 3], [0.0; 3], DEPTH_MASK);
    floor.fraction != 1.0 || floor.sflags & SURF_TYPE_MASK != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{SimBrush, SimContentBuilder};

    const CLAYMORE: u32 = 5;
    const TAC: u32 = 6;
    const SOLID: u32 = 0x1;
    const WATER: u32 = 0x20;

    fn aabb(min: [f32; 3], max: [f32; 3], contents: u32, surf: u32) -> SimBrush {
        SimBrush {
            planes: vec![
                [-1.0, 0.0, 0.0, -min[0]],
                [1.0, 0.0, 0.0, max[0]],
                [0.0, -1.0, 0.0, -min[1]],
                [0.0, 1.0, 0.0, max[1]],
                [0.0, 0.0, -1.0, -min[2]],
                [0.0, 0.0, 1.0, max[2]],
            ],
            contents,
            plane_surface_flags: vec![surf; 6],
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: surf,
        }
    }

    /// Ground for x < 0; for x > 0 a pool `depth` deep (water over a floor).
    fn world(depth: f32, ceiling: bool) -> SimState {
        let mut b = vec![
            aabb([-2000.0, -2000.0, -16.0], [0.0, 2000.0, 0.0], SOLID, 0),
            aabb([0.0, -2000.0, -depth], [2000.0, 2000.0, 0.0], WATER, SURF_TYPE_WATER),
            aabb([0.0, -2000.0, -depth - 16.0], [2000.0, 2000.0, -depth], SOLID, 0),
        ];
        if ceiling {
            b.push(aabb([-300.0, -100.0, 50.0], [-100.0, 100.0, 66.0], SOLID, 0));
        }
        let mut c = SimContentBuilder::default();
        c.set_clip_brushes(b);
        let mut sim = SimState::new();
        sim.install_content(c.finish());
        sim
    }

    fn facts() -> FactsTable {
        let mut t = FactsTable::new();
        let mut cl = weapon_t6::facts::T6WeaponFacts::default();
        cl.core.weap_type = weap_type::GRENADE;
        cl.core.plantable = true;
        cl.offhand.projectile_speed = 100;
        t.set(CLAYMORE, "claymore_mp", cl);
        let mut tac = cl;
        tac.offhand.tactical_insertion = true;
        t.set(TAC, "tactical_insertion_mp", tac);
        t
    }

    /// A player standing on the ground at x = -200, looking 30 degrees down along `yaw`.
    fn player(yaw: f32) -> PlayerState {
        PlayerState {
            // Resting on the floor leaves the movement code's 1/8 unit gap (touching counts as inside).
            origin: [-200.0, 0.0, 0.125],
            viewangles: [30.0, yaw, 0.0],
            view_height_current: 60.0,
            ground_entity_num: ENTITYNUM_WORLD,
            ..crate::world::blank_player_state()
        }
    }

    fn gun(w: u32) -> T6WeaponPs {
        let mut g = T6WeaponPs::default();
        g.give(w);
        g
    }

    #[test]
    fn a_claymore_aimed_into_deep_water_is_refused() {
        let f = facts();
        let g = gun(CLAYMORE);
        let keys = button::FRAG;
        // Aimed at the pool (the eye ray meets the water at x ~ -96... then on past 0).
        let mut ps = player(0.0);
        ps.origin[0] = -50.0;
        assert_eq!(answers(&world(300.0, false), &ps, &g, &f, keys, 0).0, false);
        // A puddle has a floor 3 units under the surface.
        assert_eq!(answers(&world(3.0, false), &ps, &g, &f, keys, 0).0, true);
        // Aimed back at the ground.
        assert_eq!(answers(&world(300.0, false), &player(180.0), &g, &f, keys, 0).0, true);
        // Nobody asking: no traces, both yes.
        assert_eq!(answers(&world(300.0, false), &ps, &g, &f, 0, 0), (true, true));
    }

    #[test]
    fn a_tactical_insertion_needs_room_to_stand_on_the_world() {
        let f = facts();
        let g = gun(TAC);
        let keys = button::FRAG;
        let open = world(300.0, false);
        assert_eq!(answers(&open, &player(180.0), &g, &f, keys, 0).1, true);
        // Under a ledge 50 units up: a standing player does not fit.
        assert_eq!(answers(&world(300.0, true), &player(180.0), &g, &f, keys, 0).1, false);
        // In the air, or on a mover.
        let mut ps = player(180.0);
        ps.ground_entity_num = ENTITYNUM_NONE;
        assert_eq!(answers(&open, &ps, &g, &f, keys, 0).1, false);
        // Climbing, or blocked by script.
        assert_eq!(answers(&open, &player(180.0), &g, &f, keys, pmf::MANTLE).1, false);
        let mut blocked = gun(TAC);
        blocked.weap_flags |= wf::RIOTSHIELD_CANT_PLACE;
        assert_eq!(answers(&open, &player(180.0), &blocked, &f, keys, 0).1, false);
        // A claymore does not ask.
        assert_eq!(answers(&world(300.0, true), &player(180.0), &gun(CLAYMORE), &f, keys, 0).1, true);
    }

    #[test]
    fn a_barred_hill_stops_a_tactical_insertion_mid_throw() {
        let f = facts();
        let mut sim = world(300.0, false);
        // A hill box around x = -200.
        sim.set_tac_insert_blockers(vec![([-260.0, -60.0, -10.0], [-140.0, 60.0, 120.0])]);
        let mut g = gun(TAC);
        g.off_hand_weapon = TAC;
        // Not throwing: nothing to bar.
        assert!(!entity_blocks_tac_insert(&sim, &player(0.0), &g, &f));
        g.weaponstate = state::OFFHAND_INIT;
        assert!(entity_blocks_tac_insert(&sim, &player(0.0), &g, &f));
        // Out of the hill, or a claymore in the throw.
        let mut out = player(0.0);
        out.origin[0] = -400.0;
        assert!(!entity_blocks_tac_insert(&sim, &out, &g, &f));
        g.off_hand_weapon = CLAYMORE;
        assert!(!entity_blocks_tac_insert(&sim, &player(0.0), &g, &f));
    }
}
