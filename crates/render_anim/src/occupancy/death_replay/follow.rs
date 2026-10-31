use super::*;

impl KillcamCamera {
    /// Camera for this frame while the kill cam is running: pose, field of
    /// view and the distance to focus on (when the mode uses one).
    pub fn chase_frame(
        &mut self,
        presented: &ShownWorldFrame,
        viewer_in: ClientId,
        now: i32,
        replaying_final_death: bool,
        weapons: Option<&StagedWeapons>,
        solids: Option<&CollisionClipData>,
    ) -> Option<(WorldEyePlacement, f32, Option<f32>)> {
        let pstate = presented.player(viewer_in)?;
        if !replaying_final_death || now < self.previous_stamp {
            *self = Self::default();
        }
        let previous_time = std::mem::replace(&mut self.previous_stamp, now);
        if !replaying_final_death {
            return None;
        }
        let snapshot = presented.snapshot()?;
        let sim_time = presented.trajectory_time_ms(now);
        let number = pstate.death_replay_target;
        if number == NO_ENTITY_SLOT {
            self.stop_following(WorldEyePlacement {
                origin: [
                    pstate.origin[0],
                    pstate.origin[1],
                    pstate.origin[2] + pstate.eye_height_now,
                ],
                turn: pstate.look_angles,
            });
            return None;
        }
        let entity = snapshot.meta.entities.iter().find(|e| e.slot_index == number);
        let shell = snapshot.projectiles.iter().find(|p| p.ent_no == number);

        if self.followed != Some(number) {
            let mode = match shell {
                Some(p) => weapons.and_then(|w| w.0.summary_for(p.gun)).map_or(
                    DeathReplayStyle::GuidedMissileView,
                    |facts| {
                        mode_for_weapon(facts.missile_guidance, facts.gun_kind, facts.gun_family_code)
                    },
                ),
                None => mode_for_entity_type(i32::from(entity?.e_type))?,
            };
            self.start_following(number, mode, now);
        }

        if let Some(p) = shell {
            self.subject = Vec3::from_array(presented.projectile_origin_at(p, now));
            self.subject_angles = evaluate_trajectory(&p.turn_path, sim_time);
            let grounded_now =
                entity.is_some_and(|e| i32::from(e.ground_entity_num) != NO_ENTITY_SLOT);
            let sticks = weapons
                .and_then(|w| w.0.summary_for(p.gun))
                .is_some_and(|facts| matches!(facts.stickiness, 3 | 4));
            self.resting_on_ground = grounded_now && sticks;
        } else if let Some(e) = entity {
            self.subject = entity_origin(e, sim_time);
            self.subject_angles = evaluate_trajectory(&e.lerp.apos, sim_time);
        }

        let watched = match pstate.death_replay_focus {
            NO_ENTITY_SLOT => viewer_in.0 as i32,
            rest => rest,
        };
        if let Some(e) = snapshot.meta.entities.iter().find(|e| e.slot_index == watched) {
            self.victim = entity_origin(e, sim_time) + Vec3::Z * 60.0;
        } else if watched != viewer_in.0 as i32
            && let Some(other) = presented.player(ClientId(watched as u32))
        {
            self.victim = Vec3::from_array(other.origin) + Vec3::Z * other.eye_height_now;
        }

        let mode = self.mode?;
        let (desired, angles, fov) = self.view_for(mode, shell, now, sim_time, solids)?;
        self.arm_glide(mode, now, solids);

        let mut pose = WorldEyePlacement {
            origin: desired.to_array(),
            turn: angles,
        };
        if now < self.blend_until
            && let Some(from) = self.last_pose
        {
            let tracking = shell.is_some() || entity.is_some();
            self.ease_from(&mut pose, from, desired, previous_time, now, tracking, solids);
        }
        self.previous_subject = Some((now, self.subject));
        self.last_pose = Some(pose);
        let focus = has_focus_distance(mode).then(|| (self.victim - desired).length());
        Some((pose, fov, focus))
    }
}
