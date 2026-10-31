//! How one part moves over a time slice: free flight along its velocity
//! graphs and gravity, or the same with collision against the world.

use fx_t6::runtime::{
    PARTICLE_NEVER_SETTLED, Vec3Spread, collide_marks_at_rest, collide_substep_schedule,
    collision_reflect_base_vel_delta, elem_dies_on_touch, elem_gravity_accel_z_sampled,
    elem_update_has_velocity_graph, elem_uses_collision, elem_uses_vel_local, elem_uses_vel_world,
    get_at_rest_fraction, get_velocity_at_time, impact_child_speed_allows,
    integrate_velocity_graph, orientation_pos_from_world, orientation_pos_to_world,
    sample_reflection_factor, trace_mask, vec3_length_sq,
};

use crate::tick::{ParticleMoveAsk, ParticleMoveOutcome, ParticleRayContact, ContactBirth};

/// A collision sweep splits one time slice into at most this many pieces
/// per sub-step (one for each bounce that stays in the slice).
const BOUNCES_PER_SUBSTEP: usize = 8;

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn scale3(v: [f32; 3], factor: f32) -> [f32; 3] {
    v.map(|c| c * factor)
}

fn blend_triple(a: [f32; 3], b: [f32; 3], share: f32) -> [f32; 3] {
    add3(a, scale3(sub3(b, a), share))
}

/// Milliseconds from `from` to `to`, never negative.
fn ms_between(from: i32, to: i32) -> f32 {
    to.saturating_sub(from).max(0) as f32
}

/// What stays fixed for one part while it is moved: its flags, its gravity
/// range, its random seed and its two velocity graphs.
#[derive(Clone, Copy)]
struct Flight<'g> {
    flags: i32,
    gravity: (f32, f32),
    seed: u32,
    local_graph: &'g [Vec3Spread],
    world_graph: &'g [Vec3Spread],
}

impl Flight<'_> {
    /// Moves a part through the time slice with nothing in its way: the local
    /// and world velocity graphs, then base velocity and gravity.
    fn glide(&self, q: ParticleMoveAsk<'_>) -> ParticleMoveOutcome {
        let graph_shift = |graph: &[Vec3Spread]| {
            integrate_velocity_graph(graph, q.age_from, q.age_to, q.lifespan_ms, self.seed)
        };
        let (frame_origin, frame_axis) = (q.orient.origin, q.orient.axis);

        let mut local_pos = q.anchor;
        if elem_uses_vel_local(self.flags) && self.local_graph.len() >= 2 {
            local_pos = add3(local_pos, graph_shift(self.local_graph));
        }
        let mut world_pos = orientation_pos_to_world(frame_origin, frame_axis, local_pos);
        if elem_uses_vel_world(self.flags) && self.world_graph.len() >= 2 {
            world_pos = add3(world_pos, graph_shift(self.world_graph));
        }

        let mut motion_vec = q.launch_speed_vec;
        if elem_update_has_velocity_graph(self.flags) {
            let secs = q.step_sec;
            let pull = elem_gravity_accel_z_sampled(self.gravity.0, self.gravity.1, self.seed);
            world_pos = add3(world_pos, scale3(motion_vec, secs));
            world_pos[2] -= pull * secs * secs * 0.5;
            motion_vec[2] -= pull * secs;
            local_pos = orientation_pos_from_world(frame_origin, frame_axis, world_pos);
        }

        let mut moved = Sweep::from_query(&q);
        moved.travelled = sub3(local_pos, q.anchor);
        moved.launch_speed_vec = motion_vec;
        moved.into_result()
    }
}

/// Motion for a part that does not collide; `None` when its flags say it
/// does (use [`compute_bouncing_particle_path`] for those).
pub fn compute_particle_path(
    flags: i32,
    fall_pull_floor: f32,
    fall_pull_spread: f32,
    seed_bits: u32,
    vel_graph_local: &[Vec3Spread],
    vel_graph_world: &[Vec3Spread],
    q: ParticleMoveAsk<'_>,
) -> Option<ParticleMoveOutcome> {
    if elem_uses_collision(flags) {
        return None;
    }
    let flight = Flight {
        flags,
        gravity: (fall_pull_floor, fall_pull_spread),
        seed: seed_bits,
        local_graph: vel_graph_local,
        world_graph: vel_graph_world,
    };
    Some(flight.glide(q))
}

/// Where a collision sweep stands between bounces.
struct Sweep {
    spot: [f32; 3],
    launch_speed_vec: [f32; 3],
    travelled: [f32; 3],
    removed: bool,
    impact: Option<ContactBirth>,
    rest: Option<u8>,
}

