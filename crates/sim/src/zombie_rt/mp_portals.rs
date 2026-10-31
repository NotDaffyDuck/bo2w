//! bo2mp: the Portal Gun and the Super one in a match.
//!
//! Fire puts his blue portal on the wall or floor he aims at, aim (the
//! right button) his orange one; a bot (or the test player) alternates.
//! Once both are up, any player who walks, falls or is pushed into one
//! comes out of the other, his speed and view turned with it, as Minecraft
//! Zombies' `portalgun_play` does it (its numbers, in map units: a block is
//! 36). The Super Portal Gun also opens his portal under the enemy in his
//! sights, who drops through it at once if the other is up.

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use glam::{Mat3, Vec3};

use super::{Closest, Zm, frame, with_vm};
use crate::guest_arms::{self, Guest, PORTAL_HALF, GuestPortal};
use crate::sim_parts::ClientId;

/// How far the shot goes.
const RANGE: f32 = 4096.0;
/// A shot every half second (`REFIRE`).
const REFIRE_MS: i64 = 500;
/// Through one, not again for half a second (`COOL`).
const COOL_MS: i64 = 500;
/// MC Zombies' portal opens in a fifth of a second and takes no one before.
const OPEN_MS: i64 = 200;
/// A player's box (radius, height).
const RADIUS: f32 = 15.0;
const HEIGHT: f32 = 70.0;
/// Out of a wall at least this fast; out of a floor up and on (`OUT_*`).
const OUT_WALL: f32 = 120.0;
const OUT_UP: f32 = 250.0;
/// Minecraft's 3 blocks a second lands him past its smaller oval; ours is
/// 54 long, so on at 136 lands him about 85 out, clear of it.
const OUT_SIDE: f32 = 136.0;
/// The Super one's aim: the enemy within this cone of his look.
const CONE: f32 = 0.97;
/// The shot's reach for the oval to fit, nudged up to this many times.
/// How far an edge of the portal may sit off the centre's surface (a bumpy
/// lawn, a kerb's lip) and still hold it.
const BUMP: f32 = 10.0;
const NUDGE: f32 = 8.0;
const NUDGES: usize = 8;

/// A portal on a surface: its middle, the way out of the surface, and its
/// up (a wall portal's top; a floor portal's way ahead).
#[derive(Clone, Copy, Debug)]
struct Portal {
    centre: Vec3,
    normal: Vec3,
    up: Vec3,
    /// When it opened (it grows from a dot) and whether the Super gun shot it.
    born: i64,
    upgraded: bool,
}

impl Portal {
    fn right(&self) -> Vec3 {
        self.up.cross(self.normal).normalize_or(Vec3::X)
    }

    /// Its axes as columns: right, up, out.
    fn frame(&self) -> Mat3 {
        Mat3::from_cols(self.right(), self.up, self.normal)
    }

    fn wall(&self) -> bool {
        self.normal.z.abs() < 0.7
    }

    /// The point in its axes.
    fn local(&self, p: Vec3) -> Vec3 {
        let d = p - self.centre;
        Vec3::new(d.dot(self.right()), d.dot(self.up), d.dot(self.normal))
    }

    fn inside_oval(&self, l: Vec3) -> bool {
        (l.x / PORTAL_HALF[0]).powi(2) + (l.y / PORTAL_HALF[1]).powi(2) < 1.0
    }
}

/// From one portal to the other: in through `from`, out of `to` facing
/// away from it, left and right kept (`turn`).
fn turn(from: &Portal, to: &Portal) -> Mat3 {
    to.frame() * Mat3::from_diagonal(Vec3::new(-1.0, 1.0, -1.0)) * from.frame().transpose()
}

#[derive(Resource, Default)]
pub(super) struct McPortals {
    /// Each owner's blue and orange.
    portals: BTreeMap<u32, [Option<Portal>; 2]>,
    /// When each owner last shot, and the next colour a bot shoots.
    shot: BTreeMap<u32, (i64, bool)>,
    /// When each player last came through.
    cool: BTreeMap<u32, i64>,
    /// Whose fire and aim buttons were down last tick.
    buttons: BTreeMap<u32, (bool, bool)>,
    /// The floor portal each player last came up out of: not back into it
    /// until his feet are off its oval (he would loop on it).
    out_of: BTreeMap<u32, Vec3>,
}

