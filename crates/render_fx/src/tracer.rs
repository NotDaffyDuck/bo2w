use std::collections::HashMap;

use asset_game::{OwnedTracerDef, TracerDefinitions};
use assets::{AssetEdge, TracerSpace};
use bevy::prelude::*;
use entity_t6::trajectory::tr_type;
use entity_t6::{Trajectory, evaluate_trajectory};
use fx::{
    FxMsec, LE_MOVING_TRACER, LE_TR_LINEAR, LocalEntityPool, LocalEntitySlot, tracer_travel_msec,
};
use fx_t6::runtime::vec3_normalize;
use fx_t6::{
    FX_BEAM_ADD_CAP, FX_TRACER_FIRST_PERSON_MAX_WIDTH, FX_TRACER_MIN_DIST, FxBeamKind, FxBeamTess,
    FxBeamVert, FxTracerBeam, beam_color_key, beam_sample_color, tracer_segment_count,
};

use crate::host::CombatFxDump;

// The tracer's local entity flies on BO2's linear trajectory.
const _: () = assert!(LE_TR_LINEAR == tr_type::LINEAR);

#[derive(Resource, Default)]
pub struct PreparedTracers(pub TracerDefinitions);

#[derive(Resource, Default)]
pub struct TracerDrawGate {
    counts: HashMap<u32, u8>,

    shots: HashMap<(u32, u32, u16), ()>,
}

impl TracerDrawGate {
    pub fn first_segment_of_pellet(
        &mut self,
        source_id: u32,
        correlation: u32,
        pellet: u16,
    ) -> bool {
        self.shots
            .insert((source_id, correlation, pellet), ())
            .is_none()
    }

    pub fn should_spawn(&mut self, source_id: u32, draw_interval: u32) -> bool {
        if draw_interval == 0 {
            return false;
        }
        let period = draw_interval.min(255) as u8;
        let entry = self.counts.entry(source_id).or_insert(0);
        if *entry == 0 || period < *entry {
            *entry = period;
        }
        *entry = entry.saturating_sub(1);
        if *entry != 0 {
            return false;
        }
        *entry = period;
        true
    }
}

pub struct QueuedBeam {
    pub tess: FxBeamTess,
    pub material: Option<usize>,
}

#[derive(Resource, Default)]
pub struct TracerWorld {
    pub pool: LocalEntityPool,
    pub queued: Vec<QueuedBeam>,
}

impl TracerWorld {
    pub fn live_count(&self) -> usize {
        self.pool.live_count()
    }

    pub fn tr_time_min(&self) -> Option<i32> {
        self.pool.tr_time_min()
    }

    pub fn end_time_min(&self) -> Option<i32> {
        self.pool.end_time_min()
    }
}

#[derive(Debug)]
pub enum TracerSpawnSkip {
    NoDef,
    Interval,
    Short,
}

pub fn try_spawn_tracer(
    gate: &mut TracerDrawGate,
    world: &mut TracerWorld,
    catalog: &TracerDefinitions,
    tracer: AssetEdge<TracerSpace>,
    source_id: u32,
    start: [f32; 3],
    end: [f32; 3],
    own_shot: bool,
    clock: FxMsec,
    combat: &mut CombatFxDump,
) -> Result<(), TracerSpawnSkip> {
    let Some(def) = tracer.bound_index().and_then(|index| catalog.def_at(index)) else {
        return Err(TracerSpawnSkip::NoDef);
    };
    combat.last_tracer_name = Some(def.name.clone());
    combat.last_tracer_mat_edge = Some(def.material.edge_kind().to_owned());
    combat.last_tracer_mat_index = def.material.bound_index().map(|i| i as i64);
    combat.last_tracer_material = def.present_name().map(str::to_owned);
    combat.last_tracer_bind = def
        .present_name()
        .map(|name| asset_game::material_bind_name(name).to_owned());
    combat.last_tracer_speed = Some(def.speed);
    combat.last_tracer_beam_length = Some(def.beam_length);
    combat.last_tracer_draw_interval = Some(i64::from(def.draw_interval));
    if !gate.should_spawn(source_id, def.draw_interval) {
        return Err(TracerSpawnSkip::Interval);
    }
    spawn_moving_tracer(world, def, start, end, own_shot, clock).map_err(|_| TracerSpawnSkip::Short)
}

