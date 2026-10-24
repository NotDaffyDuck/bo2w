//! bo2mp: a Black Ops II map's breakables (Nuketown's mannequins, its cars,
//! its clock; the clipmap's loose and breakable props, Raid's pots and
//! bottles). A hit breaks pieces in t5_destructible; the breaks reach the
//! map's scripts here, as BO2's `codecallback_destructibleevent` ("broken",
//! "breakafter"), and the entity's G_Damage ("damage", then "death" once
//! its health is gone).

use bevy_ecs::prelude::World;
use gsc_t6::{Value, Vm};

use super::{Zm, frame, tick, weapon_text, with_vm};
use crate::t5_destructible::{DamageKind, Notice};
use crate::world::ClientId;
use crate::{AuthorityModelOwner, ScriptModelId};

/// Which of a piece's scales a hit counts by, from its means of death.
pub(super) fn kind(means: &str) -> DamageKind {
    match means {
        "MOD_MELEE" => DamageKind::Melee,
        "MOD_IMPACT" => DamageKind::Impact,
        "MOD_BURNED" => DamageKind::Burned,
        "MOD_EXPLOSIVE" => DamageKind::Blast,
        _ if means.contains("SPLASH") => DamageKind::Splash,
        _ if ["GRENADE", "PROJECTILE", "EXPLOSIVE"]
            .iter()
            .any(|k| means.contains(k)) =>
        {
            DamageKind::Explosive
        }
        _ if means.contains("BULLET") || means == "MOD_HEAD_SHOT" => DamageKind::Bullet,
        _ => DamageKind::Script,
    }
}

/// Whether thing `id` takes damage on a BO2 map (`takedamage`): Some(true)
/// for a breakable (BO2 makes each one take damage), Some(false) for a live
/// AI or what a script let take damage; None for anything else, or when no
/// BO2 map runs.
pub(crate) fn takes_damage(world: &World, id: ScriptModelId) -> Option<bool> {
    if !world.contains_resource::<super::T6Runtime>() {
        return None;
    }
    let zm = world.resource::<Zm>();
    let n = zm
        .presences
        .by_ent
        .iter()
        .find(|(_, s)| s.id == id)
        .map(|(n, _)| *n)?;
    let ent = zm.ents.get(&n);
    let breakable = ent.is_some_and(|e| e.destructible.is_some());
    (breakable || ent.is_some_and(|e| e.can_damage) || zm.actors.get(&n).is_some_and(|a| a.alive))
        .then_some(breakable)
}

/// Whether `name` is a Black Ops II loose or breakable prop's own
/// definition (t6_m2 `dyn_ent_props`).
pub(crate) fn is_dyn(name: &str) -> bool {
    name.starts_with("dyn_")
}

/// The loop a breakable burns with once lit, from BO2's own defs (the
/// stage after the fire notify loops `dst_car_fire` / `exp_barrel_fire`
/// until it blows up). The stage's loop is not played by the engine here,
/// so it rides the entity's `playloopsound` row.
fn fire_loop(notify: &str) -> Option<&'static str> {
    match notify {
        "destructible_car_fire" => Some("dst_car_fire"),
        "destructible_barrel_fire" => Some("exp_barrel_fire"),
        _ => None,
    }
}

/// The model a breakable is shown by (its presence).
fn shown(zm: &Zm, n: u32) -> Option<ScriptModelId> {
    zm.presences.by_ent.get(&n).map(|p| p.id)
}