fn state(world: &mut World) -> bevy_ecs::world::Mut<'_, McPortals> {
    if !world.contains_resource::<McPortals>() {
        world.insert_resource(McPortals::default());
    }
    world.resource_mut::<McPortals>()
}

fn trace(world: &mut World, a: Vec3, b: Vec3, mask: u32) -> trace_t6::Trace {
    super::shot_trace(world, a.to_array(), b.to_array(), mask)
}

/// His Portal Gun out: fire shoots blue, aim shoots orange; a bot (or the
/// test player) taps fire, and alternates. Returns the colour shot.
pub(super) fn hands(sim: &mut World, ch: u32, gun: Guest, btns: u32, tapper: bool, now_ms: i64) -> Option<bool> {
    use playerstate_t6::game::buttons::{ADS, ATTACK};
    let (fire, aim) = (btns & ATTACK != 0, btns & ADS != 0);
    let (fire_was, aim_was) = state(sim).buttons.get(&ch).copied().unwrap_or_default();
    state(sim).buttons.insert(ch, (fire, aim));
    let (last, next) = state(sim).shot.get(&ch).copied().unwrap_or((i64::MIN / 2, false));
    if now_ms - last < REFIRE_MS {
        return None;
    }
    let orange = if tapper {
        if !fire {
            return None;
        }
        next
    } else if fire && !fire_was {
        false
    } else if aim && !aim_was {
        true
    } else {
        return None;
    };
    state(sim).shot.insert(ch, (now_ms, !orange));
    shoot(sim, ch, orange, gun == Guest::SuperPortalGun, now_ms);
    Some(orange)
}

/// One shot: the portal on what he aims at (or, with the Super one, under
/// the enemy in his sights).
fn shoot(world: &mut World, c: u32, orange: bool, upgraded: bool, now: i64) {
    let Some(pstate) = frame(world).avatar(ClientId(c)).copied() else { return };
    let eye_at = Vec3::new(pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now);
    let look = Vec3::from_array(crate::shot_falloff::angles_to_forward(pstate.look_angles));
    let colour = if orange { "orange" } else { "blue" };
    let mut placed = None;
    if upgraded && let Some((victim, at)) = under_enemy(world, c, eye_at, look) {
        diag::info!(Sim, "bo2mp guest portal gun: client {c} opens his {colour} portal under client {victim}");
        placed = Some(at);
    }
    let mut why = String::new();
    let placed = placed.or_else(|| place(world, eye_at, look, &mut why));
    let Some(mut p) = placed else {
        diag::info!(Sim, "bo2mp guest portal gun: client {c}'s {colour} shot fizzles ({why})");
        return;
    };
    p.born = now;
    p.upgraded = upgraded;
    let mut s = state(world);
    let pair = s.portals.entry(c).or_default();
    let other = pair[usize::from(!orange)];
    if other.is_some_and(|o| o.normal.dot(p.normal) > 0.9 && (o.centre - p.centre).length() < PORTAL_HALF[1] * 1.5) {
        diag::info!(Sim, "bo2mp guest portal gun: client {c}'s {colour} shot lands on his other portal and fizzles");
        return;
    }
    pair[usize::from(orange)] = Some(p);
    let linked = other.is_some();
    diag::info!(
        Sim,
        "bo2mp guest portal gun: client {c} puts his {colour} portal on a {} at ({:.0} {:.0} {:.0}){}",
        if p.wall() { "wall" } else if p.normal.z > 0.0 { "floor" } else { "ceiling" },
        p.centre.x,
        p.centre.y,
        p.centre.z,
        if linked { ", linked" } else { "" }
    );
}

