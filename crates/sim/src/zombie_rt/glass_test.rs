//! bo2mp: BO2MP_T6_GLASSTEST, the glass test (a headless check of every way
//! a player breaks a window). Each step is `how id cx cy cz nx ny nz dist`:
//! pane `id` with its centre and facing, and the player stands `dist` units
//! out along the facing (negative: the other side), looking at the centre.
//! `how` is gun (six shots), knife (two), walk (forward 2 s), jump (walk and
//! jump), dive (sprint, then prone within 90 units of the pane: the dolphin
//! dive) or frag (one thrown).
//! Steps split by `;` start at 30 s (past the prematch), 8 s apart. Every 250 ms the player's
//! place and the pane's state and health are logged.

use std::sync::OnceLock;

use bevy_ecs::prelude::World;
use playerstate_t6::game::buttons;

use super::autoplay::look;
use super::frame;
use crate::sim_parts::ClientId;

struct Step {
    how: String,
    pane: u32,
    centre: [f32; 3],
    facing: [f32; 3],
    dist: f32,
}

const FIRST_MS: i64 = 30_000;
const STEP_MS: i64 = 8_000;

fn steps() -> &'static [Step] {
    static STEPS: OnceLock<Vec<Step>> = OnceLock::new();
    STEPS.get_or_init(|| {
        let Ok(spec) = std::env::var("BO2MP_T6_GLASSTEST") else {
            return Vec::new();
        };
        spec.split(';')
            .filter_map(|part| {
                let mut words = part.split_whitespace();
                let how = words.next()?.to_owned();
                let n: Vec<f32> = words.filter_map(|w| w.parse().ok()).collect();
                (n.len() == 8).then(|| Step {
                    how,
                    pane: n[0] as u32,
                    centre: [n[1], n[2], n[3]],
                    facing: [n[4], n[5], n[6]],
                    dist: n[7],
                })
            })
            .collect()
    })
}

fn press(world: &mut World, client: ClientId, bits: u32, thrust: i8) {
    let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
    for (id, cmd) in &mut req.input.cmds {
        if *id == client {
            cmd.buttons |= bits;
            if thrust != 0 {
                cmd.thrust_ahead = thrust;
            }
        }
    }
}

pub(crate) fn glass_test(world: &mut World, now: i64) {
    let steps = steps();
    if steps.is_empty() || now < FIRST_MS {
        return;
    }
    let i = ((now - FIRST_MS) / STEP_MS) as usize;
    let Some(step) = steps.get(i) else {
        return;
    };
    let within = now - FIRST_MS - i as i64 * STEP_MS;
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let client = ClientId(0);
    let Some(ps) = frame(world).avatar(client).copied() else {
        return;
    };
    let c = step.centre;
    let n = step.facing;
    if within < tick {
        let healths = { frame(world).world_objects().t6_glass_healths() };
        diag::info!(Sim, "bo2mp glass test step {i}: {} at pane {} from {:.0} out; panes {healths:?}", step.how, step.pane, step.dist);
        // The floor along the pane's facing, -160 to 160 out: where he can stand on each side.
        let floors: Vec<(i32, i32)> = (-8..=8)
            .map(|k| {
                let out = k as f32 * 20.0;
                let top = [c[0] + n[0] * out, c[1] + n[1] * out, c[2] + 40.0];
                let down = [top[0], top[1], top[2] - 400.0];
                let t = frame(world).trace_static_world(
                    top,
                    down,
                    crate::shot_trace::BODY_BOX_LOWER,
                    crate::shot_trace::BODY_BOX_UPPER,
                    crate::shot_trace::PLAYER_BODY_MASK,
                );
                (out as i32, (top[2] - 400.0 * t.fraction) as i32)
            })
            .collect();
        diag::info!(Sim, "bo2mp glass test step {i}: floor (out, z) {floors:?}");
    }
    // Placed until 400 ms in; the moves (walk, jump, dive) then run on their own.
    let moving = matches!(step.how.as_str(), "walk" | "jump" | "dive");
    if within < 400 || !moving {
        let out = [c[0] + n[0] * step.dist, c[1] + n[1] * step.dist, c[2]];
        let down = [out[0], out[1], out[2] - 300.0];
        let t = frame(world).trace_static_world(
            out,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        let feet: [f32; 3] = std::array::from_fn(|k| out[k] + (down[k] - out[k]) * t.fraction);
        super::teleport_player(world, client, feet);
        let eye = [feet[0], feet[1], feet[2] + ps.eye_height_now];
        let view = if moving { [0.0, look(eye, c)[1], 0.0] } else { look(eye, c) };
        super::set_player_view(world, client, view);
    }
    match step.how.as_str() {
        "gun" if (500..2500).contains(&within) && within % 300 < tick => {
            {
                let mut f = frame(world);
                crate::client_script::set_ammo_clip(&mut f, client, ps.weapon, 8);
            }
            press(world, client, buttons::ATTACK, 0);
        }
        "knife" if (500..2500).contains(&within) && within % 1000 < tick => {
            press(world, client, weapon_t6::melee_trace::BUTTON_MELEE, 0);
        }
        "frag" if (500..700).contains(&within) => press(world, client, buttons::SAMPLE_FRAG_ASSET, 0),
        "walk" if (400..2400).contains(&within) => press(world, client, 0, 127),
        "jump" if (400..2400).contains(&within) => {
            let jump = if (700..800).contains(&within) { buttons::LEAP } else { 0 };
            press(world, client, jump, 127);
        }
        "dive" if (400..2400).contains(&within) => {
            // Prone once he has sprinted a while and is within a dive of the pane.
            let gap = ((ps.origin[0] - c[0]) * n[0] + (ps.origin[1] - c[1]) * n[1]).abs();
            let prone = if within >= 650 && gap < 90.0 { buttons::LYING_FLAT } else { 0 };
            press(world, client, buttons::SPRINT | prone, 127);
        }
        _ => {}
    }
    if within % 250 < tick && within < 6000 {
        let (health, state) = {
            let f = frame(world);
            let objects = f.world_objects();
            let health = objects.t6_glass_healths().into_iter().find(|h| h.0 == step.pane).map(|h| h.1);
            (health, objects.pane_condition(step.pane))
        };
        let o = ps.origin;
        diag::info!(
            Sim,
            "bo2mp glass test {i} {} at {within}ms: player ({:.0} {:.0} {:.0}) eye {:.0}, pane {} {state:?} health {health:?}; weapon {:?} flags {:#x} state {} shots {}",
            step.how,
            o[0],
            o[1],
            o[2],
            ps.eye_height_now,
            step.pane,
            ps.weapon,
            ps.weapon_state_bits,
            ps.weapon_state_t6,
            ps.shots_fired
        );
    }
}
