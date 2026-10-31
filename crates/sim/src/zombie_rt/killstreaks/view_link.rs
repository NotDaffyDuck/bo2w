//! A player's view on another entity's tag (`player
//! playerlinkweaponviewtodelta(ent, tag, fraction, right, left, up, down)`):
//! the Lodestar (_remotemortar.gsc) links his view to its drone's
//! `tag_player`. His body stays where he stood (BO2's script wants him on
//! the ground first); his view sits at the tag and turns with it as the
//! drone circles, kept to the arcs the script gives around the tag's own
//! facing. `player unlink()` ends it, and so does his spawning (BO2's
//! `ClientSpawn` lets go). Dying does not: a pilot shot where he stands
//! keeps flying (BO2's `player_die` stops short for a view-only link; the
//! script holds his respawn until the Lodestar is done).
//!
//! His screen rides the tag (the client's vehicle view,
//! `bo2mp_vehicle_view`, which also hides the drone from him, as the
//! script's `setinvisibletoplayer(owner)` does), `geteye` answers from the
//! tag (the script aims its laser and missiles from there), and BO2's HUD
//! hears the Lodestar (`bo2mp_vehicle` "remote_mortar_mp"). The gun in his
//! hands does not fire while his view is away (his trigger is the script's:
//! `attackbuttonpressed` fires the drone's missiles), but he still aims it:
//! BO2's Lodestar weapon (remote_mortar_mp) zooms to 10 on aim.
//!
//! While his screen rides the tag he carries BO2's view-only link flag
//! (`PLF_WEAPONVIEW_ONLY`): his view turns dead or alive, his client draws
//! the linked fov (45, his gun's ADS zoom when he aims), and his body keeps
//! the facing it had when his view left it.

use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::super::{Zm, arg, entnum, frame, text};
use super::streaks;
use crate::sim_parts::ClientId;

/// His view on an entity's tag.
#[derive(Clone, Debug)]
pub(crate) struct ViewLink {
    pub ent: u32,
    /// The tag in its model's space.
    pub at: [f32; 3],
    /// The tag's own facing in its model: yaw, and pitch (down is +).
    pub heading: f32, pub tilt: f32,
    /// How far he may look from the tag's facing: right, left, up, down.
    pub arcs: [f32; 4],
    /// The entity's yaw last tick (his view turns with it).
    pub last_yaw: f32,
    /// `playerlinkedsetusebaseangleforviewclamp`: the entity's base facing
    /// (a turret's, set down with), which his arcs keep to instead of the
    /// tag's; his view does not turn with the entity (the driven Sentry Gun
    /// turns to where he looks).
    pub base: Option<f32>,
    /// What BO2's HUD hears ("" none).
    pub hud: &'static str,
    pub view_sent: bool,
    /// His body's facing when his view left it (BO2 keeps it there).
    pub body_yaw: f32,
    /// He has died on the link (a spawn after it lets go).
    pub died: bool,
}

/// BO2's view-only link flag (`PLF_WEAPONVIEW_ONLY`) on his player state.
const PLF_WEAPONVIEW_ONLY: u32 = 4;

/// The lowest he may ever look (the game's own limit).
const PITCH_DOWN_MAX: f32 = 85.0;

/// A tag's model-space place and rotation (x, y, z, w).
fn tag_pose(game: &mut World, mdl: &str, tag_name: &str) -> Option<([f32; 3], [f32; 4])> {
    let cap = frame(game).model_capability(mdl)??;
    let bone = cap.pose.joint_labels.iter().position(|n| n.eq_ignore_ascii_case(tag_name))?;
    cap.pose
        .foundation_surface
        .get(bone)
        .map(|(q, t)| (t.to_array(), q.to_array()))
}

fn wrap(a: f32) -> f32 {
    let a = a.rem_euclid(360.0);
    if a > 180.0 { a - 360.0 } else { a }
}

/// His pitch limits ([up, down], as `view_pitch_clamp`) on this link.
fn pitch_clamp(link: &ViewLink) -> [f32; 2] {
    [
        -(link.tilt - link.arcs[2]),
        (link.tilt + link.arcs[3]).min(PITCH_DOWN_MAX),
    ]
}

/// The tag's place in the world on its entity now.
fn tag_world(world: &World, link: &ViewLink) -> Option<[f32; 3]> {
    let e = world.resource::<Zm>().ents.get(&link.ent)?;
    let (f, r, u) = gsc_t6::math::facing_axes(e.rot);
    let [x, y, z] = link.at;
    Some(std::array::from_fn(|i| e.origin[i] + f[i] * x - r[i] * y + u[i] * z))
}

/// His eye while his view is linked: the tag.
pub(super) fn eye(world: &mut World, client: u32) -> Option<[f32; 3]> {
    let link = streaks(world).view_links.get(&client).cloned()?;
    tag_world(world, &link)
}

/// Ends his view link: his view and his own screen back.
pub(super) fn end(world: &mut World, client: u32) {
    let_go(world, client, false);
}

