//! What happens when a missile's move ends against something: each kind of thing it can meet has its own answer
//! (stick, bounce, go off, die as a dud), and some of them leave damage owing.

use entity_t6::flight::path_kind::AT_REST;
use trace_t6::surface_kind_of_flags;

use super::blast::{Blast, DirectHit, Debts};
use super::bounce::{FLESH_SURFACE_FLAGS, bounce_off, unarmed_dud};
use super::events::{PROJ_EXPLOSION_BOLT, StuckTo, push_stick_event};
use super::facts::GadgetLiveStats;
use super::rider::ride_on;
use super::scene::{SURF_FLESH, SURF_RIOTSHIELD, tell_struck_entity};
use super::flying::FlyingProjectile;
use super::stick::{attach_bolt_grenade, knife_impact, settle_on_surface, sticks_to_surface, stick_missile};
use super::vec3::unit;
use crate::shot_trace::{HITLOC_SHIELD, PlayerHitboxPose};
use crate::step_view::StepWorldView;
use crate::match_ids::HarmOrigin;
use crate::zombie_rt::grenade_stuck;
use crate::zombie_rt::grenade_stuck::BounceHit;
use crate::{HitTargetId, StepNo};

/// Where and against what a move ended.
#[derive(Clone, Copy)]
pub(super) struct Touch {
    pub to: [f32; 3],
    pub face_dir: [f32; 3],
    pub collider: Option<HitTargetId>,
    pub share: f32,
}

/// What the answer to a touch needs to know about the tick it happened in.
pub(super) struct Moment<'a> {
    pub time: i32,
    /// Velocity at the end of the move being resolved.
    pub motion: [f32; 3],
    pub bodies: &'a [PlayerHitboxPose],
    /// How far a short move between two points gets before something blocks it.
    pub step_off: &'a dyn Fn([f32; 3], [f32; 3]) -> f32,
}

/// The surface material under a world hit, if the thing hit is the world.
fn world_surface_kind(struck_on: Option<HitTargetId>) -> Option<u8> {
    match struck_on {
        Some(HitTargetId::World { surface_flags: face_bits, .. }) => Some(surface_kind_of_flags(face_bits)),
        _ => None,
    }
}

fn blast_at(missile: &FlyingProjectile, touch: &Touch, surface: u8, splash: bool) -> Blast {
    Blast { missile: *missile, spot: touch.to, face_dir: touch.face_dir, surface, splash }
}

/// A scripted entity's bone or linked brush, and the model it belongs to.
fn scripted_owner(struck_on: HitTargetId) -> Option<crate::match_ids::ScriptedModel> {
    match struck_on {
        HitTargetId::PosedBone { owner: holder, .. } | HitTargetId::AttachedBrush { owner: holder, .. } => holder.scripted_model(),
        HitTargetId::World { .. } | HitTargetId::Player { .. } => None,
    }
}

/// Resolves the move of `missile` ending at `touch`. Returns the blast when the contact sets it off.
pub(super) fn settle_touch(
    world: &mut StepWorldView,
    tick: StepNo,
    missile: &mut FlyingProjectile,
    facts: GadgetLiveStats,
    touch: Touch,
    moment: &Moment,
    debts: &mut Debts,
) -> Option<Blast> {
    let armed = missile.is_forecast_live(facts.shell_arming_range);
    if facts.is_retrievable_knife() {
        knife_contact(world, tick, missile, facts, touch, moment, debts);
        return None;
    }
    match touch.collider {
        Some(HitTargetId::Player { client, life, hitloc }) => {
            player_contact(world, tick, missile, facts, touch, (client, life, hitloc), armed, moment, debts)
        }
        Some(entity @ (HitTargetId::PosedBone { .. } | HitTargetId::AttachedBrush { .. })) => {
            entity_contact(world, tick, missile, facts, touch, entity, armed, moment, debts)
        }
        Some(HitTargetId::World { glass_hit_code, surface_flags: face_bits, .. }) => {
            world_contact(world, tick, missile, facts, touch, (glass_hit_code, face_bits), armed, moment)
        }
        None => open_contact(missile, facts, touch, armed),
    }
}

