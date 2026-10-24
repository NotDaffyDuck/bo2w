use super::*;

impl KillcamCamera {
    pub(super) fn stop_following(&mut self, eye: WorldCameraPose) {
        self.followed = None;
        self.mode = None;
        self.glide = None;
        self.previous_subject = None;
        self.last_pose = Some(eye);
    }

    pub(super) fn start_following(&mut self, number: i32, mode: KillCamMode, now: i32) {
        self.resting_on_ground = false;
        self.followed = Some(number);
        self.mode = Some(mode);
        self.glide = None;
        self.previous_subject = None;
        self.entered_at = now;
        self.blend_until = killcam_lerp_deadline_ms(self.last_pose.is_none(), true, mode, now);
    }

    /// Where the camera wants to be, which way it looks and its field of view.
    pub(super) fn view_for(
        &self,
        mode: KillCamMode,
        projectile: Option<&ProjectileState>,
        now: i32,
        sim_time: i32,
        clip: Option<&ClipCollision>,
    ) -> Option<View> {
        let anchor = self.glide.map_or(self.subject, |glide| glide.position(now));
        let (forward, right, up) = game_math::angle_vectors(self.subject_angles);
        let (forward, right, up) = (
            Vec3::from_array(forward),
            Vec3::from_array(right),
            Vec3::from_array(up),
        );
        match mode {
            KillCamMode::Mode0 | KillCamMode::Mode1Heli => None,
            KillCamMode::Mode2Airstrike => {
                let origin = anchor.with_z(anchor.z.max(self.victim.z + 24.0));
                Some((origin, facing_angles(self.victim - origin), DEFAULT_FOV))
            }
            KillCamMode::Mode6Turret => Some((
                anchor - forward * 10.0 + up * 10.0,
                self.subject_angles,
                DEFAULT_FOV,
            )),
            KillCamMode::Mode7Javelin => {
                Some(self.javelin_view(projectile, anchor, now, sim_time, (forward, right, up)))
            }
            _ => {
                let mut toward = self.victim - anchor;
                toward.z = toward.z.min(0.0);
                let toward = toward.normalize_or_zero();
                let lift = toward.cross(flat_left(toward));
                let (height, back) = chase_offsets(mode, self.resting_on_ground);
                let origin = pull_back(clip, self.subject, anchor + lift * height - toward * back);
                let aim = look_at_both(self.victim, self.subject, origin);
                Some((origin, facing_angles(aim), DEFAULT_FOV))
            }
        }
    }
}