/// Ends his view link. BO2's unlink (`EntUnlink`) takes the flag off and
/// turns his view back to his body's facing; a spawn (`spawned`) sets his
/// view itself.
fn let_go(game: &mut World, peer: u32, spawned: bool) {
    let Some(link) = streaks(game).view_links.remove(&peer) else {
        return;
    };
    let flagged = {
        let mut f = frame(game);
        f.occupant_mut(ClientId(peer)).is_some_and(|ps| {
            let on = ps.attach_bits & PLF_WEAPONVIEW_ONLY != 0;
            ps.attach_bits &= !PLF_WEAPONVIEW_ONLY;
            on
        })
    };
    if flagged && !spawned {
        super::super::set_player_view(game, ClientId(peer), [0.0, link.body_yaw, 0.0]);
    }
    {
        let mut f = frame(game);
        if f.client_meta(ClientId(peer)).is_some() {
            let m = f.client_meta_mut(ClientId(peer));
            m.controls.jump_disabled = false;
            m.controls.fire_disabled = false;
            m.view_pitch_clamp = None;
        }
    }
    super::super::set_client_dvar(game, peer, "bo2mp_vehicle_view", "");
    if !link.hud.is_empty() {
        super::clear_vehicle_hud(game, peer, link.hud);
    }
    diag::info!(Sim, "bo2mp view link: player {peer} lets go of ent {}", link.ent);
}

pub(super) fn bind(vm: &mut Vm<World>) {
    vm.bind("playerlinkweaponviewtodelta", true, |vm, world, s, a| {
        let id = super::super::client(vm, world, s)?;
        let Some(ent) = entnum(vm, arg(a, 0)) else {
            return Ok(ScriptValue::Undefined);
        };
        let tag = text(vm, a, 1);
        let num = |i: usize| arg(a, i).as_float().unwrap_or(0.0);
        let arcs = [num(3), num(4), num(5), num(6)];
        let Some((model, ent_yaw)) = world
            .resource::<Zm>()
            .ents
            .get(&ent)
            .map(|e| (e.mesh.clone(), e.rot[1]))
        else {
            return Ok(ScriptValue::Undefined);
        };
        end(world, id.0);
        super::ride::end_ride_in(vm, world, id.0);
        let body_yaw = frame(world).avatar(id).map_or(0.0, |ps| ps.look_angles[1]);
        let (at, q) = tag_pose(world, &model, &tag).unwrap_or(([0.0; 3], [0.0, 0.0, 0.0, 1.0]));
        // The tag's facing: its x axis turned by its rotation.
        let [qx, qy, qz, qw] = q;
        let fwd = [
            1.0 - 2.0 * (qy * qy + qz * qz),
            2.0 * (qx * qy + qw * qz),
            2.0 * (qx * qz - qw * qy),
        ];
        let yaw = fwd[1].atan2(fwd[0]).to_degrees();
        let pitch = -fwd[2].clamp(-1.0, 1.0).asin().to_degrees();
        // The Lodestar's drone: BO2's HUD has its own screen (ReaperHUD).
        let hud = if model.starts_with("veh_t6_drone_pegasus") {
            "remote_mortar_mp"
        } else {
            ""
        };
        let link = ViewLink {
            ent,
            at,
            heading: yaw,
            tilt: pitch,
            arcs,
            last_yaw: ent_yaw,
            base: None,
            hud,
            view_sent: false,
            body_yaw,
            died: false,
        };
        let clamp = pitch_clamp(&link);
        {
            let mut f = frame(world);
            if f.client_meta(id).is_some() {
                let m = f.client_meta_mut(id);
                m.view_pitch_clamp = Some(clamp);
                m.controls.jump_disabled = true;
                m.controls.fire_disabled = true;
            }
        }
        if !hud.is_empty() {
            super::super::set_client_dvar(world, id.0, "bo2mp_vehicle", &format!("{hud} 0 - - - - -"));
        }
        diag::info!(
            Sim,
            "bo2mp view link: player {} on ent {ent} ({model} {tag} at {at:?}, facing yaw {yaw:.1} \
             pitch {pitch:.1}, arcs {arcs:?}, pitch limits {clamp:?})",
            id.0
        );
        streaks(world).view_links.insert(id.0, link);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playerlinkedsetusebaseangleforviewclamp", true, |vm, world, s, a| {
        let id = super::super::client(vm, world, s)?;
        let on = gsc_t6::truthy(arg(a, 0));
        let Some(ent) = streaks(world).view_links.get(&id.0).map(|l| l.ent) else {
            return Ok(ScriptValue::Undefined);
        };
        let now = world.resource::<Zm>().ents.get(&ent).map(|e| e.rot[1]);
        let set_down = streaks(world).turrets.get(&ent).and_then(|t| t.base_yaw);
        if let Some(l) = streaks(world).view_links.get_mut(&id.0) {
            l.base = if on { set_down.or(now) } else { None };
            l.last_yaw = now.unwrap_or(l.last_yaw);
        }
        Ok(ScriptValue::Undefined)
    });
}

