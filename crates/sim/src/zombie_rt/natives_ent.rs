//! Entities: the map's entities and structs at level start, `spawn`,
//! `getent(array)`, models, showing and hiding, links, movers, triggers' hints.

use bevy_ecs::prelude::World;
use gsc_t6::{Key, ObjKind, ObjRef, ScriptValue, Vm};

use super::{Ent, Zm, arg, entnum, flag, is_player, list, num, origin_of, text, vec3};

/// Map entity keys the engine types (the rest stay strings).
fn typed(vm: &mut Vm<World>, key: &str, val: &str, entity: bool) -> ScriptValue {
    // Exactly three numbers make a vector.
    let floats = |s: &str| -> Option<[f32; 3]> {
        let numbers: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        <[f32; 3]>::try_from(numbers.as_slice()).ok()
    };
    match key {
        // The engine's vector fields: a door's open turn (`script_angles`)
        // and a debris pile's slide (`script_vector`) go to rotateto/moveto.
        "origin" | "angles" | "script_angles" | "script_vector" => match floats(val) {
            Some(v) => ScriptValue::Vec3(v),
            None => vm.string(val),
        },
        "spawnflags" | "count" | "health" | "dmg" | "script_forcespawn" | "script_int"
            if entity =>
        {
            ScriptValue::Int(val.trim().parse().unwrap_or(0))
        }
        "radius" | "height" | "speed" | "wait" | "delay" if entity => {
            ScriptValue::Float(val.trim().parse().unwrap_or(0.0))
        }
        // Names stay text even when they look like numbers.
        "targetname" | "target" | "script_noteworthy" | "script_string" | "classname" | "model"
        | "script_flag" | "script_label" | "script_parameters" | "script_linkto"
        | "script_linkname" | "script_sound" | "script_fxid" | "script_animation"
        | "script_location" => vm.string(val),
        // Numbers are numbers (BO2's scripts compare `self.zombie_cost` with
        // a score and halve it; `trigger.script_chance` with a randomfloat).
        _ => {
            let t = val.trim();
            if let Ok(i) = t.parse::<i32>() {
                ScriptValue::Int(i)
            } else if let Ok(f) = t.parse::<f32>()
                && t.contains('.')
            {
                ScriptValue::Float(f)
            } else {
                vm.string(val)
            }
        }
    }
}

