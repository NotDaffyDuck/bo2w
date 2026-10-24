//! The effects a flying projectile carries: its smoke trail, its beacon
//! light and its ignition flash, all bolted to the missile's `tag_fx`, and
//! the sound it makes when the motor lights.
//!
//! Each row of the missile occupancy is looked at once per frame. What has
//! already played is remembered per entity in [`MissileBoltState`], so a
//! looping trail starts once and a missile that is replaced by another
//! (same entity number, new projectile) starts over.

use asset_game::{FxName, WeaponSoundSlot};
use assets::{PreparedProjectileMeshes, PreparedWeapons};
use bevy::prelude::*;
use render_fx::combat::missile_bolt_target;

use super::stage::FxStage;
use super::weapon_audio::WeaponVoices;
use crate::adapters::anim::dobj_pose::HostDObjPoseFrame;
use crate::adapters::anim::missile::{MissileBoltRow, MissileBoltState, MissileOccupancy};

fn fresh_row(projectile: Option<u32>, weapon: u32) -> MissileBoltRow {
    MissileBoltRow {
        projectile,
        weapon,
        trail_played: false,
        beacon_played: false,
        ignition_played: false,
        ignition_fx_played: false,
    }
}

/// The three bolted looks a projectile can carry.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    Trail,
    Beacon,
    Ignition,
}

impl Look {
    const ALL: [Look; 3] = [Look::Trail, Look::Beacon, Look::Ignition];

    /// The effect the weapon authored for this look.
    fn authored<'a>(
        self,
        weapons: &'a asset_game::WeaponRegistry,
        weapon: u32,
    ) -> Option<FxName<'a>> {
        match self {
            Look::Trail => weapons.proj_trail_of(weapon),
            Look::Beacon => weapons.proj_beacon_of(weapon),
            Look::Ignition => weapons.proj_ignition_of(weapon),
        }
    }

    fn already_played(self, row: &MissileBoltRow) -> bool {
        match self {
            Look::Trail => row.trail_played,
            Look::Beacon => row.beacon_played,
            Look::Ignition => row.ignition_fx_played,
        }
    }

    fn mark_played(self, row: &mut MissileBoltRow) {
        match self {
            Look::Trail => row.trail_played = true,
            Look::Beacon => row.beacon_played = true,
            Look::Ignition => row.ignition_fx_played = true,
        }
    }

    /// The beacon shows from the start; the trail and the ignition flash
    /// wait for the motor.
    fn needs_ignition(self) -> bool {
        !matches!(self, Look::Beacon)
    }

    /// A trail or beacon with no pose to ride is a gap worth counting; the
    /// one-shot ignition flash simply waits for the next frame.
    fn counts_pose_gap(self) -> bool {
        !matches!(self, Look::Ignition)
    }

    fn due(self, ignited: bool, row: &MissileBoltRow) -> bool {
        (ignited || !self.needs_ignition()) && !self.already_played(row)
    }
}

