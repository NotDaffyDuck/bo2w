//! Sounds and effects the scripts play: entity events the client turns
//! into Black Ops II sound aliases and effects by name (the same events
//! MW2's scripts use).

use bevy_ecs::prelude::World;
use gsc_t6::{Key, ObjKind, Value, Vm};

use super::{Zm, arg, entnum, frame, int, num, origin_of, text, tick};
use crate::EventAudience;
use crate::world::ClientId;

fn event(
    world: &mut World,
    audience: EventAudience,
    kind: entity_t6::EntityEventKind,
    parm: u8,
    origin: [f32; 3],
    dir: [f32; 3],
) {
    event_on(world, audience, kind, parm, origin, dir, i32::from(trace_t6::ENTITYNUM_WORLD));
}

fn event_on(
    world: &mut World,
    audience: EventAudience,
    kind: entity_t6::EntityEventKind,
    parm: u8,
    origin: [f32; 3],
    dir: [f32; 3],
    number: i32,
) {
    let t = tick(world);
    frame(world).push_entity_event(
        t,
        audience,
        kind,
        crate::EntityEventPayload {
            number,
            event_parm: i32::from(parm),
            origin,
            direction: dir,
            ..Default::default()
        },
    );
}

pub(super) fn sound(world: &mut World, audience: EventAudience, alias: &str, origin: [f32; 3]) {
    if alias.is_empty() {
        return;
    }
    // bo2mp: BO2MP_SNDLOG=1 logs every sound the scripts play (test aid:
    // checked against the client's own sound starts).
    if std::env::var_os("BO2MP_SNDLOG").is_some() {
        diag::info!(Sim, "bo2mp sndlog play {alias} {audience:?}");
    }
    // Black Ops II's own aliases (the client looks names up by game).
    let index = frame(world).sound_alias_index(&format!("t6:{}", alias.to_ascii_lowercase()));
    event(
        world,
        audience,
        entity_t6::EntityEventKind::SOUND_ALIAS,
        index,
        origin,
        [0.0; 3],
    );
}

/// A sound only the one player hears, flat (not placed in the world):
/// BO2's playlocalsound, e.g. "your care package was stolen". The client
/// plays an alias event on the local-sound entity number without a position.
fn local_sound(world: &mut World, audience: EventAudience, alias: &str) {
    if alias.is_empty() {
        return;
    }
    if std::env::var_os("BO2MP_SNDLOG").is_some() {
        diag::info!(Sim, "bo2mp sndlog local {alias} {audience:?}");
    }
    let index = frame(world).sound_alias_index(&format!("t6:{}", alias.to_ascii_lowercase()));
    event_on(
        world,
        audience,
        entity_t6::EntityEventKind::SOUND_ALIAS,
        index,
        [0.0; 3],
        [0.0; 3],
        entity_t6::LOCAL_SOUND_ENTITY,
    );
}

/// A script's rumble on an entity (BO2 adds the event to the entity
/// itself): a player by his client number, any other entity by the number
/// its clients know it by; one they are not shown carries none.
fn rumble_on(
    vm: &mut Vm<World>,
    world: &mut World,
    s: &Value,
    a: &[Value],
    kind: entity_t6::EntityEventKind,
) -> Result<Value, String> {
    let name = text(vm, a, 0);
    let n = entnum(vm, s).ok_or("not an entity")?;
    let target = if world.resource::<Zm>().players.contains_key(&n) {
        Some((n as i32, origin_of(vm, world, s).unwrap_or([0.0; 3])))
    } else {
        shown(world, n).map(|(number, _, _, origin)| (number, origin))
    };
    if let Some((number, origin)) = target {
        rumble_event(world, kind, number, Some(&name), origin);
    }
    Ok(Value::Undefined)
}

/// A script's rumble at a spot (on the world entity).
fn rumble_at(
    vm: &mut Vm<World>,
    world: &mut World,
    a: &[Value],
    kind: entity_t6::EntityEventKind,
) -> Result<Value, String> {
    let name = text(vm, a, 0);
    let origin = arg(a, 1).as_vec3().ok_or("the rumble's position is not a vector")?;
    rumble_event(world, kind, i32::from(trace_t6::ENTITYNUM_WORLD), Some(&name), origin);
    Ok(Value::Undefined)
}