impl Sweep {
    /// A part where the query has it, not yet moved, alive and in flight.
    fn from_query(q: &ParticleMoveAsk<'_>) -> Self {
        let (pos, launch_speed_vec) = (q.anchor, q.launch_speed_vec);
        Self { spot: pos, launch_speed_vec, travelled: [0.0; 3], removed: false, impact: None, rest: None }
    }

    fn settled(&self) -> bool {
        self.removed || self.rest.is_some()
    }

    fn into_result(self) -> ParticleMoveOutcome {
        ParticleMoveOutcome {
            position_shift: self.travelled,
            launch_speed_vec: self.launch_speed_vec,
            died_on_touch: self.removed,
            settled_share: self.rest,
            birth_on_contact: self.impact,
        }
    }
}

/// Motion for any part. A part that collides is swept in sub-steps; `trace`
/// is asked to move a box from one world point to another and report what it
/// hit. A part already at rest stays put.
pub fn compute_bouncing_particle_path(
    flags: i32,
    fall_pull_floor: f32,
    fall_pull_spread: f32,
    bounce_base: f32,
    bounce_amp: f32,
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    collide_with_pickups: bool,
    spawns_on_impact: bool,
    seed_bits: u32,
    local_vel_curve: &[Vec3Spread],
    world_vel_curve: &[Vec3Spread],
    q: ParticleMoveAsk<'_>,
    mut cast_ray: impl FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> ParticleRayContact,
) -> Option<ParticleMoveOutcome> {
    let flight = Flight {
        flags,
        gravity: (fall_pull_floor, fall_pull_spread),
        seed: seed_bits,
        local_graph: local_vel_curve,
        world_graph: world_vel_curve,
    };
    if !elem_uses_collision(flags) {
        return Some(flight.glide(q));
    }

    let mut sweep = Sweep::from_query(&q);
    // Already resting: it keeps its place, velocity and rest point.
    match q.settled_share {
        PARTICLE_NEVER_SETTLED => {}
        resting => {
            sweep.rest = Some(resting);
            return Some(sweep.into_result());
        }
    }

    let clip_mask = trace_mask(collide_with_pickups);
    // A life under a millisecond is treated as one millisecond.
    let lifespan_ms = if q.lifespan_ms >= 1.0 { q.lifespan_ms } else { 1.0 };
    let (frame_origin, frame_axis) = (q.orient.origin, q.orient.axis);
    let to_world = |p: [f32; 3]| orientation_pos_to_world(frame_origin, frame_axis, p);
    let to_local = |p: [f32; 3]| orientation_pos_from_world(frame_origin, frame_axis, p);

    for substep in collide_substep_schedule(q.last_ms, q.clock_ms_now) {
        let mut clock = substep.window_begin_ms;
        let leg_end = substep.window_close_ms;

        for _ in 0..BOUNCES_PER_SUBSTEP {
            if clock >= leg_end || sweep.settled() {
                break;
            }
            // Fly freely from the clock to the end of the sub-step, then see
            // how far the world lets it get.
            let open_air = flight.glide(ParticleMoveAsk {
                age_from: ms_between(q.window_open_ms, clock) / lifespan_ms,
                age_to: ms_between(q.window_open_ms, leg_end) / lifespan_ms,
                lifespan_ms,
                step_sec: ms_between(clock, leg_end) * 0.001,
                launch_speed_vec: sweep.launch_speed_vec,
                part_seed: seed_bits,
                anchor: sweep.spot,
                last_ms: clock,
                clock_ms_now: leg_end,
                ..q
            });
            let from_local = sweep.spot;
            let stop_local = add3(from_local, open_air.position_shift);
            let (from_w, to_w) = (to_world(from_local), to_world(stop_local));
            let probe = cast_ray(from_w, to_w, box_lo, box_hi, clip_mask);

            if probe.startsolid || probe.fully_embedded {
                sweep.launch_speed_vec = [0.0; 3];
                break;
            }
            if probe.hit_frac >= 1.0 {
                sweep.travelled = add3(sweep.travelled, open_air.position_shift);
                sweep.spot = stop_local;
                sweep.launch_speed_vec = open_air.launch_speed_vec;
                break;
            }

            // Stopped short: stand where it hit.
            let touch_w = blend_triple(from_w, to_w, probe.hit_frac);
            let touch_local = to_local(touch_w);
            sweep.travelled = add3(sweep.travelled, sub3(touch_local, from_local));
            sweep.spot = touch_local;

            let leg_ms = ms_between(clock, leg_end);
            let age_ms_at_touch = ms_between(q.window_open_ms, clock) + leg_ms * probe.hit_frac;
            let incoming = get_velocity_at_time(
                flags,
                sweep.launch_speed_vec,
                age_ms_at_touch,
                lifespan_ms,
                local_vel_curve,
                world_vel_curve,
                q.fx_basis,
                seed_bits,
            );
            if spawns_on_impact && impact_child_speed_allows(vec3_length_sq(incoming)) {
                sweep.impact = Some(ContactBirth {
                    spot: touch_w,
                    vel_before: incoming,
                });
            }

            if elem_dies_on_touch(flags) {
                sweep.removed = true;
                break;
            }

            let restitution = sample_reflection_factor(bounce_base, bounce_amp, seed_bits);
            let springback = scale3(incoming, restitution);
            // Touching the surface at the very start of the move with too
            // little rebound left to leave it means it has come to rest.
            if probe.hit_frac <= 0.0 && collide_marks_at_rest(springback, probe.facing[2]) {
                let share = get_at_rest_fraction(clock as f32, q.window_open_ms as f32, 1.0 / lifespan_ms);
                sweep.rest = Some(share as u8);
                sweep.launch_speed_vec = [0.0; 3];
                break;
            }

            sweep.launch_speed_vec = add3(
                sweep.launch_speed_vec,
                collision_reflect_base_vel_delta(incoming, probe.facing, restitution),
            );

            // Carry on from the hit point; always make some progress.
            clock += (leg_ms * probe.hit_frac) as i32;
            if clock <= substep.window_begin_ms {
                clock = substep.window_begin_ms.saturating_add(1);
            }
        }
        if sweep.settled() {
            break;
        }
    }

    Some(sweep.into_result())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UNTURNED_BASIS;
    use fx_t6::runtime::EffectFacing;


    fn colliding_flags() -> i32 {
        (0..31)
            .map(|bit| 1i32 << bit)
            .find(|&f| elem_uses_collision(f))
            .expect("some flag bit means collision")
    }

    fn still_query() -> ParticleMoveAsk<'static> {
        ParticleMoveAsk {
            definition_name: "",
            ledger_index: 0,
            definition_slot: 0,
            age_from: 0.0,
            age_to: 0.1,
            lifespan_ms: 1000.0,
            step_sec: 0.1,
            launch_speed_vec: [0.0; 3],
            part_seed: 1,
            anchor: [0.0; 3],
            last_ms: 0,
            clock_ms_now: 100,
            window_open_ms: 0,
            fx_basis: UNTURNED_BASIS,
            orient: EffectFacing {
                origin: [0.0; 3],
                axis: UNTURNED_BASIS,
            },
            settled_share: PARTICLE_NEVER_SETTLED,
        }
    }

    #[test]
    fn vector_helpers_work_per_component() {
        assert_eq!(add3([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), [5.0, 7.0, 9.0]);
        assert_eq!(sub3([4.0, 5.0, 6.0], [1.0, 2.0, 3.0]), [3.0, 3.0, 3.0]);
        assert_eq!(scale3([1.0, -2.0, 0.5], 2.0), [2.0, -4.0, 1.0]);
        assert_eq!(blend_triple([0.0; 3], [10.0, 20.0, 30.0], 0.5), [5.0, 10.0, 15.0]);
    }

    #[test]
    fn time_spans_never_run_backwards() {
        assert_eq!(ms_between(100, 250), 150.0);
        assert_eq!(ms_between(250, 100), 0.0);
    }

    #[test]
    fn a_colliding_part_is_not_a_free_one() {
        let verdict = compute_particle_path(colliding_flags(), 0.0, 0.0, 1, &[], &[], still_query());
        assert!(verdict.is_none());
    }

    #[test]
    fn a_free_part_without_graphs_or_gravity_stays_put() {
        let moved = compute_particle_path(0, 0.0, 0.0, 1, &[], &[], still_query())
            .expect("a part with no collision flag moves freely");
        assert_eq!(moved.position_shift, [0.0; 3]);
        assert!(!moved.died_on_touch && moved.birth_on_contact.is_none());
    }

    #[test]
    fn a_part_already_at_rest_never_calls_the_tracer() {
        let resting = ParticleMoveAsk {
            settled_share: 12,
            ..still_query()
        };
        let moved = compute_bouncing_particle_path(
            colliding_flags(),
            0.0,
            0.0,
            0.0,
            0.0,
            [0.0; 3],
            [0.0; 3],
            false,
            false,
            1,
            &[],
            &[],
            resting,
            |_, _, _, _, _| panic!("a resting part is not traced"),
        )
        .expect("collision motion always answers");
        assert_eq!(moved.settled_share, Some(12));
        assert_eq!(moved.position_shift, [0.0; 3]);
    }
}
