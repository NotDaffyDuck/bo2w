//! One hit on one player: refused, or scaled by where it landed and handed to the victim's scripts.

use super::{DamageAttempt, DamageOutcome, DamageRefusal};
use crate::bullet_collision::HITLOC_SHIELD;
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence};
use crate::match_state::ClientLifecycle;
use crate::world::Tick;

/// Tries to hurt the attempt's target. The scripts decide what the damage does; this only turns away what cannot
/// land (no damage, no such player, a dead one, or one who has respawned since the shot) and applies the hit
/// location's multiplier.
pub(crate) fn apply_damage_attempt(world: &mut FrameWorld, tick: Tick, attempt: &DamageAttempt) -> DamageOutcome {
    let target = world.client_meta(attempt.target).map(|meta| (meta.lifecycle, meta.life_sequence));
    if let Some(refusal) = refusal(attempt, target) {
        return DamageOutcome::Refused(refusal);
    }
    let amount = located_amount(world, attempt);
    if amount <= 0 {
        return DamageOutcome::Refused(DamageRefusal::NonPositive);
    }
    crate::script_player::damage(world, tick, attempt, amount)
}

/// Why `attempt` cannot land on `target` (his lifecycle and life), if it cannot. The checks run in this order.
fn refusal(attempt: &DamageAttempt, target: Option<(ClientLifecycle, LifeSequence)>) -> Option<DamageRefusal> {
    if attempt.amount <= 0 {
        return Some(DamageRefusal::NonPositive);
    }
    let Some((lifecycle, life)) = target else {
        return Some(DamageRefusal::MissingTarget);
    };
    if lifecycle != ClientLifecycle::Alive {
        return Some(DamageRefusal::TargetNotAlive);
    }
    (life != attempt.target_life).then_some(DamageRefusal::StaleLife)
}

/// The attempt's damage after the hit location's multiplier from the weapon's table, as a whole number of at least 1.
fn located_amount(world: &FrameWorld, attempt: &DamageAttempt) -> i32 {
    if !takes_location_scale(attempt.source, attempt.hitloc, attempt.blast) {
        return attempt.amount;
    }
    let scale = world.combat_facts_for(attempt.weapon).map_or(1.0, |facts| facts.location_scale(attempt.hitloc));
    weapon_t6::hitloc::scaled_damage(attempt.amount, scale)
}

/// Whether a hit is multiplied by its hit location. Melee never is, and neither is a missile's blast (no location). A
/// hit the riot shield stopped is not either: the scripts count the bullet's whole damage as what the shield blocked.
fn takes_location_scale(source: DamageSource, hitloc: u8, blast: bool) -> bool {
    match source {
        DamageSource::Melee => false,
        DamageSource::Projectile(_) if blast => false,
        _ => hitloc != HITLOC_SHIELD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectileId;
    use crate::identities::{PelletId, ShotId};
    use crate::world::ClientId;
    use weapon_t6::hitloc::{HITLOC_HEAD, scaled_damage};

    fn attempt(amount: i32) -> DamageAttempt {
        DamageAttempt {
            source: DamageSource::Melee,
            pellet: PelletId(0),
            attacker: ClientId(1),
            attacker_life: LifeSequence(1),
            target: ClientId(2),
            target_life: LifeSequence(4),
            weapon: 0,
            amount,
            killcam_entity_start_time: 0,
            inflictor_origin: None,
            hitloc: 0,
            blast: false,
            penetrated: false,
            impact: false,
        }
    }

    const ALIVE_IN_LIFE_4: Option<(ClientLifecycle, LifeSequence)> = Some((ClientLifecycle::Alive, LifeSequence(4)));

    #[test]
    fn a_live_target_in_the_same_life_takes_the_hit() {
        assert_eq!(refusal(&attempt(30), ALIVE_IN_LIFE_4), None);
    }

    #[test]
    fn no_damage_is_turned_away_before_the_target_is_even_looked_up() {
        assert_eq!(refusal(&attempt(0), None), Some(DamageRefusal::NonPositive));
        assert_eq!(refusal(&attempt(-3), ALIVE_IN_LIFE_4), Some(DamageRefusal::NonPositive));
    }

    #[test]
    fn a_missing_or_dead_or_respawned_target_is_turned_away_in_that_order() {
        assert_eq!(refusal(&attempt(30), None), Some(DamageRefusal::MissingTarget));
        let dead = Some((ClientLifecycle::Dead, LifeSequence(4)));
        assert_eq!(refusal(&attempt(30), dead), Some(DamageRefusal::TargetNotAlive));
        let dead_and_stale = Some((ClientLifecycle::Dead, LifeSequence(9)));
        assert_eq!(refusal(&attempt(30), dead_and_stale), Some(DamageRefusal::TargetNotAlive));
        let respawned = Some((ClientLifecycle::Alive, LifeSequence(5)));
        assert_eq!(refusal(&attempt(30), respawned), Some(DamageRefusal::StaleLife));
    }

    #[test]
    fn a_missiles_own_hit_takes_the_multiplier_but_its_blast_does_not() {
        let missile = DamageSource::Projectile(ProjectileId(1));
        assert!(takes_location_scale(missile, HITLOC_HEAD, false));
        assert!(!takes_location_scale(missile, 0, true));
    }

    #[test]
    fn a_bullet_takes_the_multiplier_wherever_it_lands_but_the_shield() {
        let shot = DamageSource::Shot(ShotId(1));
        assert!(takes_location_scale(shot, HITLOC_HEAD, false));
        assert!(takes_location_scale(shot, 0, false));
        assert!(!takes_location_scale(shot, HITLOC_SHIELD, false));
        assert!(!takes_location_scale(DamageSource::Projectile(ProjectileId(1)), HITLOC_SHIELD, false));
    }

    #[test]
    fn melee_is_never_multiplied() {
        assert!(!takes_location_scale(DamageSource::Melee, HITLOC_HEAD, false));
        assert!(!takes_location_scale(DamageSource::Melee, 0, false));
    }

    #[test]
    fn a_multiplied_hit_is_cut_to_a_whole_number_of_at_least_one() {
        assert_eq!(scaled_damage(150, 1.5), 225);
        assert_eq!(scaled_damage(150, 0.0), 1);
        assert_eq!(scaled_damage(100, 1.0), 100);
    }
}
