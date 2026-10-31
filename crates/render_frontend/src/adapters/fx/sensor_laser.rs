//! bo2mp: the Sensor Grenade's scan lasers (BO2's CG_SensorGrenade, drawn
//! through CG_AddCore): a white "gfx_laser" beam from the grenade to each
//! enemy it sweeps, fading out along its length. The server sends one
//! `EV_SENSOR_LASER` a tick while a beam is clear; each is held a little past
//! the next tick so the beam stays whole between them.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx_t6::{RAY_QUEUE_LIMIT, FxBeamKind, EffectBeamStitch};
use render_fx::{EffectsSystemHandle, PendingTracerBeam, StreakLevel};

use crate::assemble::draw_items::MaterialEpoch;

/// How long one laser event keeps its beam up: a server tick and a bit.
const HOLD_MS: i32 = 90;
/// CG_AddCore's core radius (the laser's).
const RADIUS: f32 = 1.4;
/// laserFlarePct's default: how much the far end widens with distance.
const FLARE_PCT: f32 = 0.25;

#[derive(Clone, Copy)]
struct Laser {
    open_at: [f32; 3],
    end: [f32; 3],
    deadline_millis: i32,
}

impl Laser {
    fn is_up(&self, clock_millis: i32) -> bool {
        self.deadline_millis.wrapping_sub(clock_millis) > 0
    }

    /// The beam as seen from `view`: wider at the far end the farther it is.
    fn beam(&self, view: [f32; 3], material: Option<usize>) -> PendingTracerBeam {
        let far = Vec3::from_array(view).distance(Vec3::from_array(self.end));
        let white = [255, 255, 255, 255];
        PendingTracerBeam {
            tess: EffectBeamStitch {
                kind: FxBeamKind::Laser {
                    finish_reach: (far * FLARE_PCT * 0.01 + 1.0) * RADIUS,
                },
                near_layer_draw: false,
                draw_surf_override: 0,
                begin: self.open_at,
                end: self.end,
                // A laser reads two keys, begin and end: white, fading out.
                colors: [white, [255, 255, 255, 0], [0; 4], [0; 4], [0; 4]],
                radius: RADIUS,
                piece_tally: 1,
            },
            mat_ref: material,
        }
    }
}

/// The beams up now, by grenade.
#[derive(Resource, Default)]
pub(crate) struct SensorLasers {
    by_grenade: HashMap<u32, Laser>,
}

pub(crate) fn note(
    fx_play: On<net::EffectAt>,
    host: Res<EffectsSystemHandle>,
    mut lasers: ResMut<SensorLasers>,
) {
    let heard = &fx_play.event;
    if heard.event.0 != entity_t6::event::ours::SENSOR_LASER {
        return;
    }
    let payload = heard.payload;
    let deadline_millis = host.0.clock_ms_now.wrapping_add(HOLD_MS);
    let laser = Laser {
        open_at: payload.origin,
        end: payload.origin2,
        deadline_millis,
    };
    lasers.by_grenade.insert(payload.correlation, laser);
}

#[derive(SystemParam)]
pub(crate) struct SensorLaserQueue<'w> {
    lasers: ResMut<'w, SensorLasers>,
    runtime: Option<Res<'w, MaterialEpoch>>,
}

impl SensorLaserQueue<'_> {
    /// Drops the beams whose time is up and queues the rest.
    pub(crate) fn queue(&mut self, tracers: &mut StreakLevel, clock_millis: i32, view: [f32; 3]) {
        self.lasers.by_grenade.retain(|_, laser| laser.is_up(clock_millis));
        if self.lasers.by_grenade.is_empty() {
            return;
        }
        let laser_material = self.runtime.as_ref().and_then(|runtime| {
            let found = runtime
                .catalog
                .material_by_name(asset_game::material_bind_name("gfx_laser"))?;
            Some(usize::from(found.asset_id.0))
        });
        let room = RAY_QUEUE_LIMIT.saturating_sub(tracers.pending_list.len());
        let beams = self.lasers.by_grenade.values().take(room);
        let drawn = beams.map(|laser| laser.beam(view, laser_material));
        tracers.pending_list.extend(drawn);
    }
}
