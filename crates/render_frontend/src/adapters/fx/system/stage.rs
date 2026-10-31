//! What a handler needs to start an effect, bundled so it asks for one
//! parameter instead of five: the parsed effect catalog, the per-element
//! cache built from it, the host the effects live in, and the world they are
//! drawn against.

use asset_game::{FxDefinitions, FxName};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx::{EffectBoltTarget, EffectSystemHost, PlaybackResult};
use render_fx::battle_fx::{
    report_combat_fx_gaps, spawn_casing_ejection, try_play_gun_fx_at_origin, try_play_gun_fx_bolted,
};
use render_fx::{
    BattleEffectsSnapshot, EffectJournalCursor, EffectsSystemHandle, ReadyFxBook,
    PreparedFxElemTable,
};

use super::EffectStageAccess;
use render_fx::effect_play::{
    EffectElementInfoMemo, EffectStage, spawn_named_attached_world_effect, play_named_oriented_at_msec,
    play_named_oriented_in_level, spawn_named_oriented_in_world,
};
use crate::adapters::fx::world_decal::FrontendEffectStage;

#[derive(SystemParam)]
pub(super) struct FxStage<'w> {
    fx_catalog: Option<Res<'w, ReadyFxBook>>,
    infos: ResMut<'w, PreparedFxElemTable>,
    host: ResMut<'w, EffectsSystemHandle>,
    world: EffectStageAccess<'w>,
}

impl FxStage<'_> {
    /// The stage ready for playing, or `None` while no effect catalog is
    /// loaded (the caller books that as a gap in its own counter).
    pub(super) fn playback(&mut self) -> Option<Playback<'_>> {
        let Self {
            fx_catalog: catalog,
            infos,
            host,
            world,
        } = self;
        let catalog = catalog.as_deref()?;
        infos.0.restock(&catalog.0);
        Some(Playback {
            host: &mut host.0,
            defs: &catalog.0,
            infos: &mut infos.0,
            scene: world.view(),
        })
    }

    /// The host without a catalog, for the handlers that only stop or
    /// shape effects already running.
    pub(super) fn host(&mut self) -> &mut EffectSystemHost {
        &mut self.host.0
    }

    /// The pieces side by side, for the routines that take the catalog, the
    /// cache, the host and the scene as arguments of their own.
    pub(super) fn parts(&mut self) -> StageParts<'_> {
        let Self {
            fx_catalog: catalog,
            infos,
            host,
            world,
        } = self;
        let fx_catalog = catalog.as_deref();
        if let Some(catalog) = fx_catalog {
            infos.0.restock(&catalog.0);
        }
        StageParts {
            catalog: fx_catalog,
            infos: &mut infos.0,
            host: &mut **host,
            scene: world.view(),
        }
    }
}

pub(super) struct StageParts<'a> {
    pub(super) catalog: Option<&'a ReadyFxBook>,
    pub(super) infos: &'a mut EffectElementInfoMemo,
    pub(super) host: &'a mut EffectsSystemHandle,
    pub(super) scene: Option<FrontendEffectStage<'a>>,
}

/// A [`FxStage`] with its catalog present. The scene is held next to the
/// host as separate fields, so each method borrows them side by side.
pub(super) struct Playback<'a> {
    host: &'a mut EffectSystemHost,
    defs: &'a FxDefinitions,
    infos: &'a mut EffectElementInfoMemo,
    scene: Option<FrontendEffectStage<'a>>,
}

/// Where an effect plays and which way it faces.
#[derive(Clone, Copy, Debug)]
pub(super) struct Pose {
    pub(super) anchor: [f32; 3],
    pub(super) turn: [[f32; 3]; 3],
}

impl Pose {
    /// The axes a pose takes from where it points and which way is its top:
    /// the forward, the left the two make, and the up.
    pub(super) fn axes_from(forward: Vec3, up: Vec3) -> [[f32; 3]; 3] {
        [
            forward.to_array(),
            up.cross(forward).to_array(),
            up.to_array(),
        ]
    }
}

