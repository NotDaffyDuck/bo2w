//! What happens when a missile's move ends against something: each kind of thing it can meet has its own answer
//! (stick, bounce, go off, die as a dud), and some of them leave damage owing.

use entity_t6::trajectory::tr_type::STATIONARY;
use trace_t6::surface_type_from_flags;

use super::blast::{Blast, DirectHit, Debts};
use super::bounce::{FLESH_SURFACE_FLAGS, bounce_off, unarmed_dud};
use super::events::{PROJ_EXPLOSION_BOLT, StuckTo, push_stick_event};
use super::facts::EquipmentRuntimeFacts;
use super::rider::ride_on;
use super::scene::{SURF_FLESH, SURF_RIOTSHIELD, tell_struck_entity};
use super::state::ProjectileState;
use super::stick::{attach_bolt_grenade, knife_impact, settle_on_surface, sticks_to_surface, stick_missile};
use super::vec3::unit;
use crate::bullet_collision::{HITLOC_SHIELD, PlayerCollisionPose};
use crate::frame::FrameWorld;
use crate::identities::DamageSource;
use crate::t6::grenade_stuck::{self, BounceHit};
use crate::{ColliderId, Tick};

/// Where and against what a move ended.
#[derive(Clone, Copy)]
pub(super) struct Touch {
    pub end: [f32; 3],
    pub normal: [f32; 3],
    pub collider: Option<ColliderId>,
    pub fraction: f32,
}

/// What the answer to a touch needs to know about the tick it happened in.
pub(super) struct Moment<'a> {
    pub time: i32,
    /// Velocity at the end of the move being resolved.
    pub velocity: [f32; 3],
    pub bodies: &'a [PlayerCollisionPose],
    /// How far a short move between two points gets before something blocks it.
    pub step_off: &'a dyn Fn([f32; 3], [f32; 3]) -> f32,
}

/// The surface material under a world hit, if the thing hit is the world.
fn world_surface(collider: Option<ColliderId>) -> (Option<u32>, Option<u8>) {
    match collider {
        Some(ColliderId::World { surface_flags, .. }) => (Some(surface_flags), Some(surface_type_from_flags(surface_flags))),
        _ => (None, None),
    }
}

fn blast_at(missile: &ProjectileState, touch: &Touch, surface: u8, splash: bool) -> Blast {
    Blast { missile: *missile, origin: touch.end, normal: touch.normal, surface, splash }
}

/// A scripted entity's bone or linked brush, and the model it belongs to.
fn scripted_owner(collider: ColliderId) -> Option<crate::identities::ScriptModelId> {
    match collider {
        ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. } => owner.script_model(),
        ColliderId::World { .. } | ColliderId::Player { .. } => None,
    }
}

/// Resolves the move of `missile` ending at `touch`. Returns the blast when the contact sets it off.
pub(super) fn resolve(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    facts: EquipmentRuntimeFacts,
    touch: Touch,
    moment: &Moment,
    debts: &mut Debts,
) -> Option<Blast> {
    let armed = missile.is_armed(facts.projectile_activate_dist);
    if facts.is_retrievable_knife() {
        knife_contact(world, tick, missile, facts, touch, moment, debts);
        return None;
    }
    match touch.collider {
        Some(ColliderId::Player { client, life, hitloc }) => {
            player_contact(world, tick, missile, facts, touch, (client, life, hitloc), armed, moment, debts)
        }
        Some(entity @ (ColliderId::EntityDObjBone { .. } | ColliderId::EntityLinkedBrush { .. })) => {
            entity_contact(world, tick, missile, facts, touch, entity, armed, moment, debts)
        }
        Some(ColliderId::World { glass_encoded, surface_flags, .. }) => {
            world_contact(world, tick, missile, facts, touch, (glass_encoded, surface_flags), armed, moment)
        }
        None => open_contact(missile, facts, touch, armed),
    }
}

