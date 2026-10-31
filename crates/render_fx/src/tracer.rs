//! Bullet tracers: spawning them as moving local entities, ticking them along their
//! straight paths and turning the live ones into beams for the beam tessellator.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use asset_game::{SelfContainedTracerDef, TracerDefinitions};
use assets::{AssetDependencyLink, TracerRange};
use bevy::prelude::*;
use entity_t6::flight::path_kind;
use entity_t6::{Trajectory, evaluate_trajectory};
use fx::{
    EffectClockMs, THING_FLYING_STREAK, PATH_STRAIGHT_LINE, ClientOnlyThingBank, ClientOnlyThingSlot, tracer_flight_ms,
};
use fx_t6::runtime::vec3_normalize;
use fx_t6::{
    RAY_QUEUE_LIMIT, OWN_BULLET_STREAK_WIDEST, BULLET_STREAK_SHORTEST, FxBeamKind, EffectBeamStitch,
    RayCorner, FxTracerBeam, beam_color_key, beam_sample_color, tracer_segment_count,
};

use crate::host::{BattleEffectsSnapshot, TracerSample};

// Tracers fly on the engine's linear trajectory type.
const _: () = assert!(PATH_STRAIGHT_LINE == path_kind::STRAIGHT);

/// A tracer fired from the player's own weapon leaves the barrel this many milliseconds
/// after the shot. Black Ops II's first-person muzzle flash lasts 15-20 ms and covers the
/// barrel, and its pistol tracer (30 units long at 4,000 units/s) is only a streak over
/// its first ~100 units, so it used to be spent under the flash and show as a three-pixel
/// dash at the crosshair. This delay is our own choice, not the game's: the tracer starts
/// on the first frame after it, whole from the barrel, and lands that much late.
const OWN_TRACER_DELAY_MSEC: i32 = 25;

#[derive(Resource, Default)]
pub struct StagedStreaks(pub TracerDefinitions);

/// Decides which shots get a tracer: one every `render_spacing` shots per source, and one
/// per pellet of a shotgun blast (its first segment only).
#[derive(Resource, Default)]
pub struct TracerRenderGuard {
    shots_until_draw: HashMap<u32, u8>,
    seen_pellets: HashSet<(u32, u32, u16)>,
}

impl TracerRenderGuard {
    /// True the first time this (source, shot, pellet) is asked about.
    pub fn opening_leg_of_pellet(
        &mut self,
        emitter_key: u32,
        shot_key: u32,
        pellet_ix: u16,
    ) -> bool {
        self.seen_pellets.insert((emitter_key, shot_key, pellet_ix))
    }

    /// Counts a shot for `emitter_key`; true when it is the one that draws.
    pub fn wants_spawn(&mut self, emitter_key: u32, render_spacing: u32) -> bool {
        if render_spacing == 0 {
            return false;
        }
        let period = render_spacing.min(255) as u8;
        let counter = self.shots_until_draw.entry(emitter_key).or_insert(0);
        // A fresh source, or one whose interval got shorter, restarts its countdown.
        if *counter == 0 || period < *counter {
            *counter = period;
        }
        *counter -= 1;
        if *counter > 0 {
            return false;
        }
        *counter = period;
        true
    }
}

/// Why a shot got no tracer.
#[derive(Debug)]
pub enum TracerSpawnRefusal {
    NoDef,
    Interval,
    Short,
}

/// One beam waiting for the tessellator, with the material it draws with.
pub struct PendingTracerBeam {
    pub tess: EffectBeamStitch,
    pub mat_ref: Option<usize>,
}

/// The tracers in flight, and the beams they were turned into at the last tick.
#[derive(Resource, Default)]
pub struct StreakLevel {
    pub pool: ClientOnlyThingBank,
    pub pending_list: Vec<PendingTracerBeam>,
}

/// Notes the tracer a shot looked up, for the `fx_dump` report.
fn record_tracer_sample(sample: &mut TracerSample, def: &SelfContainedTracerDef) {
    let present = def.existing_label();
    *sample = TracerSample {
        name: Some(def.name.clone()),
        shader: present.map(str::to_owned),
        bind: present.map(|name| asset_game::material_bind_name(name).to_owned()),
        has_color: sample.has_color,
        speed: Some(def.speed),
        ray_extent: Some(def.ray_extent),
    };
}