/// A portal where the shot lands, if the oval fits there: every point
/// round its edge on the same flat surface, nudged off an edge it overhangs.
fn place(world: &mut World, eye: Vec3, look: Vec3, why: &mut String) -> Option<Portal> {
    use crate::shot_trace::MASK_SHOT;
    let tr = trace(world, eye, eye + look * RANGE, MASK_SHOT);
    if tr.fraction >= 1.0 || tr.starts_inside || tr.fully_embedded {
        *why = "nothing in range".into();
        return None;
    }
    let normal = Vec3::from_array(tr.normal).normalize_or_zero();
    if normal == Vec3::ZERO {
        *why = "no surface".into();
        return None;
    }
    let up = if normal.z.abs() < 0.7 {
        (Vec3::Z - normal * normal.z).normalize_or_zero()
    } else {
        // A floor's (or ceiling's) way ahead is his look.
        (look - normal * look.dot(normal)).normalize_or(Vec3::X)
    };
    let mut portal = Portal { centre: eye.lerp(eye + look * RANGE, tr.fraction) + normal, normal, up, born: 0, upgraded: false };
    for _ in 0..NUDGES {
        let right = portal.right();
        let mut push = Vec3::ZERO;
        let mut missed = 0;
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let off = right * a.cos() * PORTAL_HALF[0] * 0.9 + up * a.sin() * PORTAL_HALF[1] * 0.9;
            let p = portal.centre + off;
            // Minecraft's blocks are flat; a BO2 lawn or path is not, so the
            // edge may sit a few units above or below the centre's surface.
            let back = trace(world, p + normal * BUMP, p - normal * BUMP, MASK_SHOT);
            let flat = back.fraction < 1.0 && !back.starts_inside && Vec3::from_array(back.normal).dot(normal) > 0.8;
            let clear = trace(world, portal.centre + normal * BUMP, p + normal * BUMP, MASK_SHOT).fraction >= 1.0;
            if !(flat && clear) {
                missed = 1 + missed;
                push -= off.normalize_or_zero();
            }
        }
        if missed == 0 {
            return Some(portal);
        }
        // Edges missed on both sides at once: the surface is too small for it.
        if push.length() < 0.5 {
            *why = format!("too small a surface at ({:.0} {:.0} {:.0}) facing ({:.2} {:.2} {:.2}), {missed} of 8 edge points off", portal.centre.x, portal.centre.y, portal.centre.z, normal.x, normal.y, normal.z);
            return None;
        }
        portal.centre += push.normalize_or_zero() * NUDGE;
    }
    *why = format!("off the edge at ({:.0} {:.0} {:.0}) facing ({:.2} {:.2} {:.2})", portal.centre.x, portal.centre.y, portal.centre.z, normal.x, normal.y, normal.z);
    None
}

/// The Super one: the enemy nearest his look within the cone, in sight, on
/// the ground: a floor portal under his feet, its way ahead away from the
/// shooter.
fn under_enemy(world: &mut World, c: u32, eye: Vec3, look: Vec3) -> Option<(u32, Portal)> {
    use crate::shot_trace::MASK_SHOT;
    let enemies = with_vm(world, |vm, world| super::bots::enemies_of(vm, world, c, true)).unwrap_or_default();
    let mut pick = Closest::new();
    for e in enemies {
        let Some(ps) = frame(world).avatar(ClientId(e)).copied() else { continue };
        let feet = Vec3::from_array(ps.origin);
        let chest = feet + Vec3::Z * 40.0;
        let dot = (chest - eye).normalize_or_zero().dot(look);
        if dot < CONE || !pick.beaten_by(-dot) {
            continue;
        }
        if trace(world, eye, chest, MASK_SHOT).fraction < 1.0 {
            continue;
        }
        pick.offer(-dot, (e, feet));
    }
    let (victim, feet) = pick.into_item()?;
    let tr = trace(world, feet + Vec3::Z * 8.0, feet - Vec3::Z * 64.0, MASK_SHOT);
    let normal = Vec3::from_array(tr.normal);
    if tr.fraction >= 1.0 || tr.starts_inside || normal.z < 0.7 {
        return None;
    }
    let away = Vec3::new(feet.x - eye.x, feet.y - eye.y, 0.0);
    let up = (away - normal * away.dot(normal)).normalize_or(Vec3::X);
    Some((victim, Portal { centre: (feet + Vec3::Z * 8.0).lerp(feet - Vec3::Z * 64.0, tr.fraction) + normal, normal, up, born: 0, upgraded: false }))
}

