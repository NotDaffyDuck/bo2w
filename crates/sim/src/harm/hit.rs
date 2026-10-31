//! One hit on one player: refused, or scaled by where it landed and handed to the victim's scripts.

use super::{HarmRequest, HarmResult, HarmDenial};
use crate::shot_trace::HITLOC_SHIELD;
use crate::step_view::StepWorldView;
use crate::match_ids::{HarmOrigin, LifeCount};
use crate::player_ledger::PlayerSessionStage;
use crate::sim_parts::StepNo;

/// Tries to hurt the attempt's target. The scripts decide what the damage does; this only turns away what cannot
/// land (no damage, no such player, a dead one, or one who has respawned since the shot) and applies the hit
/// location's multiplier.
pub(crate) fn land_harm(world: &mut StepWorldView, tick: StepNo, attempt: &HarmRequest) -> HarmResult {
    let target = world.client_meta(attempt.target).map(|meta| (meta.lifecycle, meta.life_sequence));
    if let Some(refusal) = refusal(attempt, target) {
        return HarmResult::Declined(refusal);
    }
    let amount = located_amount(world, attempt);
    if amount > 0 {
        return crate::client_script::damage(world, tick, attempt, amount);
    }
    HarmResult::Declined(HarmDenial::NoDamage)
}

/// Why `attempt` cannot land on `target` (his lifecycle and life), if it cannot. The checks run in this order.
fn refusal(attempt: &HarmRequest, target: Option<(PlayerSessionStage, LifeCount)>) -> Option<HarmDenial> {
    if attempt.much <= 0 {
        return Some(HarmDenial::NoDamage);
    }
    let Some((lifecycle, life)) = target else {
        return Some(HarmDenial::NoSuchTarget);
    };
    if lifecycle != PlayerSessionStage::Alive {
        return Some(HarmDenial::TargetDead);
    }
    (life != attempt.victim_life_index).then_some(HarmDenial::StaleLife)
}

/// The attempt's damage after the hit location's multiplier from the weapon's table, as a whole number of at least 1.
fn located_amount(world: &StepWorldView, attempt: &HarmRequest) -> i32 {
    if !takes_location_scale(attempt.source, attempt.hit_part, attempt.blast) {
        return attempt.much;
    }
    let scale = world.damage_profile_of(attempt.gun).map_or(1.0, |facts| facts.location_scale(attempt.hit_part));
    weapon_t6::hitloc::scaled_damage(attempt.much, scale)
}

/// Whether a hit is multiplied by its hit location. Melee never is, and neither is a missile's blast (no location). A
/// hit the riot shield stopped is not either: the scripts count the bullet's whole damage as what the shield blocked.
fn takes_location_scale(source: HarmOrigin, hitloc: u8, blast: bool) -> bool {
    let never_scaled = match source {
        HarmOrigin::Melee => true,
        HarmOrigin::Projectile(_) => blast,
        _ => false,
    };
    !never_scaled && hitloc != HITLOC_SHIELD
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThrownId;
    use crate::match_ids::{BuckshotId, BulletNo};
    use crate::sim_parts::SeatNo;
    use weapon_t6::hitloc::{STRUCK_SKULL, scaled_damage};

    fn attempt(amount: i32) -> HarmRequest {
        let melee = HarmOrigin::Melee;
        HarmRequest {
            source: melee,
            pellet: BuckshotId(0),
            attacker: SeatNo(1),
            killer_life: LifeCount(1),
            target: SeatNo(2),
            victim_life_index: LifeCount(4),
            gun: 0,
            much: amount,
            cam_ent_since: 0,
            attacker_source_pos: Default::default(),
            hit_part: 0,
            blast: false,
            passed_through_wall: false,
            impact: false,
        }
    }

    const ALIVE_IN_LIFE_4: Option<(PlayerSessionStage, LifeCount)> = Some((PlayerSessionStage::Alive, LifeCount(4)));

    #[test]
    fn a_live_target_in_the_same_life_takes_the_hit() {
        assert_eq!(refusal(&attempt(30), ALIVE_IN_LIFE_4), None);
    }

    #[test]
    fn no_damage_is_turned_away_before_the_target_is_even_looked_up() {
        assert_eq!(refusal(&attempt(0), None), Some(HarmDenial::NoDamage));
        assert_eq!(refusal(&attempt(-3), ALIVE_IN_LIFE_4), Some(HarmDenial::NoDamage));
    }

    #[test]
    fn a_missing_or_dead_or_respawned_target_is_turned_away_in_that_order() {
        assert_eq!(refusal(&attempt(30), None), Some(HarmDenial::NoSuchTarget));
        let dead = Some((PlayerSessionStage::Dead, LifeCount(4)));
        assert_eq!(refusal(&attempt(30), dead), Some(HarmDenial::TargetDead));
        let dead_and_stale = Some((PlayerSessionStage::Dead, LifeCount(9)));
        assert_eq!(refusal(&attempt(30), dead_and_stale), Some(HarmDenial::TargetDead));
        let respawned = Some((PlayerSessionStage::Alive, LifeCount(5)));
        assert_eq!(refusal(&attempt(30), respawned), Some(HarmDenial::StaleLife));
    }

    #[test]
    fn a_missiles_own_hit_takes_the_multiplier_but_its_blast_does_not() {
        let missile = HarmOrigin::Projectile(ThrownId(1));
        assert!(takes_location_scale(missile, STRUCK_SKULL, false));
        assert!(!takes_location_scale(missile, 0, true));
    }

    #[test]
    fn a_bullet_takes_the_multiplier_wherever_it_lands_but_the_shield() {
        let shot = HarmOrigin::Shot(BulletNo(1));
        assert!(takes_location_scale(shot, STRUCK_SKULL, false));
        assert!(takes_location_scale(shot, 0, false));
        assert!(!takes_location_scale(shot, HITLOC_SHIELD, false));
        assert!(!takes_location_scale(HarmOrigin::Projectile(ThrownId(1)), HITLOC_SHIELD, false));
    }

    #[test]
    fn melee_is_never_multiplied() {
        assert!(!takes_location_scale(HarmOrigin::Melee, STRUCK_SKULL, false));
        assert!(!takes_location_scale(HarmOrigin::Melee, 0, false));
    }

    #[test]
    fn a_multiplied_hit_is_cut_to_a_whole_number_of_at_least_one() {
        assert_eq!(scaled_damage(150, 1.5), 225);
        assert_eq!(scaled_damage(150, 0.0), 1);
        assert_eq!(scaled_damage(100, 1.0), 100);
    }
}