/// A script's `radiusdamage` (by `means`, from `inflictor`): the breakables
/// in reach take it as a blast (a car's explosion sets off the barrel
/// beside it); the blast sees through the inflictor's own model.
#[allow(clippy::too_many_arguments)]
pub(super) fn script_blast(
    world: &mut World,
    origin: [f32; 3],
    range: f32,
    amount_at: impl Fn(f32) -> f32,
    means: &str,
    inflictor: &Value,
    attacker: &Value,
    weapon: &str,
) {
    if world.resource::<Zm>().destructibles.is_empty() {
        return;
    }
    let attacker = attacker.as_obj().and_then(|o| {
        world
            .resource::<Zm>()
            .players
            .iter()
            .find(|(_, p)| p.obj == o)
            .map(|(n, _)| ClientId(*n))
    });
    let skip: Vec<AuthorityModelOwner> = inflictor
        .as_obj()
        .and_then(|o| {
            let zm = world.resource::<Zm>();
            zm.ents
                .iter()
                .find(|(_, e)| e.obj == Some(o))
                .and_then(|(n, _)| shown(zm, *n))
        })
        .map(AuthorityModelOwner::ScriptModel)
        .into_iter()
        .collect();
    let means = crate::world_objects::t6_means_from_name(means);
    let t = tick(world);
    let mut f = frame(world);
    let weapon = crate::script_player::weapon_named(&f, weapon).unwrap_or(0);
    for (_, notifies) in crate::t5_destructible::apply_blast_by(
        &mut f, t, origin, range, amount_at, means, &skip, attacker, weapon,
    ) {
        for notify in notifies {
            diag::warn!(Sim, "T5 destructible script notify unhosted: {notify}");
        }
    }
}

/// BO2MP_T6_FRAGTEST=<secs>@x,y,z (test aid): once, at that match time, the
/// first player's frag grenade goes off at x,y,z through the grenades' own
/// blast (its radius and damage; players, glass and breakables in reach),
/// so a prop's breaks can be checked against a thrower and his weapon.
fn frag_test(world: &mut World, now: i64) {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};
    static AT: OnceLock<Option<(i64, [f32; 3])>> = OnceLock::new();
    static DONE: AtomicBool = AtomicBool::new(false);
    let Some((at, spot)) = *AT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_FRAGTEST").ok()?;
        let (s, spot) = v.split_once('@')?;
        let at = (s.trim().parse::<f32>().ok()? * 1000.0) as i64;
        let n: Vec<f32> = spot.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (n.len() == 3).then(|| (at, [n[0], n[1], n[2]]))
    }) else {
        return;
    };
    if now < at || DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(attacker) = world.resource::<Zm>().players.keys().next().map(|n| ClientId(*n)) else {
        return;
    };
    let t = tick(world);
    let mut f = frame(world);
    let Ok(weapon) = crate::script_player::weapon_named(&f, "frag_grenade_mp") else {
        return;
    };
    let Some(facts) = f.equipment_facts_for(weapon) else {
        return;
    };
    let blast = crate::damage::ExplosionBlast {
        cone: None,
        origin: spot,
        radius: facts.explosion_radius.max(facts.explosion_radius_min).max(0) as f32,
        inner_damage: facts.explosion_inner_damage as f32,
        outer_damage: facts.explosion_outer_damage.max(0) as f32,
        weapon,
        source: crate::identities::DamageSource::Projectile(crate::identities::ProjectileId(0)),
        attacker,
        attacker_life: Default::default(),
        killcam_entity_start_time: 0,
    };
    diag::info!(Sim, "codmp frag test: client {} frag at {spot:?}, radius {}", attacker.0, blast.radius);
    crate::damage::apply_explosion_blast(&mut f, t, &blast);
}