/// One rumble event to everyone. BO2MP_RUMBLELOG=1 logs each (test aid).
fn rumble_event(
    world: &mut World,
    kind: entity_t6::EntityEventKind,
    number: i32,
    name: Option<&str>,
    origin: [f32; 3],
) {
    let parm = name.map_or(0, |name| {
        frame(world).sound_alias_index(&format!("rumble:{}", name.to_ascii_lowercase()))
    });
    if std::env::var_os("BO2MP_RUMBLELOG").is_some() {
        diag::info!(
            Sim,
            "bo2mp rumble {:#x} {} on {number} parm {parm}",
            kind.0,
            name.unwrap_or("-")
        );
    }
    let t = tick(world);
    frame(world).push_entity_event(t, EventAudience::All, kind, rumble_payload(number, parm, origin));
}

/// The rumble event's payload: its spot in whole units (BO2 sends it so).
fn rumble_payload(number: i32, parm: u8, origin: [f32; 3]) -> crate::EntityEventPayload {
    crate::EntityEventPayload {
        number,
        event_parm: i32::from(parm),
        origin: origin.map(f32::trunc),
        ..Default::default()
    }
}

/// An effect turned by both its forward and up (sent in origin2).
pub(super) fn effect_axis(
    world: &mut World,
    name: &str,
    origin: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
) {
    effect_axis_to(world, EventAudience::All, name, origin, forward, up);
}

/// An effect with its up as well, seen by `audience` alone (the Sensor
/// Grenade's scan: one look to its owner's side, another to the rest).
pub(super) fn effect_axis_to(
    world: &mut World,
    audience: EventAudience,
    name: &str,
    origin: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
) {
    if name.is_empty() {
        return;
    }
    let index = frame(world).effect_name_index(name);
    let t = tick(world);
    frame(world).push_entity_event(
        t,
        audience,
        entity_t6::EntityEventKind::PLAY_FX,
        crate::EntityEventPayload {
            number: i32::from(trace_t6::ENTITYNUM_WORLD),
            event_parm: i32::from(index),
            origin,
            origin2: up,
            direction: forward,
            ..Default::default()
        },
    );
}

/// An effect that stays (`spawnfx` + `triggerfx`): a looping one goes on.
/// Without an up its client turns it by its forward alone.
pub(super) fn effect_held(
    world: &mut World,
    name: &str,
    origin: [f32; 3],
    forward: [f32; 3],
    up: Option<[f32; 3]>,
) {
    if name.is_empty() {
        return;
    }
    let index = frame(world).effect_name_index(name);
    let t = tick(world);
    frame(world).push_entity_event(
        t,
        EventAudience::All,
        entity_t6::EntityEventKind::PLAY_FX,
        crate::EntityEventPayload {
            number: i32::from(trace_t6::ENTITYNUM_WORLD),
            event_parm: i32::from(index),
            origin,
            origin2: up.unwrap_or([0.0; 3]),
            direction: forward,
            simulation_flags: 0x20,
            ..Default::default()
        },
    );
}

/// A field of a script object.
fn obj_field(vm: &mut Vm<World>, o: &Value, name: &str) -> Value {
    let Some(o) = o.as_obj() else {
        return Value::Undefined;
    };
    let f = vm.intern(name);
    vm.raw_field(o, f)
}

/// `activateclientexploder(id)`: the client's half of an exploder (Nuketown's
/// perk machines landing). The client scripts would play each createfx
/// exploder's effects; the server's own functions for them play them here.
pub(super) fn client_exploder(vm: &mut Vm<World>, world: &mut World, id: &Value) {
    let level = Value::Object(vm.level);
    let ids = obj_field(vm, &level, "_exploder_ids");
    let Value::Array(ids) = ids else { return };
    let num = {
        let ids = ids.read();
        ids.keys()
            .find(|k| ids.get(k).and_then(Value::as_int) == id.as_int())
    };
    let Some(num) = num else { return };
    fire_exploder(vm, world, &num);
}

