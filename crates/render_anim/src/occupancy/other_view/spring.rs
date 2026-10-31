use super::*;

impl VehicleSpring {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Extra camera range this frame, for the vehicle at `at` turned by
    /// `turn`, `secs` seconds after the previous frame.
    pub fn step(&mut self, view: &VehicleView, at: Vec3, turn: Quat, secs: f32) -> f32 {
        if self.mdl != Some(view.xmodel) {
            *self = Self {
                mdl: Some(view.xmodel),
                ..Self::default()
            };
        }
        let prior = self.prev_origin.replace(at);
        let [distance, seconds] = view.spring;
        let inert = distance.abs() < 0.001 || seconds <= 0.0 || view.top_velocity <= 0.0 || secs <= 0.0;
        if inert {
            return self.offset;
        }
        if let Some(prior) = prior {
            let forward_speed = (at - prior).dot(turn * Vec3::X) / secs;
            self.speed += (forward_speed - self.speed) * (secs / 0.1).min(1.0);
        }
        let share = (self.speed / view.top_velocity).clamp(0.0, 1.0);
        let limit = distance.abs();
        let closing = ((limit / (seconds * 1000.0)) / secs * share).clamp(0.1, 1.0);
        let goal = (distance * share).clamp(-limit, limit);
        self.offset += (goal - self.offset) * closing;
        self.offset
    }
}
