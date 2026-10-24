//! What a handler needs to start an effect, bundled so it asks for one
//! parameter instead of five: the parsed effect catalog, the per-element
//! cache built from it, the host the effects live in, and the world they are
//! drawn against.

use asset_game::{FxDefinitions, FxName};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx::{FxBoltTarget, FxSystemHost, PlayResult};
use render_fx::combat::{
    log_combat_fx_gaps, play_shell_eject, sync_combat_dump, try_play_weapon_fx_at_origin,
    try_play_weapon_fx_bolted,
};
use render_fx::{
    CombatFxDump, FxJournalCursor, HostFxSystem, PreparedFxCatalog, PreparedFxElemInfos,
};

use super::FxSceneAccess;
use crate::adapters::fx::present::{
    FxElemInfoCache, FxScene, play_named_bolted_in_world, play_named_oriented_at_msec,
    play_named_oriented_in_world, spawn_named_oriented_in_world,
};
use crate::adapters::fx::world_mark::FrontendFxScene;

#[derive(SystemParam)]
pub(super) struct FxStage<'w> {
    catalog: Option<Res<'w, PreparedFxCatalog>>,
    infos: ResMut<'w, PreparedFxElemInfos>,
    host: ResMut<'w, HostFxSystem>,
    world: FxSceneAccess<'w>,
}

impl FxStage<'_> {
    /// The stage ready for playing, or `None` while no effect catalog is
    /// loaded (the caller books that as a gap in its own counter).
    pub(super) fn playback(&mut self) -> Option<Playback<'_>> {
        let Self {
            catalog,
            infos,
            host,
            world,
        } = self;
        let catalog = catalog.as_deref()?;
        infos.0.sync(&catalog.0);
        Some(Playback {
            host: &mut host.0,
            defs: &catalog.0,
            infos: &mut infos.0,
            scene: world.view(),
        })
    }

    /// The host without a catalog, for the handlers that only stop or
    /// shape effects already running.
    pub(super) fn host(&mut self) -> &mut FxSystemHost {
        &mut self.host.0
    }

    /// The pieces side by side, for the routines that take the catalog, the
    /// cache, the host and the scene as arguments of their own.
    pub(super) fn parts(&mut self) -> StageParts<'_> {
        let Self {
            catalog,
            infos,
            host,
            world,
        } = self;
        let catalog = catalog.as_deref();
        if let Some(catalog) = catalog {
            infos.0.sync(&catalog.0);
        }
        StageParts {
            catalog,
            infos: &mut infos.0,
            host: &mut **host,
            scene: world.view(),
        }
    }
}

pub(super) struct StageParts<'a> {
    pub(super) catalog: Option<&'a PreparedFxCatalog>,
    pub(super) infos: &'a mut FxElemInfoCache,
    pub(super) host: &'a mut HostFxSystem,
    pub(super) scene: Option<FrontendFxScene<'a>>,
}

/// A [`FxStage`] with its catalog present. The scene is held next to the
/// host as separate fields, so each method borrows them side by side.
pub(super) struct Playback<'a> {
    host: &'a mut FxSystemHost,
    defs: &'a FxDefinitions,
    infos: &'a mut FxElemInfoCache,
    scene: Option<FrontendFxScene<'a>>,
}