/// A thrown knife or blade: a player struck takes its impact, then the knife sticks or bounces by its own rules.
fn knife_contact(
    world: &mut StepWorldView,
    tick: StepNo,
    missile: &mut FlyingProjectile,
    facts: GadgetLiveStats,
    touch: Touch,
    moment: &Moment,
    debts: &mut Debts,
) {
    let struck = match touch.collider {
        Some(HitTargetId::Player { client: seat_hit, hitloc, .. }) => Some((seat_hit, hitloc)),
        _ => None,
    };
    if let Some((seat_hit, hitloc)) = struck {
        debts.players.push(DirectHit { missile: *missile, target: seat_hit, hit_part: hitloc, impact: true });
    }
    let surface = world_surface_kind(touch.collider);
    knife_impact(world, tick, missile, touch.face_dir, touch.share, surface.unwrap_or(0), struck);
    // What a ballistic knife's blade struck, for the scripts' `stationary` hook.
    if facts.ballistic_blade {
        use crate::zombie_rt::knife::{Landing, Prey};
        let prey = match (struck, touch.collider.and_then(scripted_owner)) {
            (Some((victim, hitloc)), _) => Prey::Player { slot: victim.0, hitloc },
            (None, Some(model)) => Prey::Model(model),
            (None, None) => Prey::World,
        };
        let landing = Landing {
            finish_point: missile.origin,
            face_dir: touch.face_dir,
            aim: unit(moment.motion).unwrap_or([1.0, 0.0, 0.0]),
            prey,
        };
        crate::zombie_rt::knife::record(world.ecs(), missile.id.0, landing);
    }
    if missile.at.tr_type == AT_REST
        && let Some(HitTargetId::World { glass_hit_code, .. }) = touch.collider
        && glass_hit_code != 0
    {
        missile.stuck_pane = Some(u32::from(glass_hit_code) - 1);
    }
}