/// A thrown knife or blade: a player struck takes its impact, then the knife sticks or bounces by its own rules.
fn knife_contact(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    facts: EquipmentRuntimeFacts,
    touch: Touch,
    moment: &Moment,
    debts: &mut Debts,
) {
    let struck = match touch.collider {
        Some(ColliderId::Player { client, hitloc, .. }) => Some((client, hitloc)),
        _ => None,
    };
    if let Some((client, hitloc)) = struck {
        debts.players.push(DirectHit { missile: *missile, target: client, hitloc, impact: true });
    }
    let (_, surface) = world_surface(touch.collider);
    knife_impact(world, tick, missile, touch.normal, touch.fraction, surface.unwrap_or(0), struck);
    // What a ballistic knife's blade struck, for the scripts' `stationary` hook.
    if facts.ballistic_blade {
        use crate::t6::knife::{Landing, Prey};
        let prey = match (struck, touch.collider.and_then(scripted_owner)) {
            (Some((client, hitloc)), _) => Prey::Player { client: client.0, hitloc },
            (None, Some(model)) => Prey::Model(model),
            (None, None) => Prey::World,
        };
        let landing = Landing {
            endpos: missile.origin,
            normal: touch.normal,
            dir: unit(moment.velocity).unwrap_or([1.0, 0.0, 0.0]),
            prey,
        };
        crate::t6::knife::record(world.ecs(), missile.id.0, landing);
    }
    if missile.pos.tr_type == STATIONARY
        && let Some(ColliderId::World { glass_encoded, .. }) = touch.collider
        && glass_encoded != 0
    {
        missile.stuck_pane = Some(u32::from(glass_encoded) - 1);
    }
}