impl<'a> Playback<'a> {
    pub(super) fn defs(&self) -> &'a FxDefinitions {
        self.defs
    }

    pub(super) fn host(&mut self) -> &mut EffectSystemHost {
        self.host
    }

    pub(super) fn now(&self) -> i32 {
        self.host.clock_ms_now
    }

    /// Plays a weapon's effect where the shot or blast happened. `played`
    /// is bumped when it starts; the answer says whether it did.
    pub(super) fn at_origin(
        &mut self,
        name: Option<FxName<'_>>,
        at: Pose,
        fired_count: &mut u32,
    ) -> bool {
        let Self { host, defs, infos, scene } = self;
        let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
        try_play_gun_fx_at_origin(host, defs, infos, name, at.anchor, at.turn, fired_count, scene)
    }

    /// The same, riding a bone of the thing that fired it.
    pub(super) fn bolted(
        &mut self,
        name: Option<FxName<'_>>,
        target: Option<EffectBoltTarget>,
        fired_count: &mut u32,
    ) -> bool {
        let Self { host, defs, infos, scene } = self;
        let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
        try_play_gun_fx_bolted(host, defs, infos, name, target, fired_count, scene)
    }

    /// The brass a shot throws, if its weapon has any.
    pub(super) fn shell_eject(
        &mut self,
        combat_fx: Option<&asset_game::WeaponBattleEffects>,
        own_view: bool,
        prior_shot: bool,
        target: Option<EffectBoltTarget>,
        ledger: &mut FxLedger,
    ) {
        let Self { host, defs, infos, scene } = self;
        let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
        let cursor = &mut ledger.cursor;
        spawn_casing_ejection(host, defs, infos, combat_fx, own_view, prior_shot, target, cursor, scene);
    }

    pub(super) fn bolted_by_name(
        &mut self,
        name: FxName<'_>,
        target: EffectBoltTarget,
    ) -> Option<PlaybackResult> {
        let Self { host, defs, infos, scene } = self;
        let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
        spawn_named_attached_world_effect(host, defs, infos, name, target, scene)
    }

    pub(super) fn oriented(&mut self, name: FxName<'_>, at: Pose) -> Option<PlaybackResult> {
        let Self { host, defs, infos, scene } = self;
        let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
        play_named_oriented_in_level(host, defs, infos, name, at.anchor, at.turn, scene)
    }

    /// Starts an effect that is kept (a looping one stays up until stopped).
    pub(super) fn spawned(&mut self, name: FxName<'_>, at: Pose, msec: i32) -> Option<PlaybackResult> {
        let Self { host, defs, infos, scene } = self;
        let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
        spawn_named_oriented_in_world(host, defs, infos, name, at.anchor, at.turn, msec, scene)
    }

    /// Plays an effect as of an earlier (or later) host time.
    pub(super) fn oriented_at(
        &mut self,
        name: FxName<'_>,
        at: Pose,
        msec: i32,
    ) -> Option<PlaybackResult> {
        let Self { host, defs, infos, scene } = self;
        let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
        play_named_oriented_at_msec(host, defs, infos, name, at.anchor, at.turn, msec, scene)
    }
}

/// The running tallies the combat-effect console reads, kept as one
/// parameter: a handler books its gaps on the cursor and calls
/// [`FxLedger::close_out`] last. The dump holds the tracer a pellet looked up.
#[derive(SystemParam)]
pub(super) struct FxLedger<'w> {
    pub(super) cursor: ResMut<'w, EffectJournalCursor>,
    pub(super) combat: ResMut<'w, BattleEffectsSnapshot>,
}

impl FxLedger<'_> {
    pub(super) fn close_out(&mut self) {
        report_combat_fx_gaps(&mut self.cursor);
    }
}