/// bo2mp: exploder `num`'s createfx effects and sounds, as the client's
/// `clientscripts\mp\_fx::exploder( num )` plays them.
pub(super) fn fire_exploder(vm: &mut Vm<World>, world: &mut World, num: &Key) {
    let level = Value::Object(vm.level);
    let Value::Array(all) = obj_field(vm, &level, "createfxexploders") else {
        return;
    };
    let Some(Value::Array(ents)) = all.get(num) else {
        return;
    };
    let ents: Vec<Value> = ents.read().values_in_order().cloned().collect();
    for ent in ents {
        let Value::Array(v) = obj_field(vm, &ent, "v") else {
            continue;
        };
        let has = |vm: &mut Vm<World>, k: &str| {
            let key = Key::Str(vm.intern(k));
            v.get(&key).filter(|x| !x.is_undefined())
        };
        let mut run = |vm: &mut Vm<World>, world: &mut World, f: &str| {
            vm.spawn_named(world, "maps/mp/_utility", f, ent.clone(), Vec::new());
        };
        if has(vm, "firefx").is_some() {
            run(vm, world, "fire_effect");
        }
        let fxid = has(vm, "fxid").map(|x| vm.to_text(&x));
        if fxid.is_some_and(|f| f != "No FX") {
            run(vm, world, "cannon_effect");
        } else if has(vm, "soundalias").is_some() {
            run(vm, world, "sound_effect");
        }
    }
}

/// bo2mp: where exploder `num`'s first effect is
/// (`level.createfxexploders[num][0].v["origin"]`).
pub(super) fn exploder_origin(vm: &mut Vm<World>, num: i32) -> Option<[f32; 3]> {
    let level = Value::Object(vm.level);
    let Value::Array(all) = obj_field(vm, &level, "createfxexploders") else {
        return None;
    };
    let Some(Value::Array(ents)) = all.get(&Key::Int(num)) else {
        return None;
    };
    let first = ents.get(&Key::Int(0))?;
    let Value::Array(v) = obj_field(vm, &first, "v") else {
        return None;
    };
    let key = Key::Str(vm.intern("origin"));
    v.get(&key)?.as_vec3()
}

pub(super) fn effect(world: &mut World, name: &str, origin: [f32; 3], forward: [f32; 3]) {
    if name.is_empty() {
        return;
    }
    let index = frame(world).effect_name_index(name);
    event(
        world,
        EventAudience::All,
        entity_t6::EntityEventKind::PLAY_FX,
        index,
        origin,
        forward,
    );
}

/// bo2mp: BO2's `sv_fx_blockMinDist`: smoke does not hide two points this
/// close to each other (units).
const FX_BLOCK_MIN_DIST: f32 = 80.0;
/// bo2mp: how many smoke clouds can block sight at once (BO2's
/// `SV_FX_SetVisBlocker` slots).
const VIS_BLOCKERS: usize = 32;

/// bo2mp: how much of the line from `a` to `b` smoke lets through (BO2's
/// `SV_FX_GetVisibility`): 0 when it passes inside a cloud (the sphere
/// `fxblocksight` puts round its entity), else 1; also 1 when the points
/// are within `sv_fx_blockMinDist` of each other, and outside
/// multiplayer.
pub(crate) fn fx_visibility(world: &World, a: [f32; 3], b: [f32; 3]) -> f32 {
    let zm = world.resource::<Zm>();
    if !zm.mp || zm.vis_blockers.is_empty() {
        return 1.0;
    }
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if len2 < FX_BLOCK_MIN_DIST * FX_BLOCK_MIN_DIST {
        return 1.0;
    }
    let blocked = zm.vis_blockers.iter().any(|&(n, radius)| {
        zm.ents
            .get(&n)
            .is_some_and(|e| segment_in_sphere(a, d, len2, e.origin, radius))
    });
    if blocked { 0.0 } else { 1.0 }
}

/// Does the segment from `a` along `d` (`len2` its length squared) come
/// strictly inside the sphere (BO2's `SegmentIntersectsSphere`: its point
/// nearest the centre)?
fn segment_in_sphere(a: [f32; 3], d: [f32; 3], len2: f32, centre: [f32; 3], radius: f32) -> bool {
    let c = [centre[0] - a[0], centre[1] - a[1], centre[2] - a[2]];
    let t = ((c[0] * d[0] + c[1] * d[1] + c[2] * d[2]) / len2).clamp(0.0, 1.0);
    let p = [d[0] * t - c[0], d[1] * t - c[1], d[2] * t - c[2]];
    p[0] * p[0] + p[1] * p[1] + p[2] * p[2] < radius * radius
}

