//! Sounds and effects the scripts play: entity events the client turns
//! into Black Ops II sound aliases and effects by name (the same events
//! MW2's scripts use).

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use entity_t6::EventCode;
use gsc_t6::{Key, ObjKind, ScriptValue, Vm};

use super::{Zm, arg, entnum, frame, int, num, origin_of, text, tick};
use crate::sim_parts::ClientId;
use crate::{EntityEventPayload, EventRecipients};

/// Play-effect flag: the effect keeps running until it is stopped.
const FX_HELD: u8 = 0x20;
/// Play-effect flag: the effect goes on the carrier's bone and follows it.
const FX_ATTACH: u8 = 0x40;
/// Play-effect flag: the effect comes off the carrier's bone.
const FX_DETACH: u8 = 0x80;
/// Detach, and also clear what the effect already put on screen.
const FX_DETACH_NOW: u8 = FX_DETACH | 0x08;
/// Play-effect flag: drawn on the owner's first-person gun, for the owner only.
const FX_VIEWMODEL: u8 = 0x10;

/// Number of the world entity, which carries events that belong to no entity.
fn world_number() -> i32 {
    i32::from(trace_t6::ENTITYNUM_WORLD)
}

/// Queues `payload` as a `kind` event in this tick, for `audience`.
fn publish(world: &mut World, audience: EventRecipients, kind: EventCode, payload: EntityEventPayload) {
    let now = tick(world);
    frame(world).post_entity_event(now, audience, kind, payload);
}

/// A payload carried by entity `number` with parameter `parm`; every other field zero.
fn carried(num_no: i32, parm: u8) -> EntityEventPayload {
    EntityEventPayload { number: num_no, event_parm: parm.into(), ..Default::default() }
}

/// A sound on entity `s` (`playsound`, `playsoundwithnotify`,
/// `playsoundontag`): a player's own sound (his character's quotes,
/// `_zm_audio`) is his own to him and comes from where he stands to the
/// others; any other entity's comes from where it stands.
fn entity_sound(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, snd_alias: &str) {
    let spot = origin_of(vm, game, st).unwrap_or([0.0; 3]);
    match entnum(vm, st).filter(|nn| game.resource::<Zm>().players.contains_key(nn)) {
        Some(nn) => {
            local_sound(game, EventRecipients::Client(ClientId(nn)), snd_alias);
            sound(game, EventRecipients::AllExcept(ClientId(nn)), snd_alias, spot);
        }
        None => sound(game, EventRecipients::All, snd_alias, spot),
    }
}

/// Plays script sound `alias` as an alias event on entity `number` at `origin`. `label` tags the debug log line
/// (`BO2MP_SNDLOG=1` lists every script sound, to compare with what the client starts).
fn play_alias(game: &mut World, audience: EventRecipients, snd_alias: &str, ident: &str, num_no: i32, spot: [f32; 3]) {
    if snd_alias.is_empty() {
        return;
    }
    if std::env::var_os("BO2MP_SNDLOG").is_some() {
        diag::info!(Sim, "bo2mp sndlog {ident} {snd_alias} {audience:?}");
    }
    // The client keeps one snd_alias table for several games; ours sit under the `t6:` prefix.
    let parm = frame(game).sound_alias_index(&format!("t6:{}", snd_alias.to_ascii_lowercase()));
    // BO2MP_T6_SNDLOG=<parts of names, comma-separated> (1 = all): each
    // server sound, so a headless run shows the map's sounds play (the
    // sign, the clock, the bus horn).
    if let Ok(want) = std::env::var("BO2MP_T6_SNDLOG")
        && (want == "1" || want.split(',').any(|w| snd_alias.contains(w)))
    {
        let now = game.resource::<Zm>().clock_millis;
        diag::info!(Sim, "bo2zm t6 sound {snd_alias} index {parm} at {spot:?} t={now}");
    }
    publish(game, audience, EventCode::SOUND_ALIAS, EntityEventPayload { origin: spot, ..carried(num_no, parm) });
}

/// A script sound heard in the world at `origin`.
pub(super) fn sound(game: &mut World, audience: EventRecipients, snd_alias: &str, spot: [f32; 3]) {
    play_alias(game, audience, snd_alias, "play", world_number(), spot);
}

/// A sound its listener hears as his own, not from a place: BO2's
/// `playlocalsound` (the announcer's lines, `_zm_audio_announcer`
/// `playleaderdialogonplayer`) and a player's sound to himself
/// (`self playsoundtoplayer(alias, self)`: his own quotes, `_zm_audio`).
/// Sent from the local sound entity, his client plays it unplaced; placed
/// where he stood, it faded as he ran on.
pub(super) fn local_sound(game: &mut World, audience: EventRecipients, snd_alias: &str) {
    play_alias(game, audience, snd_alias, "local", entity_t6::LOCAL_SOUND_ENTITY, [0.0; 3]);
}

