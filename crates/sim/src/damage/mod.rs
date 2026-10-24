//! Damage to players and the things around them: one hit as an attempt the victim's scripts judge, an explosion as a
//! blast over a radius (players, entities, map breakables and glass), and the riot shield's splash and the flash that
//! go with a blast.
//!
//! `hit` turns an attempt into the scripts' damage call and `blast` spreads an explosion. `scripted` does the same for
//! the blasts and hits the scripts ask for. `shield` and `flash` are the two side effects, and `death` is what a death
//! does to the victim's view.

use gamemode_t6::{G_CAN_DAMAGE_CONTENTS_MASK, can_damage_player_vis_scale, radius_damage_amount, radius_damage_distance_to_aabb};

use crate::area_world::AreaBox;
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, PelletId};
use crate::world::ClientId;

mod blast;
mod death;
mod flash;
mod hit;
mod scripted;
mod shield;

pub(crate) use blast::{ExplosionBlast, apply_explosion_blast, apply_shared_glass_blast};
pub(crate) use death::play_death;
pub(crate) use flash::apply_flashbang_blast;
pub(crate) use hit::apply_damage_attempt;
pub(crate) use scripted::{apply_script_blast, apply_script_hit};
pub(crate) use shield::shield_splash_event;
use shield::ShieldSplash;

/// An area query may name every entity slot there is.
const AREA_ENTITY_CAPACITY: usize = entity_t6::MAX_GENTITIES as usize;

// Level time is told in whole ticks of 50 ms; the blast code turns a tick into milliseconds with that.
const _: () = assert!(crate::MATCH_TICK_MS == 50);

/// One hit on one player, as the shooter's side saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageAttempt {
    pub source: DamageSource,
    pub pellet: PelletId,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,
    pub target: ClientId,
    pub target_life: LifeSequence,
    pub weapon: u32,
    pub amount: i32,
    pub killcam_entity_start_time: i32,
    pub inflictor_origin: Option<[f32; 3]>,
    pub hitloc: u8,
    /// An explosion's blast (radius damage), not a direct hit by the missile or bolt itself.
    pub blast: bool,
    /// A bullet that went through something first (a wall, another player): the scripts' `idflags` carry BO2's
    /// penetration bit.
    pub penetrated: bool,
    /// A missile's own hit that does not go off on it (a grenade before it arms, a stuck Semtex, a bolt, a blade):
    /// BO2's MOD_IMPACT.
    pub impact: bool,
}

/// Why an attempt did nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageRefusal {
    FriendlyFire,
    NonPositive,
    MissingTarget,
    TargetNotAlive,
    /// The target has died and come back since the shot was fired.
    StaleLife,
}

/// What a killing blow looked like, handed to the scripts' death callback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathCommit {
    pub victim: ClientId,
    pub victim_life: LifeSequence,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,
    pub source: DamageSource,
    pub pellet: PelletId,
    pub weapon: u32,
    pub amount: i32,
    pub killcam_entity_start_time: i32,
    pub inflictor_origin: Option<[f32; 3]>,
    pub hitloc: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DamageOutcome {
    Refused(DamageRefusal),
    Nonlethal { health_after: i32 },
    Died(DeathCommit),
}

fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}

/// A blast's damage by distance: `inner` at its centre falling to `outer` at `radius`.
#[derive(Clone, Copy, Debug)]
struct Falloff {
    inner: f32,
    outer: f32,
    radius: f32,
}

impl Falloff {
    /// The whole points of damage at `distance` for a target `open` (0 to 1) to the blast.
    fn at(self, distance: f32, open: f32) -> i32 {
        radius_damage_amount(self.inner, self.outer, self.radius, distance, open)
    }
}

/// The client numbers of the entities a blast of `radius` around `origin` could reach; the caller keeps the ones that
/// are players. Empty, with a warning, if the blast has no valid area.
fn reachable_clients(world: &FrameWorld, origin: [f32; 3], radius: f32) -> Vec<ClientId> {
    let half = gamemode_t6::radius_damage_area_half_extent(radius);
    let Some(area) = AreaBox::from_mid_half(origin, [half; 3]) else {
        diag::warn!(Sim, "radius damage at {origin:?} r={radius} has no valid area");
        return Vec::new();
    };
    world
        .area_entity_candidates(area, u32::MAX, AREA_ENTITY_CAPACITY)
        .into_iter()
        .map(|number| ClientId(u32::from(number)))
        .collect()
}

/// How open `target` is to a blast at `blast_origin`: 1.0 in the clear, down to 0.0 behind solid cover. A target that
/// is not a player in the world counts as in the clear.
fn open_to_blast(world: &FrameWorld, blast_origin: [f32; 3], target: ClientId) -> f32 {
    let Some(ps) = world.player(target) else {
        return 1.0;
    };
    let eye = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current];
    can_damage_player_vis_scale(ps.origin, eye, blast_origin, None, |from, to| {
        let trace = world.trace_world(from, to, [0.0; 3], [0.0; 3], G_CAN_DAMAGE_CONTENTS_MASK);
        trace.fraction >= 1.0 && !trace.startsolid
    })
}

/// What a blast does to one player in its radius.
struct PlayerShare {
    /// Whole points, after cover and the riot shield took theirs.
    amount: i32,
    /// Set when a riot shield took part of the blast.
    splash: Option<ShieldSplash>,
}

/// The share of a blast at `origin` that `target` (standing in `bounds`) takes. A riot shield on his arm takes some of
/// it, and his view shakes by what would have reached him unshielded.
fn player_share(world: &FrameWorld, origin: [f32; 3], falloff: Falloff, target: ClientId, bounds: &AreaBox) -> PlayerShare {
    let distance = radius_damage_distance_to_aabb(origin, bounds.mid(), bounds.half());
    let mut open = open_to_blast(world, origin, target);
    let mut splash = None;
    if let Some(ps) = world.player(target) {
        let blocked = world.shield_blast_protection(target, ps, origin);
        if blocked > 0.0 {
            splash = Some(ShieldSplash { id: target, origin: ps.origin, unshielded: falloff.at(distance, open) });
        }
        open *= 1.0 - blocked;
    }
    PlayerShare { amount: falloff.at(distance, open), splash }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_helpers_agree_with_the_arithmetic_done_by_hand() {
        assert_eq!(minus([5.0, 2.0, -1.0], [1.0, 2.0, 3.0]), [4.0, 0.0, -4.0]);
        assert_eq!(dot([1.0, 2.0, 3.0], [4.0, -5.0, 6.0]), 12.0);
        assert_eq!(length([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(length([0.0; 3]), 0.0);
    }

    #[test]
    fn falloff_is_the_gamemode_formula_at_its_ends_and_nothing_past_the_radius() {
        let falloff = Falloff { inner: 100.0, outer: 20.0, radius: 200.0 };
        assert_eq!(falloff.at(0.0, 1.0), radius_damage_amount(100.0, 20.0, 200.0, 0.0, 1.0));
        assert_eq!(falloff.at(120.0, 0.5), radius_damage_amount(100.0, 20.0, 200.0, 120.0, 0.5));
        assert!(falloff.at(0.0, 1.0) >= falloff.at(100.0, 1.0));
        assert!(falloff.at(100.0, 1.0) >= falloff.at(190.0, 1.0));
        assert_eq!(falloff.at(0.0, 0.0), 0, "a target behind cover takes nothing");
    }
}