/// BO2MP_T6_PROPBLAST=<secs>[@x,y,z,r] (test aid, pictures): once, at that
/// match time, a blast of 500 on each Black Ops II prop, as if a grenade
/// went off on it; with a spot, one blast of radius r at x,y,z instead
/// (the glass in it too). With a fifth number, the panes in reach only take
/// that much, as a rifle bullet does (to crack one).
/// The count in reach is logged.
pub(super) fn prop_blast_test(world: &mut World, now: i64) {
    frag_test(world, now);
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};
    static AT: OnceLock<Option<(i64, Option<([f32; 3], f32)>, Option<f32>)>> = OnceLock::new();
    static DONE: AtomicBool = AtomicBool::new(false);
    let Some((at, near, bullet)) = *AT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_PROPBLAST").ok()?;
        let (s, spot) = v.split_once('@').unwrap_or((&v, ""));
        let at = (s.trim().parse::<f32>().ok()? * 1000.0) as i64;
        let n: Vec<f32> = spot.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        Some((at, (n.len() >= 4).then(|| ([n[0], n[1], n[2]], n[3])), n.get(4).copied()))
    }) else {
        return;
    };
    if now < at || DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    if let (Some((c, r)), Some(dmg)) = (near, bullet) {
        super::glass::script_blast(world, c, r, dmg, dmg, "MOD_RIFLE_BULLET");
        diag::info!(Sim, "codmp glass bullet test: {dmg} to the panes within {r} of {c:?}");
        return;
    }
    let spots: Vec<[f32; 3]> = world
        .resource::<Zm>()
        .ents
        .values()
        .filter(|e| !e.hidden && e.destructible.as_deref().is_some_and(is_dyn))
        .map(|e| e.origin)
        .filter(|o| near.is_none_or(|(c, r)| gsc_t6::math::length(gsc_t6::math::sub(*o, c)) < r))
        .collect();
    match near {
        Some((c, r)) => {
            script_blast(world, c, r, |_| 500.0, "MOD_EXPLOSIVE", &Value::Undefined, &Value::Undefined, "");
            // And the panes, as the scripts' radiusdamage reaches them.
            super::glass::script_blast(world, c, r, 500.0, 500.0, "MOD_EXPLOSIVE");
        }
        None => {
            for o in &spots {
                script_blast(
                    world,
                    *o,
                    24.0,
                    |_| 500.0,
                    "MOD_EXPLOSIVE",
                    &Value::Undefined,
                    &Value::Undefined,
                    "",
                );
            }
        }
    }
    diag::info!(Sim, "codmp prop blast: {} props", spots.len());
}

/// The breakables a blast at `origin` reaches: (shown model, its nearest
/// part, distance).
pub(super) fn radius_targets(
    world: &mut World,
    origin: [f32; 3],
    radius: f32,
) -> Vec<(ScriptModelId, [f32; 3], f32)> {
    let shown: Vec<(ScriptModelId, [f32; 3])> = {
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(_, e)| e.destructible.is_some())
            .filter_map(|(n, e)| Some((shown(zm, *n)?, e.origin)))
            .collect()
    };
    if shown.is_empty() {
        return Vec::new();
    }
    let f = frame(world);
    shown
        .into_iter()
        .filter_map(|(id, at)| {
            let parts: Vec<[f32; 3]> = f
                .entity_collision_capabilities()
                .iter()
                .find(|row| row.owner.script_model() == Some(id))
                .and_then(|row| row.dobj.as_ref())
                .and_then(|d| d.current_collision.as_ref())
                .map(|c| c.bones.iter().map(|b| b.center).collect())
                .unwrap_or_default();
            let (mid, d) = parts
                .into_iter()
                .chain(std::iter::once(at))
                .map(|p| (p, gsc_t6::math::length(gsc_t6::math::sub(p, origin))))
                .min_by(|a, b| a.1.total_cmp(&b.1))?;
            (d < radius).then_some((id, mid, d))
        })
        .collect()
}

/// A script's `dodamage` on entity `n` from `point` (with BO2's damage
/// `flags`): a breakable takes it the way BO2's ScrCmd_DoDamage gives it (a
/// burning car's fire running out). False for anything else.
#[allow(clippy::too_many_arguments)]
pub(super) fn script_damage(
    world: &mut World,
    n: u32,
    point: [f32; 3],
    amount: i32,
    means: &str,
    flags: i32,
    attacker: Option<ClientId>,
    weapon: &str,
) -> bool {
    let id = {
        let zm = world.resource::<Zm>();
        if !zm.ents.get(&n).is_some_and(|e| e.destructible.is_some()) {
            return false;
        }
        let Some(id) = shown(zm, n) else {
            return false;
        };
        id
    };
    let t = tick(world);
    let mut f = frame(world);
    let weapon = crate::script_player::weapon_named(&f, weapon).unwrap_or(0);
    crate::t5_destructible::apply_script_damage(
        &mut f,
        t,
        AuthorityModelOwner::ScriptModel(id),
        point,
        amount as f32,
        crate::world_objects::t6_means_from_name(means),
        kind(means),
        flags,
        attacker,
        weapon,
    )
}

