//! Script HUD elements (`newhudelem`, `newclienthudelem`): text and values
//! the zombies scripts put on screen (Game Over, "you survived N rounds",
//! Max Ammo...). The engine keeps each element's text (the game's English
//! with its arguments), fades and moves; every tick each player's visible
//! elements go to his client as one client dvar (`bo2zm_hud`), drawn by
//! the zombies HUD.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, ScriptValue, Vm};

use super::{Zm, arg, entnum, frame, with_vm};

/// Separators in the published list: elements, then fields.
pub const ELEM_SEP: char = '\u{1e}';
pub const FIELD_SEP: char = '\u{1f}';

#[derive(Clone, Debug, Default)]
pub(crate) struct HudElem {
    pub obj: Option<ObjRef>,
    /// Shown to one player (`newclienthudelem`), else to all.
    pub client: Option<u32>,
    pub caption: String,
    /// A fade in progress: (alpha it started from, start ms, length ms).
    pub fade: Option<(f32, i64, i64)>,
    /// A move in progress: (x, y it started from, start ms, length ms).
    pub mv: Option<([f32; 2], i64, i64)>,
    /// A text size change in progress (`changefontscaleovertime`): (size
    /// it started from, start ms, length ms).
    pub grow: Option<(f32, i64, i64)>,
    /// A timer counting down from (seconds, set at ms).
    pub timer: Option<(f32, i64)>,
    /// A picture instead of text (`setshader(name, width, height)`).
    pub shader: Option<(String, f32, f32)>,
    /// bo2mp: a picture's size changing (`scaleovertime`: a progress
    /// bar filling): (size it started from, start ms, length ms) toward
    /// the shader's size.
    pub scale: Option<([f32; 2], i64, i64)>,
    /// bo2mp: shown to one team only (`newteamhudelem(team)`).
    pub team: Option<String>,
    /// bo2mp: a picture in the world (`setwaypoint(constant size)`): over
    /// its target entity (`settargetent`) by its x/y/z, else at x/y/z.
    pub waypoint: Option<bool>,
    pub target: Option<ScriptValue>,
    /// bo2mp: a text effect (`setpulsefx`, `setcod7decodefx`,
    /// `settypewriterfx`): [set at ms, ms per letter, decay start ms, decay
    /// ms]. The letters come in one by one, then the text fades away (how
    /// BO2 takes down the objective text and the outcome title).
    pub fx: Option<[i64; 4]>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Huds {
    pub elems: BTreeMap<u32, HudElem>,
    /// What each player was last sent.
    pub sent: BTreeMap<u32, String>,
    /// bo2mp: and his world pictures (`publish_waypoints`).
    pub sent_points: BTreeMap<String, String>,
}

fn field(vm: &mut Vm<World>, o: ObjRef, name: &str) -> ScriptValue {
    let f = vm.intern(name);
    vm.raw_field(o, f)
}

fn num(vm: &mut Vm<World>, o: ObjRef, name: &str, default: f32) -> f32 {
    field(vm, o, name).as_float().unwrap_or(default)
}

fn txt(vm: &mut Vm<World>, o: ObjRef, name: &str, default: &str) -> String {
    match field(vm, o, name) {
        ScriptValue::Undefined => default.to_owned(),
        v => vm.to_text(&v),
    }
}

/// A new element: the engine's defaults, kept for drawing.
fn create(vm: &mut Vm<World>, game: &mut World, peer: Option<u32>) -> ScriptValue {
    let o = vm.alloc_object(ObjKind::HudElem(0));
    for (k, v) in [
        ("x", ScriptValue::Int(0)),
        ("y", ScriptValue::Int(0)),
        ("z", ScriptValue::Int(0)),
        ("alpha", ScriptValue::Float(1.0)),
        ("fontscale", ScriptValue::Float(1.0)),
        ("sort", ScriptValue::Int(0)),
        ("color", ScriptValue::Vec3([1.0, 1.0, 1.0])),
        ("glowalpha", ScriptValue::Float(0.0)),
        ("glowcolor", ScriptValue::Vec3([0.0, 0.0, 0.0])),
        ("width", ScriptValue::Int(0)),
        ("height", ScriptValue::Int(0)),
    ] {
        let f = vm.intern(k);
        vm.set_raw_field(o, f, v);
    }
    game.resource_mut::<Zm>().huds.elems.insert(
        o.index,
        HudElem {
            obj: Some(o),
            client: peer,
            ..Default::default()
        },
    );
    ScriptValue::Object(o)
}

fn elem_mut<'a>(zm: &'a mut Zm, s: &ScriptValue) -> Option<&'a mut HudElem> {
    let ScriptValue::Object(o) = s else { return None };
    zm.huds
        .elems
        .get_mut(&o.index)
        .filter(|e| e.obj == Some(*o))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    for name in ["newhudelem", "newdebughudelem"] {
        vm.bind(name, false, |vm, world, _, _| Ok(create(vm, world, None)));
    }
    // newteamhudelem(team): seen by that team only (BO2's equipment head
    // icons: the planter's team sees whose claymore it is).
    vm.bind("newteamhudelem", false, |vm, world, _, a| {
        let team = match arg(a, 0) {
            ScriptValue::Undefined => None,
            v => Some(vm.to_text(v)),
        };
        let e = create(vm, world, None);
        if let Some(h) = elem_mut(&mut world.resource_mut::<Zm>(), &e) {
            h.team = team;
        }
        Ok(e)
    });
    // setwaypoint(constant size, ...): drawn in the world, not on the
    // screen; settargetent(ent) / cleartargetent(): over that entity.
    vm.bind("setwaypoint", true, |_, world, s, a| {
        let constant = arg(a, 0).as_float().unwrap_or(0.0) != 0.0;
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.waypoint = Some(constant);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("settargetent", true, |_, world, s, a| {
        let target = arg(a, 0).clone();
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.target = Some(target);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("cleartargetent", true, |_, world, s, _| {
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.target = None;
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: `newdamageindicatorhudelem(player)` too: the hit marker is his
    // own (it showed every player's, so a bot's hit flashed on his screen).
    for name in ["newclienthudelem", "newscorehudelem", "newdamageindicatorhudelem"] {
        vm.bind(name, false, |vm, world, _, a| {
            let client = entnum(vm, arg(a, 0));
            Ok(create(vm, world, client))
        });
    }
    vm.bind("settext", true, |vm, world, s, a| {
        let args = super::name_players(vm, world, a.get(1..).unwrap_or(&[]));
        let text = super::localize(vm, world, arg(a, 0), &args);
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.caption = text;
            e.timer = None;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("setvalue", true, |vm, world, s, a| {
        let text = match arg(a, 0) {
            ScriptValue::Float(f) if f.fract() != 0.0 => format!("{f}"),
            v => v
                .as_float()
                .map_or_else(|| vm.to_text(v), |f| format!("{}", f as i64)),
        };
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.caption = text;
            e.timer = None;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("setshader", true, |vm, world, s, a| {
        let name = vm.to_text(arg(a, 0));
        let w = arg(a, 1).as_float().unwrap_or(0.0);
        let h = arg(a, 2).as_float().unwrap_or(0.0);
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            // BO2MP_HITLOG: each hit marker beside the hits it answers.
            if name.starts_with("damage_feedback") && std::env::var_os("BO2MP_HITLOG").is_some() {
                diag::info!(Sim, "bo2mp hit marker: {name} {w}x{h} to {:?} team {:?}", e.client, e.team);
            }
            e.shader = Some((name, w, h));
            e.scale = None;
            e.caption.clear();
        }
        Ok(ScriptValue::Undefined)
    });
    // `scaleovertime(seconds, width, height)`: its picture grows or shrinks
    // to that size (BO2's progress bars: a care package's capture).
    vm.bind("scaleovertime", true, |_, world, s, a| {
        let secs = arg(a, 0).as_float().unwrap_or(0.0);
        let to = [arg(a, 1).as_float().unwrap_or(0.0), arg(a, 2).as_float().unwrap_or(0.0)];
        let now = world.resource::<Zm>().clock_millis;
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            let from = shown_size(e, now);
            if let (Some((_, w, h)), Some(from)) = (e.shader.as_mut(), from) {
                (*w, *h) = (to[0], to[1]);
                e.scale = Some((from, now, (secs * 1000.0) as i64));
            }
        }
        Ok(ScriptValue::Undefined)
    });
    for name in ["settimer", "settenthstimer"] {
        vm.bind(name, true, |_, world, s, a| {
            let secs = arg(a, 0).as_float().unwrap_or(0.0);
            let now = world.resource::<Zm>().clock_millis;
            if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
                e.timer = Some((secs, now));
            }
            Ok(ScriptValue::Undefined)
        });
    }
    vm.bind("fadeovertime", true, |vm, world, s, a| {
        let secs = arg(a, 0).as_float().unwrap_or(0.0);
        let now = world.resource::<Zm>().clock_millis;
        let ScriptValue::Object(o) = s else {
            return Ok(ScriptValue::Undefined);
        };
        let from = shown_alpha(vm, world, *o, now);
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            if let Some((name, ..)) = e.shader.as_ref().filter(|p| p.0.starts_with("damage_feedback"))
                && std::env::var_os("BO2MP_HITLOG").is_some()
            {
                diag::info!(Sim, "bo2mp hit marker shown: {name} from alpha {from} over {secs}s to {:?} at {now}", e.client);
            }
            e.fade = Some((from, now, (secs * 1000.0) as i64));
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("moveovertime", true, |vm, world, s, a| {
        let secs = arg(a, 0).as_float().unwrap_or(0.0);
        let now = world.resource::<Zm>().clock_millis;
        let ScriptValue::Object(o) = s else {
            return Ok(ScriptValue::Undefined);
        };
        let from = [num(vm, *o, "x", 0.0), num(vm, *o, "y", 0.0)];
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.mv = Some((from, now, (secs * 1000.0) as i64));
        }
        Ok(ScriptValue::Undefined)
    });
    // Grief's messages land big and shrink, then grow as they fade out;
    // without the change the size jumped and a giant faded copy hung there.
    vm.bind("changefontscaleovertime", true, |vm, world, s, a| {
        let secs = arg(a, 0).as_float().unwrap_or(0.0);
        let now = world.resource::<Zm>().clock_millis;
        let ScriptValue::Object(o) = s else {
            return Ok(ScriptValue::Undefined);
        };
        let from = shown_scale(vm, world, *o, now);
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.grow = Some((from, now, (secs * 1000.0) as i64));
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("destroy", true, |vm, world, s, _| {
        if let Some(o) = s.as_obj()
            && matches!(vm.kind(o), Some(ObjKind::HudElem(_)))
        {
            world.resource_mut::<Zm>().huds.elems.remove(&o.index);
            vm.free_object(world, o);
        }
        Ok(ScriptValue::Undefined)
    });
}

/// A picture's size now (mid-scale or its own).
fn shown_size(e: &HudElem, now: i64) -> Option<[f32; 2]> {
    let (_, w, h) = e.shader.as_ref()?;
    Some(match e.scale {
        Some((from, start, len)) if len > 0 && now < start + len => {
            let t = (now - start) as f32 / len as f32;
            [from[0] + (w - from[0]) * t, from[1] + (h - from[1]) * t]
        }
        _ => [*w, *h],
    })
}

/// The alpha an element shows now (mid-fade or its field).
fn shown_alpha(vm: &mut Vm<World>, world: &World, o: ObjRef, now: i64) -> f32 {
    let target = num(vm, o, "alpha", 1.0);
    match world
        .resource::<Zm>()
        .huds
        .elems
        .get(&o.index)
        .and_then(|e| e.fade)
    {
        Some((from, start, len)) if len > 0 && now < start + len => {
            let t = (now - start) as f32 / len as f32;
            from + (target - from) * t
        }
        _ => target,
    }
}

/// The text size an element shows now (mid-change or its field).
fn shown_scale(vm: &mut Vm<World>, world: &World, o: ObjRef, now: i64) -> f32 {
    let target = num(vm, o, "fontscale", 1.0);
    match world
        .resource::<Zm>()
        .huds
        .elems
        .get(&o.index)
        .and_then(|e| e.grow)
    {
        Some((from, start, len)) if len > 0 && now < start + len => {
            let t = (now - start) as f32 / len as f32;
            from + (target - from) * t
        }
        _ => target,
    }
}

/// Each tick: every player's visible elements, sent when they change.
pub(crate) fn publish(world: &mut World) {
    let now = world.resource::<Zm>().clock_millis;
    let elems: Vec<(u32, HudElem)> = world
        .resource::<Zm>()
        .huds
        .elems
        .iter()
        .map(|(k, en)| (*k, en.clone()))
        .collect();
    // Each player's team (a team element shows to its team only).
    let players: Vec<(u32, ObjRef)> = world.resource::<Zm>().players.iter().map(|(c, p)| (*c, p.obj)).collect();
    /// One element's line, who sees it (a client, a team, or all) and
    /// whether it is a picture in the world.
    struct Shown {
        client: Option<u32>,
        team: Option<String>,
        line: String,
        in_world: bool,
    }
    let (table_rows, teams): (Vec<Shown>, Vec<(u32, String)>) = with_vm(world, |vm, world| {
        let teams: Vec<(u32, String)> = players.iter().map(|(c, o)| (*c, txt(vm, *o, "team", ""))).collect();
        let mut table_rows = Vec::new();
        for (_, en) in &elems {
            let Some(o) = en.obj else { continue };
            if !vm.alive(o) {
                continue;
            }
            // A picture in the world: "material,x,y,z,width,height,alpha,
            // constant" (bo2mp_waypoints; the HUD projects it).
            if let (Some(constant), Some((ident, wd, h))) = (en.waypoint, &en.shader) {
                let alpha = shown_alpha(vm, world, o, now);
                let off = [num(vm, o, "x", 0.0), num(vm, o, "y", 0.0), num(vm, o, "z", 0.0)];
                let at = match &en.target {
                    Some(t) => match super::origin_of(vm, world, t) {
                        Some(p) => [p[0] + off[0], p[1] + off[1], p[2] + off[2]],
                        None => continue,
                    },
                    None => off,
                };
                if alpha > 0.01 {
                    let row = format!(
                        "{ident},{:.1},{:.1},{:.1},{wd},{h},{alpha:.2},{}",
                        at[0],
                        at[1],
                        at[2],
                        u8::from(constant)
                    );
                    table_rows.push(Shown { client: en.client, team: en.team.clone(), line: row, in_world: true });
                }
                continue;
            }
            let mut alpha = shown_alpha(vm, world, o, now);
            let mut text = en.caption.clone();
            if let Some((secs, at)) = en.timer {
                let left = (secs - (now - at) as f32 / 1000.0).max(0.0);
                text = format!("{}:{:02}", left as i32 / 60, left as i32 % 60);
            }
            // `label`: a game string shown before the text, or around it
            // where it holds `&&1` (`elem.label = &"MP_..."` before setvalue).
            // Max Ammo's "Max Ammo!" is a label alone (Zombies; in
            // multiplayer an element with no text yet shows nothing).
            let label = field(vm, o, "label");
            if !matches!(label, ScriptValue::Undefined)
                && (!text.is_empty() || !world.resource::<Zm>().mp)
            {
                let l = super::localize(vm, world, &label, &[]);
                text = if l.contains("&&1") {
                    l.replace("&&1", &text)
                } else {
                    format!("{l}{text}")
                };
            }
            // bo2mp: a text effect: so many letters so far, then the decay.
            if let Some([at, letter, start, len]) = en.fx {
                let t = now - at;
                if letter > 0 {
                    let n = usize::try_from(t / letter + 1).unwrap_or(0);
                    text = text.chars().take(n).collect();
                }
                if start > 0 && t > start {
                    alpha *= (1.0 - (t - start) as f32 / len.max(1) as f32).clamp(0.0, 1.0);
                }
            }
            // A picture rides in the text field as `#shader:<ident>:<wd>:<h>`.
            if let (Some((ident, _, _)), Some([wd, h])) = (&en.shader, shown_size(en, now)) {
                text = format!("#shader:{ident}:{wd:.1}:{h:.1}");
            }
            if text.is_empty() || alpha <= 0.01 {
                continue;
            }
            let (mut x, mut y) = (num(vm, o, "x", 0.0), num(vm, o, "y", 0.0));
            if let Some((from, start, len)) = en.mv
                && len > 0
                && now < start + len
            {
                let t = (now - start) as f32 / len as f32;
                x = from[0] + (x - from[0]) * t;
                y = from[1] + (y - from[1]) * t;
            }
            let color = field(vm, o, "color").as_vec3().unwrap_or([1.0; 3]);
            let row = [
                format!("{x:.1}"),
                format!("{y:.1}"),
                txt(vm, o, "alignx", "left"),
                txt(vm, o, "aligny", "top"),
                txt(vm, o, "horzalign", "fullscreen"),
                txt(vm, o, "vertalign", "fullscreen"),
                format!("{:.2}", shown_scale(vm, world, o, now)),
                format!("{:.2}", color[0]),
                format!("{:.2}", color[1]),
                format!("{:.2}", color[2]),
                format!("{alpha:.2}"),
                (num(vm, o, "sort", 0.0) as i32).to_string(),
                text.replace([ELEM_SEP, FIELD_SEP], " "),
                // bo2mp: `hidewheninmenu` (the objective text, the outcome
                // screen): "m", hidden while his menus are open.
                if field(vm, o, "hidewheninmenu").as_float().unwrap_or(0.0) != 0.0 { "m".to_owned() } else { String::new() },
                // bo2mp: the element's `font` (createfontstring's ident:
                // default, objective, big, small, extrabig, bigfixed,
                // smallfixed), so the drawing side picks BO2's font and size
                // as the engine does.
                txt(vm, o, "font", "default"),
            ]
            .join(&FIELD_SEP.to_string());
            table_rows.push(Shown { client: en.client, team: en.team.clone(), line: row, in_world: false });
        }
        (table_rows, teams)
    })
    .unwrap_or_default();
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        if super::is_bot(world, c) {
            continue;
        }
        let team = teams.iter().find(|(t, _)| *t == c).map(|(_, t)| t.as_str()).unwrap_or("");
        let lines = |in_world: bool| -> Vec<&str> {
            let seen = |s: &&Shown| s.client.is_none_or(|wd| wd == c) && s.team.as_deref().is_none_or(|t| t == team);
            table_rows.iter().filter(|s| s.in_world == in_world).filter(seen).map(|s| s.line.as_str()).collect()
        };
        let points = lines(true);
        publish_waypoints(world, c, points.join(";"));
        let mine = lines(false);
        let value = mine.join(&ELEM_SEP.to_string());
        if world.resource::<Zm>().huds.sent.get(&c) == Some(&value) {
            continue;
        }
        if std::env::var_os("BO2MP_T6_HUDLOG").is_some() {
            let now = world.resource::<Zm>().clock_millis;
            diag::info!(
                Sim,
                "bo2zm t6 hud at {}ms: {}",
                now,
                value.replace([ELEM_SEP], " | ").replace([FIELD_SEP], ",")
            );
        }
        let mut f = frame(world);
        if f.client_meta(crate::sim_parts::ClientId(c)).is_none() {
            continue;
        }
        let dvars = &mut f.client_meta_mut(crate::sim_parts::ClientId(c)).client_dvars;
        match dvars.iter_mut().find(|(k, _)| k == "bo2zm_hud") {
            Some(row) => row.1 = value.clone(),
            None => dvars.push(("bo2zm_hud".to_owned(), value.clone())),
        }
        world.resource_mut::<Zm>().huds.sent.insert(c, value);
    }
}

/// bo2mp: a player's world pictures (`setwaypoint` elements he sees) as
/// his client dvar `bo2mp_waypoints`, when they change.
fn publish_waypoints(game: &mut World, ch: u32, val: String) {
    let key = format!("w{ch}");
    let sent = game.resource::<Zm>().huds.sent_points.get(&key) == Some(&val);
    if sent {
        return;
    }
    let mut f = frame(game);
    if f.client_meta(crate::sim_parts::ClientId(ch)).is_none() {
        return;
    }
    let dvars = &mut f.client_meta_mut(crate::sim_parts::ClientId(ch)).client_dvars;
    match dvars.iter_mut().find(|(k, _)| k == "bo2mp_waypoints") {
        Some(row) => row.1 = val.clone(),
        None => dvars.push(("bo2mp_waypoints".to_owned(), val.clone())),
    }
    game.resource_mut::<Zm>().huds.sent_points.insert(key, val);
}