pub(super) fn tick_missile_present_state(
    occupancy: Option<Res<MissileOccupancy>>,
    weapons: Option<Res<PreparedWeapons>>,
    projectile_meshes: Option<Res<PreparedProjectileMeshes>>,
    mut bolts: ResMut<MissileBoltState>,
    poses: Option<Res<HostDObjPoseFrame>>,
    mut stage: FxStage,
    mut voices: WeaponVoices,
) {
    let Some(occupancy) = occupancy.as_deref() else {
        return;
    };
    let Some(registry) = weapons
        .as_deref()
        .map(|prepared| &*prepared.0)
        .filter(|registry| !registry.is_empty())
    else {
        return;
    };
    let Some(meshes) = projectile_meshes
        .as_deref()
        .map(|prepared| &prepared.0)
        .filter(|meshes| !meshes.is_empty())
    else {
        return;
    };
    let Some(mut play) = stage.playback() else {
        return;
    };

    let mut live = Vec::with_capacity(occupancy.rows.len());
    for missile in &occupancy.rows {
        let Some(entnum) = missile.entnum else {
            bolts.predicted_rows_skipped = bolts.predicted_rows_skipped.saturating_add(1);
            continue;
        };
        live.push(entnum);

        let remembered = bolts
            .rows
            .entry(entnum)
            .or_insert_with(|| fresh_row(missile.id, missile.weapon));
        if remembered.weapon != missile.weapon || remembered.projectile != missile.id {
            *remembered = fresh_row(missile.id, missile.weapon);
        }
        let sound_due = missile.ignited && !remembered.ignition_played;

        for look in Look::ALL {
            let due = bolts
                .rows
                .get(&entnum)
                .is_some_and(|row| look.due(missile.ignited, row));
            if !due {
                continue;
            }
            let Some(name) = look.authored(registry, missile.weapon) else {
                continue;
            };
            let Some(target) = missile_bolt_target(
                poses.as_deref(),
                meshes,
                missile.namespace,
                &missile.name,
                entnum,
            ) else {
                if look.counts_pose_gap() {
                    bolts.pose_gaps = bolts.pose_gaps.saturating_add(1);
                }
                continue;
            };
            let mut started = 0;
            if play.bolted(Some(name), Some(target), &mut started) {
                if let Some(row) = bolts.rows.get_mut(&entnum) {
                    look.mark_played(row);
                }
            } else {
                bolts.play_gaps = bolts.play_gaps.saturating_add(1);
            }
        }

        if sound_due && voices.authors(missile.weapon, WeaponSoundSlot::ProjIgnition) {
            match voices.alias(missile.weapon, WeaponSoundSlot::ProjIgnition) {
                Some(alias) => {
                    // Only a sound that reached the queue counts as played;
                    // without one the next frame tries again.
                    if voices.say(missile.weapon, alias, Some(missile.origin), Some(entnum))
                        && let Some(row) = bolts.rows.get_mut(&entnum)
                    {
                        row.ignition_played = true;
                    }
                }
                None => bolts.ignition_gaps = bolts.ignition_gaps.saturating_add(1),
            }
        }
    }
    bolts.rows.retain(|entnum, _| live.contains(entnum));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> MissileBoltRow {
        fresh_row(Some(7), 3)
    }

    #[test]
    fn a_fresh_row_has_played_nothing() {
        let row = row();
        for look in Look::ALL {
            assert!(!look.already_played(&row), "{look:?}");
        }
        assert!(!row.ignition_played);
        assert_eq!((row.projectile, row.weapon), (Some(7), 3));
    }

    #[test]
    fn the_beacon_shows_before_the_motor_lights() {
        let row = row();
        assert!(Look::Beacon.due(false, &row));
        assert!(!Look::Trail.due(false, &row));
        assert!(!Look::Ignition.due(false, &row));
    }

    #[test]
    fn every_look_is_due_once_the_motor_is_lit() {
        let row = row();
        for look in Look::ALL {
            assert!(look.due(true, &row), "{look:?}");
        }
    }

    #[test]
    fn a_look_is_marked_and_never_due_again() {
        for look in Look::ALL {
            let mut row = row();
            look.mark_played(&mut row);
            assert!(look.already_played(&row), "{look:?}");
            assert!(!look.due(true, &row), "{look:?}");
            // And marking one look leaves the others waiting.
            for other in Look::ALL.into_iter().filter(|other| *other != look) {
                assert!(!other.already_played(&row), "{look:?} marked {other:?}");
            }
        }
    }

    #[test]
    fn the_ignition_flash_is_marked_apart_from_the_ignition_sound() {
        let mut row = row();
        Look::Ignition.mark_played(&mut row);
        assert!(row.ignition_fx_played);
        assert!(!row.ignition_played);
    }

    #[test]
    fn only_trail_and_beacon_count_a_missing_pose() {
        assert!(Look::Trail.counts_pose_gap());
        assert!(Look::Beacon.counts_pose_gap());
        assert!(!Look::Ignition.counts_pose_gap());
    }
}