/// bo2mp: the entity is being freed: it no longer blocks sight, and is no
/// timed or placed effect (nor kept from bots' aim); its client fields go.
pub(super) fn forget_timed(world: &mut World, n: u32) {
    let mut zm = world.resource_mut::<Zm>();
    zm.vis_blockers.retain(|&(b, _)| b != n);
    zm.no_bot_target.remove(&n);
    zm.client_field_vals.retain(|k, _| k.0 != n);
    zm.timed_fx.remove(&n);
    zm.fx_ents.remove(&n);
}

/// bo2mp: free the `spawntimedfx` entities whose time is up (BO2's timed
/// entities): "death" first (smoke's `blocksight` loop ends on it), then
/// the smoke stops blocking sight and the entity is gone.
pub(super) fn free_timed_fx(vm: &mut Vm<World>, world: &mut World, now: i64) {
    let due: Vec<u32> = {
        let zm = world.resource::<Zm>();
        zm.timed_fx
            .iter()
            .filter(|&(_, &t)| t < now)
            .map(|(&n, _)| n)
            .collect()
    };
    for n in due {
        let obj = world.resource::<Zm>().ents.get(&n).and_then(|e| e.obj);
        if let Some(o) = obj
            && vm.alive(o)
        {
            vm.notify_queued(world, o, "death", &[]);
        }
        forget_timed(world, n);
        {
            let mut zm = world.resource_mut::<Zm>();
            zm.movers.list.remove(&n);
            zm.ents.remove(&n);
        }
        if let Some(o) = obj {
            vm.free_object(world, o);
        }
    }
}

/// The effect a `loadfx` id names.
pub(super) fn fx_name(world: &World, id: i32) -> Option<String> {
    let zm = world.resource::<Zm>();
    zm.fx.get(usize::try_from(id - 1).ok()?).cloned()
}

/// bo2mp: the player `v` is, else Black Ops II's "entity %i is not a
/// player".
fn player_client(vm: &Vm<World>, world: &World, v: &Value) -> Result<ClientId, String> {
    match entnum(vm, v) {
        Some(n) if world.resource::<Zm>().players.contains_key(&n) => Ok(ClientId(n)),
        Some(n) => Err(format!("entity {n} is not a player")),
        None => Err("not an entity".into()),
    }
}

/// bo2mp: the players on session team `team` (`sessionteam`).
fn team_clients(vm: &mut Vm<World>, world: &World, team: &str) -> Vec<ClientId> {
    let players: Vec<(u32, gsc_t6::ObjRef)> = world
        .resource::<Zm>()
        .players
        .iter()
        .map(|(n, p)| (*n, p.obj))
        .collect();
    let f = vm.intern("sessionteam");
    players
        .into_iter()
        .filter(|(_, o)| vm.to_text(&vm.raw_field(*o, f)) == team)
        .map(|(n, _)| ClientId(n))
        .collect()
}

fn player_audience(vm: &Vm<World>, world: &World, v: &Value) -> Option<EventAudience> {
    let n = entnum(vm, v)?;
    world
        .resource::<Zm>()
        .players
        .contains_key(&n)
        .then_some(EventAudience::Client(ClientId(n)))
}

/// An effect bolted to entity `n`'s tag (`playfxontag`'s engine half) or,
/// with `stop`, taken off it; false when the entity shows no model.
pub(super) fn bolt_effect(world: &mut World, n: u32, name: &str, tag: &str, stop: bool) -> bool {
    bolt_effect_to(world, EventAudience::All, n, name, tag, stop)
}

/// bo2mp: an effect on an entity's tag that only some players see (a
/// client script's `playfxontag` that differs by side).
pub(super) fn bolt_effect_to(
    world: &mut World,
    audience: EventAudience,
    n: u32,
    name: &str,
    tag: &str,
    stop: bool,
) -> bool {
    bolt_effect_flags(world, audience, n, name, tag, if stop { 0x80 } else { 0x40 })
}

/// bo2mp: the RC-XD's lights as it blows up: taken off its tag with what
/// they show gone at once (it hides that tick).
pub(super) fn bolt_effect_kill(world: &mut World, audience: EventAudience, n: u32, name: &str, tag: &str) -> bool {
    bolt_effect_flags(world, audience, n, name, tag, 0x88)
}

