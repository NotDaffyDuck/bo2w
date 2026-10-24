//! Entities: the map's entities and structs at level start, `spawn`,
//! `getent(array)`, models, visibility, links, movers, triggers' hints.

use bevy_ecs::prelude::World;
use gsc_t6::{Key, ObjKind, ObjRef, Value, Vm};

use super::{Ent, Zm, arg, entnum, flag, is_player, list, num, origin_of, text, vec3};

type R = Result<Value, String>;

/// Map entity keys the engine types (the rest stay strings).
fn typed(vm: &mut Vm<World>, key: &str, val: &str, entity: bool) -> Value {
    let floats = |s: &str| -> Option<[f32; 3]> {
        let p: Vec<f32> = s
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        (p.len() == 3).then(|| [p[0], p[1], p[2]])
    };
    match key {
        // The engine's vector fields: a door's open turn (`script_angles`)
        // and a debris pile's slide (`script_vector`) go to rotateto/moveto.
        "origin" | "angles" | "script_angles" | "script_vector" => match floats(val) {
            Some(v) => Value::Vec3(v),
            None => vm.string(val),
        },
        "spawnflags" | "count" | "health" | "dmg" | "script_forcespawn" | "script_int"
            if entity =>
        {
            Value::Int(val.trim().parse().unwrap_or(0))
        }
        "radius" | "height" | "speed" | "wait" | "delay" if entity => {
            Value::Float(val.trim().parse().unwrap_or(0.0))
        }
        // Names stay text even when they look like numbers.
        "targetname" | "target" | "script_noteworthy" | "script_string" | "classname" | "model"
        | "script_flag" | "script_label" | "script_parameters" | "script_linkto"
        | "script_linkname" | "script_sound" | "script_fxid" | "script_animation" => vm.string(val),
        // Numbers are numbers (BO2's scripts compare `self.zombie_cost` with
        // a score and halve it; `trigger.script_chance` with a randomfloat).
        _ => {
            let t = val.trim();
            if let Ok(i) = t.parse::<i32>() {
                Value::Int(i)
            } else if let Ok(f) = t.parse::<f32>()
                && t.contains('.')
            {
                Value::Float(f)
            } else {
                vm.string(val)
            }
        }
    }
}

/// Create the map's entities (`script_struct` blocks through
/// `codescripts/struct::createstruct`); path nodes are kept for the AI.
pub(super) fn spawn_map_entities(vm: &mut Vm<World>, world: &mut World, texts: &[String]) {
    let createstruct = vm
        .program
        .find(&vm.strings, "codescripts/struct", "createstruct");
    let struct_s = vm.intern("struct");
    let (mut ents, mut structs, mut nodes) = (0, 0, 0);
    for text in texts {
        for e in asset_t6_entities(text) {
            let cls = e
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("classname"))
                .map_or("", |(_, v)| v.as_str())
                .to_owned();
            if cls.starts_with("node_") {
                nodes += 1;
                continue;
            }
            // bo2mp: the map's north (getnorthyaw: the minimap's turn).
            if cls == "worldspawn" {
                if let Some((_, v)) = e.iter().find(|(k, _)| k.eq_ignore_ascii_case("northyaw")) {
                    vm.dvars.insert("northyaw".to_owned(), v.trim().to_owned());
                }
            }
            if cls == "script_struct" {
                let Some(cs) = createstruct else { continue };
                let level = Value::Object(vm.level);
                vm.spawn(world, cs, level, vec![]);
                let Value::Array(a) = vm.raw_field(vm.level, struct_s) else {
                    continue;
                };
                let Some(Value::Object(s)) = a.get(&Key::Int(a.len() as i32 - 1)) else {
                    continue;
                };
                for (k, v) in &e {
                    let key = k.to_ascii_lowercase();
                    if key == "classname" {
                        continue;
                    }
                    let val = typed(vm, &key, v, false);
                    let f = vm.intern(&key);
                    vm.set_raw_field(s, f, val);
                }
                structs += 1;
                continue;
            }
            let n = world.resource_mut::<Zm>().alloc_entnum();
            let obj = vm.alloc_object(ObjKind::Entity(n));
            // BO2's SP_script_origin: no contents, and never sent to a client until a script
            // calls show(), even with a model key (mp_drone's satellite dish).
            // BO2 has no spawn function for "model" (mp_nightclub's two wire spools): it keeps
            // its fields but is never linked, so never sent and never solid.
            let origin_only = cls == "script_origin" || cls == "model";
            let mut ent = Ent {
                obj: Some(obj),
                classname: cls.clone(),
                map: true,
                solid: !origin_only,
                hidden: origin_only,
                ..Default::default()
            };
            for (k, v) in &e {
                let key = k.to_ascii_lowercase();
                match key.as_str() {
                    "classname" => continue,
                    "origin" => {
                        if let Value::Vec3(p) = typed(vm, "origin", v, true) {
                            ent.origin = p;
                        }
                        continue;
                    }
                    "angles" => {
                        if let Value::Vec3(p) = typed(vm, "angles", v, true) {
                            ent.angles = p;
                        }
                        continue;
                    }
                    "angle" => {
                        ent.angles = [0.0, v.trim().parse().unwrap_or(0.0), 0.0];
                        continue;
                    }
                    "model" if v.starts_with('*') => ent.brush = Some(v.clone()),
                    "model" => {
                        ent.model = v.clone();
                        continue;
                    }
                    "radius" => ent.radius = v.trim().parse().unwrap_or(0.0),
                    "height" => ent.height = v.trim().parse().unwrap_or(0.0),
                    // bo2mp: breaks piece by piece (t6/destructible.rs).
                    "destructibledef" => ent.destructible = Some(v.trim().to_ascii_lowercase()),
                    "codmp_dyn_destroyed" => {
                        ent.dyn_destroyed = Some(v.clone());
                        continue;
                    }
                    _ => {}
                }
                let val = typed(vm, &key, v, true);
                let f = vm.intern(&key);
                vm.set_raw_field(obj, f, val);
            }
            if cls.starts_with("zbarrier") {
                ent.zbarrier = Some(super::zbarrier::ZBarrier::from_keys(&e));
            }
            // bo2mp: a Black Ops II breakable starts with its base piece's
            // health (DestructibleUpdate); its pieces pass hits on to it.
            let health = ent
                .destructible
                .as_deref()
                .filter(|d| !super::destructible::is_dyn(d))
                .and_then(|d| world.resource::<Zm>().destructibles.get(d).cloned())
                .and_then(|def| def.pieces.first().map(|p| p.health));
            if let Some(health) = health {
                let f = vm.intern("health");
                vm.set_raw_field(obj, f, Value::Int(health));
            }
            // bo2mp: BO2's SP_trigger_damage: a shoot-to-use trigger keeps
            // 32000 health (t6/damage_triggers.rs); a "wait" of 0 or less
            // frees it once it has fired (spawnflags 0x200).
            if cls == "trigger_damage" {
                let f = vm.intern("health");
                vm.set_raw_field(obj, f, Value::Int(32000));
                let wait = e
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("wait"))
                    .and_then(|(_, v)| v.trim().parse::<f32>().ok());
                if wait.is_some_and(|w| w <= 0.0) {
                    let sf = vm.intern("spawnflags");
                    let flags = vm.raw_field(obj, sf).as_int().unwrap_or(0);
                    vm.set_raw_field(obj, sf, Value::Int(flags | 0x200));
                }
            }
            world.resource_mut::<Zm>().ents.insert(n, ent);
            ents += 1;
        }
    }
    diag::info!(
        Sim,
        "bo2zm t6: map {ents} entities, {structs} structs, {nodes} path nodes (kept for the AI)"
    );
}

