//! Melee swings: a knife slash or a bash, traced ahead of the eye and applied to whatever it touches.

use super::shot_log;
use crate::shot_trace::{
    ServerModelSource, ShotRayRequest, ShotPathLeg, HitTargetId, HITLOC_SHIELD, SHOT_WORLD_FILTER, shot_legs_with_filter,
    pane_from_hit,
};
use crate::step_view::StepWorldView;
use crate::match_ids::{HarmOrigin, LifeCount, BuckshotId};
use crate::player_ledger::{PlayerSessionStage, EventRecipients};
use crate::sim_parts::{SeatNo, StepNo};
use crate::glass_and_props::{Blow, Strike};
use entity_t6::EventCode;
use weapon_t6::melee_trace::{
    LUNGE_PROBE_SPREAD, LUNGE_BOX_TALL_STOCK, LUNGE_BOX_WIDE_STOCK, lunge_probe_total, lunge_probe_tip,
};
use weapon_t6::penetration::PierceCapability;

/// The parm bit of a melee hit event that says a riot shield (carried or planted) took the swing. BO2's client plays
/// the knife-on-shield sound for a knife and nothing for a shield bash.
pub const MELEE_PARM_SHIELD: i32 = 2;

/// Surface flag of geometry a melee trace passes through.
const SURFACE_NO_MELEE: u32 = 0x10;

/// BO2 starts a player's swing this far behind the eye, so a target pressed against the attacker is still ahead of
/// the trace.
const SWING_START_BEHIND_EYE: f32 = 10.0;

/// One melee swing's numbers: base damage, hit reach, and whether it is a knife (the hit sound).
pub(crate) struct MeleeSwing {
    pub harm: i32,
    pub range: f32,
    pub knife: bool,
}

/// What a swing's traces found.
struct SwingScan {
    from: [f32; 3],
    /// Where the first line, as far as it reaches, crosses the damage triggers.
    trigger_end: Option<[f32; 3]>,
    best: Option<ShotPathLeg>,
}

/// How far apart two points are, squared.
fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let mut sum = 0.0;
    for (p, q) in a.into_iter().zip(b) {
        let d = p - q;
        sum += d * d;
    }
    sum
}

/// Traces the swing box ahead of the eye (BO2's Melee_Trace) and picks the thing hit. The first offset that touches a
/// player is the hit, even when another offset met a wall nearer; otherwise the nearest thing touched wins, and a
/// later offset wins a tie.
fn scan_swing(
    world: &StepWorldView,
    tick: StepNo,
    attacker: SeatNo,
    spot: [f32; 3],
    aim: [f32; 3],
    range: f32,
) -> SwingScan {
    let (ahead, right, up) = game_math::euler_basis(aim);
    let (width, height) = (LUNGE_BOX_WIDE_STOCK, LUNGE_BOX_TALL_STOCK);
    let probe = world.rewind_request_for(attacker, tick);
    let glass_pairs = world.world_objects().pane_harm_entries();
    let pane_stands = |piece: u16| {
        let wanted = u32::from(piece);
        let damage = glass_pairs.iter().filter(|entry| entry.0 == wanted).map(|entry| entry.1).next().unwrap_or(0);
        crate::glass_and_props::shard_blocks_shots(damage)
    };
    let start: [f32; 3] = std::array::from_fn(|i| spot[i] - ahead[i] * SWING_START_BEHIND_EYE);
    let (poses, entries) = (&probe.players.bodies_then, &probe.entities.rows);

    let mut scan = SwingScan { from: start, trigger_end: None, best: None };
    let mut nearest = 1.0f32;
    let lines = lunge_probe_total(width, height);
    for (index, offset) in LUNGE_PROBE_SPREAD.iter().take(lines).enumerate() {
        let end = lunge_probe_tip(spot, ahead, right, up, range, width, height, *offset);
        let mut ray = ShotRayRequest { from: start, to: end, bits: SHOT_WORLD_FILTER, skip_client_hit: None, ignore_model: None, ignore: None };
        ray.ignore = Some(attacker);
        let (legs, _) = shot_legs_with_filter(
            world.collision_solids(),
            world.collision_tree(),
            world.collision_submodel_set(),
            world.collision_triangles(),
            poses,
            entries,
            &ray,
            PierceCapability::default(),
            world.wall_pierce_depths(),
            &pane_stands,
            None,
        );
        let touched = legs.iter().find(|segment| segment.collider.is_some());
        if index == 0 {
            scan.trigger_end = Some(touched.map_or(end, |segment| segment.to));
        }
        let Some(segment) = touched else {
            continue;
        };
        if segment.face_bits & SURFACE_NO_MELEE != 0 {
            continue;
        }
        let line = distance_squared(end, start);
        if line <= 0.0 {
            continue;
        }
        // How far along the line the touch is (1: its very end, which does not count).
        let part = distance_squared(segment.to, start).sqrt() / line.sqrt();
        if part >= 1.0 {
            continue;
        }
        if matches!(segment.collider, Some(HitTargetId::Player { .. })) {
            scan.best = Some(*segment);
            break;
        }
        if part > nearest {
            continue;
        }
        nearest = part;
        scan.best = Some(*segment);
    }
    scan
}