/// One notice for the map's scripts: the entity, the notice, its weapon's
/// name.
type Named = (gsc_t6::ObjRef, Notice, String);

/// Each breakable's notices since they were last taken. A loose prop (ours)
/// ends with its base piece (its model swaps here); BO2's own hear their
/// G_Damage instead.
fn collect(world: &mut World) -> Vec<Named> {
    let rows: Vec<(u32, gsc_t6::ObjRef, ScriptModelId)> = {
        let zm = world.resource::<Zm>();
        if zm.destructibles.is_empty() {
            return Vec::new();
        }
        zm.ents
            .iter()
            .filter(|(_, e)| e.destructible.is_some())
            .filter_map(|(n, e)| Some((*n, e.obj?, shown(zm, *n)?)))
            .collect()
    };
    let mut heard = Vec::new();
    {
        let mut f = frame(world);
        for (n, obj, id) in rows {
            let Some(dobj) = f.collision_owner_mut(id).and_then(|r| r.dobj.as_mut()) else {
                continue;
            };
            let notices = crate::t5_destructible::take_notices(dobj);
            if !notices.is_empty() {
                heard.push((n, obj, notices));
            }
        }
    }
    if heard.is_empty() {
        return Vec::new();
    }
    {
        let mut zm = world.resource_mut::<Zm>();
        for (n, _, notices) in &mut heard {
            let gone = notices.iter().any(|x| matches!(x, Notice::BaseDestroyed { .. }));
            let Some(e) = zm.ents.get_mut(n) else {
                continue;
            };
            let dyn_prop = e.destructible.as_deref().is_some_and(is_dyn);
            notices.retain(|x| match x {
                Notice::BaseDestroyed { .. } => dyn_prop,
                Notice::Damage { .. } => !dyn_prop,
                _ => true,
            });
            for x in notices.iter() {
                let Notice::Broken { notify, .. } = x else {
                    continue;
                };
                if let Some(alias) = fire_loop(notify) {
                    e.loop_sound = Some(alias.to_owned());
                } else if notify.contains("explo") {
                    e.loop_sound = None;
                }
            }
            if gone {
                // Blown up: the fire loop stops with the blast.
                if e.loop_sound.as_deref().is_some_and(|a| a == "dst_car_fire" || a == "exp_barrel_fire") {
                    e.loop_sound = None;
                }
                // codmp: a Black Ops II prop (t6_m2 `dyn_ent_props`): a
                // destruct one turns into its broken model, done breaking;
                // a clutter one flew off as debris, so its place is empty.
                if dyn_prop {
                    match e.dyn_destroyed.take() {
                        Some(m) => {
                            e.model = m;
                            e.destructible = None;
                        }
                        None => e.hidden = true,
                    }
                }
            }
        }
    }
    let mut named = Vec::new();
    for (n, obj, notices) in heard {
        for notice in notices {
            // The base piece's stage loop is the entity's own (BO2 fades it in
            // over a second; ours starts at once).
            if let Notice::Loop(alias) = notice {
                if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
                    e.loop_sound = alias;
                }
                continue;
            }
            let weapon = match &notice {
                Notice::Broken { weapon, .. } | Notice::Damage { weapon, .. } => {
                    weapon_text(world, *weapon)
                }
                _ => String::new(),
            };
            named.push((obj, notice, weapon));
        }
    }
    named
}