/// Each tick: everyone who goes into a linked portal comes out of the
/// other; the portals are shown.
pub(super) fn tick(world: &mut World, now: i64) {
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    // A player gone takes his portals with him.
    state(world).portals.retain(|owner, _| players.contains(owner));
    let pairs: Vec<[Portal; 2]> = state(world)
        .portals
        .values()
        .filter_map(|p| Some([p[0]?, p[1]?]))
        .filter(|[a, b]| now - a.born >= OPEN_MS && now - b.born >= OPEN_MS)
        .collect();
    if !pairs.is_empty() {
        for c in players {
            if state(world).cool.get(&c).is_some_and(|t| now - t < COOL_MS) {
                continue;
            }
            let playing = world.resource::<Zm>().players.get(&c).is_some_and(|p| p.sessionstate == "playing");
            if !playing || !frame(world).avatar(ClientId(c)).is_some_and(|ps| ps.health > 0) {
                continue;
            }
            if let Some(centre) = state(world).out_of.get(&c).copied() {
                let feet = frame(world).avatar(ClientId(c)).map(|ps| Vec3::from_array(ps.origin));
                let over = pairs
                    .iter()
                    .flatten()
                    .find(|p| p.centre == centre)
                    .zip(feet)
                    .is_some_and(|(p, f)| p.inside_oval(p.local(f)));
                if !over {
                    state(world).out_of.remove(&c);
                }
            }
            let came_from = state(world).out_of.get(&c).copied();
            'pairs: for pair in &pairs {
                for k in 0..2 {
                    if came_from == Some(pair[k].centre) {
                        continue;
                    }
                    if went_in(world, c, &pair[k]) {
                        if come_out(world, c, &pair[k], &pair[1 - k], k == 1) {
                            state(world).cool.insert(c, now);
                        }
                        break 'pairs;
                    }
                }
            }
        }
    }
    let mut shown = Vec::new();
    for (&owner, pair) in &state(world).portals {
        let linked = pair[0].is_some() && pair[1].is_some();
        for (k, p) in pair.iter().enumerate() {
            if let Some(p) = p {
                shown.push(GuestPortal {
                    centre: p.centre.to_array(),
                    face_dir: p.normal.to_array(),
                    up_dir: p.up.to_array(),
                    orange: k == 1,
                    paired: linked,
                    age: (now - p.born) as f32 / 1000.0,
                    upgraded: p.upgraded,
                    pair: owner,
                });
            }
        }
    }
    guest_arms::set_portals(shown);
}

/// Is he going into this portal (`went_in`): against a wall portal and
/// moving or pushing into it, or standing or falling on a floor one.
fn went_in(game: &mut World, ch: u32, p: &Portal) -> bool {
    let Some(pstate) = frame(game).avatar(ClientId(ch)).copied() else { return false };
    let feet = Vec3::from_array(pstate.origin);
    let v = Vec3::from_array(pstate.velocity);
    if p.wall() {
        let l = p.local(feet);
        let touching = l.z > -8.0 && l.z < RADIUS + 9.0;
        let across = l.x.abs() < PORTAL_HALF[0];
        let tall = l.y > -PORTAL_HALF[1] - 10.0 && l.y < PORTAL_HALF[1] - 40.0;
        let wish = wish_of(game, ch, pstate.look_angles[1]);
        let into = v.dot(p.normal) < -10.0 || wish.dot(p.normal) < -0.5;
        touching && across && tall && into
    } else if p.normal.z > 0.0 {
        let l = p.local(feet);
        l.z > -2.0 && l.z < 10.0 && p.inside_oval(l) && v.dot(p.normal) < 18.0
    } else {
        let l = p.local(feet + Vec3::Z * HEIGHT);
        l.z > -2.0 && l.z < 10.0 && p.inside_oval(l) && v.dot(p.normal) < 18.0
    }
}

