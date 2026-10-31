//! Projectiles as the player sees them: the host's, plus the local player's own throws the
//! host has not confirmed yet.

mod merge;

use entity_t6::Trajectory;
use sim::{ClientId, ProjectileId, FlyingProjectile};

pub use merge::{throws_shown_for, combine_shown_missiles};

/// One projectile row to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShownProjectile {
    /// A projectile the host reported.
    HostDecided(FlyingProjectile),

    /// A throw the local player predicted that the host has not reported yet.
    Predicted {
        thrower: ClientId,
        kit: u32,
        start_point: [f32; 3],
        motion: [f32; 3],
        path: Trajectory,
        spin: Trajectory,
        spawn_moment: i32,
    },
}

/// The values both kinds of row carry, laid out the same way.
struct Fields {
    thrower: ClientId,
    kit: u32,
    start_point: [f32; 3],
    motion: [f32; 3],
    path: Trajectory,
    spin: Trajectory,
    spawn_moment: i32,
}

impl ShownProjectile {
    fn fields(&self) -> Fields {
        match *self {
            Self::HostDecided(host) => Fields {
                thrower: host.owner,
                kit: host.gun,
                start_point: host.origin,
                motion: host.motion,
                path: host.at,
                spin: host.turn_path,
                spawn_moment: host.spawn_moment,
            },
            Self::Predicted { thrower, kit, start_point, motion, path, spin, spawn_moment } => {
                Fields { thrower, kit, start_point, motion, path, spin, spawn_moment }
            }
        }
    }

    pub fn owner(&self) -> ClientId {
        self.fields().thrower
    }

    pub fn weapon(&self) -> u32 {
        self.fields().kit
    }

    pub fn start_point(&self) -> [f32; 3] {
        self.fields().start_point
    }

    /// Where the flight path puts it at `for_instant_millis`.
    pub fn point_at(&self, for_instant_millis: i32) -> [f32; 3] {
        entity_t6::evaluate_trajectory(&self.path(), for_instant_millis)
    }

    pub fn motion(&self) -> [f32; 3] {
        self.fields().motion
    }

    /// The spin path.
    pub fn spin(&self) -> Trajectory {
        self.fields().spin
    }

    /// The flight path.
    pub fn path(&self) -> Trajectory {
        self.fields().path
    }

    pub fn spawn_moment(&self) -> i32 {
        self.fields().spawn_moment
    }

    /// The host's id for it; a prediction has none yet.
    pub fn host_assigned_id(&self) -> Option<ProjectileId> {
        if let Self::HostDecided(host) = self {
            return Some(host.id);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn projectile(id: u32, owner: u32, weapon: u32) -> FlyingProjectile {
        FlyingProjectile {
            id: ProjectileId(id),
            owner: ClientId(owner),
            owner_respawn_count: sim::LifeSequence(1),
            gun: weapon,
            origin: [1.0, 2.0, 3.0],
            motion: [4.0, 5.0, 6.0],
            at: Trajectory {
                tr_type: entity_t6::path_kind::STRAIGHT,
                began_at: 100,
                base_point: [10.0, 0.0, 0.0],
                rate: [1000.0, 0.0, 0.0],
                ..Trajectory::default()
            },
            turn_path: Trajectory {
                began_at: 7,
                ..Trajectory::default()
            },
            ent_no: 90,
            spawn_moment: 123,
            spawn_time_ms: 100,
            detonate_at_ms: None,
            cleanup_at_ms: 9000,
            travel_distance: 0.0,
            in_flight: true,
            stuck_pane: None,
            grounded: false,
            guide: sim::MissileGuide::default(),
            impact_target: None,
            rider: None,
            coasting: false,
        }
    }

    fn predicted() -> ShownProjectile {
        ShownProjectile::Predicted {
            thrower: ClientId(3),
            kit: 8,
            start_point: [7.0, 8.0, 9.0],
            motion: [1.0, 1.0, 1.0],
            path: Trajectory {
                began_at: 5,
                ..Trajectory::default()
            },
            spin: Trajectory {
                began_at: 6,
                ..Trajectory::default()
            },
            spawn_moment: 55,
        }
    }

    #[test]
    fn an_authoritative_row_reads_through_to_the_hosts_projectile() {
        let state = projectile(11, 2, 40);
        let row = ShownProjectile::HostDecided(state);
        assert_eq!(row.owner(), ClientId(2));
        assert_eq!(row.weapon(), 40);
        assert_eq!(row.start_point(), [1.0, 2.0, 3.0]);
        assert_eq!(row.motion(), [4.0, 5.0, 6.0]);
        assert_eq!(row.path(), state.at);
        assert_eq!(row.spin(), state.turn_path);
        assert_eq!(row.spawn_moment(), 123);
        assert_eq!(row.host_assigned_id(), Some(ProjectileId(11)));
    }

    #[test]
    fn a_predicted_row_reads_its_own_fields_and_has_no_host_id() {
        let row = predicted();
        assert_eq!(row.owner(), ClientId(3));
        assert_eq!(row.weapon(), 8);
        assert_eq!(row.start_point(), [7.0, 8.0, 9.0]);
        assert_eq!(row.motion(), [1.0, 1.0, 1.0]);
        assert_eq!(row.path().began_at, 5);
        assert_eq!(row.spin().began_at, 6);
        assert_eq!(row.spawn_moment(), 55);
        assert_eq!(row.host_assigned_id(), None);
    }

    #[test]
    fn origin_at_follows_the_flight_path() {
        let row = ShownProjectile::HostDecided(projectile(1, 0, 1));
        // 1000 units a second from x = 10, starting at 100 ms.
        assert_eq!(row.point_at(100), [10.0, 0.0, 0.0]);
        assert_eq!(row.point_at(600), [510.0, 0.0, 0.0]);
    }
}
