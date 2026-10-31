use super::*;

impl KillcamCamera {
    pub(super) fn stop_following(&mut self, eye: WorldEyePlacement) {
        self.followed = None;
        self.mode = None;
        self.glide = None;
        self.previous_subject = None;
        self.last_pose = Some(eye);
    }

    pub(super) fn start_following(&mut self, num: i32, mode: DeathReplayStyle, now: i32) {
        self.resting_on_ground = false;
        self.followed = Some(num);
        self.mode = Some(mode);
        self.glide = None;
        self.previous_subject = None;
        self.entered_at = now;
        self.blend_until = killcam_blend_until_ms(self.last_pose.is_none(), true, mode, now);
    }

    /// Where the camera wants to be, which way it looks and its field of view.
    pub(super) fn view_for(
        &self,
        mode: DeathReplayStyle,
        projectile: Option<&FlyingProjectile>,
        now: i32,
        sim_time: i32,
        solids: Option<&CollisionClipData>,
    ) -> Option<(Vec3, [f32; 3], f32)> {
        let anchor = self.glide.map_or(self.subject, |glide| glide.position(now));
        let (ahead, right, upward) = game_math::euler_basis(self.subject_angles);
        let (fwd, right, up) = (
            Vec3::from_array(ahead),
            Vec3::from_array(right),
            Vec3::from_array(upward),
        );
        match mode {
            DeathReplayStyle::Mode0 | DeathReplayStyle::HelicopterView => None,
            DeathReplayStyle::AirstrikeView => {
                let org = anchor.with_z(anchor.z.max(self.victim.z + 24.0));
                Some((org, facing_angles(self.victim - org), DEFAULT_FOV))
            }
            DeathReplayStyle::TurretView => Some((
                anchor - fwd * 10.0 + up * 10.0,
                self.subject_angles,
                DEFAULT_FOV,
            )),
            DeathReplayStyle::AntiTankView => {
                Some(self.javelin_view(projectile, anchor, now, sim_time, (fwd, right, up)))
            }
            _ => {
                let mut toward = self.victim - anchor;
                toward.z = toward.z.min(0.0);
                let toward = toward.normalize_or_zero();
                let lift = toward.cross(flat_left(toward));
                let (height, back) = chase_offsets(mode, self.resting_on_ground);
                let org = pull_back(solids, self.subject, anchor + lift * height - toward * back);
                let aim = look_at_both(self.victim, self.subject, org);
                Some((org, facing_angles(aim), DEFAULT_FOV))
            }
        }
    }
}