/// The map's entity blocks as key/value lists.
fn asset_t6_entities(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<(String, String)>> = None;
    let mut quoted = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                cur = Some(Vec::new());
                quoted.clear();
            }
            '}' => {
                if let Some(e) = cur.take() {
                    out.push(e);
                }
                quoted.clear();
            }
            '"' => {
                let mut s = String::new();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                quoted.push(s);
                if quoted.len() == 2 {
                    let v = quoted.pop().unwrap();
                    let k = quoted.pop().unwrap();
                    if let Some(e) = cur.as_mut() {
                        e.push((k, v));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Entities whose `key` equals `value` (all with no key).
pub(super) fn matching(vm: &mut Vm<World>, world: &mut World, a: &[Value]) -> Vec<Value> {
    let want = match arg(a, 0) {
        Value::Undefined => None,
        v => Some(vm.to_text(v)),
    };
    let key = match arg(a, 1) {
        Value::Undefined => None,
        v => Some(vm.to_text(v).to_ascii_lowercase()),
    };
    let ents: Vec<(u32, ObjRef, String)> = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter_map(|(n, e)| e.obj.map(|o| (*n, o, e.classname.clone())))
        .collect();
    let players: Vec<ObjRef> = world
        .resource::<Zm>()
        .players
        .values()
        .map(|p| p.obj)
        .collect();
    let mut out = Vec::new();
    let (Some(want), Some(key)) = (want, key) else {
        return ents.iter().map(|(_, o, _)| Value::Object(*o)).collect();
    };
    if key == "classname" {
        if want == "player" {
            return players.into_iter().map(Value::Object).collect();
        }
        for (_, o, c) in &ents {
            if *c == want && vm.alive(*o) {
                out.push(Value::Object(*o));
            }
        }
        return out;
    }
    let Some(k) = vm.strings.find(&key) else {
        return out;
    };
    let Some(w) = vm.strings.find(&want) else {
        return out;
    };
    for (_, o, _) in ents {
        if !vm.alive(o) {
            continue;
        }
        if let Value::Str(s) = vm.raw_field(o, k)
            && s == w
        {
            out.push(Value::Object(o));
        }
    }
    out
}

/// A tag's place in a model at rest (model space); none for its origin.
fn tag_in_model(world: &mut World, model: &str, tag: &str) -> Option<[f32; 3]> {
    if model.is_empty() || tag.is_empty() || tag.eq_ignore_ascii_case("tag_origin") {
        return None;
    }
    let cap = super::frame(world).model_capability(model)??;
    let bone = cap
        .pose
        .bone_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(tag))?;
    cap.pose.base_mat.get(bone).map(|(_, t)| t.to_array())
}

fn ent_mut<'a>(world: &'a mut World, n: u32) -> Option<bevy_ecs::prelude::Mut<'a, Zm>> {
    let zm = world.resource_mut::<Zm>();
    zm.ents.contains_key(&n).then_some(zm)
}