fn spawn_moving_tracer(
    world: &mut TracerWorld,
    def: &OwnedTracerDef,
    start: [f32; 3],
    end: [f32; 3],
    own_shot: bool,
    clock: FxMsec,
) -> Result<(), ()> {
    let delta = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let dist = game_math::vec3_length(delta);
    // TODO(fact): BO2's tracer near cut is not in the facts; fx_t6 keeps MW2's.
    if dist <= FX_TRACER_MIN_DIST {
        return Err(());
    }
    // bo2zm: a Black Ops II smoke tracer lies along the whole path (its
    // speed is a token 1) until it has faded.
    let travel = if def.fade_msec > 0 {
        def.fade_msec as i32
    } else {
        tracer_travel_msec(dist, def.speed).ok_or(())?
    };
    let dir = vec3_normalize(delta);
    let mut width = def.beam_width;
    if own_shot {
        // TODO(fact): BO2's first-person width cap is not in the facts; fx_t6 keeps MW2's.
        width = width.min(FX_TRACER_FIRST_PERSON_MAX_WIDTH);
    }
    // bo2zm: BO2MP_TRACER_DEBUG=w,s (test aid): tracers w times wider and s
    // times slower, to see one drawn.
    static DEBUG: std::sync::OnceLock<(f32, f32)> = std::sync::OnceLock::new();
    let (wide, slow) = *DEBUG.get_or_init(|| {
        std::env::var("BO2MP_TRACER_DEBUG")
            .ok()
            .and_then(|v| {
                let (w, s) = v.split_once(',')?;
                Some((w.parse().ok()?, s.parse().ok()?))
            })
            .unwrap_or((1.0, 1.0))
    });
    let (width, speed) = (width * wide, def.speed / slow);
    let travel = if slow != 1.0 && def.fade_msec == 0 {
        tracer_travel_msec(dist, speed).ok_or(())?
    } else {
        travel
    };
    let delay = if own_shot && def.fade_msec == 0 {
        OWN_TRACER_DELAY_MSEC
    } else {
        0
    };
    world.pool.alloc(LocalEntitySlot {
        le_type: LE_MOVING_TRACER,
        pos_tr_time: clock.0.wrapping_add(delay),
        pos_tr_type: LE_TR_LINEAR,
        pos_tr_duration: 0,
        pos_tr_delta: [dir[0] * speed, dir[1] * speed, dir[2] * speed],
        pos_tr_base: start,
        end_time: clock.0.wrapping_add(delay + travel),
        material: def.material.bound_index(),
        tracer_clip_dist: dist,
        beam_length: def.beam_length.max(0.0),
        beam_width: width.max(0.0),
        screw_dist: def.screw_dist,
        screw_radius: def.screw_radius.max(0.0),
        colors: def.colors,
        own_shot,
        fade_msec: def.fade_msec as i32,
        starts_when_drawn: delay > 0,
    });
    Ok(())
}

/// bo2zm M3 fix list 3: a tracer from his own gun leaves the barrel this
/// long after the shot. Black Ops II's first-person muzzle flashes last 15-20
/// ms and cover the barrel; its pistol tracer (30 long at 4,000 units/s) is
/// only a streak in its first ~100 units, so it was spent under the flash and
/// what was left was a 3-pixel dash at the crosshair: his "I don't have a
/// tracer out of my pistol". Our choice, not the game's: it leaves on the
/// first frame after this, whole from the barrel, and lands that much late.
const OWN_TRACER_DELAY_MSEC: i32 = 25;

