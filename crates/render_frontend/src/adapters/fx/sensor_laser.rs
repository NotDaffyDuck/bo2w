//! bo2mp: the Sensor Grenade's scan lasers (BO2's CG_SensorGrenade, drawn
//! through CG_AddCore): a white "gfx_laser" beam from the grenade to each
//! enemy it sweeps, fading out along its length. The server sends one
//! `EV_SENSOR_LASER` a tick while a beam is clear; each is held a little past
//! the next tick so the beam stays whole between them.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx_t6::{FX_BEAM_ADD_CAP, FxBeamKind, FxBeamTess};
use render_fx::{HostFxSystem, QueuedBeam, TracerWorld};

use crate::assemble::drawsurf::MaterialGeneration;

/// How long one laser event keeps its beam up: a server tick and a bit.
const HOLD_MS: i32 = 90;
/// CG_AddCore's core radius (the laser's).
const RADIUS: f32 = 1.4;
/// laserFlarePct's default: how much the far end widens with distance.
const FLARE_PCT: f32 = 0.25;

#[derive(Clone, Copy)]
struct Laser {
    begin: [f32; 3],
    end: [f32; 3],
    until_ms: i32,
}

/// The beams up now, by grenade.
#[derive(Resource, Default)]
pub(crate) struct SensorLasers {
    by_grenade: HashMap<u32, Laser>,
}

pub(crate) fn note(
    play: On<net::EntityPlayFx>,
    host: Res<HostFxSystem>,
    mut lasers: ResMut<SensorLasers>,
) {
    if play.event.event.0 != entity_t6::event::ours::SENSOR_LASER {
        return;
    }
    let payload = play.event.payload;
    lasers.by_grenade.insert(
        payload.correlation,
        Laser {
            begin: payload.origin,
            end: payload.origin2,
            until_ms: host.0.msec_now.wrapping_add(HOLD_MS),
        },
    );
}

#[derive(SystemParam)]
pub(crate) struct SensorLaserQueue<'w> {
    lasers: ResMut<'w, SensorLasers>,
    runtime: Option<Res<'w, MaterialGeneration>>,
}

impl SensorLaserQueue<'_> {
    /// Drops the beams whose time is up and queues the rest.
    pub(crate) fn queue(&mut self, tracers: &mut TracerWorld, now_ms: i32, view: [f32; 3]) {
        self.lasers
            .by_grenade
            .retain(|_, l| l.until_ms.wrapping_sub(now_ms) > 0);
        if self.lasers.by_grenade.is_empty() {
            return;
        }
        let material = self.runtime.as_ref().and_then(|r| {
            r.catalog
                .material_for_name(asset_game::material_bind_name("gfx_laser"))
                .map(|m| usize::from(m.asset_id.0))
        });
        for laser in self.lasers.by_grenade.values() {
            if tracers.queued.len() >= FX_BEAM_ADD_CAP {
                break;
            }
            let far = Vec3::from_array(view).distance(Vec3::from_array(laser.end));
            tracers.queued.push(QueuedBeam {
                tess: FxBeamTess {
                    kind: FxBeamKind::Laser {
                        end_radius: (far * FLARE_PCT * 0.01 + 1.0) * RADIUS,
                    },
                    depth_hack: false,
                    draw_surf_override: 0,
                    begin: laser.begin,
                    end: laser.end,
                    // A laser reads two keys, begin and end: white, fading out.
                    colors: [[255, 255, 255, 255], [255, 255, 255, 0], [0; 4], [0; 4], [0; 4]],
                    radius: RADIUS,
                    segment_count: 1,
                },
                material,
            });
        }
    }
}
