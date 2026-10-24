//! The live record of a missile or grenade in flight, how one is launched, and the notes the simulation leaves for the
//! script host about them.

use std::collections::BTreeSet;

use bevy_ecs::prelude::Resource;
use entity_t6::{Trajectory, evaluate_trajectory, evaluate_trajectory_delta};

use super::rider::StuckRider;
use crate::frame::FrameWorld;
use crate::identities::LifeSequence;
use crate::{ClientId, ProjectileId};

/// How long a rocket may live without finding anything, in milliseconds.
pub const ROCKET_CLEANUP_MS: i32 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileState {
    pub id: ProjectileId,
    pub owner: ClientId,
    pub owner_life: LifeSequence,
    pub weapon: u32,
    pub entnum: i32,
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    /// Position and angle trajectories the client extrapolates from.
    pub pos: Trajectory,
    pub apos: Trajectory,
    pub launch_time: i32,
    pub spawn_time_ms: i32,
    pub detonate_at_ms: Option<i32>,
    pub cleanup_at_ms: i32,
    pub travel_distance: f32,
    pub live: bool,
    /// Glass pane it is stuck in.
    pub stuck_pane: Option<u32>,
    pub grounded: bool,
    pub guide: crate::MissileGuide,
    /// Player its latest impact hit landed on.
    pub impact_target: Option<ClientId>,
    /// Player it stuck to and where.
    pub rider: Option<StuckRider>,
    /// Coasting (the Hellstorm after its bomblets leave): whatever it meets, or when the fuse ends, it only
    /// vanishes, with no blast and no damage.
    pub coasting: bool,
}

impl ProjectileState {
    pub fn origin_at(&self, at_time_ms: i32) -> [f32; 3] {
        evaluate_trajectory(&self.pos, at_time_ms)
    }

    pub fn velocity_at(&self, at_time_ms: i32) -> [f32; 3] {
        evaluate_trajectory_delta(&self.pos, at_time_ms)
    }

    /// Live, and past its arming distance (if it has one).
    pub fn is_armed(&self, activate_dist: i32) -> bool {
        self.live && (activate_dist <= 0 || self.travel_distance >= activate_dist as f32)
    }
}

/// The time a missile came into being.
pub fn projectile_birth_ms(projectile: &ProjectileState) -> i32 {
    projectile.spawn_time_ms
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GrenadeLaunchKind {
    Thrown {
        remaining_fuse_ms: Option<i32>,
    },
    Launcher,
    /// What a dying player was holding, let go with its own velocity and whatever was left of its fuse (a spent fuse
    /// goes off on the next frame).
    Dropped {
        remaining_fuse_ms: i32,
        velocity: [f32; 3],
    },
}

/// Events of the weapon simulation that the script host picks up afterwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WeaponNote {
    Fired {
        owner: ClientId,
    },
    Detonated {
        id: ProjectileId,
        origin: [f32; 3],
    },
    Stuck {
        id: ProjectileId,
        /// Player it stuck to.
        player: Option<ClientId>,
        /// It stuck to the shield he carries.
        shield: bool,
    },
}

/// Projectiles a script ordered to go off. The order has to survive the tick: an item that lands the same tick (stuck
/// or at rest) must not lose it the way an item loses its own flight fuse.
#[derive(Resource, Default)]
pub(crate) struct ScriptDetonations(BTreeSet<u32>);

pub(crate) fn order_script_detonation(world: &mut bevy_ecs::world::World, id: ProjectileId) {
    let mut ordered = world.get_resource_or_insert_with(ScriptDetonations::default);
    ordered.0.insert(id.0);
}

pub(super) fn detonation_ordered(world: &FrameWorld, id: ProjectileId) -> bool {
    match world.ecs_read().get_resource::<ScriptDetonations>() {
        Some(ordered) => ordered.0.contains(&id.0),
        None => false,
    }
}

/// Sets or clears the coasting flag of missile `id`; false when no live missile has that id.
pub(crate) fn set_missile_coasting(world: &mut FrameWorld, id: u32, on: bool) -> bool {
    let entnum = crate::frame::collect_projectiles(world.ecs())
        .iter()
        .find(|p| p.id.0 == id)
        .map(|p| p.entnum);
    diag::info!(Sim, "bo2mp streaks: missile {id} coasting {on} (its bomblets are out)");
    let Some(missile) = entnum.and_then(|n| world.projectile_mut_by_number(n)) else {
        return false;
    };
    missile.coasting = on;
    true
}