pub fn tick_tracer_beams(world: &mut TracerWorld, clock: FxMsec) {
    for tr in world.pool.live_mut() {
        let late = clock.0.wrapping_sub(tr.pos_tr_time);
        if tr.starts_when_drawn && late >= 0 {
            tr.pos_tr_time = clock.0;
            tr.end_time = tr.end_time.wrapping_add(late);
            tr.starts_when_drawn = false;
        }
    }
    world.pool.free_expired(clock);
    world.queued.clear();
    for tr in world.pool.live() {
        // Not out of the barrel yet (his own tracers wait for the flash).
        if clock.0.wrapping_sub(tr.pos_tr_time) < 0 {
            continue;
        }
        let begin = evaluate_trajectory(
            &Trajectory {
                tr_time: tr.pos_tr_time,
                tr_type: tr.pos_tr_type,
                tr_duration: tr.pos_tr_duration,
                tr_delta: tr.pos_tr_delta,
                tr_base: tr.pos_tr_base,
            },
            clock.0,
        );
        let dir = vec3_normalize(tr.pos_tr_delta);
        let start_from_base = [
            begin[0] - tr.pos_tr_base[0],
            begin[1] - tr.pos_tr_base[1],
            begin[2] - tr.pos_tr_base[2],
        ];
        let length_from_base =
            start_from_base[0] * dir[0] + start_from_base[1] * dir[1] + start_from_base[2] * dir[2];
        let remaining = (tr.tracer_clip_dist - length_from_base).max(0.0);
        let beam_len = tr.beam_length.min(remaining).max(0.0);
        if beam_len <= 0.0 {
            continue;
        }
        let end = [
            begin[0] + dir[0] * beam_len,
            begin[1] + dir[1] * beam_len,
            begin[2] + dir[2] * beam_len,
        ];
        if world.queued.len() >= FX_BEAM_ADD_CAP {
            break;
        }
        // bo2zm: a smoke tracer fades out over its life.
        let mut colors = tr.colors;
        if tr.fade_msec > 0 {
            let left = (tr.end_time.wrapping_sub(clock.0)) as f32 / tr.fade_msec as f32;
            for c in &mut colors {
                c[3] *= left.clamp(0.0, 1.0);
            }
        }
        // A smoke tracer (BO2's type 1) is the one that fades.
        let smoke = tr.fade_msec > 0;
        let tracer = FxTracerBeam {
            wiggle_dist: tr.screw_radius,
            // TODO(fact): asset_game does not load BO2's tracer texture-repeat
            // field yet; 1 runs the texture once along the beam, as before.
            tex_repeats: 1.0,
            segment_len: 0.0,
        };
        world.queued.push(QueuedBeam {
            tess: FxBeamTess {
                kind: if smoke {
                    FxBeamKind::SmokeTracer(tracer)
                } else {
                    FxBeamKind::Tracer(tracer)
                },
                depth_hack: false,
                draw_surf_override: 0,
                begin,
                end,
                colors: colors.map(beam_color_key),
                radius: tr.beam_width,
                segment_count: tracer_segment_count(smoke, beam_len, tr.screw_dist),
            },
            material: tr.material,
        });
    }
}

/// Puts back the colour keys `fx_t6::beam_generate_verts` divides by 4.
///
/// TODO(fact): BO2 stores beam RGB divided by 4 on every type but the smoke
/// tracer (fx-beam-laser-t6 section 2, item 11); whether its tracer shader
/// scales that back by 4 is unknown, so the undivided keys keep today's
/// brightness until it is.
pub fn beam_undivided_colors(tess: &FxBeamTess, verts: &mut [FxBeamVert]) {
    let last = verts.len().saturating_sub(1);
    let seg = f32::from(tess.segment_count.max(1));
    for (i, v) in verts.iter_mut().enumerate() {
        // The begin cap, a pair per segment step, the end cap.
        let t = match i {
            0 => 0.0,
            i if i == last => 1.0,
            i => ((i - 1) / 2) as f32 / seg,
        };
        let key = beam_sample_color(tess.kind, &tess.colors, t);
        v.color = [key[0], key[1], key[2], v.color[3]];
    }
}