/// The entity number of `self` (not a player), or an error.
fn me(vm: &Vm<World>, world: &World, s: &Value) -> Result<u32, String> {
    let n = entnum(vm, s).ok_or("not an entity")?;
    if world.resource::<Zm>().ents.contains_key(&n) {
        Ok(n)
    } else if world.resource::<Zm>().players.contains_key(&n) {
        Err("a player, not an entity".into())
    } else {
        Err("deleted entity".into())
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    f!("getent", |vm, world, _, a| {
        let v = matching(vm, world, a);
        Ok(v.into_iter().next().unwrap_or_default())
    });
    f!("getentarray", |vm, world, _, a| {
        // An undefined class name: BO2's _spawning re-adds its spawn points
        // that way after clearing them (updateallspawnpoints reads the list
        // it just cleared); an empty answer leaves no spawn points and aborts
        // the level. The answer is every spawn point of the classes the game
        // type registered (level.spawn_point_class_names), not every entity:
        // path and launch points (the Warthog's, Hellstorm's) put the map
        // centre (findboxcenter of the spawn bounds) 5800 units up.
        if matches!(arg(a, 0), Value::Undefined)
            && !matches!(arg(a, 1), Value::Undefined)
            && vm.to_text(arg(a, 1)).eq_ignore_ascii_case("classname")
        {
            let f = vm.intern("spawn_point_class_names");
            let names: Vec<String> = match vm.raw_field(vm.level, f) {
                Value::Array(arr) => {
                    let vals: Vec<Value> = arr.read().values_in_order().cloned().collect();
                    vals.iter()
                        .filter(|v| !matches!(v, Value::Undefined))
                        .map(|v| vm.to_text(v))
                        .collect()
                }
                _ => Vec::new(),
            };
            if !names.is_empty() {
                // They come back not yet set up (`inited` cleared): the
                // clear before this emptied level.spawnpoints, and
                // addspawnpointsinternal only re-adds points it sets up, so
                // set-up ones would leave the bots' wander list (and the
                // spawn bounds) empty.
                let inited = vm.intern("inited");
                let mut v = Vec::new();
                let mut seen = std::collections::BTreeSet::new();
                for name in names {
                    let q = [vm.string(&name), vm.string("classname")];
                    for e in matching(vm, world, &q) {
                        if let Value::Object(o) = &e {
                            if !seen.insert((o.index, o.generation)) {
                                continue;
                            }
                            vm.set_raw_field(*o, inited, Value::Undefined);
                        }
                        v.push(e);
                    }
                }
                return Ok(list(v));
            }
        }
        let v = matching(vm, world, a);
        Ok(list(v))
    });
    f!("spawn", |vm, world, _, a| {
        let cls = text(vm, a, 0);
        let origin = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let mut ent = Ent {
            obj: Some(obj),
            classname: cls.clone(),
            origin,
            solid: true,
            ..Default::default()
        };
        if cls.starts_with("trigger_radius") {
            ent.radius = arg(a, 3).as_float().unwrap_or(0.0);
            ent.height = arg(a, 4).as_float().unwrap_or(0.0);
            // bo2mp: a use trigger spawned with no size (a turret's or a
            // claymore's pick-up, `spawn("trigger_radius_use", pos)`) is
            // reached from the player's use radius.
            if cls == "trigger_radius_use" && ent.radius <= 0.0 && world.resource::<Zm>().mp {
                let reach = vm
                    .dvars
                    .get("player_useradius")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(128.0);
                ent.radius = reach;
                ent.height = reach;
            }
        }
        // bo2mp: BO2's `spawn("trigger_damage", origin, spawnflags, radius,
        // height)`: a cylinder of that size, with full health.
        if cls == "trigger_damage" {
            ent.radius = arg(a, 3).as_float().unwrap_or(0.0);
            ent.height = arg(a, 4).as_float().unwrap_or(0.0);
            let (sf, hp) = (vm.intern("spawnflags"), vm.intern("health"));
            vm.set_raw_field(obj, sf, Value::Int(arg(a, 2).as_int().unwrap_or(0)));
            vm.set_raw_field(obj, hp, Value::Int(32000));
        }
        if cls.starts_with("trigger_box") {
            ent.box_dims = Some([
                arg(a, 3).as_float().unwrap_or(64.0),
                arg(a, 4).as_float().unwrap_or(64.0),
                arg(a, 5).as_float().unwrap_or(64.0),
            ]);
        }
        if std::env::var_os("BO2MP_T6_BOUNDSLOG").is_some() && cls.starts_with("trigger") {
            diag::info!(
                Sim,
                "bo2zm t6 spawned {cls} at {origin:?} box {:?} radius {}",
                ent.box_dims,
                ent.radius
            );
        }
        world.resource_mut::<Zm>().ents.insert(n, ent);
        Ok(Value::Object(obj))
    });
    // bo2mp: Black Ops II's `spawncollision( model, targetname, origin,
    // angles )` (GScr_SpawnCollision): a script_model of a collision-only
    // model, solid and never drawn (every map's main patches its holes and
    // ledges with them: Nuketown 31, "collision_physics_32x32x128" and the
    // like, named "collider"). The presence pass makes it a turned box for
    // movement, as the scripts' own collision models.
    f!("spawncollision", |vm, world, _, a| {
        let model = text(vm, a, 0);
        let targetname = arg(a, 1).clone();
        let origin = arg(a, 2).as_vec3().unwrap_or([0.0; 3]);
        let angles = arg(a, 3).as_vec3().unwrap_or([0.0; 3]);
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let key = vm.intern("targetname");
        vm.set_raw_field(obj, key, targetname);
        let ent = Ent {
            obj: Some(obj),
            classname: "script_model".to_owned(),
            origin,
            angles,
            model,
            solid: true,
            ..Default::default()
        };
        world.resource_mut::<Zm>().ents.insert(n, ent);
        Ok(Value::Object(obj))
    });
    m!("delete", |vm, world, s, _| {
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        if is_player(vm, world, s) {
            return Err("cannot delete a player".into());
        }
        let obj = s.as_obj();
        // The engine notifies "death" on a deleted entity before freeing it
        // (ScrCmd_Delete): its endons end now, its `waittill("death")`
        // threads run after it is freed (VM_Notify puts them on the current
        // time's list) and find `self` undefined. A care package's
        // helicopter, deleted once it has flown off, still ends the drop's
        // count; a destructible car the gametype deletes at the start
        // (Raid's yellow sports car in Kill Confirmed) does not blow up, as
        // `destructible_car_death_think` checks `isdefined( self )`.
        if let Some(o) = obj
            && vm.alive(o)
        {
            vm.notify_queued(world, o, "death", &[]);
        }
        // bo2mp: its smoke stops blocking sight (G_FreeEntity).
        super::natives_fx::forget_timed(world, n);
        // A player's body (`cloneplayer`) takes its engine corpse with it.
        super::playeranim::free_body(world, n);
        let item = {
            let mut zm = world.resource_mut::<Zm>();
            zm.movers.list.remove(&n);
            // A freed entity's cut paths join again (G_FreeEntity's
            // Path_ConnectPathsForEntity): Frost's and Rush's glass blockers
            // let bots through once the glass breaks.
            let cut = zm.nav.cut_count();
            zm.nav.connect(n);
            if zm.nav.cut_count() != cut && std::env::var_os("BO2MP_T6_PATHLOG").is_some() {
                diag::info!(
                    Sim,
                    "bo2zm t6 delete ent{n}: its paths join, {} links cut in all",
                    zm.nav.cut_count()
                );
            }
            zm.ents.remove(&n).and_then(|e| e.item)
        };
        if let Some(number) = item {
            super::items::forget(world, number);
        }
        if let Some(o) = obj {
            vm.free_object(world, o);
        }
        Ok(Value::Undefined)
    });
    m!("setmodel", |vm, world, s, a| {
        // bo2mp: a player's body (BO2's faction class scripts) goes to every
        // client, which draws him with it.
        if is_player(vm, world, s) {
            if let Some(n) = entnum(vm, s) {
                let model = text(vm, a, 0);
                super::playeranim::edit_body(world, n, |l| l.model = model);
            }
            return Ok(Value::Undefined);
        }
        let n = me(vm, world, s)?;
        let model = text(vm, a, 0);
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().model = model;
        }
        Ok(Value::Undefined)
    });
    m!("attach", |vm, world, s, a| {
        if is_player(vm, world, s) {
            if let Some(n) = entnum(vm, s) {
                let added = (text(vm, a, 0), text(vm, a, 1));
                super::playeranim::edit_body(world, n, |l| l.attached.push(added));
            }
            return Ok(Value::Undefined);
        }
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let model = text(vm, a, 0);
        let tag = text(vm, a, 1);
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().attached.push((model, tag));
        }
        Ok(Value::Undefined)
    });
    m!("detach", |vm, world, s, a| {
        if is_player(vm, world, s) {
            if let Some(n) = entnum(vm, s) {
                let gone = text(vm, a, 0);
                super::playeranim::edit_body(world, n, |l| {
                    if let Some(i) = l.attached.iter().position(|(m, _)| *m == gone) {
                        l.attached.remove(i);
                    }
                });
            }
            return Ok(Value::Undefined);
        }
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let model = text(vm, a, 0);
        if let Some(mut zm) = ent_mut(world, n) {
            let e = zm.ents.get_mut(&n).unwrap();
            if let Some(i) = e.attached.iter().position(|(m, _)| *m == model) {
                e.attached.remove(i);
            }
        }
        Ok(Value::Undefined)
    });
    m!("detachall", |vm, world, s, _| {
        if is_player(vm, world, s) {
            if let Some(n) = entnum(vm, s) {
                super::playeranim::detach_all(world, n);
            }
            return Ok(Value::Undefined);
        }
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().attached.clear();
        }
        Ok(Value::Undefined)
    });
    macro_rules! flagset {
        ($($name:literal => |$e:ident| $set:expr),* $(,)?) => {$(
            vm.bind($name, true, |vm, world, s, _| {
                let Ok(n) = me(vm, world, s) else { return Ok(Value::Undefined) };
                if let Some(mut zm) = ent_mut(world, n) {
                    let $e = zm.ents.get_mut(&n).unwrap();
                    $set;
                }
                Ok(Value::Undefined)
            });
        )*};
    }
    flagset!(
        "hide" => |e| e.hidden = true,
        "show" => |e| e.hidden = false,
        "ghost" => |e| e.hidden = true,
        "notsolid" => |e| e.solid = false,
        "solid" => |e| e.solid = true,
        "unlink" => |e| e.link = None,
        "stoploopsound" => |e| e.loop_sound = None,
    );
    // A weapon's world model on an entity (the box's floating gun).
    m!("useweaponmodel", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let mut model = text(vm, a, 1);
        if model.is_empty() {
            let w = super::weapon(world, &text(vm, a, 0)).unwrap_or(0);
            model = super::frame(world)
                .weapon_world_model(w)
                .map(|(m, _)| m.to_owned())
                .unwrap_or_default();
        }
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().model = model;
        }
        Ok(Value::Undefined)
    });
    // A player unlinks too (`playerlinkto`).
    m!("unlink", |vm, world, s, _| {
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        let mut zm = world.resource_mut::<Zm>();
        if let Some(p) = zm.players.get_mut(&n) {
            p.linked = None;
        } else if let Some(e) = zm.ents.get_mut(&n) {
            e.link = None;
        }
        Ok(Value::Undefined)
    });
    m!("setcandamage", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let on = flag(a, 0, true);
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().can_damage = on;
        }
        Ok(Value::Undefined)
    });
    // bo2mp: whether a grenade moving through this damage trigger damages
    // it (Black Ops II's entity flag these two set and clear).
    m!("enablegrenadetouchdamage", |vm, world, s, _| {
        if let (Ok(n), Value::Object(o)) = (me(vm, world, s), s) {
            super::damage_triggers::grenade_touch(world, n, *o, true);
        }
        Ok(Value::Undefined)
    });
    m!("disablegrenadetouchdamage", |vm, world, s, _| {
        if let (Ok(n), Value::Object(o)) = (me(vm, world, s), s) {
            super::damage_triggers::grenade_touch(world, n, *o, false);
        }
        Ok(Value::Undefined)
    });
    m!("sethintstring", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        // The text as the player reads it: the language string with its
        // arguments filled in (`&&1`, `&&2`).
        let h = super::localize(vm, world, arg(a, 0), a.get(1..).unwrap_or(&[]));
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().hint = (!h.is_empty()).then_some(h);
        }
        Ok(Value::Undefined)
    });
    m!("setcursorhint", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let h = text(vm, a, 0);
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().cursor_hint = Some(h);
        }
        Ok(Value::Undefined)
    });
    m!("playloopsound", |vm, world, s, a| {
        if std::env::var_os("BO2MP_SNDLOG").is_some() {
            let alias = text(vm, a, 0);
            let on = me(vm, world, s);
            diag::info!(Sim, "bo2mp sndlog loop {alias} on {on:?}");
        }
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let alias = text(vm, a, 0);
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().loop_sound = Some(alias);
        }
        Ok(Value::Undefined)
    });
    m!("linkto", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let Some(parent) = entnum(vm, arg(a, 0)) else {
            return Err("linkto needs an entity".into());
        };
        // An offset from a tag of the parent's model (the care package
        // under the drone's tag_care_package) is that far from the tag.
        let tag = text(vm, a, 1);
        let off = arg(a, 2).as_vec3().map(|o| {
            let parent_model = world
                .resource::<Zm>()
                .ents
                .get(&parent)
                .map(|e| e.model.clone())
                .unwrap_or_default();
            match tag_in_model(world, &parent_model, &tag) {
                Some(t) => std::array::from_fn(|i| o[i] + t[i]),
                None => o,
            }
        });
        let aoff = arg(a, 3).as_vec3().unwrap_or([0.0; 3]);
        let parent_pose = {
            let p = arg(a, 0).clone();
            let o = origin_of(vm, world, &p);
            let ang = world
                .resource::<Zm>()
                .ents
                .get(&parent)
                .map(|e| e.angles)
                .unwrap_or([0.0; 3]);
            o.map(|o| (o, ang))
        };
        let my = world
            .resource::<Zm>()
            .ents
            .get(&n)
            .map(|e| e.origin)
            .unwrap_or([0.0; 3]);
        let off = off.unwrap_or_else(|| match parent_pose {
            // Keep where it is relative to the parent.
            Some((po, pa)) => {
                let (f, r, u) = gsc_t6::math::angle_vectors(pa);
                let d = gsc_t6::math::sub(my, po);
                [
                    gsc_t6::math::dot(d, f),
                    -gsc_t6::math::dot(d, r),
                    gsc_t6::math::dot(d, u),
                ]
            }
            None => [0.0; 3],
        });
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().link = Some((parent, off, aoff));
        }
        Ok(Value::Undefined)
    });
    m!("enablelinkto", |_, _, _, _| Ok(Value::Undefined));
    m!("getorigin", |vm, world, s, _| Ok(
        origin_of(vm, world, s).map_or(Value::Undefined, Value::Vec3)
    ));
    m!("getentitynumber", |vm, _, s, _| Ok(
        entnum(vm, s).map_or(Value::Undefined, |n| Value::Int(n as i32))
    ));
    m!("gettagorigin", |vm, world, s, _| Ok(origin_of(
        vm, world, s
    )
    .map_or(Value::Undefined, Value::Vec3)));
    m!("gettagangles", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(world
            .resource::<Zm>()
            .ents
            .get(&n)
            .map_or(Value::Vec3([0.0; 3]), |e| Value::Vec3(e.angles)))
    });
    // An entity's bounds: its model's (the file's own bounds), a player's
    // or an actor's body; abs = turned by its angles and placed.
    m!("getmins", |vm, world, s, _| Ok(Value::Vec3(
        local_bounds(vm, world, s).0
    )));
    m!("getmaxs", |vm, world, s, _| Ok(Value::Vec3(
        local_bounds(vm, world, s).1
    )));
    // A point in an entity's box (ScrCmd_GetPointInBounds ->
    // G_GetEntityBoundsPoint): each fraction -1..1 runs mins to maxs on its
    // axis, then the point is turned by the angles and placed at the origin.
    m!("getpointinbounds", |vm, world, s, a| {
        let (lo, hi) = local_bounds(vm, world, s);
        let mut p = [0.0f32; 3];
        for i in 0..3 {
            let half = (hi[i] - lo[i]) * 0.5;
            p[i] = half * num(a, i)? + lo[i] + half;
        }
        let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        let angles = entnum(vm, s)
            .and_then(|n| world.resource::<Zm>().ents.get(&n).map(|e| e.angles))
            .unwrap_or([0.0; 3]);
        let (f, r, u) = gsc_t6::math::angle_vectors(angles);
        // local y is left (the right vector's opposite).
        let w: [f32; 3] =
            std::array::from_fn(|i| origin[i] + f[i] * p[0] - r[i] * p[1] + u[i] * p[2]);
        // BO2MP_T6_BOUNDSLOG=1: each asked entity, its box and the point.
        if std::env::var_os("BO2MP_T6_BOUNDSLOG").is_some() {
            diag::info!(
                Sim,
                "bo2zm t6 getpointinbounds ent{} at {origin:?} angles {angles:?}: local {lo:?}..{hi:?}, fractions {:?} -> {w:?}",
                entnum(vm, s).unwrap_or(u32::MAX),
                [num(a, 0)?, num(a, 1)?, num(a, 2)?]
            );
        }
        Ok(Value::Vec3(w))
    });
    m!("getabsmins", |vm, world, s, _| Ok(Value::Vec3(
        abs_bounds(vm, world, s).0
    )));
    m!("getabsmaxs", |vm, world, s, _| {
        let (lo, hi) = abs_bounds(vm, world, s);
        // BO2MP_T6_BOUNDSLOG=1: each asked entity's model, angles and bounds.
        if std::env::var_os("BO2MP_T6_BOUNDSLOG").is_some() {
            let (llo, lhi) = local_bounds(vm, world, s);
            let e = entnum(vm, s).and_then(|n| world.resource::<Zm>().ents.get(&n).cloned());
            if let Some(e) = e {
                diag::info!(
                    Sim,
                    "bo2zm t6 bounds {} at {:?} angles {:?}: local {llo:?}..{lhi:?} abs size {:?}",
                    e.model,
                    e.origin,
                    e.angles,
                    [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
                );
            }
        }
        Ok(Value::Vec3(hi))
    });
    m!("moveto", |vm, world, s, a| {
        let n = me(vm, world, s)?;
        let to = vec3(a, 0)?;
        let t = num(a, 1)?;
        let (acc, dec) = (
            arg(a, 2).as_float().unwrap_or(0.0),
            arg(a, 3).as_float().unwrap_or(0.0),
        );
        let mut zm = world.resource_mut::<Zm>();
        let now = zm.now_ms;
        let from = zm.ents[&n].origin;
        zm.movers.move_to(n, from, to, now, t, acc, dec);
        Ok(Value::Undefined)
    });
    m!("movex", |vm, world, s, a| move_axis(vm, world, s, a, 0));
    m!("movey", |vm, world, s, a| move_axis(vm, world, s, a, 1));
    m!("movez", |vm, world, s, a| move_axis(vm, world, s, a, 2));
    m!("movegravity", |vm, world, s, a| {
        let n = me(vm, world, s)?;
        let vel = vec3(a, 0)?;
        let t = num(a, 1)?;
        let mut zm = world.resource_mut::<Zm>();
        let now = zm.now_ms;
        let from = zm.ents[&n].origin;
        zm.movers.gravity(n, from, vel, now, t);
        Ok(Value::Undefined)
    });
    m!("rotateto", |vm, world, s, a| {
        let n = me(vm, world, s)?;
        let to = vec3(a, 0)?;
        let t = num(a, 1)?;
        let (acc, dec) = (
            arg(a, 2).as_float().unwrap_or(0.0),
            arg(a, 3).as_float().unwrap_or(0.0),
        );
        let mut zm = world.resource_mut::<Zm>();
        let now = zm.now_ms;
        let from = zm.ents[&n].angles;
        // Shortest way round, as the engine turns.
        let mut target = to;
        for i in 0..3 {
            let d = gsc_t6::math::angle_clamp180(to[i] - from[i]);
            target[i] = from[i] + d;
        }
        zm.movers.rotate_to(n, from, target, now, t, acc, dec);
        Ok(Value::Undefined)
    });
    m!("rotateyaw", |vm, world, s, a| rotate_axis(
        vm, world, s, a, 1
    ));
    m!("rotatepitch", |vm, world, s, a| rotate_axis(
        vm, world, s, a, 0
    ));
    m!("rotateroll", |vm, world, s, a| rotate_axis(
        vm, world, s, a, 2
    ));
    // `a istouching(b)`: whichever of the two has a volume (a brush, a
    // trigger cylinder or a spawned `trigger_box`, such as Hijacked's lower
    // Hardpoint zone) against the other's box (a player's, an actor's, else
    // a point).
    m!("istouching", |vm, world, s, a| {
        let other = arg(a, 0).clone();
        let (Some(sn), Some(on)) = (entnum(vm, s), entnum(vm, &other)) else {
            return Ok(Value::Int(0));
        };
        let volume_of = |world: &World, n: u32| -> Option<Ent> {
            world
                .resource::<Zm>()
                .ents
                .get(&n)
                .filter(|e| {
                    super::triggers::brush_index(e).is_some()
                        || e.radius > 0.0
                        || e.box_dims.is_some()
                })
                .cloned()
        };
        let (vol, toucher) = match (volume_of(world, on), volume_of(world, sn)) {
            (Some(v), _) => (v, sn),
            (None, Some(v)) => (v, on),
            // Two solid things (a falling care package and a player): their
            // world boxes, its own grown by the extra boundary, overlap.
            (None, None) => {
                let (mut lo, mut hi) = abs_bounds(vm, world, s);
                if lo == hi {
                    return Ok(Value::Int(0));
                }
                let grow = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
                for i in 0..3 {
                    lo[i] -= grow[i];
                    hi[i] += grow[i];
                }
                let other_box = if world.resource::<Zm>().players.contains_key(&on) {
                    super::triggers::player_box(world, on)
                } else {
                    Some(abs_bounds(vm, world, &other))
                };
                let Some((olo, ohi)) = other_box else {
                    return Ok(Value::Int(0));
                };
                let hit = (0..3).all(|i| lo[i] <= ohi[i] && olo[i] <= hi[i]);
                return Ok(Value::bool(hit));
            }
        };
        let bounds = if world.resource::<Zm>().players.contains_key(&toucher) {
            super::triggers::player_box(world, toucher)
        } else {
            let zm = world.resource::<Zm>();
            zm.ents.get(&toucher).map(|o| {
                let p = o.origin;
                match zm.actors.get(&toucher) {
                    Some(act) => (
                        [p[0] - act.radius, p[1] - act.radius, p[2]],
                        [p[0] + act.radius, p[1] + act.radius, p[2] + act.height],
                    ),
                    None => (
                        [p[0] - 1.0, p[1] - 1.0, p[2] - 1.0],
                        [p[0] + 1.0, p[1] + 1.0, p[2] + 1.0],
                    ),
                }
            })
        };
        let Some((mins, maxs)) = bounds else {
            return Ok(Value::Int(0));
        };
        let hit = super::triggers::touches(world, &vol, mins, maxs);
        Ok(Value::bool(hit))
    });
    // bo2mp: istouchingswept(other, minz?, maxz?) (BO2's
    // `ScrCmd_IsTouchingSwept`): does the box of the one that is not a brush
    // or cylinder, its bottom taken down to `minz` and its top up to `maxz`
    // (each only ever further out), meet the other? A dropped flag asks it of
    // the mine, hurt and elevator volumes on its way down.
    m!("istouchingswept", |vm, world, s, a| {
        let other = arg(a, 0).clone();
        let (Some(sn), Some(on)) = (entnum(vm, s), entnum(vm, &other)) else {
            return Err("not an entity".into());
        };
        let (sv, ov) = (has_volume(world, sn), has_volume(world, on));
        if sv && ov {
            return Err("istouchingswept cannot be called on 2 brush/cylinder entities".into());
        }
        let (toucher, target) = if sv {
            (other, s.clone())
        } else {
            (s.clone(), other)
        };
        let Some((mut lo, mut hi)) = own_box(vm, world, &toucher) else {
            return Ok(Value::Int(0));
        };
        if a.len() > 1 {
            let z = num(a, 1)?;
            if lo[2] >= z {
                lo[2] = z;
            }
        }
        if a.len() > 2 {
            let z = num(a, 2)?;
            if z >= hi[2] {
                hi[2] = z;
            }
        }
        expand_to_width(&mut lo, &mut hi);
        Ok(Value::bool(entity_contact(vm, world, &target, lo, hi)))
    });
    // bo2mp: istouchingvolume(origin, mins, maxs) (BO2's
    // `ScrCmd_IsTouchingVolume`): does the box about the origin (made at
    // least as tall as it is wide) meet this entity? A tactical insertion
    // asks it of the care packages where it would go down.
    m!("istouchingvolume", |vm, world, s, a| {
        let o = vec3(a, 0)?;
        let (mins, maxs) = (vec3(a, 1)?, vec3(a, 2)?);
        let mut lo: [f32; 3] = std::array::from_fn(|i| o[i] + mins[i]);
        let mut hi: [f32; 3] = std::array::from_fn(|i| o[i] + maxs[i]);
        expand_to_width(&mut lo, &mut hi);
        Ok(Value::bool(entity_contact(vm, world, s, lo, hi)))
    });
    // bo2mp: allowbottargetting(on) (BO2's `ScrCmd_AllowBotTargetting`):
    // while it is off, bots do not take it for a target (Drone's and
    // Vertigo's doors turn it off).
    m!("allowbottargetting", |vm, world, s, a| {
        let n = me(vm, world, s)?;
        let on = super::int(a, 0)? != 0;
        let mut zm = world.resource_mut::<Zm>();
        if on {
            zm.no_bot_target.remove(&n);
        } else {
            zm.no_bot_target.insert(n);
        }
        Ok(Value::Undefined)
    });
    // bo2mp: setscriptmoverflag(index) (BO2's `GScr_SetScriptMoverFlag`): a
    // planted riot shield sets flag 0, the only one; nothing here reads it
    // yet, so only the index is checked.
    m!("setscriptmoverflag", |_, _, _, a| {
        let i = super::int(a, 0)?;
        if i != 0 {
            return Err(format!(
                "SetScriptMoverFlag: Index {i} out of range (0 - 0)"
            ));
        }
        Ok(Value::Undefined)
    });
    // bo2mp: islinkedto(ent) (BO2's `ScrCmd_IsLinkedTo`): is it linked
    // straight to that entity (a sticky grenade on a carried object)?
    m!("islinkedto", |vm, world, s, a| {
        let parent = entnum(vm, arg(a, 0)).ok_or("not an entity")?;
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Int(0));
        };
        let zm = world.resource::<Zm>();
        let linked = match zm.players.get(&n) {
            Some(p) => p.linked,
            None => zm.ents.get(&n).and_then(|e| e.link.map(|l| l.0)),
        };
        Ok(Value::bool(linked == Some(parent)))
    });
    // bo2mp: directionalhitindicator(bits0, bits1?) (BO2's
    // `GScr_DirectionalHitIndicator`): BO2 sends the player arrows towards
    // the attackers it names (a bit per entity) for its HUD; ours has no
    // such arrows, so the call is checked and dropped.
    m!("directionalhitindicator", |vm, world, s, a| {
        super::client(vm, world, s)?;
        super::int(a, 0)?;
        if a.len() > 1 {
            super::int(a, 1)?;
        }
        Ok(Value::Undefined)
    });
    m!("setinvisibletoplayer", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let Some(p) = entnum(vm, arg(a, 0)) else {
            return Ok(Value::Undefined);
        };
        let hide = flag(a, 1, true);
        if let Some(mut zm) = ent_mut(world, n) {
            let e = zm.ents.get_mut(&n).unwrap();
            if hide {
                e.invisible_to.insert(p);
            } else {
                e.invisible_to.remove(&p);
            }
        }
        Ok(Value::Undefined)
    });
    m!("setvisibletoplayer", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let Some(p) = entnum(vm, arg(a, 0)) else {
            return Ok(Value::Undefined);
        };
        if let Some(mut zm) = ent_mut(world, n) {
            let e = zm.ents.get_mut(&n).unwrap();
            e.invisible_to.remove(&p);
            e.invisible_to_all = false;
        }
        Ok(Value::Undefined)
    });
    m!("setinvisibletoall", |vm, world, s, _| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().invisible_to_all = true;
        }
        Ok(Value::Undefined)
    });
    m!("setvisibletoall", |vm, world, s, _| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        if let Some(mut zm) = ent_mut(world, n) {
            let e = zm.ents.get_mut(&n).unwrap();
            e.invisible_to_all = false;
            e.invisible_to.clear();
        }
        Ok(Value::Undefined)
    });
    m!("triggerenable", |vm, world, s, a| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        let on = flag(a, 0, true);
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().trigger_off = !on;
        }
        Ok(Value::Undefined)
    });
    // A wall buy's trigger is the weapon's own small box on the wall: the
    // player uses it by looking at it from close by, not by standing in
    // it (seats and clip keep him a body away from the truck's chalk).
    m!("usetriggerrequirelookat", |vm, world, s, _| {
        let Ok(n) = me(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        if let Some(mut zm) = ent_mut(world, n) {
            zm.ents.get_mut(&n).unwrap().look_at = true;
        }
        Ok(Value::Undefined)
    });
    // bo2mp: ScrCmd_UseBy (common_scripts/utility::trigger_use): used by
    // him as if he pressed its use key.
    m!("useby", |vm, world, s, a| {
        if let Value::Object(o) = s
            && entnum(vm, arg(a, 0)).is_some()
        {
            vm.notify_str(world, *o, "trigger", std::slice::from_ref(arg(a, 0)));
        }
        Ok(Value::Undefined)
    });
    for name in [
        "sethintlowpriority",
        // bo2mp: a render hint (the client skips relinking a fast mover's
        // model), and the createfx editor's turns and text.
        "setcheapflag",
        "devaddpitch",
        "devaddroll",
        "devaddyaw",
        "clearalltextafterhudelem",
        "triggerignoreteam",
        "setvisibletoallexceptteam",
        "setexcludeteamfortrigger",
        "setmovingplatformenabled",
        "setowner",
        "setteam",
        "setphysparams",
        "setturretcarried",
        "makegrenadedud",
        "setclientfield",
        "setclientfieldtoplayer",
        "stopsounds",
        "playsound",
        "playsoundtoplayer",
        "playsoundtoteam",
        "playsoundontag",
        "playsoundasmaster",
        "playlocalsound",
        "useanimtree",
        "setanim",
        "clearanim",
        "notsolidcapsule",
        "dontinterpolate",
        "setscale",
        "physicslaunch",
        "launchragdoll",
        "startragdoll",
        "setzbarrierpiecestate",
        "showzbarrierpiece",
        "hidezbarrierpiece",
        "zbarrierpieceuseboxriselogic",
        "zbarrierpieceusedefaultmodel",
        "zbarrierpieceuseupgradedmodel",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
    }
    // bo2mp: a grenade or missile the scripts hold flies again from where it
    // is, under gravity, turning at the second vector if given (the decoy's
    // hops, a combat axe let go of a live player): Black Ops II's
    // GScr_Launch. Anything else stays put here.
    m!("launch", |vm, world, s, a| {
        let vel = vec3(a, 0)?;
        if vel.iter().any(|v| !v.is_finite()) {
            return Err(format!(
                "invalid velocity parameter in launch command: {} {} {}",
                vel[0], vel[1], vel[2]
            ));
        }
        let avel = if a.len() > 1 { Some(vec3(a, 1)?) } else { None };
        if let Some(n) = entnum(vm, s) {
            super::killstreaks::relaunch(world, n, vel, avel);
        }
        Ok(Value::Undefined)
    });
    m!("playsoundwithnotify", |vm, world, s, a| {
        // The sound system is not wired to scripts yet: notify after a short
        // while so waiting scripts go on.
        let name = text(vm, a, 1);
        if let Some(o) = s.as_obj() {
            super::natives_game::notify_later(vm, world, o, &name, 500);
        }
        Ok(Value::Undefined)
    });
}

