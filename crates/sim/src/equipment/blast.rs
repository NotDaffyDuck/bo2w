//! What a tick's contacts cost the world: the damage of direct hits, and for every missile that went off, its notes,
//! its entity event and its blast.

use super::scene::required_facts;
use super::state::{ProjectileState, WeaponNote, projectile_birth_ms};
use super::vec3::look_dir;
use crate::damage::{DamageAttempt, ExplosionBlast};
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, PelletId};
use crate::match_state::EventAudience;
use crate::{ClientId, EntityEventPayload, Tick};
use entity_t6::EntityEventKind;

/// Weapon file `projExplosion` of a flashbang.
const PROJ_EXPLOSION_FLASH: i32 = 2;

/// A missile that went off (or died as a dud) this tick, as it stood then.
#[derive(Clone, Copy)]
pub(super) struct Blast {
    pub missile: ProjectileState,
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub surface: u8,
    /// Whether it explodes for damage (a dud only dies).
    pub splash: bool,
}

/// A player the missile struck directly, with the missile as it was at that moment.
pub(super) struct DirectHit {
    pub missile: ProjectileState,
    pub target: ClientId,
    pub hitloc: u8,
    /// The strike counts as an impact (not just the contact that sets off a blast).
    pub impact: bool,
}

/// The damage a tick's contacts owe, gathered while the missile moved and paid once it has been dealt with.
#[derive(Default)]
pub(super) struct Debts {
    pub players: Vec<DirectHit>,
    pub things: Vec<(DamageSource, crate::script::EntityHit)>,
}

impl Debts {
    pub(super) fn is_empty(&self) -> bool {
        self.players.is_empty() && self.things.is_empty()
    }
}

/// Pays what `debts` records: the impact damage to each player struck, then to each scripted thing.
pub(super) fn pay(world: &mut FrameWorld, tick: Tick, debts: Debts) {
    for strike in debts.players {
        let missile = strike.missile;
        let facts = required_facts(world, missile.weapon);
        let Some(target_meta) = world.client_meta(strike.target) else {
            continue;
        };
        let attempt = DamageAttempt {
            source: DamageSource::Projectile(missile.id),
            pellet: PelletId(0),
            attacker: missile.owner,
            attacker_life: missile.owner_life,
            target: strike.target,
            target_life: target_meta.life_sequence,
            weapon: missile.weapon,
            amount: facts.impact_damage.max(0),
            killcam_entity_start_time: projectile_birth_ms(&missile),
            inflictor_origin: Some(missile.origin),
            hitloc: strike.hitloc,
            blast: false,
            penetrated: false,
            impact: strike.impact,
        };
        let _ = crate::damage::apply_damage_attempt(world, tick, &attempt);
    }
    for (source, mut hit) in debts.things {
        if hit.means.is_empty() {
            hit.means = crate::script_player::means(world, source, hit.weapon, 0, false);
        }
        crate::script::damage_entity(world.ecs(), &hit);
    }
}

/// Announces each of `blasts` to the script host and the clients and, when this world resolves damage, deals the blast.
pub(super) fn detonate(world: &mut FrameWorld, tick: Tick, blasts: Vec<Blast>, resolves_damage: bool) {
    for blast in blasts {
        let missile = blast.missile;
        world.weapon_notes.push(WeaponNote::Detonated { id: missile.id, origin: blast.origin });
        if world.equipment_facts_for(missile.weapon).is_some_and(|row| row.laser_sensor) {
            crate::t6::sensor_grenade::detonated(world.ecs(), &missile, blast.origin);
        }
        if !blast.splash {
            continue;
        }
        let facts = required_facts(world, missile.weapon);
        let flash = facts.projectile_explosion_type == PROJ_EXPLOSION_FLASH;
        let kind = if flash { EntityEventKind::FLASHBANG_EXPLODE } else { EntityEventKind::GRENADE_EXPLODE };
        world.push_entity_event(
            tick,
            EventAudience::All,
            kind,
            EntityEventPayload {
                number: missile.entnum,
                attacker_entity_num: missile.owner.0 as i32,
                weapon: missile.weapon,
                correlation: missile.id.0,
                origin: blast.origin,
                direction: blast.normal,
                surf_type: blast.surface,
                ..Default::default()
            },
        );
        if !resolves_damage {
            continue;
        }
        if flash {
            crate::damage::apply_flashbang_blast(
                world,
                blast.origin,
                facts.explosion_radius.max(0) as f32,
                facts.explosion_radius_min.max(0) as f32,
                missile.owner,
            );
        }
        let radius = facts.explosion_radius.max(facts.explosion_radius_min).max(0) as f32;
        let (inner, outer) = (facts.explosion_inner_damage, facts.explosion_outer_damage);
        if radius <= 0.0 || (inner <= 0 && outer <= 0) {
            continue;
        }
        let cone_angle = facts.damage_cone_angle;
        let cone = (cone_angle > 0.0 && cone_angle < 180.0)
            .then(|| (look_dir(missile.apos.tr_base), cone_angle.to_radians().cos()));
        let damage = ExplosionBlast {
            cone,
            origin: blast.origin,
            radius,
            inner_damage: inner as f32,
            outer_damage: outer.max(0) as f32,
            weapon: missile.weapon,
            source: DamageSource::Projectile(missile.id),
            attacker: missile.owner,
            attacker_life: missile.owner_life,
            killcam_entity_start_time: projectile_birth_ms(&missile),
        };
        crate::damage::apply_explosion_blast(world, tick, &damage);
        // Black Ops II glass took this blast once already, in the blast.
        if !world.world_objects().has_t6_glass() {
            crate::damage::apply_shared_glass_blast(
                world,
                tick,
                blast.origin,
                inner as f32,
                outer as f32,
                radius,
                "MOD_EXPLOSIVE",
            );
        }
    }
}
