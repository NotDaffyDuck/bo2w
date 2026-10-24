use super::*;

impl KillcamCamera {
    /// Camera for this frame while the kill cam is running: pose, field of
    /// view and the distance to focus on (when the mode uses one).
    pub fn update(
        &mut self,
        presented: &PresentedSnapshot,
        viewer: ClientId,
        now: i32,
        in_killcam: bool,
        weapons: Option<&PreparedWeapons>,
        clip: Option<&ClipCollision>,
    ) -> Option<(WorldCameraPose, f32, Option<f32>)> {
        let ps = presented.player(viewer)?;
        if !in_killcam || now < self.last_time {
            *self = Self::default();
        }
        let previous_time = std::mem::replace(&mut self.last_time, now);
        if !in_killcam {
            return None;
        }
        let snapshot = presented.snapshot()?;
        let sim_time = presented.trajectory_time_ms(now);
        let number = ps.kill_cam_entity;
        if number == ENTITYNUM_NONE {
            self.stop_following(WorldCameraPose {
                origin: [
                    ps.origin[0],
                    ps.origin[1],
                    ps.origin[2] + ps.view_height_current,
                ],
                angles: ps.viewangles,
            });
            return None;
        }
        let entity = snapshot.meta.entities.iter().find(|e| e.number == number);
        let projectile = snapshot.projectiles.iter().find(|p| p.entnum == number);

        if self.followed != Some(number) {
            let mode = match projectile {
                Some(p) => weapons.and_then(|w| w.0.facts_of(p.weapon)).map_or(
                    KillCamMode::Mode3Missile,
                    |facts| {
                        mode_for_weapon(facts.missile_guidance, facts.weap_class, facts.weap_type)
                    },
                ),
                None => mode_for_entity_type(i32::from(entity?.e_type))?,
            };
            self.start_following(number, mode, now);
        }

        if let Some(p) = projectile {
            self.subject = Vec3::from_array(presented.projectile_origin_at(p, now));
            self.subject_angles = evaluate_trajectory(&p.apos, sim_time);
            let on_ground =
                entity.is_some_and(|e| i32::from(e.ground_entity_num) != ENTITYNUM_NONE);
            let sticks = weapons
                .and_then(|w| w.0.facts_of(p.weapon))
                .is_some_and(|facts| matches!(facts.stickiness, 3 | 4));
            self.resting_on_ground = on_ground && sticks;
        } else if let Some(e) = entity {
            self.subject = entity_origin(e, sim_time);
            self.subject_angles = evaluate_trajectory(&e.lerp.apos, sim_time);
        }

        let watched = match ps.kill_cam_look_at_entity {
            ENTITYNUM_NONE => viewer.0 as i32,
            other => other,
        };
        if let Some(e) = snapshot.meta.entities.iter().find(|e| e.number == watched) {
            self.victim = entity_origin(e, sim_time) + Vec3::Z * 60.0;
        } else if watched != viewer.0 as i32
            && let Some(other) = presented.player(ClientId(watched as u32))
        {
            self.victim = Vec3::from_array(other.origin) + Vec3::Z * other.view_height_current;
        }

        let mode = self.mode?;
        let (desired, angles, fov) = self.view_for(mode, projectile, now, sim_time, clip)?;
        self.arm_glide(mode, now, clip);

        let mut pose = WorldCameraPose {
            origin: desired.to_array(),
            angles,
        };
        if now < self.blend_until
            && let Some(from) = self.last_pose
        {
            let tracking = projectile.is_some() || entity.is_some();
            self.ease_from(&mut pose, from, desired, previous_time, now, tracking, clip);
        }
        self.previous_subject = Some((now, self.subject));
        self.last_pose = Some(pose);
        let focus = has_focus_distance(mode).then(|| (self.victim - desired).length());
        Some((pose, fov, focus))
    }
}