fn move_axis(vm: &mut Vm<World>, world: &mut World, s: &Value, a: &[Value], axis: usize) -> R {
    let n = me(vm, world, s)?;
    let d = num(a, 0)?;
    let t = num(a, 1)?;
    let (acc, dec) = (
        arg(a, 2).as_float().unwrap_or(0.0),
        arg(a, 3).as_float().unwrap_or(0.0),
    );
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    let from = zm.ents[&n].origin;
    let mut to = from;
    to[axis] += d;
    zm.movers.move_to(n, from, to, now, t, acc, dec);
    Ok(Value::Undefined)
}

fn rotate_axis(vm: &mut Vm<World>, world: &mut World, s: &Value, a: &[Value], axis: usize) -> R {
    let n = me(vm, world, s)?;
    let d = num(a, 0)?;
    let t = num(a, 1)?;
    let (acc, dec) = (
        arg(a, 2).as_float().unwrap_or(0.0),
        arg(a, 3).as_float().unwrap_or(0.0),
    );
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    let from = zm.ents[&n].angles;
    let mut to = from;
    to[axis] += d;
    zm.movers.rotate_to(n, from, to, now, t, acc, dec);
    Ok(Value::Undefined)
}

/// Model-space bounds of an entity (mins, maxs).
pub(super) fn local_bounds(vm: &Vm<World>, world: &mut World, s: &Value) -> ([f32; 3], [f32; 3]) {
    const BODY: ([f32; 3], [f32; 3]) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    let Some(n) = entnum(vm, s) else { return BODY };
    if is_player(vm, world, s) || world.resource::<Zm>().actors.contains_key(&n) {
        return BODY;
    }
    let Some(e) = world.resource::<Zm>().ents.get(&n).cloned() else {
        return ([0.0; 3], [0.0; 3]);
    };
    if let Some([w, l, h]) = e.box_dims {
        return ([-w * 0.5, -l * 0.5, -h * 0.5], [w * 0.5, l * 0.5, h * 0.5]);
    }
    if e.radius > 0.0 {
        return ([-e.radius, -e.radius, 0.0], [e.radius, e.radius, e.height]);
    }
    if let Some(b) = brush_bounds(world, &e) {
        return b;
    }
    if e.model.is_empty() || e.model.starts_with('*') {
        return ([0.0; 3], [0.0; 3]);
    }
    let f = super::frame(world);
    match f
        .model_capability(&e.model)
        .flatten()
        .and_then(|c| c.bounds)
    {
        Some((mid, half)) => (
            std::array::from_fn(|i| mid[i] - half[i]),
            std::array::from_fn(|i| mid[i] + half[i]),
        ),
        None => ([0.0; 3], [0.0; 3]),
    }
}