fn bolt_effect_flags(
    world: &mut World,
    audience: EventAudience,
    n: u32,
    name: &str,
    tag: &str,
    flags: u8,
) -> bool {
    let stop = flags & 0x80 != 0;
    let Some((number, model, angles, origin)) = shown(world, n) else {
        if std::env::var_os("BO2MP_T6_BOLTLOG").is_some() {
            diag::info!(Sim, "bo2zm t6 bolt: {name} on ent{n} not shown, flags {flags:#x} dropped");
        }
        return false;
    };
    let attached = world
        .resource::<Zm>()
        .ents
        .get(&n)
        .map(|e| e.attached.clone())
        .unwrap_or_default();
    let bone = composition_bone(world, &model, &attached, tag).unwrap_or(0);
    // BO2MP_T6_BOLTLOG=1: each effect bolted to a bone, and where.
    if std::env::var_os("BO2MP_T6_BOLTLOG").is_some() {
        diag::info!(
            Sim,
            "bo2zm t6 bolt: {name} on ent{n} {model} {attached:?} tag {tag} -> bone {bone}{}",
            if stop { " (off)" } else { "" }
        );
    }
    let forward = gsc_t6::math::angle_vectors(angles).0;
    let index = frame(world).effect_name_index(name);
    let t = tick(world);
    frame(world).push_entity_event(
        t,
        audience,
        entity_t6::EntityEventKind::PLAY_FX,
        crate::EntityEventPayload {
            number,
            event_parm: i32::from(index),
            origin,
            direction: forward,
            surf_type: u8::try_from(bone).unwrap_or(0),
            simulation_flags: flags,
            ..Default::default()
        },
    );
    true
}

/// bo2mp: BO2's `playviewmodelfx`: an effect on player `n`'s own gun's
/// `tag_target` that only he sees (0x10), or taken off it.
pub(super) fn viewmodel_effect(world: &mut World, n: u32, name: &str, stop: bool) {
    let index = frame(world).effect_name_index(name);
    let t = tick(world);
    frame(world).push_entity_event(
        t,
        EventAudience::Client(ClientId(n)),
        entity_t6::EntityEventKind::PLAY_FX,
        crate::EntityEventPayload {
            number: n as i32,
            event_parm: i32::from(index),
            simulation_flags: 0x10 | if stop { 0x80 } else { 0x40 },
            ..Default::default()
        },
    );
}

/// How entity `n` shows to its clients (its number, model, facing, place):
/// its own row, or the projectile it stands for.
pub(super) fn shown(world: &mut World, n: u32) -> Option<(i32, String, [f32; 3], [f32; 3])> {
    let own = {
        let zm = world.resource::<Zm>();
        zm.presences
            .by_ent
            .get(&n)
            .map(|s| (s.number, s.model.clone(), s.angles, s.origin))
    };
    own.or_else(|| super::killstreaks::thrown_shown(world, n))
}