#[allow(clippy::too_many_arguments)]
fn player_contact(
    world: &mut StepWorldView,
    step: StepNo,
    missile: &mut FlyingProjectile,
    facts: GadgetLiveStats,
    touch: Touch,
    (client, life, hitloc): (crate::SeatNo, crate::match_ids::LifeCount, u8),
    fuse_live: bool,
    moment: &Moment,
    debts: &mut Debts,
) -> Option<Blast> {
    if crate::gunfire::shot_log() {
        diag::info!(
            Sim,
            "nade tick {}: player {client:?} hitloc {hitloc} stick {} armed {fuse_live} impact explode {}",
            moment.time,
            facts.adheres_to_bodies,
            facts.shell_detonate_on_hit
        );
    }
    let first_impact = !facts.limit_grenade_impacts || missile.impact_target.replace(client) != Some(client);
    // A riot shield is its own surface; the rest of him is flesh.
    let on_shield = hitloc == HITLOC_SHIELD;
    let surface = if on_shield { SURF_RIOTSHIELD } else { SURF_FLESH };
    let record = |missile: &FlyingProjectile, impact: bool| DirectHit { missile: *missile, target: client, hit_part: hitloc, impact };
    if facts.adheres_to_bodies {
        // Black Ops II hits, then sticks (a bolt as well: the crossbow's own hit).
        let bolt = facts.shell_blast_kind == PROJ_EXPLOSION_BOLT;
        if (facts.limit_grenade_impacts || bolt) && first_impact {
            debts.players.push(record(missile, true));
        }
        stick_missile(step, missile, touch.to);
        missile.rider = ride_on(moment.bodies, client, life, hitloc, touch.to).map(|rider| super::rider::StuckRider {
            shield: on_shield,
            ..rider
        });
        let stuck = StuckTo { player: Some(client), shield: on_shield, struck: Some((client, hitloc)) };
        push_stick_event(world, step, missile, stuck, touch.face_dir, surface);
        attach_bolt_grenade(world, step, missile, &facts);
        return None;
    }
    if !fuse_live && unarmed_dud(&facts, missile, step, touch.share, touch.face_dir, !on_shield) {
        debts.players.push(record(missile, true));
        missile.in_flight = false;
        return Some(blast_at(missile, &touch, 0, false));
    }
    if (fuse_live && facts.shell_detonate_on_hit) || facts.rocket_ends_on_impact() {
        debts.players.push(record(missile, false));
        return Some(blast_at(missile, &touch, 0, true));
    }
    if first_impact {
        debts.players.push(record(missile, true));
    }
    let origin = touch.to;
    if bounce_off(world, step, missile, origin, touch.face_dir, touch.share, 0, moment.step_off) {
        grenade_stuck::bounced(world.ecs(), missile, touch.face_dir, BounceHit::Player(client.0), surface);
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn entity_contact(
    world: &mut StepWorldView,
    tick: StepNo,
    missile: &mut FlyingProjectile,
    facts: GadgetLiveStats,
    touch: Touch,
    entity: HitTargetId,
    on_now: bool,
    moment: &Moment,
    debts: &mut Debts,
) -> Option<Blast> {
    let model = scripted_owner(entity);
    if on_now && facts.shell_detonate_on_hit {
        if let Some(target) = model {
            let bone = match entity {
                HitTargetId::PosedBone { bone, .. } => Some(usize::from(bone)),
                _ => None,
            };
            debts.things.push((
                HarmOrigin::Projectile(missile.id),
                crate::gsc_host::EntityHit {
                    target,
                    amount: facts.collision_harm.max(0),
                    attacker: Some(missile.owner),
                    means: "",
                    gun: missile.gun,
                    point: touch.to,
                    way: missile.motion,
                    bone,
                    bits: 0,
                },
            ));
        }
        return Some(blast_at(missile, &touch, 0, true));
    }
    // bo2zm: a missile that does not explode still hits the entity it strikes for its impact damage (MOD_IMPACT), as
    // players are hit above: Buried's ballistic knife kills a zombie this way (BO2's `_zm_spawner` scores a
    // knife_ballistic MOD_IMPACT kill as `ballistic_knife_death`).
    if facts.collision_harm > 0
        && let Some(target) = model
    {
        let bone = match entity {
            HitTargetId::PosedBone { bone, .. } => Some(usize::from(bone)),
            _ => None,
        };
        debts.things.push((
            HarmOrigin::Projectile(missile.id),
            crate::gsc_host::EntityHit {
                target,
                amount: facts.collision_harm,
                attacker: Some(missile.owner),
                means: "MOD_IMPACT",
                gun: missile.gun,
                point: touch.to,
                way: missile.motion,
                bone,
                bits: 0,
            },
        ));
    }
    if sticks_to_surface(facts, touch.face_dir) {
        let surface = match entity {
            HitTargetId::PosedBone { surface_flags: face_bits, .. } | HitTargetId::AttachedBrush { surface_flags: face_bits, .. } => {
                surface_kind_of_flags(face_bits)
            }
            _ => 0,
        };
        settle_on_surface(world, tick, missile, touch.to, touch.face_dir, touch.share, surface);
        tell_struck_entity(world, missile.id, entity);
        return None;
    }
    if !on_now && unarmed_dud(&facts, missile, tick, touch.share, touch.face_dir, false) {
        missile.in_flight = false;
        return Some(blast_at(missile, &touch, 0, false));
    }
    if facts.rocket_ends_on_impact() {
        return Some(blast_at(missile, &touch, 0, true));
    }
    if bounce_off(world, tick, missile, touch.to, touch.face_dir, touch.share, 0, moment.step_off) {
        let hit = model.map_or(BounceHit::World, BounceHit::Model);
        grenade_stuck::bounced(world.ecs(), missile, touch.face_dir, hit, 0);
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn world_contact(
    world: &mut StepWorldView,
    tick: StepNo,
    missile: &mut FlyingProjectile,
    facts: GadgetLiveStats,
    touch: Touch,
    (glass_hit_code, surface_flags): (u16, u32),
    fuse_live: bool,
    moment: &Moment,
) -> Option<Blast> {
    let surface = surface_kind_of_flags(surface_flags);
    if fuse_live && facts.shell_detonate_on_hit {
        return Some(blast_at(missile, &touch, surface, true));
    }
    if sticks_to_surface(facts, touch.face_dir) {
        if glass_hit_code != 0 {
            let pane = u32::from(glass_hit_code).saturating_sub(1);
            if world.world_objects().pane_blocks_shots(pane) {
                missile.stuck_pane = Some(pane);
            }
        }
        settle_on_surface(world, tick, missile, touch.to, touch.face_dir, touch.share, surface);
        return None;
    }
    let flesh = surface_flags == FLESH_SURFACE_FLAGS;
    if !fuse_live && unarmed_dud(&facts, missile, tick, touch.share, touch.face_dir, flesh) {
        missile.in_flight = false;
        return Some(blast_at(missile, &touch, surface, false));
    }
    if facts.rocket_ends_on_impact() {
        return Some(blast_at(missile, &touch, surface, true));
    }
    if bounce_off(world, tick, missile, touch.to, touch.face_dir, touch.share, surface, moment.step_off) {
        grenade_stuck::bounced(world.ecs(), missile, touch.face_dir, BounceHit::World, surface);
    }
    None
}

/// A hit on something with no identity (open space the trace could not name): an armed impact fuse goes off, an
/// unarmed missile dies; anything else flies on.
fn open_contact(missile: &mut FlyingProjectile, facts: GadgetLiveStats, touch: Touch, armed: bool) -> Option<Blast> {
    if armed && facts.shell_detonate_on_hit {
        return Some(blast_at(missile, &touch, 0, true));
    }
    if !armed {
        missile.in_flight = false;
        return Some(blast_at(missile, &touch, 0, false));
    }
    None
}