/// Each tick (his command is in): his body stays still, his view turns with
/// the entity and keeps to its arcs, dead or alive; gone, or spawned again
/// after dying, the link ends.
pub(super) fn advance(world: &mut World) {
    let links: Vec<(u32, ViewLink)> = streaks(world)
        .view_links
        .iter()
        .map(|(c, l)| (*c, l.clone()))
        .collect();
    for (c, mut link) in links {
        let who = ClientId(c);
        let Some(dead) = frame(world)
            .avatar(who)
            .map(|ps| ps.movement_mode >= playerstate_t6::game::MOVE_KIND_DEAD)
        else {
            end(world, c);
            continue;
        };
        // Spawned again after dying: BO2's `ClientSpawn` lets go, his spawn
        // view standing.
        if link.died && !dead {
            let_go(world, c, true);
            continue;
        }
        link.died |= dead;
        let Some(ent_yaw) = world.resource::<Zm>().ents.get(&link.ent).map(|e| e.rot[1]) else {
            end(world, c);
            continue;
        };
        // His body stays where he stood, his own gun quiet (he still aims);
        // his last command's view.
        let cmd_angles = {
            let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
            let mut last = None;
            for (cid, cmd) in &mut req.input.cmds {
                if *cid != who {
                    continue;
                }
                (cmd.thrust_ahead, cmd.thrust_lateral) = (0, 0);
                last = Some(cmd.angles);
            }
            last
        };
        let (turn, facing) = match link.base {
            Some(base) => (0.0, base + link.heading),
            None => (wrap(ent_yaw - link.last_yaw), ent_yaw + link.heading),
        };
        link.last_yaw = ent_yaw;
        {
            let mut f = frame(world);
            if f.client_meta(who).is_some() {
                let ctl = &mut f.client_meta_mut(who).controls;
                ctl.jump_disabled = true;
                ctl.fire_disabled = true;
            }
            if let Some(ps) = f.occupant_mut(who) {
                ps.view_offset_angles[1] += turn;
                ps.look_angles[1] = wrap(ps.look_angles[1] + turn);
                let commanded = cmd_angles.map(|a| a[1] as f32 * weapon_t6::recoil::WORD16_TO_DEGREES);
                let view = commanded.map_or(ps.look_angles[1], |a| a + ps.view_offset_angles[1]);
                let rel = wrap(view - facing);
                let kept = rel.clamp(-link.arcs[0], link.arcs[1]);
                if kept != rel {
                    ps.view_offset_angles[1] += kept - rel;
                    ps.look_angles[1] = wrap(ps.look_angles[1] + kept - rel);
                }
                ps.view_offset_angles[1] = wrap(ps.view_offset_angles[1]);
                // Dead, he runs no pmove of ours (only the living's do), but
                // BO2's still turns a view-only link's view
                // (PM_UpdateViewAngles_Clamp): his command's view, its pitch
                // kept to the link's limits.
                if dead && let Some(a) = cmd_angles {
                    let [up, down] = pitch_clamp(&link);
                    let commanded = a[0] as f32 * weapon_t6::recoil::WORD16_TO_DEGREES;
                    let mut pitch = wrap(commanded + ps.view_offset_angles[0]);
                    if down < pitch || pitch < -up {
                        pitch = if down < pitch { down } else { -up };
                        ps.view_offset_angles[0] = pitch - commanded;
                    }
                    ps.look_angles[0] = pitch;
                    ps.look_angles[1] =
                        wrap(a[1] as f32 * weapon_t6::recoil::WORD16_TO_DEGREES + ps.view_offset_angles[1]);
                }
            }
        }
        // His screen rides the tag once the entity has a model on his
        // client (made the tick after it): "<model> 1 <x> <y> <z> <up>
        // <down> <fov> <number>".
        if !link.view_sent {
            let shown = world
                .resource::<Zm>()
                .presences
                .by_ent
                .get(&link.ent)
                .map(|p| (p.id.encode_for_net(), p.num));
            if let Some((m, number)) = shown {
                let [x, y, z] = link.at;
                let [up, down] = pitch_clamp(&link);
                let view = format!("{m} 1 {x} {y} {z} {up} {down} 0 {number}");
                super::super::set_client_dvar(world, c, "bo2mp_vehicle_view", &view);
                link.view_sent = true;
            }
        }
        // The flag rides with his screen (no tick of his own third-person or
        // death view before the tag's): his pmove lets him look dead, his
        // client zooms to the linked fov, his body keeps its facing.
        if link.view_sent
            && let Some(ps) = frame(world).occupant_mut(who)
        {
            ps.attach_bits |= PLF_WEAPONVIEW_ONLY;
            ps.link_weapon_angles = [0.0, link.body_yaw, 0.0];
        }
        streaks(world).view_links.insert(c, link);
    }
}
