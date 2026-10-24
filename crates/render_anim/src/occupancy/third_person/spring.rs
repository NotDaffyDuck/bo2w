use super::*;

impl VehicleSpring {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Extra camera range this frame, for the vehicle at `origin` turned by
    /// `rotation`, `dt` seconds after the previous frame.
    pub fn step(&mut self, view: &VehicleView, origin: Vec3, rotation: Quat, dt: f32) -> f32 {
        if self.model != Some(view.model) {
            *self = Self {
                model: Some(view.model),
                ..Self::default()
            };
        }
        let previous = self.last_origin.replace(origin);
        let [distance, seconds] = view.spring;
        let inert = distance.abs() < 0.001 || seconds <= 0.0 || view.max_speed <= 0.0 || dt <= 0.0;
        if inert {
            return self.offset;
        }
        if let Some(previous) = previous {
            let forward_speed = (origin - previous).dot(rotation * Vec3::X) / dt;
            self.speed += (forward_speed - self.speed) * (dt / 0.1).min(1.0);
        }
        let share = (self.speed / view.max_speed).clamp(0.0, 1.0);
        let limit = distance.abs();
        let closing = ((limit / (seconds * 1000.0)) / dt * share).clamp(0.1, 1.0);
        let goal = (distance * share).clamp(-limit, limit);
        self.offset += (goal - self.offset) * closing;
        self.offset
    }
}