/// The server's melee hit: traces the swing box ahead of the eye and damages what it finds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_melee_hit(
    level: &mut StepWorldView,
    step: StepNo,
    attacker: SeatNo,
    striker_span: LifeCount,
    gun_no: u32,
    spot: [f32; 3],
    aim: [f32; 3],
    swing: MeleeSwing,
) {
    if swing.harm <= 0 {
        return;
    }
    let forward = game_math::euler_basis(aim).0;
    let scan = scan_swing(level, step, attacker, spot, aim, swing.range);

    if let Some(to) = scan.trigger_end
        && level.emits_state_snapshot()
    {
        crate::zombie_rt::damage_triggers::bullet_path(level.ecs(), attacker, scan.from, to, swing.harm, "MOD_MELEE");
    }
    let Some(leg) = scan.best else {
        if shot_log() {
            diag::info!(Sim, "melee hit: attacker {} touched nothing", attacker.0);
        }
        return;
    };

    let amount = swing.harm + (level.combat_dice_mut().draw_word() % 5) as i32;
    // A riot shield takes a swing it stops (the scripts hear "riotshield" and hurt no one). A riot-shield bash into a
    // shield still reaches the player, with no hit location.
    let on_shield = matches!(leg.collider, Some(HitTargetId::Player { hitloc: HITLOC_SHIELD, .. }));
    let other = struck_entity_number(level, leg.collider);

    // Weapon_Melee_internal's sound and blood. On a BO2 map its rules decide (crate::zombie_rt::melee); otherwise a player or
    // a model's bones struck is a hit that bleeds, and a shield or anything else a miss.
    let bo2 = crate::zombie_rt::melee::runs(level.ecs());
    let verdict = if bo2 {
        bo2_verdict(level, attacker, leg.collider, on_shield)
    } else {
        let bleeds = !on_shield
            && matches!(
                leg.collider,
                Some(HitTargetId::Player { .. } | HitTargetId::PosedBone { .. })
            );
        crate::zombie_rt::melee::Verdict { hit: bleeds, shield: false, blood: bleeds }
    };
    let victim_alive = match leg.collider {
        Some(HitTargetId::Player { client, .. }) => level
            .client_meta(client)
            .is_some_and(|meta| meta.lifecycle == PlayerSessionStage::Alive),
        _ => true,
    };
    let glass = match leg.collider {
        Some(HitTargetId::World { .. }) => pane_from_hit(leg.impact_category, leg.struck_entity_num),
        _ => None,
    };
    if shot_log() {
        let word = if verdict.hit { "hit" } else { "miss" };
        diag::info!(Sim, "melee hit: attacker {} hit {other} ({word}) for {amount}", attacker.0);
    }

    let me = attacker.0 as i32;
    let describe = |event_arg| {
        let mut payload = crate::HappeningBody { event_parm: event_arg, weapon: gun_no, other_entity_num: other, ..Default::default() };
        (payload.number, payload.attacker_entity_num) = (me, me);
        (payload.origin, payload.direction) = (leg.to, forward);
        (payload.surf_type, payload.surface_flags) = (leg.face_kind, leg.face_bits);
        payload
    };
    // BO2 sends the blood first, then the hit or miss; a pane of glass gets neither (it breaks instead).
    if verdict.blood && victim_alive {
        level.post_entity_event(step, EventRecipients::All, EventCode::MELEE_BLOOD, describe(0));
    }
    if !(bo2 && glass.is_some()) {
        // The parm: bit 0 = a knife, not a shield bash (the hit sound); bit 1 = a riot shield took it.
        let parm = i32::from(swing.knife) | if verdict.shield { MELEE_PARM_SHIELD } else { 0 };
        let kind = if verdict.hit { EventCode::MELEE_HIT } else { EventCode::MELEE_MISS };
        level.post_entity_event(step, EventRecipients::All, kind, describe(parm));
    }

    match leg.collider {
        Some(HitTargetId::Player { client, life, hitloc }) if victim_alive => {
            let try_no = crate::HarmRequest {
                source: HarmOrigin::Melee,
                pellet: BuckshotId(0),
                attacker,
                killer_life: striker_span,
                target: client,
                victim_life_index: life,
                gun: gun_no,
                much: amount,
                cam_ent_since: 0,
                attacker_source_pos: Some(spot),
                hit_part: if on_shield && !swing.knife { 0 } else { hitloc },
                blast: false,
                passed_through_wall: false,
                impact: false,
            };
            let _ = crate::harm::land_harm(level, step, &try_no);
        }
        // A script entity (a zombie, a planted riot shield, a car) takes the knife as it takes a bullet.
        Some(HitTargetId::PosedBone { owner, bone: joint, .. }) => {
            hurt_entity(level, step, owner, Some(usize::from(joint)), &leg, attacker, gun_no, amount, forward);
        }
        Some(HitTargetId::AttachedBrush { owner, .. }) => {
            hurt_entity(level, step, owner, None, &leg, attacker, gun_no, amount, forward);
        }
        Some(HitTargetId::World { .. }) => {
            if let Some(piece) = glass {
                break_pane(level, step, piece, leg.to, forward);
            }
        }
        _ => {}
    }
}