impl<'a> Playback<'a> {
    pub(super) fn defs(&self) -> &'a FxDefinitions {
        self.defs
    }

    pub(super) fn host(&mut self) -> &mut FxSystemHost {
        self.host
    }

    pub(super) fn now(&self) -> i32 {
        self.host.msec_now
    }

    /// Plays a weapon's effect where the shot or blast happened. `played`
    /// is bumped when it starts; the answer says whether it did.
    pub(super) fn at_origin(
        &mut self,
        name: Option<FxName<'_>>,
        origin: [f32; 3],
        axis: [[f32; 3]; 3],
        played: &mut u32,
    ) -> bool {
        let scene = self.scene.as_ref().map(|s| s as &dyn FxScene);
        try_play_weapon_fx_at_origin(
            &mut *self.host,
            self.defs,
            &mut *self.infos,
            name,
            origin,
            axis,
            played,
            scene,
        )
    }

    /// The same, riding a bone of the thing that fired it.
    pub(super) fn bolted(
        &mut self,
        name: Option<FxName<'_>>,
        target: Option<FxBoltTarget>,
        played: &mut u32,
    ) -> bool {
        let scene = self.scene.as_ref().map(|s| s as &dyn FxScene);
        try_play_weapon_fx_bolted(
            &mut *self.host,
            self.defs,
            &mut *self.infos,
            name,
            target,
            played,
            scene,
        )
    }

    /// The brass a shot throws, if its weapon has any.
    pub(super) fn shell_eject(
        &mut self,
        combat_fx: Option<&asset_game::WeaponCombatFx>,
        player_view: bool,
        last_shot: bool,
        target: Option<FxBoltTarget>,
        ledger: &mut FxLedger,
    ) {
        let scene = self.scene.as_ref().map(|s| s as &dyn FxScene);
        play_shell_eject(
            &mut *self.host,
            self.defs,
            &mut *self.infos,
            combat_fx,
            player_view,
            last_shot,
            target,
            &mut ledger.cursor,
            &mut ledger.combat,
            scene,
        );
    }

    pub(super) fn bolted_by_name(
        &mut self,
        name: FxName<'_>,
        target: FxBoltTarget,
    ) -> Option<PlayResult> {
        let scene = self.scene.as_ref().map(|s| s as &dyn FxScene);
        play_named_bolted_in_world(&mut *self.host, self.defs, &*self.infos, name, target, scene)
    }

    pub(super) fn oriented(
        &mut self,
        name: FxName<'_>,
        origin: [f32; 3],
        axis: [[f32; 3]; 3],
    ) -> Option<PlayResult> {
        let scene = self.scene.as_ref().map(|s| s as &dyn FxScene);
        play_named_oriented_in_world(
            &mut *self.host,
            self.defs,
            &*self.infos,
            name,
            origin,
            axis,
            scene,
        )
    }

    /// Starts an effect that is kept (a looping one stays up until stopped).
    pub(super) fn spawned(
        &mut self,
        name: FxName<'_>,
        origin: [f32; 3],
        axis: [[f32; 3]; 3],
        msec: i32,
    ) -> Option<PlayResult> {
        let scene = self.scene.as_ref().map(|s| s as &dyn FxScene);
        spawn_named_oriented_in_world(
            &mut *self.host,
            self.defs,
            &*self.infos,
            name,
            origin,
            axis,
            msec,
            scene,
        )
    }

    /// Plays an effect as of an earlier (or later) host time.
    pub(super) fn oriented_at(
        &mut self,
        name: FxName<'_>,
        origin: [f32; 3],
        axis: [[f32; 3]; 3],
        msec: i32,
    ) -> Option<PlayResult> {
        let scene = self.scene.as_ref().map(|s| s as &dyn FxScene);
        play_named_oriented_at_msec(
            &mut *self.host,
            self.defs,
            &*self.infos,
            name,
            origin,
            axis,
            msec,
            scene,
        )
    }
}

/// The running tallies the combat-effect console reads, kept as one
/// parameter: a handler books its gaps on the cursor and calls
/// [`FxLedger::close_out`] last.
#[derive(SystemParam)]
pub(super) struct FxLedger<'w> {
    pub(super) cursor: ResMut<'w, FxJournalCursor>,
    pub(super) combat: ResMut<'w, CombatFxDump>,
}

impl FxLedger<'_> {
    pub(super) fn close_out(&mut self) {
        log_combat_fx_gaps(&mut self.cursor, &self.combat);
        sync_combat_dump(&self.cursor, &mut self.combat);
    }
}
