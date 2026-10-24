use super::*;

impl KillcamCamera {
    /// Guided missile: flies along with a falling one, otherwise swings out to
    /// one side while climbing and settles above and behind once it dives.
    pub(super) fn javelin_view(
        &self,
        projectile: Option<&ProjectileState>,
        anchor: Vec3,
        now: i32,
        sim_time: i32,
        (forward, right, up): (Vec3, Vec3, Vec3),
    ) -> View {
        let falling = projectile.is_some_and(|p| p.pos.tr_type == tr_type::GRAVITY);
        let diving = !falling && forward.z <= 0.0;
        let age_ms =
            projectile.map_or(now - self.entered_at, |p| (sim_time - p.launch_time).max(0));
        let age = age_ms as f32 / 1000.0;

        let mut origin = if falling {
            anchor
        } else if forward.z > 0.0 {
            let swing = (age * 2.0 * std::f32::consts::PI / 5.0)
                .min(1.5 * std::f32::consts::PI)
                .sin();
            anchor - forward * (swing * 200.0) + up * (50.0 * (1.0 - swing.abs()))
        } else {
            anchor + Vec3::Z * (175.0 + 25.0 * forward.z)
        };

        let angles = if falling {
            self.subject_angles
        } else {
            let along = (anchor - origin).normalize_or_zero();
            let mut rotation =
                Quat::from_mat3(&Mat3::from_cols(along, -right, along.cross(-right)));
            if diving {
                let toward_victim = (self.victim - origin).normalize_or_zero();
                let side = flat_left(toward_victim);
                let victim_facing = Quat::from_mat3(&Mat3::from_cols(
                    toward_victim,
                    side,
                    toward_victim.cross(side),
                ));
                let closeness = (1.0 - (anchor.z - self.victim.z) / 3000.0).clamp(0.0, 1.0);
                rotation = rotation.slerp(victim_facing, closeness);
            }
            game_math::axis_to_angles(Mat3::from_quat(rotation).to_cols_array_2d())
        };
        if diving {
            origin.z = origin.z.max(self.victim.z + 128.0);
        }
        (origin, angles, JAVELIN_FOV)
    }

    /// A close-in rocket or missile starts gliding to a stop once it has been
    /// seen moving.
    pub(super) fn arm_glide(&mut self, mode: KillCamMode, now: i32, clip: Option<&ClipCollision>) {
        let glides = matches!(
            mode,
            KillCamMode::Mode4MissileAlt | KillCamMode::Mode5Rocket | KillCamMode::Mode8Remote
        );
        let near_victim = (self.subject - self.victim).length() < 350.0
            || (mode == KillCamMode::Mode8Remote && self.subject.z - self.victim.z < 350.0);
        if self.glide.is_some() || !glides || !near_victim {
            return;
        }
        let Some((seen_at, seen_from)) = self.previous_subject.filter(|(time, _)| *time < now)
        else {
            return;
        };
        let velocity = (self.subject - seen_from) / (now - seen_at) as f32;
        let speed = velocity.length();
        let clear_fraction = clip.map_or(1.0, |c| {
            let ahead = self.subject + velocity.normalize_or_zero() * 100.0;
            sweep(c, self.subject, ahead, 0.0).fraction
        });
        self.glide = Some(Glide {
            started_at: now,
            duration_ms: if speed > 0.0 {
                200.0 * clear_fraction / speed
            } else {
                0.0
            },
            from: self.subject,
            velocity,
        });
    }

    /// Blends the new pose from the last one while the entry blend runs,
    /// leading the target by how fast the followed thing moves.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ease_from(
        &self,
        pose: &mut WorldCameraPose,
        from: WorldCameraPose,
        desired: Vec3,
        previous_time: i32,
        now: i32,
        tracking: bool,
        clip: Option<&ClipCollision>,
    ) {
        let step = (now - previous_time).max(0) as f32;
        let span = (self.blend_until - previous_time).max(1) as f32;
        let t = (step / span).clamp(0.0, 1.0);
        let velocity = self
            .previous_subject
            .filter(|(time, _)| *time < now && tracking)
            .map_or(Vec3::ZERO, |(time, seen)| {
                (self.subject - seen) / (now - time) as f32
            });
        let mut lead = desired + velocity * (self.blend_until - now) as f32;
        if let Some(clip) = clip {
            let hit = sweep(clip, desired, lead, CAMERA_HALF_BOX);
            lead = if hit.startsolid {
                desired
            } else {
                desired.lerp(lead, hit.fraction)
            };
        }
        pose.origin = Vec3::from_array(from.origin).lerp(lead, t).to_array();
        for (angle, start) in pose.angles.iter_mut().zip(from.angles) {
            *angle = start + game_math::angle_subtract(*angle, start) * t;
        }
    }
}
