//! bo2mp: the laser sight on a gun's `tag_laser` (BO2's CG_LaserDraw and
//! AddCore, fx_t6 `laser_sight`): a beam out to what the sight hits, and its
//! post light, for the local player's gun and each other player's.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx_t6::{
    RAY_QUEUE_LIMIT, FX_LASER_ADD_CAP, FX_LASER_MATERIAL, FX_LASER_TRACE_CONTENTS, FxLaserAim, FxLaserDvars,
    FxLaserHit, FxLaserTraceCache, laser_sight,
};
use fx::EffectBoltTarget;
use net::{ClientGuessState, HostViewClient, ShownWorldFrame};
use render_fx::{PostEffectLightSet, LaserSettings, PendingTracerBeam, StreakLevel};

use crate::assemble::draw_items::MaterialEpoch;

#[derive(Default)]
pub(crate) struct TagLaserTraces {
    cache: FxLaserTraceCache,
    frame: u32,
}

#[derive(SystemParam)]
pub(crate) struct TagLaserQueue<'w, 's> {
    dvars: Res<'w, LaserSettings>,
    fpv_bolts: Res<'w, crate::adapters::motion::gun_present::AttachTagGoals>,
    remotes: Query<'w, 's, &'static crate::adapters::motion::peer_body::OtherBodyEffectTags>,
    prediction: Option<Res<'w, ClientGuessState>>,
    shown: Res<'w, ShownWorldFrame>,
    viewer: Res<'w, HostViewClient>,
    runtime: Option<Res<'w, MaterialEpoch>>,
    traces: Local<'s, TagLaserTraces>,
}

/// Which gun a beam comes from: the local player's own or another player's.
#[derive(Clone, Copy, PartialEq)]
enum Wielder {
    Own,
    Other,
}

/// The aim of one gun's sight.
fn aim_of(goal: &EffectBoltTarget, wielder: Wielder, dvars: &FxLaserDvars, own_pos_frac: f32) -> FxLaserAim {
    let own = wielder == Wielder::Own;
    FxLaserAim {
        origin: goal.orientation.origin,
        heading: goal.orientation.axis[0],
        range: dvars.range_for(own),
        viewmodel: own,
        // TODO(fact): which fraction fades another player's sight; the
        // client has none for them, so theirs never fades.
        gun_raise_share: if own { own_pos_frac } else { 0.0 },
    }
}

/// BO2 keys the trace cache by entity number; here it is the gun model and tag.
fn cache_key(target: &EffectBoltTarget) -> u32 {
    target.dobj.rotate_left(16) ^ u32::from(target.bone)
}

impl TagLaserQueue<'_, '_> {
    /// The material id of a laser material, once the renderer knows it.
    fn material_id(&self, wanted: &str) -> Option<usize> {
        let runtime = self.runtime.as_ref()?;
        let found = runtime
            .catalog
            .material_by_name(asset_game::material_bind_name(wanted))?;
        Some(usize::from(found.asset_id.0))
    }

    /// Queues this frame's laser beams and lights.
    pub(crate) fn queue(
        &mut self,
        tracers: &mut StreakLevel,
        after_lights: &mut PostEffectLightSet,
        view: [f32; 3],
    ) {
        let dvars = self.dvars.0;
        let world = self
            .prediction
            .as_ref()
            .filter(|p| p.0.is_forecast_live() && p.0.world().has_static_collision())
            .map(|p| p.0.world());
        // TODO(fact): BO2 also traces players (a locational trace under entity
        // 18, a capsule trace above) and leaves out the owner; this stops at the
        // world and its glass only.
        let trace = |aim: &FxLaserAim, reach: f32| {
            let Some(world) = world else {
                return FxLaserHit::MISS;
            };
            let end = std::array::from_fn(|i| aim.origin[i] + aim.heading[i] * reach);
            let swept = world.trace_static_world(
                aim.origin,
                end,
                [0.0; 3],
                [0.0; 3],
                FX_LASER_TRACE_CONTENTS,
            );
            FxLaserHit {
                fraction: swept.fraction,
                face_bits: swept.sflags,
                began_inside: swept.starts_inside,
            }
        };
        let own_pos_frac = self
            .shown
            .player(self.viewer.0)
            .map(|ps| ps.weapon_raise_fraction)
            .unwrap_or(0.0);

        let mut guns: Vec<(&EffectBoltTarget, Wielder)> = self
            .fpv_bolts
            .laser
            .iter()
            .flatten()
            .map(|target| (target, Wielder::Own))
            .collect();
        guns.extend(
            self.remotes
                .iter()
                .filter_map(|bolts| bolts.laser.as_ref())
                .map(|target| (target, Wielder::Other)),
        );
        guns.truncate(FX_LASER_ADD_CAP);

        self.traces.frame = self.traces.frame.wrapping_add(1);
        let frame = self.traces.frame;
        for (goal, wielder) in guns {
            let aim = aim_of(goal, wielder, &dvars, own_pos_frac);
            let hit = self
                .traces
                .cache
                .hit(frame, cache_key(goal), || trace(&aim, aim.range));
            let light_hit = dvars
                .light_retrace_range(aim.viewmodel, hit)
                .map(|range| (range, trace(&aim, range)));
            let Some(sight) = laser_sight(&dvars, aim, hit, light_hit, view) else {
                continue;
            };
            if tracers.pending_list.len() < RAY_QUEUE_LIMIT {
                // A missing viewmodel laser material draws with the plain one.
                let material = self
                    .material_id(sight.material)
                    .or_else(|| self.material_id(FX_LASER_MATERIAL));
                tracers.pending_list.push(PendingTracerBeam {
                    tess: sight.beam,
                    mat_ref: material,
                });
            }
            if let Some(light) = sight.light {
                after_lights.bump_by(light);
            }
        }
    }
}