/// Rumble `a[0]` started on the entity `s`. A player carries it under his client number, any other entity under
/// the number its clients see; an entity they are not shown gets no rumble.
fn rumble_on(
    vm: &mut Vm<World>,
    world: &mut World,
    s: &ScriptValue,
    a: &[ScriptValue],
    kind: EventCode,
) -> Result<ScriptValue, String> {
    let rumble = text(vm, a, 0);
    let ent = entnum(vm, s).ok_or("not an entity")?;
    let carrier = if world.resource::<Zm>().players.contains_key(&ent) {
        let at = origin_of(vm, world, s).unwrap_or_default();
        Some((ent as i32, at))
    } else {
        shown(world, ent).map(|(number, _, _, at)| (number, at))
    };
    if let Some((number, at)) = carrier {
        rumble_event(world, kind, number, Some(&rumble), at);
    }
    Ok(ScriptValue::Undefined)
}

/// Rumble `a[0]` started at the point `a[1]`, carried by the world entity.
fn rumble_at(vm: &mut Vm<World>, game: &mut World, lhs: &[ScriptValue], kind: EventCode) -> Result<ScriptValue, String> {
    let rumble = text(vm, lhs, 0);
    let at = arg(lhs, 1).as_vec3().ok_or("the rumble's position is not a vector")?;
    rumble_event(game, kind, world_number(), Some(&rumble), at);
    Ok(ScriptValue::Undefined)
}

/// Sends one rumble event to every client; with no name its parameter is 0. `BO2MP_RUMBLELOG=1` logs each.
fn rumble_event(game: &mut World, kind: EventCode, num_no: i32, label: Option<&str>, spot: [f32; 3]) {
    let parm = match label {
        Some(rumble) => frame(game).sound_alias_index(&format!("rumble:{}", rumble.to_ascii_lowercase())),
        None => 0,
    };
    if std::env::var_os("BO2MP_RUMBLELOG").is_some() {
        diag::info!(Sim, "bo2mp rumble {:#x} {} on {num_no} parm {parm}", kind.0, label.unwrap_or("-"));
    }
    publish(game, EventRecipients::All, kind, rumble_payload(num_no, parm, spot));
}

/// bo2mp: `playrumbleonposition( localclientnum, rumble, origin )` from a
/// client script: only that player feels it.
pub(super) fn rumble_to(game: &mut World, client: u32, rumble: &str, spot: [f32; 3]) {
    let parm = frame(game).sound_alias_index(&format!("rumble:{rumble}"));
    let to = EventRecipients::Client(crate::sim_parts::ClientId(client));
    publish(game, to, EventCode::PLAY_RUMBLE_ON_POS, rumble_payload(world_number(), parm, spot));
}

/// A rumble's payload. Its position is sent truncated to whole units.
fn rumble_payload(num_no: i32, parm: u8, spot: [f32; 3]) -> EntityEventPayload {
    EntityEventPayload { origin: spot.map(f32::trunc), ..carried(num_no, parm) }
}

const UP: [f32; 3] = [0.0, 0.0, 1.0];

/// `v` scaled to length 1; straight up when `v` has no length.
fn unit_or_up(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|c| c * c).sum::<f32>().sqrt();
    if length > 0.0 { v.map(|c| c / length) } else { UP }
}

/// A one-shot effect payload on the world entity: effect `name` at `origin`, pointing along `forward`.
fn world_fx(game: &mut World, ident: &str, spot: [f32; 3], ahead: [f32; 3]) -> EntityEventPayload {
    let parm = frame(game).effect_name_index(ident);
    EntityEventPayload { origin: spot, direction: ahead, ..carried(world_number(), parm) }
}

/// A one-shot effect for everyone, oriented by both `forward` and `up`.
pub(super) fn fx_basis(game: &mut World, ident: &str, spot: [f32; 3], ahead: [f32; 3], upward: [f32; 3]) {
    effect_axis_to(game, EventRecipients::All, ident, spot, ahead, upward);
}

/// A one-shot effect oriented by `forward` and `up`, for `audience` only (so each side of a match can be shown its
/// own version). `up` travels in the payload's second origin.
pub(super) fn effect_axis_to(
    game: &mut World,
    audience: EventRecipients,
    ident: &str,
    spot: [f32; 3],
    ahead: [f32; 3],
    up: [f32; 3],
) {
    if ident.is_empty() {
        return;
    }
    let fx_event = EntityEventPayload { origin2: up, ..world_fx(game, ident, spot, ahead) };
    publish(game, audience, EventCode::PLAY_FX, fx_event);
}

