//! What the held-loop systems have in common (a gun's, a slide's, a prop's):
//! reaching the bank and the clip store, deciding how a loop's level carries to
//! the listener, and the entity a running loop lives on.

use std::sync::Arc;

use asset_audio::{SnapshotSoundAlias, AudioRoster};
use asset_core::AssetCategoryScope;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::map_sound_set::{LevelBackdropSound, LevelSoundSource};
use crate::clip_bank::{ClipRepository, waveforms_for_cue};
use crate::sample_feed::ClipPcm;
use crate::play_core::{CommonPlaybackStore, AudioCatalogRes};

/// A sound that never fades with distance.
const HELD_LEVEL: [[f32; 2]; 2] = [[0.0, 1.0], [1.0, 1.0]];
/// What an alias without a curve of its own falls off by.
const STRAIGHT_FADE: [[f32; 2]; 2] = [[0.0, 1.0], [1.0, 0.0]];

pub(crate) fn logging() -> bool {
    static SWITCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var_os("BO2MP_SOUND_LOG").is_some())
}

/// How a loop's level reaches the listener.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Carry {
    /// Sits on the ear: the same level wherever the listener goes.
    Ear,
    /// Sits in the world and fades along the alias's own curve.
    World,
}

/// Where a loop's clip stands.
pub(crate) enum Clip {
    /// The bank knows no clip for the alias.
    Unknown,
    /// Asked for, but not decoded yet (or it will not decode).
    Decoding,
    Done(ClipPcm),
}

/// What a new loop is spawned with, beyond what its alias row says.
pub(crate) struct Plan<'a> {
    pub(crate) row: &'a SnapshotSoundAlias,
    pub(crate) at: [f32; 3],
    pub(crate) gain: f32,
    pub(crate) speed: f32,
    pub(crate) knots: Arc<[[f32; 2]]>,
    pub(crate) far: f32,
}

/// The shared assets a loop system reaches for.
#[derive(SystemParam)]
pub(crate) struct LoopKit<'w> {
    bank: Option<Res<'w, AudioCatalogRes>>,
    store: Option<ResMut<'w, ClipRepository>>,
    assets: ResMut<'w, Assets<ClipPcm>>,
    curves: ResMut<'w, CommonPlaybackStore>,
}

impl LoopKit<'_> {
    /// The bank, but only when the clip store is installed beside it.
    pub(crate) fn catalog(&self) -> Option<Arc<AudioRoster>> {
        self.store.as_ref()?;
        let installed = self.bank.as_ref()?;
        Some(Arc::clone(&installed.0))
    }

    /// Asks for the alias's first clip and says how far along it is.
    pub(crate) fn reel(&mut self, ns: AssetCategoryScope, cue_id: &str) -> Clip {
        let (Some(snd_bank), Some(store)) = (self.bank.as_ref(), self.store.as_mut()) else {
            return Clip::Decoding;
        };
        let first = waveforms_for_cue(&snd_bank.0, ns, cue_id).into_iter().next();
        let Some(key) = first else {
            return Clip::Unknown;
        };
        store.req_in(key.clone());
        match store.ready_pcm(&key) {
            Some(Ok(audio)) => Clip::Done(audio),
            _ => Clip::Decoding,
        }
    }

    /// The alias row's own falloff curve, shared between loops that use it.
    pub(crate) fn fade(&mut self, row: &SnapshotSoundAlias) -> Option<Arc<[[f32; 2]]>> {
        let curve = row.volume_falloff.as_ref()?;
        Some(self.curves.curve_knots(&curve.name, &curve.knots))
    }

    pub(crate) fn knots_for(&mut self, row: &SnapshotSoundAlias, carry: Carry) -> Arc<[[f32; 2]]> {
        match carry {
            Carry::Ear => Arc::from(HELD_LEVEL.as_slice()),
            Carry::World => self
                .fade(row)
                .unwrap_or_else(|| Arc::from(STRAIGHT_FADE.as_slice())),
        }
    }

    /// Puts a loop in the world: `tag` marks it for its own system.
    pub(crate) fn spawn(
        &mut self,
        commands: &mut Commands,
        tag: impl Bundle,
        reel: ClipPcm,
        plan: Plan,
    ) {
        let row = plan.row;
        let emitter = LevelSoundSource {
            category: row.menu_category,
            fixed_pan: row.stereo_pan,
            active_stereo_handle: Option::default(),
            speed: plan.speed,
            pcm: self.assets.add(reel),
            rest_gain: plan.gain,
            knots: plan.knots,
            reach_upper: plan.far,
            reach_lower: plan.row.reach_lower,
            position_in_inches: plan.at,
            line_end_inches: None,
        };
        let placed = Transform::from_translation(Vec3::from_array(plan.at));
        let mut spawned = commands.spawn((tag, LevelBackdropSound, emitter, placed));
        crate::duck::tag(&mut spawned, Some(row), 0);
    }
}