/// bo2mp: a brush entity's bounds about its origin: its brush model's
/// (BO2's `SV_SetBrushModel` makes them its mins and maxs).
fn brush_bounds(world: &mut World, e: &Ent) -> Option<([f32; 3], [f32; 3])> {
    let i = super::triggers::brush_index(e).or_else(|| e.model.strip_prefix('*')?.parse().ok())?;
    let f = super::frame(world);
    let m = f.clip_cmodels().models.get(i as usize)?;
    Some((m.mins, m.maxs))
}

/// bo2mp: a box no wider than it is tall: a flat one grows up and down
/// alike until it is (BO2's `ExpandBoundsToWidth`).
pub(super) fn expand_to_width(lo: &mut [f32; 3], hi: &mut [f32; 3]) {
    let width = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    let tall = hi[2] - lo[2];
    if tall < width {
        let grow = (width - tall) * 0.5;
        lo[2] -= grow;
        hi[2] += grow;
    }
}

/// bo2mp: is the entity a brush or a cylinder (a trigger's volume, a
/// turned box)?
fn has_volume(world: &World, n: u32) -> bool {
    world.resource::<Zm>().ents.get(&n).is_some_and(|e| {
        super::triggers::brush_index(e).is_some() || e.radius > 0.0 || e.box_dims.is_some()
    })
}

