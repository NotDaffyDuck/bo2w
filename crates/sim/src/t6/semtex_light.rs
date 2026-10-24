//! bo2mp gun brain, window 5 (gun/offhand): the semtex's blinking light and beep. BO2 does this
//! in its client script for the sticky grenade, from the moment it is thrown until it goes off:
//! each blink takes the last light off its `tag_fx`, shows a new one (green to his side, red to
//! the other) and plays the alert beep on it, then waits. The wait starts at 0.3 s and each one
//! is the last divided by 1.2, never below 0.08 s, so the blinks speed up towards the blast.
//! His side is his team in a team game, else only himself.
//!
//! The crossbow's stuck explosive bolt blinks and beeps the same way (BO2's client script for
//! the explosive bolt), its light on its `tag_origin`.
//!
//! Not done here: the buzz and the "stuck" pulse on the screen of an enemy it sticks to.

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};

use super::{Zm, killstreaks, natives_fx, weapon_text, with_vm};
use crate::EventAudience;
use crate::world::ClientId;

const SEMTEX: &str = "sticky_grenade_mp";
const TAG: &str = "tag_fx";
/// The grenade a stuck crossbow bolt leaves, and where its light goes.
const BOLT: &str = "explosive_bolt_mp";
const BOLT_TAG: &str = "tag_origin";
const FRIEND_FX: &str = "weapon/crossbow/fx_trail_crossbow_blink_grn_os";
const ENEMY_FX: &str = "weapon/crossbow/fx_trail_crossbow_blink_red_os";
const BEEP: &str = "wpn_semtex_alert";
/// BO2's waits between blinks: the first, the shortest, and the divisor each takes.
const FIRST_MS: f32 = 300.0;
const SHORTEST_MS: f32 = 80.0;
const SPEEDUP: f32 = 1.2;

#[derive(Resource, Default)]
pub(crate) struct SemtexLights {
    /// Each live semtex (projectile id): its next blink and the wait after it.
    live: BTreeMap<u32, Blink>,
}

#[derive(Clone, Copy)]
struct Blink {
    due_ms: i64,
    wait_ms: f32,
    /// Whether a light is on it to take off first.
    lit: bool,
}

pub(super) fn advance(world: &mut World, now: i64) {
    if !world.resource::<Zm>().mp {
        return;
    }
    let semtex: Vec<(u32, u32, [f32; 3], &str)> = crate::frame::collect_projectiles(world)
        .into_iter()
        .filter(|p| p.live)
        .filter_map(|p| {
            let name = weapon_text(world, p.weapon);
            let tag = if name == SEMTEX || name.starts_with("sticky_grenade_mp+") {
                TAG
            } else if name == BOLT {
                BOLT_TAG
            } else {
                return None;
            };
            Some((p.id.0, p.owner.0, p.origin, tag))
        })
        .collect();
    if semtex.is_empty() && world.get_resource::<SemtexLights>().is_none_or(|s| s.live.is_empty()) {
        return;
    }
    if world.get_resource::<SemtexLights>().is_none() {
        world.insert_resource(SemtexLights::default());
    }
    let mut live = std::mem::take(&mut world.resource_mut::<SemtexLights>().live);
    live.retain(|id, _| semtex.iter().any(|(s, ..)| s == id));
    for (id, owner, origin, tag) in semtex {
        let mut b = *live.entry(id).or_insert(Blink { due_ms: now, wait_ms: FIRST_MS, lit: false });
        if now >= b.due_ms {
            // The scripts' stand-in for it (made the tick it was thrown).
            if let Some(ent) = killstreaks::thrown_ent(world, id) {
                let (friends, foes) = sides(world, owner);
                for (fx, to) in [(FRIEND_FX, friends), (ENEMY_FX, foes)] {
                    if b.lit {
                        natives_fx::bolt_effect_to(world, to.clone(), ent, fx, tag, true);
                    }
                    natives_fx::bolt_effect_to(world, to, ent, fx, tag, false);
                }
                b.lit = true;
                natives_fx::sound(world, EventAudience::All, BEEP, origin);
                if std::env::var_os("BO2MP_SEMTEXLOG").is_some() {
                    diag::info!(Sim, "bo2mp semtex blink {id} (ent {ent}) at {now} next in {:.0} ms", b.wait_ms);
                }
                b.due_ms = now + b.wait_ms.round() as i64;
                b.wait_ms = (b.wait_ms / SPEEDUP).clamp(SHORTEST_MS, FIRST_MS);
            }
        }
        live.insert(id, b);
    }
    world.resource_mut::<SemtexLights>().live = live;
}

/// Who sees the green light (his side) and who the red: his team in a team game, else himself.
fn sides(world: &mut World, owner: u32) -> (EventAudience, EventAudience) {
    let players: Vec<(u32, gsc_t6::ObjRef)> =
        world.resource::<Zm>().players.iter().map(|(n, p)| (*n, p.obj)).collect();
    let teams = with_vm(world, |vm, _| {
        let (team_s, tb) = (vm.intern("team"), vm.intern("teambased"));
        let teambased = gsc_t6::truthy(&vm.raw_field(vm.level, tb));
        let team: BTreeMap<u32, String> =
            players.iter().map(|(n, o)| (*n, vm.to_text(&vm.raw_field(*o, team_s)))).collect();
        (teambased, team)
    });
    let (teambased, team) = teams.unwrap_or_default();
    let his = team.get(&owner).cloned().unwrap_or_default();
    let (mut friends, mut foes) = (Vec::new(), Vec::new());
    for (n, _) in &players {
        let friend = *n == owner || (teambased && !his.is_empty() && team.get(n) == Some(&his));
        if friend { &mut friends } else { &mut foes }.push(ClientId(*n));
    }
    (EventAudience::Clients(friends), EventAudience::Clients(foes))
}