/// The first variant of an alias.
pub(crate) fn lead_row<'a>(
    catalog: &'a AudioRoster,
    ns: AssetCategoryScope,
    cue_id: &str,
) -> Option<&'a SnapshotSoundAlias> {
    catalog.entry_in(ns, cue_id)?.aliases.first()
}

/// The far edge of a loop's reach: never closer than a unit past the near edge.
pub(crate) fn widened(row: &SnapshotSoundAlias) -> f32 {
    f32::max(row.reach_upper, row.reach_lower + 1.0)
}

/// Moves a loop to `to`, leaving the change marks alone when it is already there.
pub(crate) fn follow(emitter: &mut Mut<LevelSoundSource>, xf: &mut Mut<Transform>, to: [f32; 3]) {
    if emitter.position_in_inches == to {
        return;
    }
    emitter.position_in_inches = to;
    xf.translation = Vec3::from_array(to);
}

/// Writes a loop's level and pitch scale, marking a change only for a new value.
pub(crate) fn retune(emitter: &mut Mut<LevelSoundSource>, gain: f32, speed: f32) {
    emitter
        .reborrow()
        .map_unchanged(|e| &mut e.rest_gain)
        .set_if_neq(gain);
    emitter
        .reborrow()
        .map_unchanged(|e| &mut e.speed)
        .set_if_neq(speed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_audio::{SnapshotAudioCurve, SnapshotSoundEntry};
    use bevy::ecs::system::RunSystemOnce;

    #[derive(Component)]
    struct Mark;

    fn row_with(dist: [f32; 2]) -> SnapshotSoundAlias {
        let mut row = SnapshotSoundAlias::default();
        (row.reach_lower, row.reach_upper) = (dist[0], dist[1]);
        row
    }

    fn kit_world() -> World {
        let mut world = World::new();
        world.insert_resource(Assets::<ClipPcm>::default());
        world.insert_resource(CommonPlaybackStore::default());
        world
    }

    fn reel() -> ClipPcm {
        ClipPcm::from_parts(Arc::from([0.0f32, 0.5].as_slice()), 1, 8_000).unwrap()
    }

    #[test]
    fn the_far_edge_is_at_least_a_unit_past_the_near_one() {
        assert_eq!(widened(&row_with([100.0, 50.0])), 101.0);
        assert_eq!(widened(&row_with([10.0, 400.0])), 400.0);
        assert_eq!(widened(&row_with([0.0, 0.0])), 1.0);
    }

    #[test]
    fn the_lead_row_is_the_first_variant() {
        let mut catalog = AudioRoster::default();
        catalog.add_sound(SnapshotSoundEntry {
            name: "twin".to_owned(),
            aliases: vec![row_with([1.0, 2.0]), row_with([3.0, 4.0])],
        });
        let lead = lead_row(&catalog, AssetCategoryScope::T6, "twin").unwrap();
        assert_eq!(lead.reach_lower, 1.0);
        assert!(lead_row(&catalog, AssetCategoryScope::T6, "other").is_none());
    }

    #[test]
    fn a_world_loop_uses_the_alias_curve_or_a_straight_fade_and_an_ear_loop_holds() {
        let mut world = kit_world();
        let curved = SnapshotSoundAlias {
            volume_falloff: Some(SnapshotAudioCurve {
                name: "bend".to_owned(),
                knots: vec![(0.0, 1.0), (0.5, 0.25)],
            }),
            ..SnapshotSoundAlias::default()
        };
        let got = world
            .run_system_once(move |mut kit: LoopKit| {
                [
                    kit.knots_for(&curved, Carry::World),
                    kit.knots_for(&SnapshotSoundAlias::default(), Carry::World),
                    kit.knots_for(&curved, Carry::Ear),
                ]
                .map(|knots| knots.to_vec())
            })
            .unwrap();
        assert_eq!(got[0], vec![[0.0, 1.0], [0.5, 0.25]]);
        assert_eq!(got[1], vec![[0.0, 1.0], [1.0, 0.0]]);
        assert_eq!(got[2], vec![[0.0, 1.0], [1.0, 1.0]]);
    }

    #[test]
    fn a_spawned_loop_carries_the_plan_and_the_alias_row() {
        let mut world = kit_world();
        let row = SnapshotSoundAlias {
            reach_lower: 30.0,
            reach_upper: 90.0,
            menu_category: Some(2),
            ..SnapshotSoundAlias::default()
        };
        world
            .run_system_once(move |mut commands: Commands, mut kit: LoopKit| {
                let plan = Plan {
                    row: &row,
                    at: [1.0, 2.0, 3.0],
                    gain: 0.5,
                    speed: 1.25,
                    knots: Arc::from(HELD_LEVEL.as_slice()),
                    far: widened(&row),
                };
                kit.spawn(&mut commands, Mark, reel(), plan);
            })
            .unwrap();
        let mut query = world.query_filtered::<(&LevelSoundSource, &Transform), With<Mark>>();
        let (emitter, transform) = query.single(&world).unwrap();
        assert_eq!(emitter.position_in_inches, [1.0, 2.0, 3.0]);
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!((emitter.reach_lower, emitter.reach_upper), (30.0, 90.0));
        assert_eq!((emitter.rest_gain, emitter.speed), (0.5, 1.25));
        assert_eq!(emitter.category, Some(2));
        assert!(emitter.active_stereo_handle.is_none() && emitter.fixed_pan.is_none());
        assert_eq!(emitter.knots.len(), 2);
        let mut with_ambient = world.query_filtered::<Entity, (With<Mark>, With<LevelBackdropSound>)>();
        assert_eq!(with_ambient.iter(&world).count(), 1);
        assert_eq!(world.resource::<Assets<ClipPcm>>().len(), 1);
    }

    fn still_emitter() -> LevelSoundSource {
        let unmoving = LevelSoundSource {
            position_in_inches: [1.0; 3],
            reach_lower: 1.0,
            reach_upper: 2.0,
            knots: Arc::from(HELD_LEVEL.as_slice()),
            rest_gain: 0.5,
            pcm: Handle::default(),
            speed: 1.0,
            active_stereo_handle: Option::default(),
            fixed_pan: None,
            category: None,
            line_end_inches: None,
        };
        unmoving
    }

    #[test]
    fn following_and_retuning_touch_nothing_that_did_not_change() {
        let mut world = kit_world();
        let entity = world
            .spawn((
                still_emitter(),
                Transform::from_xyz(1.0, 1.0, 1.0),
            ))
            .id();
        let still = move |mut q: Query<(&mut LevelSoundSource, &mut Transform)>| {
            let (mut emitter, mut transform) = q.get_mut(entity).unwrap();
            follow(&mut emitter, &mut transform, [1.0; 3]);
            retune(&mut emitter, 0.5, 1.0);
        };
        world.run_system_once(still).unwrap();
        let emitter = world.entity(entity).get_ref::<LevelSoundSource>().unwrap();
        assert_eq!(emitter.last_changed(), emitter.added());
        let transform = world.entity(entity).get_ref::<Transform>().unwrap();
        assert_eq!(transform.last_changed(), transform.added());
        let moved = move |mut q: Query<(&mut LevelSoundSource, &mut Transform)>| {
            let (mut emitter, mut transform) = q.get_mut(entity).unwrap();
            follow(&mut emitter, &mut transform, [4.0, 5.0, 6.0]);
            retune(&mut emitter, 0.25, 2.0);
        };
        world.run_system_once(moved).unwrap();
        let emitter = world.entity(entity).get_ref::<LevelSoundSource>().unwrap();
        assert_ne!(emitter.last_changed(), emitter.added());
        assert_eq!(emitter.position_in_inches, [4.0, 5.0, 6.0]);
        assert_eq!((emitter.rest_gain, emitter.speed), (0.25, 2.0));
        let transform = world.get::<Transform>(entity).unwrap();
        assert_eq!(transform.translation, Vec3::new(4.0, 5.0, 6.0));
    }
}