/// A lasting effect: a looping one keeps playing. With no `up` the client orients it by `forward` alone.
pub(super) fn effect_held(game: &mut World, ident: &str, spot: [f32; 3], ahead: [f32; 3], up: Option<[f32; 3]>) {
    if ident.is_empty() {
        return;
    }
    let fx_event = EntityEventPayload {
        origin2: up.unwrap_or_default(),
        wall_pass_bits: FX_HELD,
        ..world_fx(game, ident, spot, ahead)
    };
    publish(game, EventRecipients::All, EventCode::PLAY_FX, fx_event);
}

/// BO2's `stopfx` on an effect `playfx` played in the world (Buried's "?"
/// chalk when its wall buy is built): its client stops the held effect of
/// that name at that spot.
pub(super) fn effect_stop_world(game: &mut World, ident: &str, spot: [f32; 3]) {
    if ident.is_empty() {
        return;
    }
    let parm = frame(game).effect_name_index(ident);
    let fx_event = EntityEventPayload { origin: spot, wall_pass_bits: FX_DETACH, ..carried(world_number(), parm) };
    publish(game, EventRecipients::All, EventCode::PLAY_FX, fx_event);
}

/// A field of a script object.
fn obj_field(vm: &mut Vm<World>, o: &ScriptValue, name: &str) -> ScriptValue {
    let Some(o) = o.as_obj() else {
        return ScriptValue::Undefined;
    };
    let f = vm.intern(name);
    vm.raw_field(o, f)
}

/// `activateclientexploder(id)`: the client's half of an exploder (Nuketown's
/// perk machines landing). The client scripts would play each createfx
/// exploder's effects; the server's own functions for them play them here.
pub(super) fn client_exploder(vm: &mut Vm<World>, game: &mut World, cid: &ScriptValue) {
    let level = ScriptValue::Object(vm.level);
    let ids = obj_field(vm, &level, "_exploder_ids");
    let ScriptValue::Array(ids) = ids else { return };
    let num = {
        let ids = ids.read();
        ids.keys()
            .find(|k| ids.get(k).and_then(ScriptValue::as_int) == cid.as_int())
    };
    let Some(num) = num else { return };
    fire_exploder(vm, game, &num);
}

/// bo2mp: exploder `num`'s createfx effects and sounds, as the client's
/// `clientscripts\mp\_fx::exploder( num )` plays them.
pub(super) fn fire_exploder(vm: &mut Vm<World>, game: &mut World, num: &Key) {
    let level = ScriptValue::Object(vm.level);
    let ScriptValue::Array(all) = obj_field(vm, &level, "createfxexploders") else {
        return;
    };
    let Some(ScriptValue::Array(ents)) = all.get(num) else {
        return;
    };
    let ents: Vec<ScriptValue> = ents.read().values_in_order().cloned().collect();
    for en in ents {
        let ScriptValue::Array(v) = obj_field(vm, &en, "v") else {
            continue;
        };
        let has = |vm: &mut Vm<World>, k: &str| {
            let key = Key::Str(vm.intern(k));
            v.get(&key).filter(|x| !x.is_undefined())
        };
        let run = |vm: &mut Vm<World>, game: &mut World, fl: &str| {
            vm.spawn_named(game, "maps/mp/_utility", fl, en.clone(), vec![]);
        };
        if has(vm, "firefx").is_some() {
            run(vm, game, "fire_effect");
        }
        let fxid = has(vm, "fxid").map(|x| vm.to_text(&x));
        if fxid.is_some_and(|fl| fl != "No FX") {
            run(vm, game, "cannon_effect");
        } else if has(vm, "soundalias").is_some() {
            run(vm, game, "sound_effect");
        }
    }
}

/// bo2mp: where exploder `num`'s first effect is
/// (`level.createfxexploders[num][0].v["origin"]`).
pub(super) fn exploder_origin(vm: &mut Vm<World>, num: i32) -> Option<[f32; 3]> {
    let level = ScriptValue::Object(vm.level);
    let ScriptValue::Array(all) = obj_field(vm, &level, "createfxexploders") else {
        return None;
    };
    let Some(ScriptValue::Array(ents)) = all.get(&Key::Int(num)) else {
        return None;
    };
    let first = ents.get(&Key::Int(0))?;
    let ScriptValue::Array(v) = obj_field(vm, &first, "v") else {
        return None;
    };
    let key = Key::Str(vm.intern("origin"));
    v.get(&key)?.as_vec3()
}