#[allow(clippy::too_many_arguments)]
fn player_contact(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    facts: EquipmentRuntimeFacts,
    touch: Touch,
    (client, life, hitloc): (crate::ClientId, crate::identities::LifeSequence, u8),
    armed: bool,
    moment: &Moment,
    debts: &mut Debts,
) -> Option<Blast> {
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "nade tick {}: player {client:?} hitloc {hitloc} stick {} armed {armed} impact explode {}",
            moment.time,
            facts.stick_to_players,
            facts.proj_impact_explode
        );
    }
    let first_impact = !facts.limit_grenade_impacts || missile.impact_target.replace(client) != Some(client);
    // A riot shield is its own surface; the rest of him is flesh.
    let on_shield = hitloc == HITLOC_SHIELD;
    let surface = if on_shield { SURF_RIOTSHIELD } else { SURF_FLESH };
    let record = |missile: &ProjectileState, impact: bool| DirectHit { missile: *missile, target: client, hitloc, impact };
    if facts.stick_to_players {
        // Black Ops II hits, then sticks (a bolt as well: the crossbow's own hit).
        let bolt = facts.projectile_explosion_type == PROJ_EXPLOSION_BOLT;
        if (facts.limit_grenade_impacts || bolt) && first_impact {
            debts.players.push(record(missile, true));
        }
        stick_missile(tick, missile, touch.end);
        missile.rider = ride_on(moment.bodies, client, life, hitloc, touch.end).map(|rider| super::rider::StuckRider {
            shield: on_shield,
            ..rider
        });
        let stuck = StuckTo { player: Some(client), shield: on_shield, struck: Some((client, hitloc)) };
        push_stick_event(world, tick, missile, stuck, touch.normal, surface);
        attach_bolt_grenade(world, tick, missile, &facts);
        return None;
    }
    if !armed && unarmed_dud(&facts, missile, tick, touch.fraction, touch.normal, !on_shield) {
        debts.players.push(record(missile, true));
        missile.live = false;
        return Some(blast_at(missile, &touch, 0, false));
    }
    if armed && facts.proj_impact_explode {
        debts.players.push(record(missile, false));
        return Some(blast_at(missile, &touch, 0, true));
    }
    if first_impact {
        debts.players.push(record(missile, true));
    }
    let origin = touch.end;
    if bounce_off(world, tick, missile, origin, touch.normal, touch.fraction, 0, moment.step_off) {
        grenade_stuck::bounced(world.ecs(), missile, touch.normal, BounceHit::Player(client.0), surface);
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn entity_contact(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    facts: EquipmentRuntimeFacts,
    touch: Touch,
    entity: ColliderId,
    armed: bool,
    moment: &Moment,
    debts: &mut Debts,
) -> Option<Blast> {
    let model = scripted_owner(entity);
    if armed && facts.proj_impact_explode {
        if let Some(target) = model {
            let bone = match entity {
                ColliderId::EntityDObjBone { bone, .. } => Some(usize::from(bone)),
                _ => None,
            };
            debts.things.push((
                DamageSource::Projectile(missile.id),
                crate::script::EntityHit {
                    target,
                    amount: facts.impact_damage.max(0),
                    attacker: Some(missile.owner),
                    means: "",
                    weapon: missile.weapon,
                    point: touch.end,
                    dir: missile.velocity,
                    bone,
                    flags: 0,
                },
            ));
        }
        return Some(blast_at(missile, &touch, 0, true));
    }
    if sticks_to_surface(facts, touch.normal) {
        let surface = match entity {
            ColliderId::EntityDObjBone { surface_flags, .. } | ColliderId::EntityLinkedBrush { surface_flags, .. } => {
                surface_type_from_flags(surface_flags)
            }
            _ => 0,
        };
        settle_on_surface(world, tick, missile, touch.end, touch.normal, touch.fraction, surface);
        tell_struck_entity(world, missile.id, entity);
        return None;
    }
    if !armed && unarmed_dud(&facts, missile, tick, touch.fraction, touch.normal, false) {
        missile.live = false;
        return Some(blast_at(missile, &touch, 0, false));
    }
    if bounce_off(world, tick, missile, touch.end, touch.normal, touch.fraction, 0, moment.step_off) {
        let hit = model.map_or(BounceHit::World, BounceHit::Model);
        grenade_stuck::bounced(world.ecs(), missile, touch.normal, hit, 0);
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn world_contact(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    facts: EquipmentRuntimeFacts,
    touch: Touch,
    (glass_encoded, surface_flags): (u16, u32),
    armed: bool,
    moment: &Moment,
) -> Option<Blast> {
    let surface = surface_type_from_flags(surface_flags);
    if armed && facts.proj_impact_explode {
        return Some(blast_at(missile, &touch, surface, true));
    }
    if sticks_to_surface(facts, touch.normal) {
        if glass_encoded != 0 {
            let pane = u32::from(glass_encoded).saturating_sub(1);
            if world.world_objects().glass_is_solid(pane) {
                missile.stuck_pane = Some(pane);
            }
        }
        settle_on_surface(world, tick, missile, touch.end, touch.normal, touch.fraction, surface);
        return None;
    }
    let flesh = surface_flags == FLESH_SURFACE_FLAGS;
    if !armed && unarmed_dud(&facts, missile, tick, touch.fraction, touch.normal, flesh) {
        missile.live = false;
        return Some(blast_at(missile, &touch, surface, false));
    }
    if bounce_off(world, tick, missile, touch.end, touch.normal, touch.fraction, surface, moment.step_off) {
        grenade_stuck::bounced(world.ecs(), missile, touch.normal, BounceHit::World, surface);
    }
    None
}

/// A hit on something with no identity (open space the trace could not name): an armed impact fuse goes off, an
/// unarmed missile dies; anything else flies on.
fn open_contact(missile: &mut ProjectileState, facts: EquipmentRuntimeFacts, touch: Touch, armed: bool) -> Option<Blast> {
    if armed && facts.proj_impact_explode {
        return Some(blast_at(missile, &touch, 0, true));
    }
    if !armed {
        missile.live = false;
        return Some(blast_at(missile, &touch, 0, false));
    }
    None
}