/// A tag's bone index in an entity's composition (its model, then each
/// attached model, as its client numbers them).
fn composition_bone(
    world: &mut World,
    model: &str,
    attached: &[(String, String)],
    tag: &str,
) -> Option<usize> {
    let f = frame(world);
    let base = f.model_capability(model).flatten()?;
    let mut caps = vec![(base, None::<String>)];
    for (m, t) in attached {
        if let Some(c) = f.model_capability(m).flatten() {
            caps.push((c, Some(t.clone())));
        }
    }
    let models: Vec<(
        &xmodel_runtime::ModelPoseSrc,
        Option<xmodel_runtime::Attach>,
    )> = caps
        .iter()
        .map(|(c, t)| {
            let attach = t.as_ref().map(|t| xmodel_runtime::Attach {
                parent_model: 0,
                tag: if t.is_empty() {
                    xmodel_runtime::empty_tag_attach(&caps[0].0.pose, &c.pose)
                } else {
                    t.clone()
                },
            });
            (&c.pose, attach)
        })
        .collect();
    let dobj = xmodel_runtime::DObj::build(&models).ok()?;
    let want = tag.to_ascii_lowercase();
    dobj.bones
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case(&want))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // bo2mp: BO2's script rumbles, one event each to everyone (a client
    // feels one on himself, or a broadcast one within its range; the
    // killcam feels none). The rumble's name rides a sound alias slot
    // (`rumble:<name>`). Not yet as BO2: precacherumble's table (47 names,
    // shared with the weapons' and vehicles' own rumbles) and the error for
    // a name never in it: the weapons' rumble names are not in our facts
    // (remotemissile plays `sniper_fire`, which only a weapon registers).
    vm.bind("playrumbleonentity", true, |vm, world, s, a| {
        rumble_on(vm, world, s, a, entity_t6::EntityEventKind::PLAY_RUMBLE_ON_ENT)
    });
    vm.bind("playrumblelooponentity", true, |vm, world, s, a| {
        rumble_on(vm, world, s, a, entity_t6::EntityEventKind::PLAY_RUMBLELOOP_ON_ENT)
    });
    vm.bind("stoprumble", true, |vm, world, s, a| {
        rumble_on(vm, world, s, a, entity_t6::EntityEventKind::STOP_RUMBLE)
    });
    vm.bind("playrumbleonposition", false, |vm, world, _, a| {
        rumble_at(vm, world, a, entity_t6::EntityEventKind::PLAY_RUMBLE_ON_POS)
    });
    vm.bind("playrumblelooponposition", false, |vm, world, _, a| {
        rumble_at(vm, world, a, entity_t6::EntityEventKind::PLAY_RUMBLELOOP_ON_POS)
    });
    vm.bind("stopallrumbles", false, |_, world, _, _| {
        let world_num = i32::from(trace_t6::ENTITYNUM_WORLD);
        rumble_event(world, entity_t6::EntityEventKind::STOP_ALL_RUMBLES, world_num, None, [0.0; 3]);
        Ok(Value::Undefined)
    });
    // Is there such a sound alias (the scripts count an alias' variants
    // with it: `vox_..._0`, `_1`, ...)?
    vm.bind("soundexists", false, |vm, world, _, a| {
        let alias = text(vm, a, 0).to_ascii_lowercase();
        Ok(Value::bool(
            world.resource::<Zm>().sound_aliases.contains(&alias),
        ))
    });
    vm.bind("playsound", true, |vm, world, s, a| {
        let alias = text(vm, a, 0);
        let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        sound(world, EventAudience::All, &alias, origin);
        // A notify name: sent when the sound would be done (lengths are not
        // known here yet: a second).
        if let (Value::Str(_), Some(o)) = (arg(a, 1), s.as_obj()) {
            let n = text(vm, a, 1);
            super::natives_game::notify_later(vm, world, o, &n, 1000);
        }
        Ok(Value::Undefined)
    });
    vm.bind("playsoundwithnotify", true, |vm, world, s, a| {
        let alias = text(vm, a, 0);
        let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        sound(world, EventAudience::All, &alias, origin);
        if let Some(o) = s.as_obj() {
            let n = text(vm, a, 1);
            super::natives_game::notify_later(vm, world, o, &n, 1000);
        }
        Ok(Value::Undefined)
    });
    vm.bind("playsoundontag", true, |vm, world, s, a| {
        let alias = text(vm, a, 0);
        let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        sound(world, EventAudience::All, &alias, origin);
        Ok(Value::Undefined)
    });
    vm.bind("playsoundasmaster", true, |vm, world, s, a| {
        let alias = text(vm, a, 0);
        let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        sound(world, EventAudience::All, &alias, origin);
        Ok(Value::Undefined)
    });
    vm.bind("playsoundtoplayer", true, |vm, world, s, a| {
        let alias = text(vm, a, 0);
        let who = arg(a, 1).clone();
        let Some(audience) = player_audience(vm, world, &who) else {
            return Ok(Value::Undefined);
        };
        let origin = origin_of(vm, world, s)
            .or_else(|| origin_of(vm, world, &who))
            .unwrap_or([0.0; 3]);
        sound(world, audience, &alias, origin);
        Ok(Value::Undefined)
    });
    vm.bind("playlocalsound", true, |vm, world, s, a| {
        let alias = text(vm, a, 0);
        let Some(audience) = player_audience(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        local_sound(world, audience, &alias);
        Ok(Value::Undefined)
    });
    // bo2mp: Black Ops II checks the team only in Zombies; in multiplayer
    // everyone hears it but the player named third.
    vm.bind("playsoundtoteam", true, |vm, world, s, a| {
        let alias = text(vm, a, 0);
        let team = text(vm, a, 1);
        let except = match arg(a, 2) {
            Value::Undefined => None,
            v => Some(player_client(vm, world, v)?),
        };
        let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        let audience = if world.resource::<Zm>().mp {
            except.map_or(EventAudience::All, EventAudience::AllExcept)
        } else {
            let heard = team_clients(vm, world, &team);
            EventAudience::Clients(heard.into_iter().filter(|c| Some(*c) != except).collect())
        };
        sound(world, audience, &alias, origin);
        Ok(Value::Undefined)
    });
    // bo2mp: a battle chatter callout (_battlechatter_mp.gsc
    // dolocationalsound) that only the named team hears, at the speaker.
    // Its first line: Black Ops II's client chains the second after it,
    // and its only caller (mpsaylocationallocalsound) is never called.
    vm.bind("playbattlechattertoteam", true, |vm, world, s, a| {
        let first = text(vm, a, 0);
        let team = text(vm, a, 2);
        if !matches!(arg(a, 3), Value::Undefined) {
            player_client(vm, world, arg(a, 3))?;
        }
        let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        let heard = team_clients(vm, world, &team);
        sound(world, EventAudience::Clients(heard), &first, origin);
        Ok(Value::Undefined)
    });
    vm.bind("playsoundatposition", false, |vm, world, _, a| {
        let alias = text(vm, a, 0);
        let origin = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        sound(world, EventAudience::All, &alias, origin);
        Ok(Value::Undefined)
    });
    vm.bind("playsound", false, |vm, world, _, a| {
        // playsound(localclientnum or 0, alias, origin) on the server: the
        // alias and where.
        let alias = text(vm, a, 1);
        let origin = arg(a, 2).as_vec3().unwrap_or([0.0; 3]);
        sound(world, EventAudience::All, &alias, origin);
        Ok(Value::Undefined)
    });
    vm.bind("playfx", false, |vm, world, _, a| {
        let Some(name) = int(a, 0).ok().and_then(|id| fx_name(world, id)) else {
            return Ok(Value::Undefined);
        };
        let origin = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let forward = arg(a, 2).as_vec3().unwrap_or([0.0, 0.0, 1.0]);
        // bo2mp: BO2MP_FXLOG=1 logs every effect the scripts play (test aid,
        // like BO2MP_SNDLOG for sounds).
        if std::env::var_os("BO2MP_FXLOG").is_some() {
            diag::info!(Sim, "bo2mp fxlog play {name} at {:.0} {:.0} {:.0}", origin[0], origin[1], origin[2]);
        }
        match arg(a, 3).as_vec3() {
            Some(up) => effect_axis(world, &name, origin, forward, up),
            None => effect(world, &name, origin, forward),
        }
        let _ = vm;
        Ok(Value::Undefined)
    });
    // spawnfx(fx, origin, forward, up): an effect placed, started by
    // triggerfx.
    vm.bind("spawnfx", false, |vm, world, _, a| {
        let name = int(a, 0)
            .ok()
            .and_then(|id| fx_name(world, id))
            .unwrap_or_default();
        let origin = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let forward = arg(a, 2).as_vec3().unwrap_or([0.0, 0.0, 1.0]);
        let up = arg(a, 3).as_vec3();
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let mut zm = world.resource_mut::<Zm>();
        zm.ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                classname: "fx".into(),
                origin,
                ..Default::default()
            },
        );
        zm.fx_ents.insert(n, (name, origin, forward, up));
        Ok(Value::Object(obj))
    });
    // bo2mp: spawntimedfx(weapon, origin, dir = up, seconds = 10) (BO2's
    // `GScr_SpawnTimedFX`): an entity at the origin (each part cut to a
    // whole number) that plays the weapon's explosion effect along `dir`
    // and is freed when the seconds are up. Smoke grenades make their
    // cloud with it, then `fxblocksight` it.
    vm.bind("spawntimedfx", false, |vm, world, _, a| {
        let weapon = text(vm, a, 0);
        let p = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let origin = p.map(|v| v as i32 as f32);
        let d = arg(a, 2).as_vec3().unwrap_or([0.0, 0.0, 1.0]);
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dir = if len > 0.0 {
            d.map(|v| v / len)
        } else {
            [0.0, 0.0, 1.0]
        };
        let seconds = int(a, 3).unwrap_or(10);
        let fx = super::weapon(world, &weapon)
            .ok()
            .filter(|&w| w != 0)
            .and_then(|w| {
                frame(world)
                    .weapon_script_sounds(w)
                    .and_then(|s| s.proj_explosion_fx.clone())
            });
        match fx {
            Some(fx) => effect(world, &fx, origin, dir),
            None => diag::info!(Sim, "bo2mp spawntimedfx: {weapon} has no explosion effect"),
        }
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let mut zm = world.resource_mut::<Zm>();
        zm.ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                classname: "noclass".into(),
                origin,
                ..Default::default()
            },
        );
        let until = zm.now_ms + i64::from(seconds) * 1000;
        zm.timed_fx.insert(n, until);
        Ok(Value::Object(obj))
    });
    // bo2mp: fxblocksight(ent, radius) (BO2's `Scr_FXBlockSight`): the
    // entity's sphere of that radius blocks sight (bots', and bullet
    // traces that ask), or takes the new radius when it already does.
    vm.bind("fxblocksight", false, |vm, world, _, a| {
        let target = arg(a, 0).clone();
        if super::is_player(vm, world, &target) {
            return Err("FXSightBlock: entity must be ET_GENERAL type".into());
        }
        let Some(n) = entnum(vm, &target) else {
            return Ok(Value::Undefined);
        };
        let radius = num(a, 1)?;
        let mut zm = world.resource_mut::<Zm>();
        if let Some(b) = zm.vis_blockers.iter_mut().find(|b| b.0 == n) {
            b.1 = radius;
        } else if zm.vis_blockers.len() < VIS_BLOCKERS {
            zm.vis_blockers.push((n, radius));
        } else {
            diag::warn!(Sim, "SV_FX_SetVisBlocker: no free visibility blockers");
        }
        Ok(Value::Undefined)
    });
    vm.bind("triggerfx", false, |vm, world, _, a| {
        let Some(n) = entnum(vm, arg(a, 0)) else {
            return Ok(Value::Undefined);
        };
        let placed = world.resource::<Zm>().fx_ents.get(&n).cloned();
        if let Some((name, origin, forward, up)) = placed {
            effect_held(world, &name, origin, forward, up);
        }
        Ok(Value::Undefined)
    });
    vm.bind("activateclientexploder", false, |vm, world, _, a| {
        let id = arg(a, 0).clone();
        client_exploder(vm, world, &id);
        Ok(Value::Undefined)
    });
    vm.bind("playfxontag", false, |vm, world, _, a| {
        let Some(name) = int(a, 0).ok().and_then(|id| fx_name(world, id)) else {
            return Ok(Value::Undefined);
        };
        let who = arg(a, 1).clone();
        let tag = text(vm, a, 2);
        let origin = origin_of(vm, world, &who).unwrap_or([0.0; 3]);
        if std::env::var_os("BO2MP_FXLOG").is_some() {
            diag::info!(Sim, "bo2mp fxlog playontag {name} on ent {:?} tag {tag} at {:.0} {:.0} {:.0}", entnum(vm, &who), origin[0], origin[1], origin[2]);
        }
        // Bolted to the entity's shown model: its client keeps the effect
        // on that tag while the entity lives.
        if let Some(n) = entnum(vm, &who)
            && bolt_effect(world, n, &name, &tag, false)
        {
            return Ok(Value::Undefined);
        }
        effect(world, &name, origin, [0.0, 0.0, 1.0]);
        Ok(Value::Undefined)
    });
}