/// Where his stick points, flat (his last move's forward and side).
fn wish_of(world: &mut World, c: u32, yaw: f32) -> Vec3 {
    let Some([forward, right]) = frame(world).old_cmd_move(ClientId(c)) else { return Vec3::ZERO };
    let y = yaw.to_radians();
    let ahead = Vec3::new(y.cos(), y.sin(), 0.0);
    let side = Vec3::new(y.sin(), -y.cos(), 0.0);
    (ahead * f32::from(forward) + side * f32::from(right)).normalize_or_zero()
}

/// Out of `to` (`come_out`): his place, speed and look turned through.
fn come_out(sim: &mut World, c: u32, from: &Portal, to: &Portal, from_orange: bool) -> bool {
    use crate::shot_trace::PLAYER_BODY_MASK;
    let Some(ps) = frame(sim).avatar(ClientId(c)).copied() else { return false };
    let r = turn(from, to);
    let feet = Vec3::from_array(ps.origin);
    let l = from.local(feet);
    let mut v = r * Vec3::from_array(ps.velocity);
    let out_speed = v.dot(to.normal);
    let mut spot = if to.wall() {
        // His side-to-side kept (mirrored), his feet near the oval's bottom.
        let x = (-l.x).clamp(-PORTAL_HALF[0] + RADIUS, PORTAL_HALF[0] - RADIUS);
        if out_speed < OUT_WALL {
            v += to.normal * (OUT_WALL - out_speed);
        }
        to.centre + to.normal * (RADIUS + 4.0) + to.right() * x - Vec3::Z * (PORTAL_HALF[1] - 2.0)
    } else if to.normal.z > 0.0 {
        if out_speed < OUT_UP {
            v += to.normal * (OUT_UP - out_speed);
        }
        // On along the way he was already going (Minecraft's), or its way ahead.
        let side = v - to.normal * v.dot(to.normal);
        if side.length() < OUT_SIDE {
            v += side.try_normalize().unwrap_or(to.up) * (OUT_SIDE - side.length());
        }
        to.centre + to.normal * 2.0
    } else {
        to.centre + to.normal * (HEIGHT + 2.0)
    };
    // Room for him there: raised a step at a time if not.
    let room = |sim: &mut World, at: Vec3| {
        let tt = frame(sim).trace_world(at.to_array(), at.to_array(), [-RADIUS, -RADIUS, 0.0], [RADIUS, RADIUS, HEIGHT], PLAYER_BODY_MASK);
        !tt.starts_inside && !tt.fully_embedded
    };
    let mut fits = room(sim, spot);
    for _step in 0..4 {
        if fits {
            break;
        }
        spot += Vec3::Z * 9.0;
        fits = room(sim, spot);
    }
    if !fits {
        return false;
    }
    let look = r * Vec3::from_array(crate::shot_falloff::angles_to_forward(ps.look_angles));
    let pitch = (-look.z.clamp(-1.0, 1.0).asin().to_degrees()).clamp(-85.0, 85.0);
    let yaw = look.y.atan2(look.x).to_degrees();
    let id = ClientId(c);
    super::teleport_player(sim, id, spot.to_array());
    if let Some(ps) = frame(sim).occupant_mut(id) {
        ps.velocity = v.to_array();
        ps.standing_on = playerstate_t6::game::NO_ENTITY_SLOT;
    }
    super::set_player_view(sim, id, [pitch, yaw, 0.0]);
    if to.normal.z > 0.7 {
        state(sim).out_of.insert(c, to.centre);
    } else {
        state(sim).out_of.remove(&c);
    }
    let (a, b) = if from_orange { ("orange", "blue") } else { ("blue", "orange") };
    diag::info!(
        Sim,
        "bo2mp guest portal gun: client {c} goes in the {a} portal, out the {b} at ({:.0} {:.0} {:.0}) at {:.0} u/s",
        spot.x,
        spot.y,
        spot.z,
        v.length()
    );
    true
}