/// The entity number the swing struck: a player's slot, a script entity's slot, or the world.
fn struck_entity_number(world: &StepWorldView, shape_hit: Option<HitTargetId>) -> i32 {
    let world_slot = i32::from(trace_t6::WORLD_ENTITY_SLOT);
    match shape_hit {
        Some(HitTargetId::Player { client: who, .. }) => who.0 as i32,
        Some(HitTargetId::PosedBone { owner, .. } | HitTargetId::AttachedBrush { owner, .. }) => owner
            .scripted_model()
            .and_then(|model| world.game_actor_number(model))
            .unwrap_or(world_slot),
        _ => world_slot,
    }
}

/// BO2's rules for what a swing at `collider` sounds like and whether it bleeds.
fn bo2_verdict(
    world: &mut StepWorldView,
    attacker: SeatNo,
    collider: Option<HitTargetId>,
    on_shield: bool,
) -> crate::zombie_rt::melee::Verdict {
    let struck = match collider {
        Some(HitTargetId::Player { client: who, .. }) => {
            Some(crate::zombie_rt::melee::Struck::Player { slot: who.0, shield: on_shield })
        }
        Some(HitTargetId::PosedBone { owner, .. } | HitTargetId::AttachedBrush { owner, .. }) => {
            owner.scripted_model().map(crate::zombie_rt::melee::Struck::Model)
        }
        _ => None,
    };
    struck
        .and_then(|struck| crate::zombie_rt::melee::verdict(world.ecs(), attacker.0, struck))
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn hurt_entity(
    world: &mut StepWorldView,
    tick: StepNo,
    owner: ServerModelSource,
    bone: Option<usize>,
    leg: &ShotPathLeg,
    attacker: SeatNo,
    gun: u32,
    harm: i32,
    dir: [f32; 3],
) {
    // bo2zm: a breakable prop (a mannequin) takes the knife as a hit.
    if let Some(bone) = bone.and_then(|bone| u16::try_from(bone).ok())
        && crate::staged_props::resolve_destructible_hit(
            world,
            tick,
            owner,
            bone,
            harm.max(0) as u32,
            crate::glass_and_props::t6_means_from_name("MOD_MELEE"),
            leg.to,
            dir,
            Some(attacker),
            gun,
        )
    {
        return;
    }
    let Some(target) = owner.scripted_model() else {
        return;
    };
    crate::gsc_host::damage_entity(
        world.ecs(),
        &crate::gsc_host::EntityHit {
            target,
            amount: harm,
            attacker: Some(attacker),
            means: "MOD_MELEE",
            gun,
            point: leg.to,
            way: dir,
            bone,
            bits: 0,
        },
    );
}

/// Black Ops II: a melee swing breaks a glass pane at once.
fn break_pane(world: &mut StepWorldView, tick: StepNo, piece: u32, at: [f32; 3], heading: [f32; 3]) {
    let for_instant_millis = i32::try_from(tick.0.saturating_mul(crate::SIM_STEP_MILLIS)).unwrap_or(i32::MAX);
    world.strike_glass(Blow {
        pane: piece,
        harm: crate::glass_and_props::T6_GLASS_MELEE_DAMAGE,
        means: crate::glass_and_props::T6_MOD_MELEE,
        at_ms: for_instant_millis,
        strike: Strike { at, way: heading },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shield_parm_is_its_own_bit() {
        assert_eq!(MELEE_PARM_SHIELD & 1, 0);
        assert_eq!(i32::from(true) | MELEE_PARM_SHIELD, 3);
    }

    #[test]
    fn vector_helpers_work_on_all_three_axes() {
        assert_eq!(distance_squared([5.0, 7.0, 9.0], [1.0, 2.0, 3.0]), 77.0);
        assert_eq!(distance_squared([1.0, 2.0, 2.0], [0.0; 3]), 9.0);
    }
}