#[cfg(test)]
mod tests {
    use super::{rumble_payload, segment_in_sphere};

    #[test]
    fn a_rumble_spot_goes_out_in_whole_units() {
        let p = rumble_payload(1022, 7, [-15255.75, 2484.5, -0.9]);
        assert_eq!((p.number, p.event_parm), (1022, 7));
        // Each axis cut toward zero, as BO2 sends it.
        assert_eq!(p.origin, [-15255.0, 2484.0, -0.0]);
    }

    fn hits(a: [f32; 3], b: [f32; 3], centre: [f32; 3], radius: f32) -> bool {
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        segment_in_sphere(a, d, len2, centre, radius)
    }

    #[test]
    fn smoke_hides_a_line_through_its_cloud() {
        let (a, b) = ([0.0, 0.0, 0.0], [1000.0, 0.0, 0.0]);
        assert!(hits(a, b, [500.0, 60.0, 0.0], 64.0));
        assert!(!hits(a, b, [500.0, 70.0, 0.0], 64.0));
        // Strictly inside: a line just touching the cloud is not hidden.
        assert!(!hits(a, b, [500.0, 64.0, 0.0], 64.0));
        // The segment ends: a cloud past its end hides it only by the end.
        assert!(!hits(a, b, [1100.0, 0.0, 0.0], 64.0));
        assert!(hits(a, b, [1050.0, 0.0, 0.0], 64.0));
    }
}
