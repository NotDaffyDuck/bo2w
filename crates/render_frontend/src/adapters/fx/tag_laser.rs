//! bo2mp: the laser sight on a gun's `tag_laser` (BO2's CG_LaserDraw and
//! AddCore, fx_t6 `laser_sight`): a beam out to what the sight hits, and its
//! post light, for the local player's gun and each other player's.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx_t6::{
    FX_BEAM_ADD_CAP, FX_LASER_ADD_CAP, FX_LASER_MATERIAL, FX_LASER_TRACE_CONTENTS, FxLaserAim,
    FxLaserHit, FxLaserTraceCache, laser_sight,
};
use net::{ClientPredictionState, LocalPresentClient, PresentedSnapshot};
use render_fx::{HostFxPostLights, LaserDvars, QueuedBeam, TracerWorld};

use crate::assemble::drawsurf::MaterialGeneration;

#[derive(Default)]
pub(crate) struct TagLaserTraces {
    cache: FxLaserTraceCache,
    frame: u32,
}

#[derive(SystemParam)]
pub(crate) struct TagLaserQueue<'w, 's> {
    dvars: Res<'w, LaserDvars>,
    fpv_bolts: Res<'w, crate::adapters::anim::fpv_present::FpvBoltTargets>,
    remotes: Query<'w, 's, &'static crate::adapters::anim::remote_body::RemoteFxBolts>,
    prediction: Option<Res<'w, ClientPredictionState>>,
    presented: Res<'w, PresentedSnapshot>,
    local: Res<'w, LocalPresentClient>,
    runtime: Option<Res<'w, MaterialGeneration>>,
    traces: Local<'s, TagLaserTraces>,
}

impl TagLaserQueue<'_, '_> {
    /// Queues this frame's laser beams and lights.
    pub(crate) fn queue(
        &mut self,
        tracers: &mut TracerWorld,
        post_lights: &mut HostFxPostLights,
        view: [f32; 3],
    ) {
        let dvars = self.dvars.0;
        let world = self
            .prediction
            .as_ref()
            .filter(|p| p.0.is_armed() && p.0.world().has_world_clip())
            .map(|p| p.0.world());
        // TODO(fact): BO2 also traces players (a locational trace under entity
        // 18, a capsule trace above) and leaves out the owner; this stops at the
        // world and its glass only.
        let trace = |aim: &FxLaserAim, range: f32| {
            let Some(world) = world else {
                return FxLaserHit::MISS;
            };
            let end = std::array::from_fn(|i| aim.origin[i] + aim.dir[i] * range);
            let t = world.trace_static_world(
                aim.origin,
                end,
                [0.0; 3],
                [0.0; 3],
                FX_LASER_TRACE_CONTENTS,
            );
            FxLaserHit {
                fraction: t.fraction,
                surface_flags: t.sflags,
                startsolid: t.startsolid,
            }
        };
        let material = |name| {
            self.runtime.as_ref().and_then(|r| {
                r.catalog
                    .material_for_name(asset_game::material_bind_name(name))
                    .map(|m| usize::from(m.asset_id.0))
            })
        };
        let own_pos_frac = self
            .presented
            .player(self.local.0)
            .map_or(0.0, |ps| ps.f_weapon_pos_frac);

        let traces = &mut *self.traces;
        traces.frame = traces.frame.wrapping_add(1);
        let own = self.fpv_bolts.laser.iter().flatten().map(|t| (t, true));
        let others = self
            .remotes
            .iter()
            .filter_map(|b| b.laser.as_ref())
            .map(|t| (t, false));
        for (target, local_player) in own.chain(others).take(FX_LASER_ADD_CAP) {
            let aim = FxLaserAim {
                origin: target.orientation.origin,
                dir: target.orientation.axis[0],
                range: dvars.range_for(local_player),
                viewmodel: local_player,
                // TODO(fact): which fraction fades another player's sight; the
                // client has none for them, so theirs never fades.
                weapon_pos_frac: if local_player { own_pos_frac } else { 0.0 },
            };
            // BO2 keys the cache by entity number; here it is the gun model and tag.
            let key = target.dobj.rotate_left(16) ^ u32::from(target.bone);
            let hit = traces
                .cache
                .hit(traces.frame, key, || trace(&aim, aim.range));
            let light_hit = dvars
                .light_retrace_range(local_player, hit)
                .map(|range| (range, trace(&aim, range)));
            let Some(sight) = laser_sight(&dvars, aim, hit, light_hit, view) else {
                continue;
            };
            if tracers.queued.len() < FX_BEAM_ADD_CAP {
                tracers.queued.push(QueuedBeam {
                    tess: sight.beam,
                    // A missing viewmodel laser material draws with the plain one.
                    material: material(sight.material).or_else(|| material(FX_LASER_MATERIAL)),
                });
            }
            if let Some(light) = sight.light {
                post_lights.add(light);
            }
        }
    }
}