/// Looks the tracer up and notes it in the combat dump, applies the draw interval and
/// spawns the moving tracer.
pub fn attempt_bullet_streak(
    gate: &mut TracerRenderGuard,
    world: &mut StreakLevel,
    catalog: &TracerDefinitions,
    tracer: AssetDependencyLink<TracerRange>,
    emitter_key: u32,
    from_pt: [f32; 3],
    end: [f32; 3],
    fired_by_me: bool,
    clock: EffectClockMs,
    dump: &mut BattleEffectsSnapshot,
) -> Result<(), TracerSpawnRefusal> {
    let def = tracer
        .extent_position()
        .and_then(|index| catalog.recipe_at(index))
        .ok_or(TracerSpawnRefusal::NoDef)?;
    record_tracer_sample(&mut dump.sample, def);
    if !gate.wants_spawn(emitter_key, def.render_spacing) {
        return Err(TracerSpawnRefusal::Interval);
    }
    launch_travelling_tracer(world, def, from_pt, end, fired_by_me, clock).ok_or(TracerSpawnRefusal::Short)
}

/// Test aid: `BO2MP_TRACER_DEBUG=w,s` makes tracers `w` times wider and `s` times slower
/// so one can be seen at all. Unset or malformed means (1, 1).
fn debug_scale() -> (f32, f32) {
    static SCALE: OnceLock<(f32, f32)> = OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("BO2MP_TRACER_DEBUG")
            .ok()
            .and_then(|raw_text| {
                let (wide, slow) = raw_text.split_once(',')?;
                Some((wide.parse().ok()?, slow.parse().ok()?))
            })
            .unwrap_or((1.0, 1.0))
    })
}

/// Allocates the flying tracer, or `None` when the shot is too short or too slow to draw.
fn launch_travelling_tracer(
    world: &mut StreakLevel,
    def: &SelfContainedTracerDef,
    from_pt: [f32; 3],
    end: [f32; 3],
    fired_by_me: bool,
    clock: EffectClockMs,
) -> Option<()> {
    let offset = [end[0] - from_pt[0], end[1] - from_pt[1], end[2] - from_pt[2]];
    let distance = game_math::vec3_norm(offset);
    // TODO(fact): BO2's near cut for tracers is not in the facts; fx_t6 keeps the old value.
    if distance <= BULLET_STREAK_SHORTEST {
        return None;
    }
    let (wide, slow) = debug_scale();
    let speed = def.speed / slow;
    // A Black Ops II smoke tracer (speed is a token 1) lies along the whole path until it
    // has faded; every other tracer lives as long as its flight takes.
    let smoke = def.fade_msec > 0;
    let lifetime = if smoke {
        def.fade_msec as i32
    } else {
        tracer_flight_ms(distance, speed)?
    };
    let mut width = def.ray_thickness;
    if fired_by_me {
        // TODO(fact): BO2's first-person width cap is not in the facts either.
        width = width.min(OWN_BULLET_STREAK_WIDEST);
    }
    let width = width * wide;
    let delay = if fired_by_me && !smoke {
        OWN_TRACER_DELAY_MSEC
    } else {
        0
    };
    let heading = vec3_normalize(offset);
    world.pool.claim_slot(ClientOnlyThingSlot {
        thing_category: THING_FLYING_STREAK,
        path_start_ms: clock.0.wrapping_add(delay),
        pos_tr_type: PATH_STRAIGHT_LINE,
        path_length_ms: 0,
        path_drift: heading.map(|axis| axis * speed),
        path_start_point: from_pt,
        finish_stamp: clock.0.wrapping_add(delay + lifetime),
        shader_ix: def.material.extent_position(),
        tracer_cut: distance,
        ray_extent: def.ray_extent.max(0.0),
        ray_thickness: width.max(0.0),
        spiral_pitch_distance: def.spiral_pitch_distance,
        spiral_radius: def.spiral_radius.max(0.0),
        colors: def.colors,
        fired_by_me,
        fade_msec: def.fade_msec as i32,
        starts_when_drawn: delay > 0,
    });
    Some(())
}

