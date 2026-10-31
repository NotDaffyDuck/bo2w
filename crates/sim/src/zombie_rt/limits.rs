//! What the scripts allow a player's body, as BO2's engine keeps it.
//! TranZit's bus takes sprint and prone from its riders, a zombie clinging
//! to the bus takes crouch, a Denizen on his head halves his speed, and a
//! death in the lava or on the moving bus leaves him lying down. The
//! switches (`allowsprint`, `allowprone`, `allowcrouch`, `allowstand`,
//! `allowmelee`, `allowads`) are the player's `ScriptControls`
//! (t6::natives_player; they reach his PC too, so his own guess of his moves
//! agrees), and `script_player::constrain_cmd` lets go of the taken moves
//! with BO2's Zombies stance rule. Here: `setmovespeedscale`, and a sprint
//! already going ends when sprint is taken.

use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};
use playerstate_t6::game::buttons;
use playerstate_t6::game::movement_state_bits::SPRINTING;

use super::{Zm, arg, client, frame};
use crate::sim_parts::ClientId;

/// His switches (all allowed by default and after a spawn).
fn controls(world: &mut World, c: u32) -> crate::player_ledger::ScriptControls {
    frame(world)
        .client_meta(ClientId(c))
        .map(|m| m.controls)
        .unwrap_or_default()
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // player setmovespeedscale(scale): his run speed times scale.
    vm.bind("setmovespeedscale", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let scale = arg(a, 0).as_float().unwrap_or(1.0).max(0.0);
        if let Some(ps) = frame(world).occupant_mut(id) {
            ps.run_speed_modifier = scale;
        }
        Ok(ScriptValue::Undefined)
    });
}

/// Each tick, before the move step: a sprint already going ends once
/// sprint is taken (letting go of sprint does not end one; the move step's
/// constrain_cmd lets go of the taken buttons).
pub(super) fn tick(world: &mut World) {
    press_test(world);
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let no_sprint: Vec<u32> = clients
        .into_iter()
        .filter(|c| controls(world, *c).sprint_disabled)
        .collect();
    let mut f = frame(world);
    for c in no_sprint {
        if let Some(ps) = f.occupant_mut(ClientId(c)) {
            if ps.movement_state_bits & SPRINTING != 0 {
                ps.movement_state_bits &= !SPRINTING;
                ps.sprint_state.sprint_wait = 0;
                ps.sprint_state.sprint_stop_ms = ps.last_cmd_ms;
            }
        }
    }
}

/// BO2MP_T6_PRESS="sprint+forward@30-33,prone@34-37,melee@50-50.3,..."
/// (seconds): player 0 holds those buttons then (tests that the bus takes
/// sprint and prone from its riders, and that melee taps knock a Denizen
/// off his head); once a second it logs what he kept and how he stands.
/// `use` holds the use key (TranZit's navcard pickup and navcomputer),
/// `attack` the fire key (a Jet Gun spun at the navcomputer's tower).
fn press_test(world: &mut World) {
    static PRESS: std::sync::OnceLock<Option<Vec<(u32, bool, i64, i64)>>> = std::sync::OnceLock::new();
    let Some(press) = PRESS.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_PRESS").ok()?;
        let mut out = Vec::new();
        for part in v.split(',') {
            let Some((names, when)) = part.split_once('@') else {
                continue;
            };
            let Some((a, z)) = when.split_once('-') else {
                continue;
            };
            let (Ok(a), Ok(z)) = (a.trim().parse::<f32>(), z.trim().parse::<f32>()) else {
                continue;
            };
            let mut b = 0;
            let mut forward = false;
            for n in names.split('+') {
                match n.trim() {
                    "sprint" => b |= buttons::SPRINT,
                    "prone" => b |= buttons::LYING_FLAT,
                    "crouch" => b |= buttons::DUCK_BUTTON_BIT,
                    "melee" => b |= buttons::MELEE_CHARGE,
                    "jump" => b |= buttons::LEAP,
                    "use" => b |= buttons::INTERACT_KEY,
                    "attack" => b |= buttons::ATTACK,
                    "frag" => b |= buttons::SAMPLE_FRAG_ASSET,
                    "smoke" => b |= buttons::OBSCURANT,
                    "forward" => forward = true,
                    _ => {}
                }
            }
            out.push((b, forward, (a * 1000.0) as i64, (z * 1000.0) as i64));
        }
        Some(out)
    }) else {
        return;
    };
    let now = world.resource::<Zm>().clock_millis;
    let Some(&(b, forward, ..)) = press.iter().find(|p| now >= p.2 && now < p.3) else {
        return;
    };
    let id = ClientId(0);
    {
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        for (who, cmd) in &mut req.input.cmds {
            if *who == id {
                cmd.buttons |= b;
                if forward {
                    cmd.thrust_ahead = 127;
                }
            }
        }
    }
    if now % 1000 >= i64::from(crate::SIM_STEP_MILLIS) {
        return;
    }
    let l = controls(world, id.0);
    if let Some(ps) = frame(world).avatar(id) {
        use playerstate_t6::game::movement_state_bits;
        // What the move step keeps of his press (BO2's Zombies stance rule,
        // then the taken moves let go).
        let mut c = playerstate_t6::game::InputCommand {
            buttons: b,
            ..Default::default()
        };
        crate::client_script::bo2_zombies_stance(&l, ps.movement_state_bits, &mut c);
        crate::client_script::apply_allow_controls(&l, ps.movement_state_bits, &mut c);
        let kept = c.buttons;
        diag::info!(
            Sim,
            "bo2zm t6 limits: player 0 pressed {b:#x} kept {kept:#x} (no sprint {}, no prone {}): sprinting {}, prone {}, crouched {}",
            l.sprint_disabled,
            l.prone_disabled,
            ps.movement_state_bits & SPRINTING != 0,
            ps.movement_state_bits & movement_state_bits::LYING_FLAT != 0,
            ps.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT != 0
        );
    }
}
