//! Where a projectile is drawn between two snapshots.

use std::sync::Arc;

use sim::{FlyingProjectile, Snapshot};

use super::ShownWorldFrame;

impl ShownWorldFrame {
    pub fn trajectory_time_ms(&self, draw_clock_millis: i32) -> i32 {
        self.sample_clock_ms.unwrap_or(draw_clock_millis)
    }

    pub(crate) fn set_trajectory_sample(
        &mut self,
        clock_stamp_ms: Option<i32>,
        next: Option<Arc<Snapshot>>,
    ) {
        self.sample_clock_ms = clock_stamp_ms;
        self.sample_next = next;
    }

    /// Where `projectile` is at the presented time. Without a trajectory sample that is its own
    /// extrapolation to `draw_clock_millis`. With one, a projectile the next snapshot moves to a
    /// different path (a bounce or an impact) is blended toward its next position instead of
    /// being predicted_ahead through the change, which would carry the camera past the victim.
    pub fn projectile_origin_at(
        &self,
        projectile: &FlyingProjectile,
        draw_clock_millis: i32,
    ) -> [f32; 3] {
        let Some(clock_stamp_ms) = self.sample_clock_ms else {
            return projectile.spot_at(draw_clock_millis);
        };
        let Some(current) = self.snapshot() else {
            return projectile.spot_at(clock_stamp_ms);
        };
        let current_time = sim::level_time_ms(current.tick);
        let Some(next) = self.sample_next.as_deref() else {
            return projectile.spot_at(clock_stamp_ms.min(current_time));
        };
        let next_time = sim::level_time_ms(next.tick);
        let same_projectile = |other: &&FlyingProjectile| {
            other.id == projectile.id
                && other.ent_no == projectile.ent_no
                && other.owner_respawn_count == projectile.owner_respawn_count
        };
        if let Some(moved) = next.projectiles.iter().find(same_projectile)
            && next_time > current_time
            && moved.at != projectile.at
        {
            let fraction = ((clock_stamp_ms - current_time) as f32 / (next_time - current_time) as f32)
                .clamp(0.0, 1.0);
            return std::array::from_fn(|axis| {
                projectile.origin[axis] + (moved.origin[axis] - projectile.origin[axis]) * fraction
            });
        }
        projectile.spot_at(clock_stamp_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_sample_the_render_time_is_used() {
        let presented = ShownWorldFrame::default();
        assert_eq!(presented.trajectory_time_ms(1234), 1234);
    }

    #[test]
    fn a_sample_overrides_the_render_time() {
        let mut presented = ShownWorldFrame::default();
        presented.set_trajectory_sample(Some(77), None);
        assert_eq!(presented.trajectory_time_ms(1234), 77);
        presented.set_trajectory_sample(None, None);
        assert_eq!(presented.trajectory_time_ms(1234), 1234);
    }
}