/// The notices to the map's scripts, in order.
fn dispatch(vm: &mut Vm<World>, world: &mut World, named: Vec<Named>) {
    let cb = super::mp::destructible_callbacks(world);
    for (obj, notice, weapon) in named {
        match notice {
            Notice::Broken {
                notify, attacker, ..
            } => {
                diag::info!(Sim, "bo2mp destructible broken: {notify} ({weapon})");
                let args = vec![
                    vm.string("broken"),
                    vm.string(&notify),
                    player(world, attacker),
                    vm.string(&weapon),
                ];
                vm.spawn_named(
                    world,
                    cb,
                    "codecallback_destructibleevent",
                    Value::Object(obj),
                    args,
                );
            }
            Notice::BreakAfter {
                piece,
                time,
                damage,
            } => {
                let args = vec![
                    vm.string("breakafter"),
                    Value::Int(i32::try_from(piece).unwrap_or(0)),
                    Value::Float(time),
                    Value::Int(damage),
                ];
                vm.spawn_named(
                    world,
                    cb,
                    "codecallback_destructibleevent",
                    Value::Object(obj),
                    args,
                );
            }
            Notice::BaseDestroyed { attacker } => {
                let who = player(world, attacker);
                vm.notify_str(world, obj, "death", &[who]);
            }
            Notice::Damage {
                amount,
                attacker,
                means,
                point,
                dir,
                flags,
                ..
            } => {
                // BO2's G_Damage: the health loses it, the entity hears
                // "damage", then (the health read again: a script may have
                // topped it up) "death" once it is below 1.
                let f = vm.intern("health");
                let health = int_of(&vm.raw_field(obj, f)).saturating_sub(amount);
                vm.set_raw_field(obj, f, Value::Int(health));
                let means = crate::world_objects::t6_means_name(means);
                if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
                    diag::info!(
                        Sim,
                        "bo2mp destructible damage {amount} {means} flags {flags}: health {health}"
                    );
                }
                let who = player(world, attacker);
                let args = vec![
                    Value::Int(amount),
                    who.clone(),
                    dir.map_or(Value::Undefined, Value::Vec3),
                    Value::Vec3(point),
                    vm.string(means),
                    vm.string(""),
                    vm.string(""),
                    vm.string(""),
                    vm.string(&weapon),
                    Value::Int(flags),
                ];
                vm.notify_str(world, obj, "damage", &args);
                let now = int_of(&vm.raw_field(obj, f));
                if now < 1 {
                    vm.set_raw_field(obj, f, Value::Int(now.max(-999)));
                    vm.notify_str(world, obj, "death", &[who]);
                }
            }
            // Taken in `collect` (the entity's loop row).
            Notice::Loop(_) => {}
        }
    }
}

fn int_of(v: &Value) -> i32 {
    match v {
        Value::Int(i) => *i,
        Value::Float(x) => *x as i32,
        _ => 0,
    }
}

/// Each breakable's breaks and hits since last tick, to the map's scripts.
pub(super) fn flush(world: &mut World) {
    let named = collect(world);
    if !named.is_empty() {
        with_vm(world, |vm, world| dispatch(vm, world, named));
    }
}

/// [`flush`] from inside a script's call: BO2 notifies at once (a
/// `radiusdamage` or `dodamage` is heard before the caller's next line).
pub(super) fn flush_now(vm: &mut Vm<World>, world: &mut World) {
    let named = collect(world);
    if !named.is_empty() {
        dispatch(vm, world, named);
    }
}

fn player(world: &World, who: Option<ClientId>) -> Value {
    who.and_then(|a| {
        world
            .resource::<Zm>()
            .players
            .get(&a.0)
            .map(|p| Value::Object(p.obj))
    })
    .unwrap_or(Value::Undefined)
}

#[cfg(test)]
mod tests {
    use super::kind;
    use crate::t5_destructible::DamageKind;

    #[test]
    fn each_means_counts_as_bo2_counts_it() {
        for (means, want) in [
            ("MOD_PISTOL_BULLET", DamageKind::Bullet),
            ("MOD_RIFLE_BULLET", DamageKind::Bullet),
            ("MOD_HEAD_SHOT", DamageKind::Bullet),
            ("MOD_GRENADE", DamageKind::Explosive),
            ("MOD_PROJECTILE", DamageKind::Explosive),
            ("MOD_GRENADE_SPLASH", DamageKind::Splash),
            ("MOD_PROJECTILE_SPLASH", DamageKind::Splash),
            ("MOD_EXPLOSIVE", DamageKind::Blast),
            ("MOD_MELEE", DamageKind::Melee),
            ("MOD_IMPACT", DamageKind::Impact),
            ("MOD_BURNED", DamageKind::Burned),
            ("MOD_UNKNOWN", DamageKind::Script),
            ("MOD_CRUSH", DamageKind::Script),
        ] {
            assert_eq!(kind(means), want, "{means}");
        }
    }
}