/// Advances every live tracer to `clock` and rebuilds the queue of beams to draw.
pub fn step_tracer_beams(world: &mut StreakLevel, clock: EffectClockMs) {
    // A delayed tracer starts its clock on the first tick at or after its start time, and
    // its end time slides by however late that tick was.
    for tracer in world.pool.live_mut() {
        let late = clock.0.wrapping_sub(tracer.path_start_ms);
        if tracer.starts_when_drawn && late >= 0 {
            tracer.path_start_ms = clock.0;
            tracer.finish_stamp = tracer.finish_stamp.wrapping_add(late);
            tracer.starts_when_drawn = false;
        }
    }
    world.pool.reclaim_finished(clock);
    world.pending_list.clear();

    for tracer in world.pool.live() {
        // Still waiting for the muzzle flash to clear the barrel.
        if clock.0.wrapping_sub(tracer.path_start_ms) < 0 {
            continue;
        }
        let head = evaluate_trajectory(
            &Trajectory {
                began_at: tracer.path_start_ms,
                tr_type: tracer.pos_tr_type,
                span_ms: tracer.path_length_ms,
                rate: tracer.path_drift,
                base_point: tracer.path_start_point,
            },
            clock.0,
        );
        let heading = vec3_normalize(tracer.path_drift);
        let travelled: f32 = (0..3)
            .map(|axis| (head[axis] - tracer.path_start_point[axis]) * heading[axis])
            .sum();
        // The beam never reaches past the point the shot ended.
        let visible = tracer
            .ray_extent
            .min((tracer.tracer_cut - travelled).max(0.0))
            .max(0.0);
        if visible <= 0.0 {
            continue;
        }
        if world.pending_list.len() >= RAY_QUEUE_LIMIT {
            break;
        }
        let tail = [0, 1, 2].map(|axis| head[axis] + heading[axis] * visible);

        // A smoke tracer fades out over its life; it is also the only kind with its own beam kind.
        let smoke = tracer.fade_msec > 0;
        let mut colors = tracer.colors;
        if smoke {
            let remaining = tracer.finish_stamp.wrapping_sub(clock.0) as f32 / tracer.fade_msec as f32;
            for color in &mut colors {
                color[3] *= remaining.clamp(0.0, 1.0);
            }
        }
        let beam = FxTracerBeam {
            sway_reach: tracer.spiral_radius,
            // TODO(fact): asset_game does not load BO2's tracer texture-repeat field yet; 1
            // runs the texture once along the beam, as it always has.
            tex_repeats: 1.0,
            segment_len: 0.0,
        };
        world.pending_list.push(PendingTracerBeam {
            tess: EffectBeamStitch {
                kind: if smoke {
                    FxBeamKind::SmokeTracer(beam)
                } else {
                    FxBeamKind::Tracer(beam)
                },
                near_layer_draw: false,
                draw_surf_override: 0,
                begin: head,
                end: tail,
                colors: colors.map(beam_color_key),
                radius: tracer.ray_thickness,
                piece_tally: tracer_segment_count(smoke, visible, tracer.spiral_pitch_distance),
            },
            mat_ref: tracer.shader_ix,
        });
    }
}

/// Puts back the colour keys `fx_t6::beam_generate_verts` divides by 4.
///
/// TODO(fact): BO2 stores beam RGB divided by 4 on every type but the smoke tracer
/// (fx-beam-laser-t6 section 2, item 11); whether its tracer shader scales that back by 4 is
/// unknown, so the undivided keys keep today's brightness until it is.
pub fn beam_undivided_colors(tess: &EffectBeamStitch, vertex_points: &mut [RayCorner]) {
    let last = vertex_points.len().saturating_sub(1);
    let steps = f32::from(tess.piece_tally.max(1));
    for (n, vert) in vertex_points.iter_mut().enumerate() {
        // Layout: begin cap, one pair of vertices per segment step, end cap.
        let along = match n {
            0 => 0.0,
            n if n == last => 1.0,
            n => ((n - 1) / 2) as f32 / steps,
        };
        let [r, g, b, _] = beam_sample_color(tess.kind, &tess.colors, along);
        vert.color = [r, g, b, vert.color[3]];
    }
}
