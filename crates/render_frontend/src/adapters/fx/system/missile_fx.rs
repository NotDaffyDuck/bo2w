//! The effects a flying projectile carries: its smoke trail, its beacon
//! light and its ignition flash, all bolted to the missile's `tag_fx`, and
//! the sound it makes when the motor lights.
//!
//! Each row of the missile occupancy is looked at once per frame. What has
//! already played is remembered per entity in [`RocketAttachState`], so a
//! looping trail starts once and a missile that is replaced by another
//! (same entity number, new projectile) starts over.

use asset_game::{FxName, WeaponAudioRole};
use assets::{ReadiedProjectileMeshes, StagedWeapons};
use bevy::prelude::*;
use render_fx::battle_fx::rocket_arrow_victim;

use super::stage::{FxStage, Playback};
use super::weapon_audio::WeaponVoices;
use crate::adapters::motion::puppet_pose::HostSkeletonFrame;
use crate::adapters::motion::missile::{
    RocketAttachRow, RocketAttachState, RocketSceneSlot, LiveRocketEntry,
};

fn fresh_row(proj_ent: Option<u32>, weapon: u32) -> RocketAttachRow {
    let trail_played = false;
    RocketAttachRow {
        projectile: proj_ent,
        weapon,
        trail_done: trail_played,
        marker_effect_done: false,
        ignition_effect_done: false,
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
        weapons: &'a asset_game::WeaponLedger,
        gun_no: u32,
    ) -> Option<FxName<'a>> {
        match self {
            Look::Trail => weapons.shot_wake_for(gun_no),
            Look::Beacon => weapons.shot_light_for(gun_no),
            Look::Ignition => weapons.shot_ignite_for(gun_no),
        }
    }

    fn already_played(self, row: &RocketAttachRow) -> bool {
        match self {
            Look::Trail => row.trail_done,
            Look::Beacon => row.marker_effect_done,
            Look::Ignition => row.ignition_fx_played,
        }
    }

    fn mark_played(self, row: &mut RocketAttachRow) {
        match self {
            Look::Trail => row.trail_done = true,
            Look::Beacon => row.marker_effect_done = true,
            Look::Ignition => row.ignition_fx_played = true,
        }
    }

    /// The beacon shows from the start; the trail and the ignition flash
    /// wait for the motor.
    fn needs_ignition(self) -> bool {
        self != Look::Beacon
    }

    /// A trail or beacon with no pose to ride is a gap worth counting; the
    /// one-shot ignition flash simply waits for the next frame.
    fn counts_pose_gap(self) -> bool {
        self != Look::Ignition
    }

    fn due(self, ignited: bool, row: &RocketAttachRow) -> bool {
        (ignited || !self.needs_ignition()) && !self.already_played(row)
    }
}

/// Counts one more of something that went missing.
fn tally(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

/// What a frame of missiles is looked at against.
struct Flight<'a> {
    registry: &'a asset_game::WeaponLedger,
    meshes: &'a asset_model::MissileMeshRoster,
    poses: Option<&'a HostSkeletonFrame>,
}

/// Starts each look of one missile that is due and has a pose to ride.
fn start_looks(
    missile: &LiveRocketEntry,
    ent_no: u32,
    flight: &Flight<'_>,
    bolts: &mut RocketAttachState,
    play: &mut Playback<'_>,
) {
    for look in Look::ALL {
        let due = bolts
            .rows
            .get(&ent_no)
            .is_some_and(|row| look.due(missile.ignited, row));
        let name = due.then(|| look.authored(flight.registry, missile.weapon)).flatten();
        let Some(name) = name else {
            continue;
        };
        let target = rocket_arrow_victim(
            flight.poses,
            flight.meshes,
            missile.namespace,
            &missile.name,
            ent_no,
        );
        let Some(goal) = target else {
            if look.counts_pose_gap() {
                tally(&mut bolts.posing_shortfalls);
            }
            continue;
        };
        let mut began = 0;
        if !play.bolted(Some(name), Some(goal), &mut began) {
            tally(&mut bolts.playback_shortfalls);
        } else if let Some(row) = bolts.rows.get_mut(&ent_no) {
            look.mark_played(row);
        }
    }
}

/// Says the motor's ignition sound once; without a queued sound the next frame tries again.
fn ignition_sound(
    missile: &LiveRocketEntry,
    ent_no: u32,
    bolts: &mut RocketAttachState,
    voices: &mut WeaponVoices,
) {
    let slot = WeaponAudioRole::ProjIgnition;
    if !voices.authors(missile.weapon, slot) {
        return;
    }
    let Some(alias) = voices.alias(missile.weapon, slot) else {
        return tally(&mut bolts.ignition_shortfalls);
    };
    let said = voices.say(missile.weapon, alias, Some(missile.origin), Some(ent_no));
    if let (true, Some(row)) = (said, bolts.rows.get_mut(&ent_no)) {
        row.ignition_effect_done = true;
    }
}

pub(super) fn tick_missile_present_state(
    occupancy: Option<Res<RocketSceneSlot>>,
    weapons: Option<Res<StagedWeapons>>,
    shell_visual_models: Option<Res<ReadiedProjectileMeshes>>,
    mut bolt_list: ResMut<RocketAttachState>,
    poses: Option<Res<HostSkeletonFrame>>,
    mut stage: FxStage,
    mut voices: WeaponVoices,
) {
    let registry = weapons.as_deref().map(|prepared| &*prepared.0);
    let meshes = shell_visual_models.as_deref().map(|prepared| &prepared.0);
    let (Some(occupancy), Some(registry), Some(meshes)) = (
        occupancy.as_deref(),
        registry.filter(|registry| !registry.is_empty()),
        meshes.filter(|meshes| !meshes.is_empty()),
    ) else {
        return;
    };
    let Some(mut play) = stage.playback() else {
        return;
    };
    let flight = Flight {
        registry,
        meshes,
        poses: poses.as_deref(),
    };

    let mut alive = Vec::with_capacity(occupancy.rows.len());
    for missile in &occupancy.rows {
        let Some(entnum) = missile.entnum else {
            tally(&mut bolt_list.guessed_rows_dropped);
            continue;
        };
        alive.push(entnum);

        let remembered = bolt_list
            .rows
            .entry(entnum)
            .or_insert_with(|| fresh_row(missile.entry_id, missile.weapon));
        if (remembered.weapon, remembered.projectile) != (missile.weapon, missile.entry_id) {
            *remembered = fresh_row(missile.entry_id, missile.weapon);
        }
        let sound_due = missile.ignited && !remembered.ignition_effect_done;

        start_looks(missile, entnum, &flight, &mut bolt_list, &mut play);
        if sound_due {
            ignition_sound(missile, entnum, &mut bolt_list, &mut voices);
        }
    }
    bolt_list.rows.retain(|entnum, _| alive.contains(entnum));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> RocketAttachRow {
        fresh_row(Some(7), 3)
    }

    #[test]
    fn a_fresh_row_has_played_nothing() {
        let row = row();
        for look in Look::ALL {
            assert!(!look.already_played(&row), "{look:?}");
        }
        assert!(!row.ignition_effect_done);
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
        assert!(!row.ignition_effect_done);
    }

    #[test]
    fn only_trail_and_beacon_count_a_missing_pose() {
        assert!(Look::Trail.counts_pose_gap());
        assert!(Look::Beacon.counts_pose_gap());
        assert!(!Look::Ignition.counts_pose_gap());
    }
}
