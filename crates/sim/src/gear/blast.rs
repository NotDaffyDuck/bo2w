//! What a tick's contacts cost the world: the damage of direct hits, and for every missile that went off, its notes,
//! its entity event and its blast.

use super::scene::required_facts;
use super::flying::{FlyingProjectile, WeaponNote, shell_born_ms};
use super::vec3::look_dir;
use crate::harm::{HarmRequest, ExplosionBlast};
use crate::step_view::StepWorldView;
use crate::match_ids::{HarmOrigin, BuckshotId};
use crate::player_ledger::EventRecipients;
use crate::{SeatNo, HappeningBody, StepNo};
use entity_t6::EventCode;

/// Weapon file `projExplosion` of a flashbang.
const PROJ_EXPLOSION_FLASH: i32 = 2;

/// A missile that went off (or died as a dud) this tick, as it stood then.
#[derive(Clone, Copy)]
pub(super) struct Blast {
    pub missile: FlyingProjectile,
    pub spot: [f32; 3],
    pub face_dir: [f32; 3],
    pub surface: u8,
    /// Whether it explodes for damage (a dud only dies).
    pub splash: bool,
}

/// A player the missile struck directly, with the missile as it was at that moment.
pub(super) struct DirectHit {
    pub missile: FlyingProjectile,
    pub target: SeatNo,
    pub hit_part: u8,
    /// The strike counts as an impact (not just the contact that sets off a blast).
    pub impact: bool,
}

/// The damage a tick's contacts owe, gathered while the missile moved and paid once it has been dealt with.
#[derive(Default)]
pub(super) struct Debts {
    pub players: Vec<DirectHit>,
    pub things: Vec<(HarmOrigin, crate::gsc_host::EntityHit)>,
}

impl Debts {
    pub(super) fn is_empty(&self) -> bool {
        self.players.is_empty() && self.things.is_empty()
    }
}

/// Pays what `debts` records: the impact damage to each player struck, then to each scripted thing.
pub(super) fn settle_debts(level: &mut StepWorldView, step: StepNo, debts: Debts) {
    for strike in debts.players {
        let missile = strike.missile;
        let facts = required_facts(level, missile.gun);
        let Some(target_meta) = level.client_meta(strike.target) else {
            continue;
        };
        let try_no = HarmRequest {
            source: HarmOrigin::Projectile(missile.id),
            pellet: BuckshotId(0),
            attacker: missile.owner,
            killer_life: missile.owner_respawn_count,
            target: strike.target,
            victim_life_index: target_meta.life_sequence,
            gun: missile.gun,
            much: facts.collision_harm.max(0),
            cam_ent_since: shell_born_ms(&missile),
            attacker_source_pos: Some(missile.origin),
            hit_part: strike.hit_part,
            blast: false,
            passed_through_wall: false,
            impact: strike.impact,
        };
        let _ = crate::harm::land_harm(level, step, &try_no);
    }
    for (giver, mut owed) in debts.things {
        if owed.means.is_empty() {
            owed.means = crate::client_script::means(level, giver, owed.gun, 0, false);
        }
        crate::gsc_host::damage_entity(level.ecs(), &owed);
    }
}

/// Announces each of `blasts` to the script host and the clients and, when this world resolves damage, deals the blast.
pub(super) fn set_off_blasts(world: &mut StepWorldView, tick: StepNo, blasts: Vec<Blast>, applies_harm: bool) {
    for blast in blasts {
        let missile = blast.missile;
        world.weapon_notes.push(WeaponNote::Detonated { id: missile.id, spot: blast.spot });
        if world.gadget_stats_lookup(missile.gun).is_some_and(|row| row.laser_sensor) {
            crate::zombie_rt::sensor_grenade::detonated(world.ecs(), &missile, blast.spot);
        }
        // BO2's scripts get the shot's `projectile_impact` at this point.
        crate::zombie_rt::note_detonation(world.ecs(), missile.id, blast.spot);
        if !blast.splash {
            continue;
        }
        let facts = required_facts(world, missile.gun);
        let flash = facts.shell_blast_kind == PROJ_EXPLOSION_FLASH;
        let kind = if flash { EventCode::FLASHBANG_EXPLODE } else { EventCode::BOMB_BURST };
        world.post_entity_event(
            tick,
            EventRecipients::All,
            kind,
            HappeningBody {
                number: missile.ent_no,
                attacker_entity_num: missile.owner.0 as i32,
                weapon: missile.gun,
                correlation: missile.id.0,
                origin: blast.spot,
                direction: blast.face_dir,
                surf_type: blast.surface,
                ..Default::default()
            },
        );
        if !applies_harm {
            continue;
        }
        if flash {
            crate::harm::blind_players_in_flash(
                world,
                blast.spot,
                facts.blast_outer_reach.max(0) as f32,
                facts.blast_reach_floor.max(0) as f32,
                missile.owner,
            );
        }
        let radius = facts.blast_outer_reach.max(facts.blast_reach_floor).max(0) as f32;
        let (inner, outer) = (facts.blast_core_damage, facts.blast_rim_damage);
        if radius <= 0.0 || (inner <= 0 && outer <= 0) {
            continue;
        }
        let cone_angle = facts.damage_cone_angle;
        let cone = (cone_angle > 0.0 && cone_angle < 180.0)
            .then(|| (look_dir(missile.turn_path.base_point), cone_angle.to_radians().cos()));
        let damage = ExplosionBlast {
            cone,
            origin: blast.spot,
            reach: radius,
            inner_damage: inner as f32,
            outer_damage: outer.max(0) as f32,
            gun: missile.gun,
            source: HarmOrigin::Projectile(missile.id),
            attacker: missile.owner,
            killer_life: missile.owner_respawn_count,
            cam_ent_since: shell_born_ms(&missile),
        };
        crate::harm::apply_explosion_blast(world, tick, &damage);
        // Black Ops II glass took this blast once already, in the blast.
        if !world.world_objects().has_t6_glass() {
            crate::harm::apply_shared_glass_blast(
                world,
                tick,
                blast.spot,
                inner as f32,
                outer as f32,
                radius,
                "MOD_EXPLOSIVE",
            );
        }
    }
}