/// bo2mp: an entity's own box in the world: its origin plus its mins and
/// maxs, not turned (a player's by his stance).
fn own_box(vm: &Vm<World>, world: &mut World, v: &Value) -> Option<([f32; 3], [f32; 3])> {
    let n = entnum(vm, v)?;
    if world.resource::<Zm>().players.contains_key(&n) {
        return super::triggers::player_box(world, n);
    }
    let o = origin_of(vm, world, v)?;
    let (lo, hi) = local_bounds(vm, world, v);
    Some((
        std::array::from_fn(|i| o[i] + lo[i]),
        std::array::from_fn(|i| o[i] + hi[i]),
    ))
}

/// bo2mp: does the box meet the entity (BO2's `SV_EntityContact`)? A
/// player by his box, a brush or cylinder by its volume, anything else by
/// its world bounds.
fn entity_contact(
    vm: &Vm<World>,
    world: &mut World,
    target: &Value,
    lo: [f32; 3],
    hi: [f32; 3],
) -> bool {
    let Some(n) = entnum(vm, target) else {
        return false;
    };
    let meets = |(bl, bh): ([f32; 3], [f32; 3])| (0..3).all(|i| lo[i] <= bh[i] && bl[i] <= hi[i]);
    if world.resource::<Zm>().players.contains_key(&n) {
        return super::triggers::player_box(world, n).is_some_and(meets);
    }
    if has_volume(world, n) {
        let Some(e) = world.resource::<Zm>().ents.get(&n).cloned() else {
            return false;
        };
        return super::triggers::touches(world, &e, lo, hi);
    }
    meets(abs_bounds(vm, world, target))
}

/// World bounds: the local box's corners turned by the angles, placed.
pub(super) fn abs_bounds(vm: &Vm<World>, world: &mut World, s: &Value) -> ([f32; 3], [f32; 3]) {
    let (lo, hi) = local_bounds(vm, world, s);
    let origin = origin_of(vm, world, s).unwrap_or([0.0; 3]);
    let angles = entnum(vm, s)
        .and_then(|n| world.resource::<Zm>().ents.get(&n).map(|e| e.angles))
        .unwrap_or([0.0; 3]);
    let (f, r, u) = gsc_t6::math::angle_vectors(angles);
    let mut mins = [f32::MAX; 3];
    let mut maxs = [f32::MIN; 3];
    for c in 0..8 {
        let p = [
            if c & 1 == 0 { lo[0] } else { hi[0] },
            if c & 2 == 0 { lo[1] } else { hi[1] },
            if c & 4 == 0 { lo[2] } else { hi[2] },
        ];
        for i in 0..3 {
            // local y is left (the right vector's opposite).
            let w = origin[i] + f[i] * p[0] - r[i] * p[1] + u[i] * p[2];
            mins[i] = mins[i].min(w);
            maxs[i] = maxs[i].max(w);
        }
    }
    (mins, maxs)
}