/// Create the map's entities (`script_struct` blocks through
/// `codescripts/struct::createstruct`); path nodes are kept for the AI.
pub(super) fn spawn_map_entities(vm: &mut Vm<World>, game: &mut World, texts: &[String]) {
    let createstruct = vm
        .program
        .find(&vm.strings, "codescripts/struct", "createstruct");
    let struct_s = vm.intern("struct");
    let (mut ents, mut structs, mut nodes) = (0, 0, 0);
    for text in texts {
        for en in asset_t6_entities(text) {
            let cls = en
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
                if let Some(v) = en.iter().find(|kv| kv.0.eq_ignore_ascii_case("northyaw")).map(|kv| &kv.1) {
                    vm.dvars.insert("northyaw".to_owned(), v.trim().to_owned());
                }
            }
            if cls == "script_struct" {
                let Some(cs) = createstruct else { continue };
                let level = ScriptValue::Object(vm.level);
                vm.spawn(game, cs, level, vec![]);
                let ScriptValue::Array(a) = vm.raw_field(vm.level, struct_s) else {
                    continue;
                };
                let Some(ScriptValue::Object(s)) = a.get(&Key::Int(a.len() as i32 - 1)) else {
                    continue;
                };
                for (k, v) in &en {
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
            let n = game.resource_mut::<Zm>().alloc_entnum();
            let obj = vm.alloc_object(ObjKind::Entity(n));
            // BO2's SP_script_origin: no contents, and never sent to a client until a script
            // calls show(), even with a model key (mp_drone's satellite dish).
            // BO2 has no spawn function for "model" (mp_nightclub's two wire spools): it keeps
            // its fields but is never linked, so never sent and never solid.
            let origin_only = cls == "script_origin" || cls == "model";
            let mut ent = Ent {
                obj: Some(obj),
                class_tag: cls.clone(),
                map: true,
                solid: !origin_only,
                unseen: origin_only,
                ..Default::default()
            };
            let mut target_name = None;
            for (k, v) in &en {
                let key = k.to_ascii_lowercase();
                match key.as_str() {
                    "classname" => continue,
                    "targetname" => target_name = Some(v.clone()),
                    "origin" => {
                        if let ScriptValue::Vec3(p) = typed(vm, "origin", v, true) {
                            ent.origin = p;
                        }
                        continue;
                    }
                    "angles" => {
                        if let ScriptValue::Vec3(p) = typed(vm, "angles", v, true) {
                            ent.rot = p;
                        }
                        continue;
                    }
                    "angle" => {
                        ent.rot = [0.0, v.trim().parse().unwrap_or(0.0), 0.0];
                        continue;
                    }
                    "model" if v.starts_with('*') => ent.brush = Some(v.clone()),
                    "model" => {
                        ent.mesh = v.clone();
                        continue;
                    }
                    "radius" => ent.reach = v.trim().parse().unwrap_or(0.0),
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
            if ent.brush.is_some() {
                ent.birth_orientation = Some(ent.rot);
            }
            if cls.starts_with("zbarrier") {
                let mut z = super::zbarrier::ZBarrier::from_keys(&en);
                z.def = super::zbarrier::def_for(game, &en);
                z.take_def_looks(&en);
                ent.zbarrier = Some(z);
            }
            // bo2mp: a Black Ops II breakable starts with its base piece's
            // health (DestructibleUpdate); its pieces pass hits on to it.
            let base_health = ent
                .destructible
                .as_deref()
                .filter(|d| !super::destructible::is_dyn(d))
                .and_then(|d| game.resource::<Zm>().destructibles.get(d).cloned())
                .and_then(|def| def.pieces.first().map(|p| p.health));
            if let Some(hp) = base_health {
                let f = vm.intern("health");
                vm.set_raw_field(obj, f, ScriptValue::Int(hp));
            }
            // bo2mp: BO2's SP_trigger_damage: a shoot-to-use trigger keeps
            // 32000 health (t6/damage_triggers.rs); a "wait" of 0 or less
            // frees it once it has fired (spawnflags 0x200).
            if cls == "trigger_damage" {
                let f = vm.intern("health");
                vm.set_raw_field(obj, f, ScriptValue::Int(32000));
                let wait = en
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("wait"))
                    .and_then(|(_, v)| v.trim().parse::<f32>().ok());
                if wait.is_some_and(|w| w <= 0.0) {
                    let sf = vm.intern("spawnflags");
                    let flags = vm.raw_field(obj, sf).as_int().unwrap_or(0);
                    vm.set_raw_field(obj, sf, ScriptValue::Int(flags | 0x200));
                }
            }
            let mut zm = game.resource_mut::<Zm>();
            zm.ents.insert(n, ent);
            if let Some(t) = target_name {
                zm.nav.attach_movers(&t, n);
            }
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
    let mut rest = text.chars();
    while let Some(ch) = rest.next() {
        match ch {
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
                // Up to the closing quote, which is used up too.
                quoted.push(rest.by_ref().take_while(|&c| c != '"').collect::<String>());
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
pub(super) fn matching(vm: &mut Vm<World>, game: &mut World, lhs: &[ScriptValue]) -> Vec<ScriptValue> {
    let want = match arg(lhs, 0) {
        ScriptValue::Undefined => None,
        v => Some(vm.to_text(v)),
    };
    let key = match arg(lhs, 1) {
        ScriptValue::Undefined => None,
        v => Some(vm.to_text(v).to_ascii_lowercase()),
    };
    let ents: Vec<(u32, ObjRef, String)> = game
        .resource::<Zm>()
        .ents
        .iter()
        .filter_map(|(nn, en)| en.obj.map(|o| (*nn, o, en.class_tag.clone())))
        .collect();
    let players: Vec<ObjRef> = game
        .resource::<Zm>()
        .players
        .values()
        .map(|p| p.obj)
        .collect();
    let mut out = Vec::new();
    let (Some(want), Some(key)) = (want, key) else {
        return ents.iter().map(|(_, o, _)| ScriptValue::Object(*o)).collect();
    };
    if key == "classname" {
        if want == "player" {
            return players.into_iter().map(ScriptValue::Object).collect();
        }
        for (_, o, c) in &ents {
            if *c == want && vm.alive(*o) {
                out.push(ScriptValue::Object(*o));
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
        if let ScriptValue::Str(s) = vm.raw_field(o, k)
            && s == w
        {
            out.push(ScriptValue::Object(o));
        }
    }
    out
}

/// A tag's place in a model at rest (model space); none for its origin.
fn tag_in_model(game: &mut World, mdl: &str, tag_name: &str) -> Option<[f32; 3]> {
    if mdl.is_empty() || tag_name.is_empty() || tag_name.eq_ignore_ascii_case("tag_origin") {
        return None;
    }
    let cap = super::frame(game).model_capability(mdl)??;
    let pose = &cap.pose;
    let joint = pose.joint_labels.iter().position(|n| n.eq_ignore_ascii_case(tag_name))?;
    let (_, rest_place) = pose.foundation_surface.get(joint)?;
    Some(rest_place.to_array())
}

fn ent_mut<'a>(world: &'a mut World, n: u32) -> Option<bevy_ecs::prelude::Mut<'a, Zm>> {
    let zm = world.resource_mut::<Zm>();
    zm.ents.contains_key(&n).then_some(zm)
}

/// The entity number of `self` (not a player), or an error.
fn me(vm: &Vm<World>, world: &World, s: &ScriptValue) -> Result<u32, String> {
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
    f!("getent", |vm, sim, _, a| {
        let v = matching(vm, sim, a);
        Ok(v.into_iter().next().unwrap_or_default())
    });
    f!("getentarray", |vm, sim, _, a| {
        // An undefined class name: BO2's _spawning re-adds its spawn points
        // that way after clearing them (updateallspawnpoints reads the list
        // it just cleared); an empty answer leaves no spawn points and aborts
        // the level. The answer is every spawn point of the classes the game
        // type registered (level.spawn_point_class_names), not every entity:
        // path and launch points (the Warthog's, Hellstorm's) put the map
        // centre (findboxcenter of the spawn bounds) 5800 units up.
        if matches!(arg(a, 0), ScriptValue::Undefined)
            && !matches!(arg(a, 1), ScriptValue::Undefined)
            && vm.to_text(arg(a, 1)).eq_ignore_ascii_case("classname")
        {
            let f = vm.intern("spawn_point_class_names");
            let names: Vec<String> = match vm.raw_field(vm.level, f) {
                ScriptValue::Array(arr) => {
                    let vals: Vec<ScriptValue> = arr.read().values_in_order().cloned().collect();
                    vals.iter()
                        .filter(|v| !matches!(v, ScriptValue::Undefined))
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
                let mut taken = std::collections::BTreeSet::new();
                for classname in &names {
                    let q = [vm.string(classname), vm.string("classname")];
                    for e in matching(vm, sim, &q) {
                        if let ScriptValue::Object(o) = &e {
                            if !taken.insert((o.index, o.generation)) {
                                continue;
                            }
                            vm.set_raw_field(*o, inited, ScriptValue::Undefined);
                        }
                        v.push(e);
                    }
                }
                return Ok(list(v));
            }
        }
        let v = matching(vm, sim, a);
        Ok(list(v))
    });
    // Spawners only (the `actor_*` entities), matched like getentarray:
    // Buried's fuel step spawns its ghosts from these.
    f!("getspawnerarray", |vm, sim, _, a| {
        let v = matching(vm, sim, a);
        let zm = sim.resource::<Zm>();
        let v = v
            .into_iter()
            .filter(|v| {
                entnum(vm, v)
                    .and_then(|nn| zm.ents.get(&nn))
                    .is_some_and(|e| e.class_tag.starts_with("actor_"))
            })
            .collect();
        Ok(list(v))
    });
    f!("spawn", |vm, sim, _, a| {
        let cls = text(vm, a, 0);
        let origin = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let nn = sim.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(nn));
        let mut en = Ent {
            obj: Some(obj),
            class_tag: cls.clone(),
            origin,
            solid: true,
            ..Default::default()
        };
        if cls.starts_with("trigger_radius") {
            en.reach = arg(a, 3).as_float().unwrap_or(0.0);
            en.height = arg(a, 4).as_float().unwrap_or(0.0);
            // bo2mp: a use trigger spawned with no size (a turret's or a
            // claymore's pick-up, `spawn("trigger_radius_use", pos)`) is
            // reached from the player's use radius.
            if cls == "trigger_radius_use" && en.reach <= 0.0 && sim.resource::<Zm>().mp {
                let reach = vm
                    .dvars
                    .get("player_useradius")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(128.0);
                en.reach = reach;
                en.height = reach;
            }
        }
        // bo2mp: BO2's `spawn("trigger_damage", origin, spawnflags, radius,
        // height)`: a cylinder of that size, with full health.
        if cls == "trigger_damage" {
            en.reach = arg(a, 3).as_float().unwrap_or(0.0);
            en.height = arg(a, 4).as_float().unwrap_or(0.0);
            let (sf, hp) = (vm.intern("spawnflags"), vm.intern("health"));
            vm.set_raw_field(obj, sf, ScriptValue::Int(arg(a, 2).as_int().unwrap_or(0)));
            vm.set_raw_field(obj, hp, ScriptValue::Int(32000));
        }
        if cls.starts_with("trigger_box") {
            en.box_dims = Some([
                arg(a, 3).as_float().unwrap_or(64.0),
                arg(a, 4).as_float().unwrap_or(64.0),
                arg(a, 5).as_float().unwrap_or(64.0),
            ]);
        }
        if std::env::var_os("BO2MP_T6_BOUNDSLOG").is_some() && cls.starts_with("trigger") {
            diag::info!(
                Sim,
                "bo2zm t6 spawned {cls} at {origin:?} box {:?} radius {}",
                en.box_dims,
                en.reach
            );
        }
        sim.resource_mut::<Zm>().ents.insert(nn, en);
        Ok(ScriptValue::Object(obj))
    });
    // bo2mp: Black Ops II's `spawncollision( mdl, targetname, origin,
    // angles )` (GScr_SpawnCollision): a script_model of a collision-only
    // mdl, solid and never drawn (every map's main patches its holes and
    // ledges with them: Nuketown 31, "collision_physics_32x32x128" and the
    // like, named "collider"). The presence pass makes it a turned box for
    // movement, as the scripts' own collision models.
    f!("spawncollision", |vm, sim, _, a| {
        let mdl = text(vm, a, 0);
        let targetname = arg(a, 1).clone();
        let origin = arg(a, 2).as_vec3().unwrap_or([0.0; 3]);
        let angles = arg(a, 3).as_vec3().unwrap_or([0.0; 3]);
        let nn = sim.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(nn));
        let key = vm.intern("targetname");
        vm.set_raw_field(obj, key, targetname);
        let en = Ent {
            obj: Some(obj),
            class_tag: "script_model".to_owned(),
            origin,
            rot: angles,
            mesh: mdl,
            solid: true,
            ..Default::default()
        };
        sim.resource_mut::<Zm>().ents.insert(nn, en);
        Ok(ScriptValue::Object(obj))
    });
    m!("delete", |vm, sim, s, _| {
        let Some(nn) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        if is_player(vm, sim, s) {
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
        // Tranzit's Denizens count down when one burrows away, powerups
        // leave level.random_weapon_powerups; Die Rise's corpse-room count
        // sees `self` undefined and returns.
        if let Some(o) = obj
            && vm.alive(o)
        {
            vm.notify_queued(sim, o, "death", &[]);
        }
        // bo2mp: its smoke stops blocking sight (G_FreeEntity).
        super::natives_fx::forget_timed(sim, nn);
        // A player's body (`cloneplayer`) takes its engine corpse with it.
        super::playeranim::free_body(sim, nn);
        let item = {
            let mut zm = sim.resource_mut::<Zm>();
            zm.movers.list.remove(&nn);
            // A freed entity's cut paths join again (G_FreeEntity's
            // Path_ConnectPathsForEntity): Frost's and Rush's glass blockers
            // let bots through once the glass breaks.
            let cut = zm.nav.cut_count();
            zm.nav.connect(nn);
            if zm.nav.cut_count() != cut && std::env::var_os("BO2MP_T6_PATHLOG").is_some() {
                diag::info!(
                    Sim,
                    "bo2zm t6 delete ent{nn}: its paths join, {} links cut in all",
                    zm.nav.cut_count()
                );
            }
            zm.ents.remove(&nn).and_then(|e| e.item)
        };
        if let Some(number) = item {
            super::items::forget(sim, number);
        }
        if let Some(o) = obj {
            vm.free_object(sim, o);
        }
        Ok(ScriptValue::Undefined)
    });
    // physicslaunch(point, force): BO2 makes the mdl a physics object
    // that falls, comes to rest and is notified `stationary` (a ballistic
    // knife left in a zombie that died, `force_drop_knives_to_ground_on_death`,
    // then moves its pickup trigger onto it). Here it falls straight down
    // under the engine's gravity to the solid floor below and is notified
    // `stationary` there. PLACEHOLDER: the launch force is not applied
    // (the mdl's mass is not known here).
    m!("physicslaunch", |vm, sim, s, _| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let (from, linked, now) = {
            let zm = sim.resource::<Zm>();
            let e = &zm.ents[&nn];
            (e.origin, e.link.is_some(), zm.clock_millis)
        };
        let Some(obj) = s.as_obj() else {
            return Ok(ScriptValue::Undefined);
        };
        if linked {
            return Ok(ScriptValue::Undefined);
        }
        let down = [from[0], from[1], from[2] - 2000.0];
        let t = super::frame(sim).trace_static_world(
            from,
            down,
            [0.0; 3],
            [0.0; 3],
            crate::shot_trace::HARD_SURFACE_FLAG,
        );
        let drop = if !t.starts_inside && t.fraction < 1.0 {
            (from[2] - down[2]) * t.fraction
        } else {
            0.0
        };
        let secs = (2.0 * drop / super::movers::GRAVITY).sqrt();
        if secs > 0.0 {
            sim
                .resource_mut::<Zm>()
                .movers
                .gravity(nn, from, [0.0; 3], now, secs);
        }
        super::natives_game::notify_later(vm, sim, obj, "stationary", (secs * 1000.0) as i64);
        Ok(ScriptValue::Undefined)
    });
    m!("setmodel", |vm, sim, s, a| {
        // bo2mp: a player's body (BO2's faction class scripts) goes to every
        // client, which draws him with it. Also kept as `self.model`, which
        // is what Mob's Afterlife body is made from.
        let mdl = text(vm, a, 0);
        if is_player(vm, sim, s) {
            if let Some(nn) = entnum(vm, s) {
                if let Some(p) = sim.resource_mut::<Zm>().players.get_mut(&nn) {
                    p.mesh = mdl.clone();
                }
                super::playeranim::edit_body(sim, nn, |l| l.mesh = mdl);
            }
            return Ok(ScriptValue::Undefined);
        }
        let nn = me(vm, sim, s)?;
        if let Some(mut zm) = ent_mut(sim, nn) {
            let e = zm.ents.get_mut(&nn).unwrap();
            // A gun's parts fit only that gun: the box's teddy bear, set on
            // the gun it floated, wears no magazine.
            if e.mesh != mdl {
                e.weapon_parts.clear();
            }
            e.mesh = mdl;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("attach", |vm, sim, s, a| {
        if is_player(vm, sim, s) {
            if let Some(nn) = entnum(vm, s) {
                let added = (text(vm, a, 0), text(vm, a, 1));
                super::playeranim::edit_body(sim, nn, |l| l.attached.push(added));
            }
            return Ok(ScriptValue::Undefined);
        }
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let mdl = text(vm, a, 0);
        // BO2 tags are case-blind (the Panzer attaches at "J_Helmet"); bones
        // are stored lowercase, so its hit boxes only build with this.
        let tag_name = text(vm, a, 1).to_ascii_lowercase();
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().attached.push((mdl, tag_name));
        }
        Ok(ScriptValue::Undefined)
    });
    m!("detach", |vm, sim, s, a| {
        if is_player(vm, sim, s) {
            if let Some(nn) = entnum(vm, s) {
                let gone = text(vm, a, 0);
                super::playeranim::edit_body(sim, nn, |l| {
                    if let Some(i) = l.attached.iter().position(|(m, _)| *m == gone) {
                        l.attached.remove(i);
                    }
                });
            }
            return Ok(ScriptValue::Undefined);
        }
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let mdl = text(vm, a, 0);
        if let Some(mut zm) = ent_mut(sim, nn) {
            let e = zm.ents.get_mut(&nn).unwrap();
            if let Some(i) = e.attached.iter().position(|(m, _)| *m == mdl) {
                e.attached.remove(i);
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("detachall", |vm, sim, s, _| {
        if is_player(vm, sim, s) {
            if let Some(nn) = entnum(vm, s) {
                super::playeranim::detach_all(sim, nn);
            }
            return Ok(ScriptValue::Undefined);
        }
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().attached.clear();
        }
        Ok(ScriptValue::Undefined)
    });
    macro_rules! flagset {
        ($($name:literal => |$e:ident| $set:expr),* $(,)?) => {$(
            vm.bind($name, true, |vm, sim, s, _| {
                let Ok(nn) = me(vm, sim, s) else { return Ok(ScriptValue::Undefined) };
                if let Some(mut zm) = ent_mut(sim, nn) {
                    let $e = zm.ents.get_mut(&nn).unwrap();
                    $set;
                }
                Ok(ScriptValue::Undefined)
            });
        )*};
    }
    flagset!(
        "hide" => |e| {
            e.unseen = true;
            e.ghosted = false;
        },
        "show" => |e| {
            e.unseen = false;
            e.ghosted = false;
        },
        "ghost" => |e| {
            e.unseen = true;
            e.ghosted = true;
        },
        "notsolid" => |e| e.solid = false,
        "solid" => |e| e.solid = true,
        "unlink" => |e| {
            e.link = None;
            e.link_tag = None;
        },
        "stoploopsound" => |e| e.repeating_audio = None,
    );
    // Parts of its mdl hidden or shown again by tag_name (a buildable table's
    // parts appear as they are added); drawn by `presence`.
    fn part(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, lhs: &[ScriptValue], hide: bool) {
        let Ok(nn) = me(vm, game, st) else { return };
        let tag_name = text(vm, lhs, 0).to_ascii_lowercase();
        if let Some(mut zm) = ent_mut(game, nn) {
            let parts = &mut zm.ents.get_mut(&nn).unwrap().hidden_parts;
            parts.retain(|p| *p != tag_name);
            if hide {
                parts.push(tag_name);
            }
        }
    }
    m!("hidepart", |vm, sim, s, a| {
        part(vm, sim, s, a, true);
        Ok(ScriptValue::Undefined)
    });
    m!("showpart", |vm, sim, s, a| {
        part(vm, sim, s, a, false);
        Ok(ScriptValue::Undefined)
    });
    // Origins' crafted staffs on their pedestals.
    m!("showallparts", |vm, sim, s, _| {
        if let Ok(nn) = me(vm, sim, s)
            && let Some(mut zm) = ent_mut(sim, nn)
        {
            zm.ents.get_mut(&nn).unwrap().hidden_parts.clear();
        }
        Ok(ScriptValue::Undefined)
    });
    // A weapon's sim mdl on an entity (the box's floating gun, a wall
    // gun, Origins' staffs): the gun (or the mdl given), the parts BO2
    // hangs on it (magazine, scope, rocket) and its hide tags hidden, the
    // tags hidden before shown again.
    m!("useweaponmodel", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let w = super::weapon(sim, &text(vm, a, 0)).unwrap_or(0);
        let look = super::weapon_look(sim, w).unwrap_or_default();
        let mut mdl = text(vm, a, 1);
        if mdl.is_empty() {
            mdl = look.mesh;
        }
        if let Some(mut zm) = ent_mut(sim, nn) {
            let e = zm.ents.get_mut(&nn).unwrap();
            e.mesh = mdl;
            e.weapon_parts = look.parts;
            e.hidden_parts = look.hide;
        }
        Ok(ScriptValue::Undefined)
    });
    // The weapon's hide tags hidden on the mdl (the tags hidden before
    // shown again); the parts it wears stay (`useweaponmodel`'s).
    m!("useweaponhidetags", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let w = super::weapon(sim, &text(vm, a, 0)).unwrap_or(0);
        let hide = super::weapon_look(sim, w).map(|l| l.hide).unwrap_or_default();
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().hidden_parts = hide;
        }
        Ok(ScriptValue::Undefined)
    });
    // A player unlinks too (`playerlinkto`).
    m!("unlink", |vm, sim, s, _| {
        let Some(nn) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let mut zm = sim.resource_mut::<Zm>();
        if let Some(p) = zm.players.get_mut(&nn) {
            p.linked = None;
        } else if let Some(e) = zm.ents.get_mut(&nn) {
            e.link = None;
            e.link_tag = None;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setcandamage", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let on = flag(a, 0, true);
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().can_damage = on;
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: whether a grenade moving through this damage trigger damages
    // it (Black Ops II's entity flag these two set and clear).
    m!("enablegrenadetouchdamage", |vm, sim, s, _| {
        if let (Ok(nn), ScriptValue::Object(o)) = (me(vm, sim, s), s) {
            super::damage_triggers::grenade_touch(sim, nn, *o, true);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("disablegrenadetouchdamage", |vm, sim, s, _| {
        if let (Ok(nn), ScriptValue::Object(o)) = (me(vm, sim, s), s) {
            super::damage_triggers::grenade_touch(sim, nn, *o, false);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("sethintstring", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        // The text as the player reads it: the language string with its
        // arguments filled in (`&&1`, `&&2`).
        let args = super::name_players(vm, sim, a.get(1..).unwrap_or(&[]));
        let h = super::localize(vm, sim, arg(a, 0), &args);
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().hint = (!h.is_empty()).then_some(h);
        }
        Ok(ScriptValue::Undefined)
    });
    // sethintlowpriority(on): a low priority trigger's hint shows only when
    // no other usable trigger is there (`triggers::dispatch`).
    m!("sethintlowpriority", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let on = !matches!(arg(a, 0), ScriptValue::Int(0) | ScriptValue::Undefined);
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().hint_low = on;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setcursorhint", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let h = text(vm, a, 0);
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().cursor_hint = Some(h);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("playloopsound", |vm, sim, s, a| {
        if std::env::var_os("BO2MP_SNDLOG").is_some() {
            let alias = text(vm, a, 0);
            let on = me(vm, sim, s);
            diag::info!(Sim, "bo2mp sndlog loop {alias} on {on:?}");
        }
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let alias = text(vm, a, 0);
        if let Ok(want) = std::env::var("BO2MP_T6_SNDLOG")
            && (want == "1" || want.split(',').any(|w| alias.contains(w)))
        {
            let at = sim.resource::<Zm>().ents.get(&nn).map(|e| e.origin);
            diag::info!(Sim, "bo2zm t6 loop sound {alias} at {at:?}");
        }
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().repeating_audio = Some(alias);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("linkto", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(parent) = entnum(vm, arg(a, 0)) else {
            return Err("linkto needs an entity".into());
        };
        // An offset from a tag_name of the parent's mdl (the care package
        // under the drone's tag_care_package) is that far from the tag_name.
        let tag_name = text(vm, a, 1);
        let off = arg(a, 2).as_vec3().map(|o| {
            let parent_model = sim
                .resource::<Zm>()
                .ents
                .get(&parent)
                .map(|e| e.mesh.clone())
                .unwrap_or_default();
            match tag_in_model(sim, &parent_model, &tag_name) {
                Some(t) => std::array::from_fn(|i| o[i] + t[i]),
                None => o,
            }
        });
        let aoff = arg(a, 3).as_vec3().unwrap_or([0.0; 3]);
        // On a tag_name of the parent's mdl (where the posed skeleton has it
        // this tick), else on the parent itself.
        let tag_name = match arg(a, 1) {
            ScriptValue::Undefined => None,
            v => Some(vm.to_text(v)).filter(|t| !t.is_empty()),
        };
        let tag_pose = tag_name
            .as_deref()
            .and_then(|t| super::tags::tag_world(sim, parent, t))
            .map(|(o, axes)| (o, super::tags::axis_to_angles(axes)));
        let parent_pose = tag_pose.or_else(|| {
            let p = arg(a, 0).clone();
            let o = origin_of(vm, sim, &p);
            let ang = sim.resource::<Zm>().ents.get(&parent).map_or([0.0; 3], |e| e.rot);
            o.map(|o| (o, ang))
        });
        let (my, my_angles) = sim
            .resource::<Zm>()
            .ents
            .get(&nn)
            .map(|e| (e.origin, e.rot))
            .unwrap_or(([0.0; 3], [0.0; 3]));
        // No offsets given: BO2 keeps its turn relative to the parent too
        // (a Die Rise perk machine stays facing out of its car's door).
        let aoff = match (&off, parent_pose) {
            (None, Some((_, pa))) => [0, 1, 2].map(|i| my_angles[i] - pa[i]),
            _ => aoff,
        };
        let off = off.unwrap_or_else(|| match parent_pose {
            // Keep where it is relative to the parent.
            Some((po, pa)) => {
                let (f, r, u) = gsc_t6::math::facing_axes(pa);
                let d = gsc_t6::math::sub(my, po);
                [
                    gsc_t6::math::dot(d, f),
                    -gsc_t6::math::dot(d, r),
                    gsc_t6::math::dot(d, u),
                ]
            }
            None => [0.0; 3],
        });
        if let Some(mut zm) = ent_mut(sim, nn) {
            let e = zm.ents.get_mut(&nn).unwrap();
            e.link = Some((parent, off, aoff));
            e.link_tag = tag_name.filter(|_| tag_pose.is_some());
        }
        Ok(ScriptValue::Undefined)
    });
    m!("enablelinkto", |_, _, _, _| Ok(ScriptValue::Undefined));
    // Players standing on it ride it (`riders`: Tranzit's bus).
    m!("setmovingplatformenabled", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let on = arg(a, 0).as_float().is_none_or(|v| v != 0.0);
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().platform = on;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("getorigin", |vm, sim, s, _| Ok(
        origin_of(vm, sim, s).map_or(ScriptValue::Undefined, ScriptValue::Vec3)
    ));
    m!("getentitynumber", |vm, _, s, _| Ok(
        entnum(vm, s).map_or(ScriptValue::Undefined, |nn| ScriptValue::Int(nn as i32))
    ));
    // The engine's entity type, as the scripts' bookmarks read it: a
    // player 1, a thrown grenade (a missile) 4, anything else 0.
    m!("getentitytype", |vm, sim, s, _| {
        let nn = entnum(vm, s);
        let zm = sim.resource::<Zm>();
        let player = zm.players.values().any(|p| Some(p.obj) == s.as_obj());
        let missile = nn.and_then(|nn| zm.ents.get(&nn)).is_some_and(|e| e.class_tag == "grenade");
        Ok(ScriptValue::Int(if player { 1 } else if missile { 4 } else { 0 }))
    });
    // Where the tag_name is on its posed mdl (`tags`); the entity's own
    // origin and angles for a tag_name its mdl lacks.
    m!("gettagorigin", |vm, sim, s, a| {
        let tag_name = vm.to_text(arg(a, 0));
        if let Some(nn) = entnum(vm, s)
            && let Some((o, _)) = super::tags::tag_world(sim, nn, &tag_name)
        {
            return Ok(ScriptValue::Vec3(o));
        }
        Ok(origin_of(vm, sim, s).map_or(ScriptValue::Undefined, ScriptValue::Vec3))
    });
    m!("gettagangles", |vm, sim, s, a| {
        let nn = entnum(vm, s).unwrap_or(u32::MAX);
        let tag_name = vm.to_text(arg(a, 0));
        if let Some(axes) = super::tags::tag_world(sim, nn, &tag_name).map(|placed| placed.1) {
            return Ok(ScriptValue::Vec3(super::tags::axis_to_angles(axes)));
        }
        Ok(sim
            .resource::<Zm>()
            .ents
            .get(&nn)
            .map_or(ScriptValue::Vec3([0.0; 3]), |e| ScriptValue::Vec3(e.rot)))
    });
    // An entity's bounds: its mdl's (the file's own bounds), a player's
    // or an actor's body; abs = turned by its angles and placed.
    m!("getmins", |vm, sim, s, _| Ok(ScriptValue::Vec3(
        local_bounds(vm, sim, s).0
    )));
    m!("getmaxs", |vm, sim, s, _| Ok(ScriptValue::Vec3(
        local_bounds(vm, sim, s).1
    )));
    // A point in an entity's box (ScrCmd_GetPointInBounds ->
    // G_GetEntityBoundsPoint): each fraction -1..1 runs mins to maxs on its
    // axis, then the point is turned by the angles and placed at the origin.
    m!("getpointinbounds", |vm, sim, s, a| {
        let (lo, hi) = local_bounds(vm, sim, s);
        let mut p = [0.0f32; 3];
        for i in 0..3 {
            let half = (hi[i] - lo[i]) * 0.5;
            p[i] = half * num(a, i)? + lo[i] + half;
        }
        let origin = origin_of(vm, sim, s).unwrap_or_default();
        let angles = entnum(vm, s)
            .and_then(|nn| sim.resource::<Zm>().ents.get(&nn).map(|e| e.rot))
            .unwrap_or_default();
        let (f, r, u) = gsc_t6::math::facing_axes(angles);
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
        Ok(ScriptValue::Vec3(w))
    });
    m!("getabsmins", |vm, sim, s, _| Ok(ScriptValue::Vec3(
        abs_bounds(vm, sim, s).0
    )));
    m!("getabsmaxs", |vm, sim, s, _| {
        let (lo, hi) = abs_bounds(vm, sim, s);
        // BO2MP_T6_BOUNDSLOG=1: each asked entity's mdl, angles and bounds.
        if std::env::var_os("BO2MP_T6_BOUNDSLOG").is_some() {
            let (llo, lhi) = local_bounds(vm, sim, s);
            let e = entnum(vm, s).and_then(|nn| sim.resource::<Zm>().ents.get(&nn).cloned());
            if let Some(e) = e {
                diag::info!(
                    Sim,
                    "bo2zm t6 bounds {} at {:?} angles {:?}: local {llo:?}..{lhi:?} abs size {:?}",
                    e.mesh,
                    e.origin,
                    e.rot,
                    [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
                );
            }
        }
        Ok(ScriptValue::Vec3(hi))
    });
    m!("moveto", |vm, sim, s, a| {
        let nn = me(vm, sim, s)?;
        let to = vec3(a, 0)?;
        let t = num(a, 1)?;
        let (acc, dec) = (
            arg(a, 2).as_float().unwrap_or(0.0),
            arg(a, 3).as_float().unwrap_or(0.0),
        );
        let mut zm = sim.resource_mut::<Zm>();
        let now = zm.clock_millis;
        let from = zm.ents[&nn].origin;
        zm.movers.move_to(nn, from, to, now, t, acc, dec);
        Ok(ScriptValue::Undefined)
    });
    m!("movex", |vm, sim, s, a| move_axis(vm, sim, s, a, 0));
    m!("movey", |vm, sim, s, a| move_axis(vm, sim, s, a, 1));
    m!("movez", |vm, sim, s, a| move_axis(vm, sim, s, a, 2));
    m!("movegravity", |vm, sim, s, a| {
        let nn = me(vm, sim, s)?;
        let motion_vec = vec3(a, 0)?;
        let t = num(a, 1)?;
        let mut zm = sim.resource_mut::<Zm>();
        let now = zm.clock_millis;
        let from = zm.ents[&nn].origin;
        zm.movers.gravity(nn, from, motion_vec, now, t);
        Ok(ScriptValue::Undefined)
    });
    m!("rotateto", |vm, sim, s, a| {
        let nn = me(vm, sim, s)?;
        let to = vec3(a, 0)?;
        let t = num(a, 1)?;
        let (acc, dec) = (
            arg(a, 2).as_float().unwrap_or(0.0),
            arg(a, 3).as_float().unwrap_or(0.0),
        );
        let mut zm = sim.resource_mut::<Zm>();
        let now = zm.clock_millis;
        let from = zm.ents[&nn].rot;
        // Shortest way round, as the engine turns.
        let mut target = to;
        for i in 0..3 {
            let d = gsc_t6::math::angle_clamp180(to[i] - from[i]);
            target[i] = from[i] + d;
        }
        zm.movers.rotate_to(nn, from, target, now, t, acc, dec);
        Ok(ScriptValue::Undefined)
    });
    // vibrate(dir, degrees, period, secs): rocks about its origin toward
    // `dir`, fading out (a landed perk machine).
    m!("vibrate", |vm, sim, s, a| {
        let nn = me(vm, sim, s)?;
        let dir = vec3(a, 0)?;
        let (amp, period, secs) = (num(a, 1)?, num(a, 2)?, num(a, 3)?);
        let mut zm = sim.resource_mut::<Zm>();
        let now = zm.clock_millis;
        let base = zm.ents[&nn].rot;
        zm.movers.vibrate(nn, base, dir, amp, period, now, secs);
        Ok(ScriptValue::Undefined)
    });
    m!("rotateyaw", |vm, sim, s, a| rotate_axis(
        vm, sim, s, a, 1
    ));
    m!("rotatepitch", |vm, sim, s, a| rotate_axis(
        vm, sim, s, a, 0
    ));
    m!("rotateroll", |vm, sim, s, a| rotate_axis(
        vm, sim, s, a, 2
    ));
    // `a istouching(b)`: whichever of the two has a volume (a brush, a
    // trigger cylinder or a spawned `trigger_box`, such as Hijacked's lower
    // Hardpoint zone) against the other's box (a player's, an actor's, else
    // a point).
    m!("istouching", |vm, sim, s, a| {
        let other = arg(a, 0).clone();
        let (Some(sn), Some(on)) = (entnum(vm, s), entnum(vm, &other)) else {
            return Ok(ScriptValue::Int(0));
        };
        let volume_of = |sim: &World, nn: u32| -> Option<Ent> {
            if !has_volume(sim, nn) {
                return None;
            }
            sim.resource::<Zm>().ents.get(&nn).cloned()
        };
        let (vol, toucher) = match (volume_of(sim, on), volume_of(sim, sn)) {
            (Some(v), _) => (v, sn),
            (None, Some(v)) => (v, on),
            // Two solid things (a falling care package and a player): their
            // sim boxes, its own grown by the extra boundary, overlap.
            (None, None) => {
                let (mut lo, mut hi) = abs_bounds(vm, sim, s);
                if lo == hi {
                    return Ok(ScriptValue::Int(0));
                }
                let grow = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
                for i in 0..3 {
                    lo[i] -= grow[i];
                    hi[i] += grow[i];
                }
                let other_box = if sim.resource::<Zm>().players.contains_key(&on) {
                    super::triggers::player_box(sim, on)
                } else {
                    Some(abs_bounds(vm, sim, &other))
                };
                let Some((olo, ohi)) = other_box else {
                    return Ok(ScriptValue::Int(0));
                };
                let hit = (0..3).all(|i| lo[i] <= ohi[i] && olo[i] <= hi[i]);
                return Ok(ScriptValue::bool(hit));
            }
        };
        // A script_model is as big as its mdl: BO2's setmodel sets its
        // bounds from the mdl and istouching places them at its origin,
        // unturned (a turbine set down in the tower counts for Maxis even
        // though its origin is on the floor, below the tower's volume).
        let mdl = sim
            .resource::<Zm>()
            .ents
            .get(&toucher)
            .filter(|o| o.class_tag == "script_model" && !o.mesh.is_empty() && !o.mesh.starts_with('*'))
            .map(|o| (o.origin, o.mesh.clone()));
        let model_box = mdl.and_then(|(p, m)| {
            let (mid, half) = super::frame(sim).model_capability(&m).flatten()?.bounds?;
            Some((
                std::array::from_fn(|i| p[i] + mid[i] - half[i]),
                std::array::from_fn(|i| p[i] + mid[i] + half[i]),
            ))
        });
        let bounds = if sim.resource::<Zm>().players.contains_key(&toucher) {
            super::triggers::player_box(sim, toucher)
        } else if model_box.is_some() {
            model_box
        } else {
            let zm = sim.resource::<Zm>();
            zm.ents.get(&toucher).map(|o| {
                let p = o.origin;
                // Buried's head chopper hangs its blade at the head of
                // Buried's 72 tall zombies. Nuketown, Die Rise and Mob set
                // theirs to 48 (zombie_init_done), so brought onto those
                // maps its blade sees a standing zombie's full 72 (a 24
                // tall crawler still passes under it, as on Buried).
                let chopper = vol
                    .link
                    .and_then(|(w, _, _)| zm.ents.get(&w))
                    .is_some_and(|w| w.mesh == "t6_wpn_zmb_chopper");
                match zm.actors.get(&toucher) {
                    Some(act) => (
                        [p[0] - act.reach, p[1] - act.reach, p[2]],
                        [
                            p[0] + act.reach,
                            p[1] + act.reach,
                            p[2] + if chopper && act.height == 48.0 { 72.0 } else { act.height },
                        ],
                    ),
                    None => (
                        [p[0] - 1.0, p[1] - 1.0, p[2] - 1.0],
                        [p[0] + 1.0, p[1] + 1.0, p[2] + 1.0],
                    ),
                }
            })
        };
        let Some((mut mins, mut maxs)) = bounds else {
            return Ok(ScriptValue::Int(0));
        };
        // istouching(b, pad): the box grows by `pad` on every side.
        if a.len() > 1 {
            let pad = vec3(a, 1)?;
            for i in 0..3 {
                mins[i] -= pad[i];
                maxs[i] += pad[i];
            }
        }
        let hit = super::triggers::touches(sim, &vol, mins, maxs);
        // BO2MP_T6_TOUCHLOG=<targetname>[,...]: each test against a volume
        // with that name, its brush bounds and the answer (Die Rise refuses
        // Who's Who's body inside an elevator volume).
        if let Ok(names) = std::env::var("BO2MP_T6_TOUCHLOG") {
            let tn = vm.intern("targetname");
            let name = vol.obj.map(|o| vm.to_text(&vm.raw_field(o, tn))).unwrap_or_default();
            if names.split(',').any(|nn| nn == name) {
                let bb = super::triggers::brush_bounds(sim, &vol);
                diag::info!(
                    Sim,
                    "bo2zm t6 touch {name} ({} {:?}) at {:?} bounds {bb:?} vs {mins:?}..{maxs:?}: {hit}",
                    vol.class_tag,
                    vol.brush,
                    vol.origin
                );
            }
        }
        Ok(ScriptValue::bool(hit))
    });
    // bo2mp: istouchingswept(other, minz?, maxz?) (BO2's
    // `ScrCmd_IsTouchingSwept`): does the box of the one that is not a brush
    // or cylinder, its bottom taken down to `minz` and its top up to `maxz`
    // (each only ever further out), meet the other? A dropped flag asks it of
    // the mine, hurt and elevator volumes on its way down.
    m!("istouchingswept", |vm, sim, s, a| {
        let other = arg(a, 0).clone();
        let (Some(sn), Some(on)) = (entnum(vm, s), entnum(vm, &other)) else {
            return Err("not an entity".into());
        };
        let (sv, ov) = (has_volume(sim, sn), has_volume(sim, on));
        if sv && ov {
            return Err("istouchingswept cannot be called on 2 brush/cylinder entities".into());
        }
        let (toucher, target) = if sv {
            (other, s.clone())
        } else {
            (s.clone(), other)
        };
        let Some((mut lo, mut hi)) = own_box(vm, sim, &toucher) else {
            return Ok(ScriptValue::Int(0));
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
        Ok(ScriptValue::bool(entity_contact(vm, sim, &target, lo, hi)))
    });
    // bo2mp: istouchingvolume(origin, mins, maxs) (BO2's
    // `ScrCmd_IsTouchingVolume`): does the box about the origin (made at
    // least as tall as it is wide) meet this entity? A tactical insertion
    // asks it of the care packages where it would go down.
    m!("istouchingvolume", |vm, sim, s, a| {
        let o = vec3(a, 0)?;
        let (mins, maxs) = (vec3(a, 1)?, vec3(a, 2)?);
        let mut lo: [f32; 3] = std::array::from_fn(|i| o[i] + mins[i]);
        let mut hi: [f32; 3] = std::array::from_fn(|i| o[i] + maxs[i]);
        expand_to_width(&mut lo, &mut hi);
        Ok(ScriptValue::bool(entity_contact(vm, sim, s, lo, hi)))
    });
    // bo2mp: allowbottargetting(on) (BO2's `ScrCmd_AllowBotTargetting`):
    // while it is off, bots do not take it for a target (Drone's and
    // Vertigo's doors turn it off).
    m!("allowbottargetting", |vm, sim, s, a| {
        let nn = me(vm, sim, s)?;
        let on = super::int(a, 0)? != 0;
        let mut zm = sim.resource_mut::<Zm>();
        if on {
            zm.no_bot_target.remove(&nn);
        } else {
            zm.no_bot_target.insert(nn);
        }
        Ok(ScriptValue::Undefined)
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
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: islinkedto(en) (BO2's `ScrCmd_IsLinkedTo`): is it linked
    // straight to that entity (a sticky grenade on a carried object)?
    m!("islinkedto", |vm, sim, s, a| {
        let parent = entnum(vm, arg(a, 0)).ok_or("not an entity")?;
        let Some(nn) = entnum(vm, s) else {
            return Ok(ScriptValue::Int(0));
        };
        let zm = sim.resource::<Zm>();
        let linked = match zm.players.get(&nn) {
            Some(p) => p.linked,
            None => zm.ents.get(&nn).and_then(|e| e.link.map(|l| l.0)),
        };
        Ok(ScriptValue::bool(linked == Some(parent)))
    });
    // bo2mp: directionalhitindicator(bits0, bits1?) (BO2's
    // `GScr_DirectionalHitIndicator`): BO2 sends the player arrows towards
    // the attackers it names (a bit per entity) for its HUD; ours has no
    // such arrows, so the call is checked and dropped.
    m!("directionalhitindicator", |vm, sim, s, a| {
        super::client(vm, sim, s)?;
        super::int(a, 0)?;
        if a.len() > 1 {
            super::int(a, 1)?;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setinvisibletoplayer", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(p) = entnum(vm, arg(a, 0)) else {
            return Ok(ScriptValue::Undefined);
        };
        let hide = flag(a, 1, true);
        if let Some(mut zm) = ent_mut(sim, nn) {
            let e = zm.ents.get_mut(&nn).unwrap();
            if hide {
                e.invisible_to.insert(p);
            } else {
                shown_to_one(e, p);
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setvisibletoplayer", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(p) = entnum(vm, arg(a, 0)) else {
            return Ok(ScriptValue::Undefined);
        };
        if let Some(mut zm) = ent_mut(sim, nn) {
            shown_to_one(zm.ents.get_mut(&nn).unwrap(), p);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setinvisibletoall", |vm, sim, s, _| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().invisible_to_all = true;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setvisibletoall", |vm, sim, s, _| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        if let Some(mut zm) = ent_mut(sim, nn) {
            let e = zm.ents.get_mut(&nn).unwrap();
            e.invisible_to_all = false;
            e.invisible_to.clear();
        }
        Ok(ScriptValue::Undefined)
    });
    // ScrCmd_SetVisibleToTeam: shown to that team's players only. The
    // Monkey Bomb's decoy body goes to the zombies' team, which has no
    // players, so after its `setinvisibletoall` no player sees it.
    m!("setvisibletoteam", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let team = vm.to_text(arg(a, 0));
        if team == "allies" {
            if let Some(mut zm) = ent_mut(sim, nn) {
                let e = zm.ents.get_mut(&nn).unwrap();
                e.invisible_to_all = false;
                e.invisible_to.clear();
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("triggerenable", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let on = flag(a, 0, true);
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().trigger_off = !on;
        }
        Ok(ScriptValue::Undefined)
    });
    // BO2 (GScr_SetTeamForTrigger, GScr_SetExcludeTeamForTrigger): the team
    // a radius, box or use trigger answers, or the one it ignores (a
    // claymore's trigger ignores its owner's side).
    m!("setteamfortrigger", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let team = vm.to_text(arg(a, 0));
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().trigger_team = Some((team, false));
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setexcludeteamfortrigger", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let team = vm.to_text(arg(a, 0));
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().trigger_team = Some((team, true));
        }
        Ok(ScriptValue::Undefined)
    });
    // A wall buy's trigger is the weapon's own small box on the wall: the
    // player uses it by looking at it from close by, not by standing in
    // it (seats and clip keep him a body away from the truck's chalk).
    m!("usetriggerrequirelookat", |vm, sim, s, _| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        if let Some(mut zm) = ent_mut(sim, nn) {
            zm.ents.get_mut(&nn).unwrap().look_at = true;
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: ScrCmd_UseBy (common_scripts/utility::trigger_use): used by
    // him as if he pressed its use key.
    m!("useby", |vm, sim, s, a| {
        if let ScriptValue::Object(o) = s
            && entnum(vm, arg(a, 0)).is_some()
        {
            vm.notify_str(sim, *o, "trigger", std::slice::from_ref(arg(a, 0)));
        }
        Ok(ScriptValue::Undefined)
    });
    for name in [
        // bo2mp: a render hint (the client skips relinking a fast mover's
        // mdl), and the createfx editor's turns and text.
        "setcheapflag",
        "devaddpitch",
        "devaddroll",
        "devaddyaw",
        "clearalltextafterhudelem",
        "triggerignoreteam",
        "setvisibletoallexceptteam",
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
        "notsolidcapsule",
        "dontinterpolate",
        "setscale",
        "setzbarrierpiecestate",
        "showzbarrierpiece",
        "hidezbarrierpiece",
        "zbarrierpieceuseboxriselogic",
        "zbarrierpieceusedefaultmodel",
        "zbarrierpieceuseupgradedmodel",
        // client draw hints (Origins' giant robots, the crystal biplane)
        "setforcenocull",
        "setlightintensity",
        "setplayercollision",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // bo2mp: a grenade or missile the scripts hold flies again from where it
    // is, under gravity, turning at the second vector if given (the decoy's
    // hops, a combat axe let go of a live player): Black Ops II's
    // GScr_Launch. Anything else stays put here.
    m!("launch", |vm, sim, s, a| {
        let motion_vec = vec3(a, 0)?;
        if motion_vec.iter().any(|v| !v.is_finite()) {
            return Err(format!(
                "invalid velocity parameter in launch command: {} {} {}",
                motion_vec[0], motion_vec[1], motion_vec[2]
            ));
        }
        let avel = if a.len() > 1 { Some(vec3(a, 1)?) } else { None };
        if let Some(nn) = entnum(vm, s) {
            super::killstreaks::relaunch(sim, nn, motion_vec, avel);
        }
        Ok(ScriptValue::Undefined)
    });
    // A limp body (a ragdoll): every zombie death goes limp part way through
    // its death anim (zm_death's `waitforragdoll`), the Trample Steam throws
    // its kills (`launchragdoll`). Here it is a mark on the body and its last
    // throw; each client drops the body from the pose it had, under gravity
    // (render_anim's ragdoll). As in Black Ops II, a living body goes limp
    // too (the traps throw theirs, then kill them), and a throw only moves a
    // body that is limp already.
    m!("startragdoll", |vm, sim, s, _| {
        if let Some(nn) = entnum(vm, s) {
            go_limp(sim, nn);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("launchragdoll", |vm, sim, s, a| {
        let throw = vec3(a, 0)?;
        if let Some(nn) = entnum(vm, s)
            && let Some(ragdoll) = sim
                .resource_mut::<Zm>()
                .into_inner()
                .ents
                .get_mut(&nn)
                .and_then(|e| e.ragdoll.as_mut())
        {
            ragdoll.launch = throw;
            ragdoll.launches = ragdoll.launches.wrapping_add(1);
            diag::info!(Sim, "bo2zm t6 launchragdoll ent{nn}: ({:.0} {:.0} {:.0})", throw[0], throw[1], throw[2]);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("isragdoll", |vm, sim, s, _| {
        let limp = entnum(vm, s)
            .and_then(|nn| sim.resource::<Zm>().ents.get(&nn))
            .is_some_and(|e| e.ragdoll.is_some());
        Ok(ScriptValue::Int(i32::from(limp)))
    });
    // A script mdl's animation (the Monkey Bomb's clap): actors play
    // theirs through their state machine, not this.
    m!("setanim", |vm, sim, s, a| {
        let Ok(nn) = me(vm, sim, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let ScriptValue::Anim(i) = arg(a, 0) else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(&(_, name)) = vm.program.anims.get(*i as usize) else {
            return Ok(ScriptValue::Undefined);
        };
        let clip = vm.strings.get(name).to_ascii_lowercase();
        let mut zm = sim.resource_mut::<Zm>();
        let now = zm.clock_millis;
        if let Some(e) = zm.ents.get_mut(&nn)
            && e.anim.as_ref().is_none_or(|(c, _)| *c != clip)
        {
            e.anim = Some((clip, now));
        }
        Ok(ScriptValue::Undefined)
    });
    m!("clearanim", |vm, sim, s, _| {
        if let Ok(nn) = me(vm, sim, s)
            && let Some(e) = sim.resource_mut::<Zm>().ents.get_mut(&nn)
        {
            e.anim = None;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("resetmissiledetonationtime", |vm, sim, s, a| {
        if let Ok(nn) = me(vm, sim, s) {
            let secs = arg(a, 0).as_float();
            let now = sim.resource::<Zm>().clock_millis;
            super::grenades::reset_fuse(sim, nn, secs, now);
        }
        Ok(ScriptValue::Undefined)
    });
    // A planted claymore a zombie walked into (or one planted where it
    // cannot stay): it goes off now.
    m!("detonate", |vm, sim, s, _| {
        if let Ok(nn) = me(vm, sim, s) {
            let now = sim.resource::<Zm>().clock_millis;
            super::grenades::detonate_now(sim, nn, now);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("playsoundwithnotify", |vm, sim, s, a| {
        // The sound system is not wired to scripts yet: notify after a short
        // while so waiting scripts go on.
        let name = text(vm, a, 1);
        if let Some(o) = s.as_obj() {
            super::natives_game::notify_later(vm, sim, o, &name, 500);
        }
        Ok(ScriptValue::Undefined)
    });
}

/// BO2's `setvisibletoplayer` clears this player's bit only
/// (ScrCmd_SetVisibleToPlayer): after `setinvisibletoall` (every bit set) the
/// others still do not see it (Mob's ghost-only props, one ghost at a time).
fn shown_to_one(e: &mut Ent, p: u32) {
    if e.invisible_to_all {
        e.invisible_to_all = false;
        e.invisible_to.extend(0..32);
    }
    e.invisible_to.remove(&p);
}

fn move_axis(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, lhs: &[ScriptValue], axis: usize) -> Result<ScriptValue, String> {
    let n = me(vm, game, st)?;
    let d = num(lhs, 0)?;
    let t = num(lhs, 1)?;
    let (acc, dec) = (
        arg(lhs, 2).as_float().unwrap_or(0.0),
        arg(lhs, 3).as_float().unwrap_or(0.0),
    );
    let mut zm = game.resource_mut::<Zm>();
    let now = zm.clock_millis;
    let from = zm.ents[&n].origin;
    let mut to = from;
    to[axis] += d;
    zm.movers.move_to(n, from, to, now, t, acc, dec);
    Ok(ScriptValue::Undefined)
}

fn rotate_axis(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, lhs: &[ScriptValue], axis: usize) -> Result<ScriptValue, String> {
    let n = me(vm, game, st)?;
    let d = num(lhs, 0)?;
    let t = num(lhs, 1)?;
    let (acc, dec) = (
        arg(lhs, 2).as_float().unwrap_or(0.0),
        arg(lhs, 3).as_float().unwrap_or(0.0),
    );
    let mut zm = game.resource_mut::<Zm>();
    let now = zm.clock_millis;
    let from = zm.ents[&n].rot;
    let mut to = from;
    to[axis] += d;
    zm.movers.rotate_to(n, from, to, now, t, acc, dec);
    Ok(ScriptValue::Undefined)
}

/// Model-space bounds of an entity (mins, maxs).
pub(super) fn local_bounds(vm: &Vm<World>, game: &mut World, st: &ScriptValue) -> ([f32; 3], [f32; 3]) {
    const BODY: ([f32; 3], [f32; 3]) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    let Some(n) = entnum(vm, st) else { return BODY };
    if is_player(vm, game, st) || game.resource::<Zm>().actors.contains_key(&n) {
        return BODY;
    }
    let Some(e) = game.resource::<Zm>().ents.get(&n).cloned() else {
        return ([0.0; 3], [0.0; 3]);
    };
    if let Some([w, l, h]) = e.box_dims {
        return ([-w * 0.5, -l * 0.5, -h * 0.5], [w * 0.5, l * 0.5, h * 0.5]);
    }
    if e.reach > 0.0 {
        return ([-e.reach, -e.reach, 0.0], [e.reach, e.reach, e.height]);
    }
    if let Some(b) = brush_bounds(game, &e) {
        return b;
    }
    if e.mesh.is_empty() || e.mesh.starts_with('*') {
        return ([0.0; 3], [0.0; 3]);
    }
    let f = super::frame(game);
    match f
        .model_capability(&e.mesh)
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
    let i = super::triggers::brush_index(e).or_else(|| e.mesh.strip_prefix('*')?.parse().ok())?;
    let f = super::frame(world);
    let m = f.collision_submodel_set().models.get(i as usize)?;
    Some((m.lo, m.hi))
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
        super::triggers::brush_index(e).is_some() || e.reach > 0.0 || e.box_dims.is_some()
    })
}

/// bo2mp: an entity's own box in the world: its origin plus its mins and
/// maxs, not turned (a player's by his stance).
fn own_box(vm: &Vm<World>, game: &mut World, v: &ScriptValue) -> Option<([f32; 3], [f32; 3])> {
    let n = entnum(vm, v)?;
    if game.resource::<Zm>().players.contains_key(&n) {
        return super::triggers::player_box(game, n);
    }
    let o = origin_of(vm, game, v)?;
    let (lo, hi) = local_bounds(vm, game, v);
    Some((
        std::array::from_fn(|i| o[i] + lo[i]),
        std::array::from_fn(|i| o[i] + hi[i]),
    ))
}

/// bo2mp: does the box meet the entity (BO2's `SV_EntityContact`)? A
/// player by his box, a brush or cylinder by its volume, anything else by
/// its world bounds.
fn entity_contact(vm: &Vm<World>, game: &mut World, tgt: &ScriptValue, low: [f32; 3], hi: [f32; 3]) -> bool {
    let Some(nn) = entnum(vm, tgt) else {
        return false;
    };
    let meets = |(bl, bh): ([f32; 3], [f32; 3])| (0..3).all(|i| low[i] <= bh[i] && bl[i] <= hi[i]);
    if game.resource::<Zm>().players.contains_key(&nn) {
        return super::triggers::player_box(game, nn).is_some_and(meets);
    }
    if has_volume(game, nn) {
        let Some(e) = game.resource::<Zm>().ents.get(&nn).cloned() else {
            return false;
        };
        return super::triggers::touches(game, &e, low, hi);
    }
    meets(abs_bounds(vm, game, tgt))
}

/// World bounds, the way BO2 links an entity: a player or zombie is its box
/// placed (never turned); a brush model is its box turned, placed; anything
/// else (a script model, a trigger, a vehicle) is a cube as wide as its box's
/// radius every way, whatever way it faces. All grow by 1. The wall buys
/// size their use box from a gun model's cube (`getabsmins`), so turning the
/// gun's own box instead gave a chalk on a wall running along x a box 3
/// units thick: a player could not buy the Bus Depot shotgun unless they stood
/// against it.
pub(super) fn abs_bounds(vm: &Vm<World>, game: &mut World, st: &ScriptValue) -> ([f32; 3], [f32; 3]) {
    let (lo, hi) = local_bounds(vm, game, st);
    let origin = origin_of(vm, game, st).unwrap_or([0.0; 3]);
    let ent = entnum(vm, st).and_then(|n| {
        let zm = game.resource::<Zm>();
        let sentient = zm.players.contains_key(&n) || zm.actors.contains_key(&n);
        Some((sentient, zm.ents.get(&n)?.clone()))
    });
    let brush = ent.as_ref().is_some_and(|(_, e)| e.mesh.starts_with('*'));
    if ent.as_ref().is_none_or(|(sentient, _)| *sentient) {
        return (
            std::array::from_fn(|i| origin[i] + lo[i] - 1.0),
            std::array::from_fn(|i| origin[i] + hi[i] + 1.0),
        );
    }
    if !brush {
        let r = (0..3)
            .map(|i| lo[i].abs().max(hi[i].abs()).powi(2))
            .sum::<f32>()
            .sqrt()
            + 1.0;
        return (origin.map(|o| o - r), origin.map(|o| o + r));
    }
    let angles = ent.map_or([0.0; 3], |(_, e)| e.rot);
    let (f, r, u) = gsc_t6::math::facing_axes(angles);
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for corner in 0..8 {
        let p = [
            if corner & 1 == 0 { lo[0] } else { hi[0] },
            if corner & 2 == 0 { lo[1] } else { hi[1] },
            if corner & 4 == 0 { lo[2] } else { hi[2] },
        ];
        for i in 0..3 {
            // local y is left (the right vector's opposite).
            let w = origin[i] + f[i] * p[0] - r[i] * p[1] + u[i] * p[2];
            low[i] = low[i].min(w);
            high[i] = high[i].max(w);
        }
    }
    (low.map(|v| v - 1.0), high.map(|v| v + 1.0))
}

/// Mark entity `n` limp (`startragdoll`), once: its fall starts now. `None`
/// for a living actor or a gone entity.
pub(super) fn go_limp(world: &mut World, n: u32) {
    let zm = world.resource_mut::<Zm>().into_inner();
    let start_ms = zm.clock_millis as i32;
    if let Some(ent) = zm.ents.get_mut(&n) {
        ent.ragdoll.get_or_insert(xmodel_runtime::DObjRagdoll {
            began_at_ms: start_ms,
            launch: [0.0; 3],
            launches: 0,
        });
    }
}