pub(super) fn effect(game: &mut World, ident: &str, spot: [f32; 3], ahead: [f32; 3]) {
    if ident.is_empty() {
        return;
    }
    let fx_event = world_fx(game, ident, spot, ahead);
    publish(game, EventRecipients::All, EventCode::PLAY_FX, fx_event);
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
fn segment_in_sphere(lhs: [f32; 3], dd: [f32; 3], len2: f32, centre: [f32; 3], reach: f32) -> bool {
    let c = [centre[0] - lhs[0], centre[1] - lhs[1], centre[2] - lhs[2]];
    let t = ((c[0] * dd[0] + c[1] * dd[1] + c[2] * dd[2]) / len2).clamp(0.0, 1.0);
    let p = [dd[0] * t - c[0], dd[1] * t - c[1], dd[2] * t - c[2]];
    p[0] * p[0] + p[1] * p[1] + p[2] * p[2] < reach * reach
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
pub(super) fn free_timed_fx(vm: &mut Vm<World>, game: &mut World, now_ms: i64) {
    let due: Vec<u32> = {
        let zm = game.resource::<Zm>();
        zm.timed_fx
            .iter()
            .filter(|&(_, &t)| t < now_ms)
            .map(|(&n, _)| n)
            .collect()
    };
    for n in due {
        let obj = game.resource::<Zm>().ents.get(&n).and_then(|e| e.obj);
        if let Some(o) = obj
            && vm.alive(o)
        {
            vm.notify_queued(game, o, "death", &[]);
        }
        forget_timed(game, n);
        {
            let mut zm = game.resource_mut::<Zm>();
            zm.movers.list.remove(&n);
            zm.ents.remove(&n);
        }
        if let Some(o) = obj {
            vm.free_object(game, o);
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
fn player_client(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Result<ClientId, String> {
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
    let fl = vm.intern("sessionteam");
    players
        .into_iter()
        .where_value(|o| vm.to_text(&vm.raw_field(*o, fl)) == team)
        .map(|(n, _)| ClientId(n))
        .collect()
}

fn player_audience(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Option<EventRecipients> {
    let n = entnum(vm, v)?;
    world
        .resource::<Zm>()
        .players
        .contains_key(&n)
        .then_some(EventRecipients::Client(ClientId(n)))
}

/// Puts effect `name` on bone `tag` of entity `n` for everyone, or takes it off with `stop`. False when the entity
/// shows no model.
pub(super) fn bolt_effect(world: &mut World, n: u32, name: &str, tag: &str, stop: bool) -> bool {
    bolt_effect_to(world, EventRecipients::All, n, name, tag, stop)
}

/// As [`bolt_effect`], for `audience` only (a bone effect that looks different to each side, as the semtex's light).
pub(super) fn bolt_effect_to(
    world: &mut World,
    audience: EventRecipients,
    n: u32,
    name: &str,
    tag: &str,
    stop: bool,
) -> bool {
    let flags = if stop { FX_DETACH } else { FX_ATTACH };
    bolt_effect_flags(world, audience, n, name, tag, flags)
}

/// Takes effect `name` off bone `tag` of entity `n` and clears what it already drew, for an entity that vanishes
/// in the same tick (lights on a vehicle that blows up).
pub(super) fn bolt_effect_kill(world: &mut World, audience: EventRecipients, n: u32, name: &str, tag: &str) -> bool {
    bolt_effect_flags(world, audience, n, name, tag, FX_DETACH_NOW)
}

/// `bolt_effect`'s stop, but at once: its particles and models go too
/// (BO1's `deletefx(lc, fx, 1)`), where a stop only ends its spawning.
/// Flag 0x08 is the engine's kill-at-once bit (0x10 is playviewmodelfx).
pub(super) fn kill_bolted(world: &mut World, n: u32, name: &str, tag: &str) -> bool {
    bolt_effect_kill(world, EventRecipients::All, n, name, tag)
}

/// Sends a bone effect event with `flags` on entity `n`, its bone index found in the entity's whole model
/// composition (a missing tag means bone 0). `BO2MP_T6_BOLTLOG=1` logs each one.
fn bolt_effect_flags(game: &mut World, audience: EventRecipients, n: u32, name: &str, tag: &str, flags: u8) -> bool {
    let logging = std::env::var_os("BO2MP_T6_BOLTLOG").is_some();
    let shown = game.resource::<Zm>().presences.by_ent.get(&n).map(|p| (p.num, p.mesh.clone(), p.rot, p.spot));
    // Not a shown model: a thrown grenade, bolted to its projectile (the
    // planted claymore's lasers fan out of its tag_fx, not up).
    let shown = shown.or_else(|| {
        let (entnum, weapon, origin, angles) = super::grenades::projectile_of(game, n)?;
        let model = frame(game).weapon_projectile_model(weapon).to_owned();
        Some((entnum, model, angles, origin))
    });
    let Some((number, model, angles, origin)) = shown else {
        if logging {
            diag::info!(Sim, "bo2zm t6 bolt: {name} on ent{n} not shown, flags {flags:#x} dropped");
        }
        return false;
    };
    let hung = match game.resource::<Zm>().ents.get(&n) {
        Some(ent) => ent.attached.clone(),
        None => Vec::new(),
    };
    let bone = composition_bone(game, &model, &hung, tag).unwrap_or(0);
    if logging {
        let off = if flags & FX_DETACH != 0 { " (off)" } else { "" };
        diag::info!(Sim, "bo2zm t6 bolt: {name} on ent{n} {model} {hung:?} tag {tag} -> bone {bone}{off}");
    }
    let (facing, _, _) = gsc_t6::math::facing_axes(angles);
    let parm = frame(game).effect_name_index(name);
    let fx_event = EntityEventPayload {
        origin,
        direction: facing,
        surf_type: u8::try_from(bone).unwrap_or(0),
        wall_pass_bits: flags,
        ..carried(number, parm)
    };
    publish(game, audience, EventCode::PLAY_FX, fx_event);
    true
}

/// Puts effect `name` on (or with `stop`, takes it off) the `tag_target` bone of player `n`'s first-person gun.
/// Only that player is sent it.
pub(super) fn viewmodel_effect(game: &mut World, nn: u32, name: &str, stop: bool) {
    let parm = frame(game).effect_name_index(name);
    let flags = FX_VIEWMODEL | if stop { FX_DETACH } else { FX_ATTACH };
    let fx_event = EntityEventPayload { wall_pass_bits: flags, ..carried(nn as i32, parm) };
    publish(game, EventRecipients::Client(ClientId(nn)), EventCode::PLAY_FX, fx_event);
}

/// What clients are shown for entity `n`, as (number, model, angles, origin): its own presence row, or failing
/// that the thrown projectile it stands for.
pub(super) fn shown(world: &mut World, n: u32) -> Option<(i32, String, [f32; 3], [f32; 3])> {
    let row = world.resource::<Zm>().presences.by_ent.get(&n).map(|p| (p.num, p.mesh.clone(), p.rot, p.spot));
    match row {
        Some(found) => Some(found),
        None => super::killstreaks::thrown_shown(world, n),
    }
}

/// Index of bone `tag` (any case) in the skeleton a client builds for `model` with the `(model, tag)` meshes in
/// `hung` mounted on it, in that order. A mesh whose model is not loaded is left out; an empty mount tag picks
/// the default mount point.
fn composition_bone(game: &mut World, mdl: &str, hung: &[(String, String)], tag_name: &str) -> Option<usize> {
    let f = frame(game);
    let root = f.model_capability(mdl).flatten()?;
    let mounted: Vec<_> =
        hung.iter().filter_map(|(mesh, on)| Some((f.model_capability(mesh).flatten()?, on.as_str()))).collect();
    let mut pieces = Vec::with_capacity(mounted.len() + 1);
    pieces.push((&root.pose, None));
    for (cap, on) in &mounted {
        let mount_on =
            if on.is_empty() { xmodel_runtime::empty_tag_attach(&root.pose, &cap.pose) } else { (*on).to_owned() };
        pieces.push((&cap.pose, Some(xmodel_runtime::MountedModel { host_model: 0, mount_tag: mount_on })));
    }
    let skeleton = xmodel_runtime::PosedSkeleton::build(&pieces).ok()?;
    skeleton.bones.iter().position(|bone| bone.name.eq_ignore_ascii_case(tag_name))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // bo2mp: BO2's script rumbles, one event each to everyone (a client
    // feels one on himself, or a broadcast one within its range; the
    // killcam feels none). The rumble's ident rides a sound snd_alias slot
    // (`rumble:<ident>`). Not yet as BO2: precacherumble's table (47 names,
    // shared with the weapons' and vehicles' own rumbles) and the error for
    // a ident never in it: the weapons' rumble names are not in our facts
    // (remotemissile plays `sniper_fire`, which only a weapon registers).
    vm.bind("playrumbleonentity", true, |vm, sim, s, a| {
        rumble_on(vm, sim, s, a, entity_t6::EventCode::PLAY_RUMBLE_ON_ENT)
    });
    vm.bind("playrumblelooponentity", true, |vm, sim, s, a| {
        rumble_on(vm, sim, s, a, entity_t6::EventCode::PLAY_RUMBLELOOP_ON_ENT)
    });
    vm.bind("stoprumble", true, |vm, sim, s, a| {
        rumble_on(vm, sim, s, a, entity_t6::EventCode::STOP_RUMBLE)
    });
    vm.bind("playrumbleonposition", false, |vm, sim, _, a| {
        rumble_at(vm, sim, a, entity_t6::EventCode::PLAY_RUMBLE_ON_POS)
    });
    vm.bind("playrumblelooponposition", false, |vm, sim, _, a| {
        rumble_at(vm, sim, a, entity_t6::EventCode::PLAY_RUMBLELOOP_ON_POS)
    });
    vm.bind("stopallrumbles", false, |_, sim, _, _| {
        let world_num = i32::from(trace_t6::ENTITYNUM_WORLD);
        rumble_event(sim, entity_t6::EventCode::STOP_ALL_RUMBLES, world_num, None, [0.0; 3]);
        Ok(ScriptValue::Undefined)
    });
    // Is there such a sound snd_alias (the scripts count an snd_alias' variants
    // with it: `vox_..._0`, `_1`, ...)?
    vm.bind("soundexists", false, |vm, sim, _, a| {
        let snd_alias = text(vm, a, 0).to_ascii_lowercase();
        Ok(ScriptValue::bool(
            sim.resource::<Zm>().sound_aliases.contains(&snd_alias),
        ))
    });
    // An snd_alias' length in ms (the VO threads wait it out). Lengths are not
    // known here: the same second playsound's notify uses; -1 when there is
    // no such snd_alias.
    vm.bind("soundgetplaybacktime", false, |vm, sim, _, a| {
        let snd_alias = text(vm, a, 0).to_ascii_lowercase();
        let known = sim.resource::<Zm>().sound_aliases.contains(&snd_alias);
        Ok(ScriptValue::Int(if known { 1000 } else { -1 }))
    });
    vm.bind("playsound", true, |vm, sim, s, a| {
        let snd_alias = text(vm, a, 0);
        entity_sound(vm, sim, s, &snd_alias);
        // A notify ident: sent when the sound would be done (lengths are not
        // known here yet: a second).
        if let (ScriptValue::Str(_), Some(o)) = (arg(a, 1), s.as_obj()) {
            let n = text(vm, a, 1);
            super::natives_game::notify_later(vm, sim, o, &n, 1000);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playsoundwithnotify", true, |vm, sim, s, a| {
        let snd_alias = text(vm, a, 0);
        entity_sound(vm, sim, s, &snd_alias);
        if let Some(o) = s.as_obj() {
            let n = text(vm, a, 1);
            super::natives_game::notify_later(vm, sim, o, &n, 1000);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playsoundontag", true, |vm, sim, s, a| {
        let snd_alias = text(vm, a, 0);
        entity_sound(vm, sim, s, &snd_alias);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playsoundasmaster", true, |vm, sim, s, a| {
        let snd_alias = text(vm, a, 0);
        let spot = origin_of(vm, sim, s).unwrap_or([0.0; 3]);
        sound(sim, EventRecipients::All, &snd_alias, spot);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playsoundtoplayer", true, |vm, sim, s, a| {
        let snd_alias = text(vm, a, 0);
        let who = arg(a, 1).clone();
        let Some(audience) = player_audience(vm, sim, &who) else {
            return Ok(ScriptValue::Undefined);
        };
        // On himself, to himself: his own line.
        if entnum(vm, s).is_some() && entnum(vm, s) == entnum(vm, &who) {
            local_sound(sim, audience, &snd_alias);
            return Ok(ScriptValue::Undefined);
        }
        let spot = origin_of(vm, sim, s)
            .or_else(|| origin_of(vm, sim, &who))
            .unwrap_or([0.0; 3]);
        sound(sim, audience, &snd_alias, spot);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playlocalsound", true, |vm, sim, s, a| {
        let snd_alias = text(vm, a, 0);
        let Some(audience) = player_audience(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        local_sound(sim, audience, &snd_alias);
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: Black Ops II checks the team only in Zombies; in multiplayer
    // everyone hears it but the player named third.
    vm.bind("playsoundtoteam", true, |vm, sim, s, a| {
        let snd_alias = text(vm, a, 0);
        let team = text(vm, a, 1);
        let except = match arg(a, 2) {
            ScriptValue::Undefined => None,
            v => Some(player_client(vm, sim, v)?),
        };
        let spot = origin_of(vm, sim, s).unwrap_or([0.0; 3]);
        let audience = if sim.resource::<Zm>().mp {
            except.map_or(EventRecipients::All, EventRecipients::AllExcept)
        } else {
            let heard = team_clients(vm, sim, &team);
            EventRecipients::Clients(heard.into_iter().filter(|c| Some(*c) != except).collect())
        };
        sound(sim, audience, &snd_alias, spot);
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: a battle chatter callout (_battlechatter_mp.gsc
    // dolocationalsound) that only the named team hears, at the speaker.
    // Its first line: Black Ops II's client chains the second after it,
    // and its only caller (mpsaylocationallocalsound) is never called.
    vm.bind("playbattlechattertoteam", true, |vm, sim, s, a| {
        let first = text(vm, a, 0);
        let team = text(vm, a, 2);
        if !matches!(arg(a, 3), ScriptValue::Undefined) {
            player_client(vm, sim, arg(a, 3))?;
        }
        let spot = origin_of(vm, sim, s).unwrap_or([0.0; 3]);
        let heard = team_clients(vm, sim, &team);
        sound(sim, EventRecipients::Clients(heard), &first, spot);
        Ok(ScriptValue::Undefined)
    });
    // earthquake(scale, duration, spot, radius): the perks landing, the
    // game-over nuke. Multiplayer binds its own (killstreaks::bind, which
    // also takes a fifth player argument); this one is for Zombies, and sends
    // the same earthquake event through `killstreaks::shake`.
    vm.bind("earthquake", false, |_, sim, _, a| {
        let (scale, seconds, radius) = (
            arg(a, 0).as_float().unwrap_or(0.0),
            arg(a, 1).as_float().unwrap_or(0.0),
            arg(a, 3).as_float().unwrap_or(0.0),
        );
        if scale > 0.0 && seconds > 0.0 && radius > 0.0 {
            let spot = arg(a, 2).as_vec3().unwrap_or([0.0; 3]);
            super::killstreaks::shake(sim, scale, seconds, spot, radius)?;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playsoundatposition", false, |vm, sim, _, a| {
        let snd_alias = text(vm, a, 0);
        let spot = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        sound(sim, EventRecipients::All, &snd_alias, spot);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playsound", false, |vm, sim, _, a| {
        // playsound(localclientnum or 0, snd_alias, spot) on the server: the
        // snd_alias and where.
        let snd_alias = text(vm, a, 1);
        let spot = arg(a, 2).as_vec3().unwrap_or([0.0; 3]);
        sound(sim, EventRecipients::All, &snd_alias, spot);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playfx", false, |vm, sim, _, a| {
        let Some(ident) = int(a, 0).ok().and_then(|id| fx_name(sim, id)) else {
            return Ok(ScriptValue::Undefined);
        };
        let spot = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let ahead = arg(a, 2).as_vec3().unwrap_or([0.0, 0.0, 1.0]);
        // bo2mp: BO2MP_FXLOG=1 logs every effect the scripts play (test aid,
        // like BO2MP_SNDLOG for sounds).
        if std::env::var_os("BO2MP_FXLOG").is_some() {
            diag::info!(Sim, "bo2mp fxlog play {ident} at {:.0} {:.0} {:.0}", spot[0], spot[1], spot[2]);
        }
        match arg(a, 3).as_vec3() {
            Some(up) => fx_basis(sim, &ident, spot, ahead, up),
            None => effect(sim, &ident, spot, ahead),
        }
        let _ = vm;
        Ok(ScriptValue::Undefined)
    });
    // spawnfx(fx, spot, ahead, up): an effect placed, started by
    // triggerfx.
    vm.bind("spawnfx", false, |vm, sim, _, a| {
        let ident = int(a, 0)
            .ok()
            .and_then(|id| fx_name(sim, id))
            .unwrap_or_default();
        let spot = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let ahead = arg(a, 2).as_vec3().unwrap_or([0.0, 0.0, 1.0]);
        let up = arg(a, 3).as_vec3();
        let n = sim.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let mut zm = sim.resource_mut::<Zm>();
        zm.ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                class_tag: "fx".into(),
                origin: spot,
                ..Default::default()
            },
        );
        zm.fx_ents.insert(n, super::PlacedFx { ident, spot, ahead, up });
        Ok(ScriptValue::Object(obj))
    });
    // bo2mp: spawntimedfx(weapon, spot, dir = up, seconds = 10) (BO2's
    // `GScr_SpawnTimedFX`): an entity at the spot (each part cut to a
    // whole number) that plays the weapon's explosion effect along `dir`
    // and is freed when the seconds are up. Smoke grenades make their
    // cloud with it, then `fxblocksight` it.
    vm.bind("spawntimedfx", false, |vm, sim, _, a| {
        let weapon = text(vm, a, 0);
        let p = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let spot = p.map(|v| v as i32 as f32);
        let dir = unit_or_up(arg(a, 2).as_vec3().unwrap_or(UP));
        let seconds = int(a, 3).unwrap_or(10);
        let fx = super::weapon(sim, &weapon)
            .ok()
            .filter(|&w| w != 0)
            .and_then(|w| {
                frame(sim)
                    .weapon_script_sounds(w)
                    .and_then(|s| s.proj_explosion_fx.clone())
            });
        match fx {
            Some(fx) => effect(sim, &fx, spot, dir),
            None => diag::info!(Sim, "bo2mp spawntimedfx: {weapon} has no explosion effect"),
        }
        let n = sim.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let mut zm = sim.resource_mut::<Zm>();
        zm.ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                class_tag: "noclass".into(),
                origin: spot,
                ..Default::default()
            },
        );
        let until = zm.clock_millis + i64::from(seconds) * 1000;
        zm.timed_fx.insert(n, until);
        Ok(ScriptValue::Object(obj))
    });
    // bo2mp: fxblocksight(ent, radius) (BO2's `Scr_FXBlockSight`): the
    // entity's sphere of that radius blocks sight (bots', and bullet
    // traces that ask), or takes the new radius when it already does.
    vm.bind("fxblocksight", false, |vm, sim, _, a| {
        let target = arg(a, 0).clone();
        if super::is_player(vm, sim, &target) {
            return Err("FXSightBlock: entity must be ET_GENERAL type".into());
        }
        let Some(n) = entnum(vm, &target) else {
            return Ok(ScriptValue::Undefined);
        };
        let radius = num(a, 1)?;
        let mut zm = sim.resource_mut::<Zm>();
        if let Some(b) = zm.vis_blockers.iter_mut().find(|b| b.0 == n) {
            b.1 = radius;
        } else if zm.vis_blockers.len() < VIS_BLOCKERS {
            zm.vis_blockers.push((n, radius));
        } else {
            diag::warn!(Sim, "SV_FX_SetVisBlocker: no free visibility blockers");
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("triggerfx", false, |vm, sim, _, a| {
        let Some(n) = entnum(vm, arg(a, 0)) else {
            return Ok(ScriptValue::Undefined);
        };
        let placed = sim.resource::<Zm>().fx_ents.get(&n).cloned();
        if let Some(super::PlacedFx { ident, spot, ahead, up }) = placed {
            effect_held(sim, &ident, spot, ahead, up);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("activateclientexploder", false, |vm, sim, _, a| {
        let id = arg(a, 0).clone();
        client_exploder(vm, sim, &id);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("playfxontag", false, |vm, sim, _, a| {
        let Some(ident) = int(a, 0).ok().and_then(|id| fx_name(sim, id)) else {
            return Ok(ScriptValue::Undefined);
        };
        let who = arg(a, 1).clone();
        let tag_name = text(vm, a, 2);
        let spot = origin_of(vm, sim, &who).unwrap_or([0.0; 3]);
        if std::env::var_os("BO2MP_FXLOG").is_some() {
            diag::info!(Sim, "bo2mp fxlog playontag {ident} on ent {:?} tag {tag_name} at {:.0} {:.0} {:.0}", entnum(vm, &who), spot[0], spot[1], spot[2]);
        }
        // Bolted to the entity's shown model: its client keeps the effect
        // on that tag_name while the entity lives.
        if let Some(n) = entnum(vm, &who) {
            if bolt_effect(sim, n, &ident, &tag_name, false) {
                return Ok(ScriptValue::Undefined);
            }
            // A model spawned this tick is drawn from the end of the tick:
            // bolt the effect to it then, not at its spot in the sim (a
            // looping effect there would stay after the entity is deleted:
            // Mob's revive effect, `afterlife_revive_do_revive`'s tag_origin
            // model, kept striking after the revive).
            let pending = sim
                .resource::<Zm>()
                .ents
                .get(&n)
                .is_some_and(|e| !e.mesh.is_empty());
            if pending {
                super::clientfields::bolt_named_queued(sim, n, &ident, &tag_name);
                return Ok(ScriptValue::Undefined);
            }
        }
        if std::env::var_os("BO2MP_T6_BOLTLOG").is_some() {
            diag::info!(Sim, "bo2zm t6 bolt: {ident} at {spot:?} (no model: played where he stands)");
        }
        effect(sim, &ident, spot, [0.0, 0.0, 1.0]);
        Ok(ScriptValue::Undefined)
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

    fn hits(lhs: [f32; 3], rhs: [f32; 3], centre: [f32; 3], reach: f32) -> bool {
        let d = [rhs[0] - lhs[0], rhs[1] - lhs[1], rhs[2] - lhs[2]];
        let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        segment_in_sphere(lhs, d, len2, centre, reach)
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
