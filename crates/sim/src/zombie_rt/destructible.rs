//! bo2mp: a Black Ops II map's breakables (Nuketown's mannequins, its cars,
//! its clock; the clipmap's loose and breakable props, Raid's pots and
//! bottles). A hit breaks pieces in t5_destructible; the breaks reach the
//! map's scripts here, as BO2's `codecallback_destructibleevent` ("broken",
//! "breakafter"), and the entity's G_Damage ("damage", then "death" once
//! its health is gone).

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, frame, tick, weapon_text, with_vm};
use crate::staged_props::{DamageKind, Notice};
use crate::sim_parts::ClientId;
use crate::{ServerModelSource, ScriptModelId};

/// Which of a piece's scales a hit counts by, from its means of death.
pub(super) fn kind(mod_name: &str) -> DamageKind {
    match mod_name {
        "MOD_MELEE" => DamageKind::Melee,
        "MOD_IMPACT" => DamageKind::Impact,
        "MOD_BURNED" => DamageKind::Burned,
        "MOD_EXPLOSIVE" => DamageKind::Blast,
        _ if mod_name.contains("SPLASH") => DamageKind::Splash,
        _ if ["GRENADE", "PROJECTILE", "EXPLOSIVE"]
            .iter()
            .any(|k| mod_name.contains(k)) =>
        {
            DamageKind::Explosive
        }
        _ if mod_name.contains("BULLET") || mod_name == "MOD_HEAD_SHOT" => DamageKind::Bullet,
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
    (breakable || ent.is_some_and(|e| e.can_damage) || zm.actors.get(&n).is_some_and(|a| a.living))
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
fn fire_loop(event: &str) -> Option<&'static str> {
    match event {
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
    game: &mut World, spot: [f32; 3],
    range: f32,
    amount_at: impl Fn(f32) -> f32,
    means: &str,
    inflictor: &ScriptValue,
    attacker: &ScriptValue,
    gun: &str,
) {
    if game.resource::<Zm>().destructibles.is_empty() {
        return;
    }
    let attacker = attacker.as_obj().and_then(|o| {
        game
            .resource::<Zm>()
            .players
            .iter()
            .find(|(_, p)| p.obj == o)
            .map(|(n, _)| ClientId(*n))
    });
    let skip: Vec<ServerModelSource> = inflictor
        .as_obj()
        .and_then(|o| {
            let zm = game.resource::<Zm>();
            zm.ents
                .iter()
                .find(|(_, e)| e.obj == Some(o))
                .and_then(|(n, _)| shown(zm, *n))
        })
        .map(ServerModelSource::ScriptModel)
        .into_iter()
        .collect();
    let means = crate::glass_and_props::t6_means_from_name(means);
    let t = tick(game);
    let mut f = frame(game);
    let gun = crate::client_script::weapon_named(&f, gun).unwrap_or(0);
    for (_, notifies) in crate::staged_props::apply_blast_by(
        &mut f, t, spot, range, amount_at, means, &skip, attacker, gun,
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
fn frag_test(world: &mut World, now_ms: i64) {
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
    if now_ms < at || DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(attacker) = world.resource::<Zm>().players.keys().next().map(|n| ClientId(*n)) else {
        return;
    };
    let t = tick(world);
    let mut f = frame(world);
    let Ok(weapon) = crate::client_script::weapon_named(&f, "frag_grenade_mp") else {
        return;
    };
    let Some(facts) = f.gadget_stats_lookup(weapon) else {
        return;
    };
    let blast = crate::harm::ExplosionBlast {
        cone: None,
        origin: spot,
        reach: facts.blast_outer_reach.max(facts.blast_reach_floor).max(0) as f32,
        inner_damage: facts.blast_core_damage as f32,
        outer_damage: facts.blast_rim_damage.max(0) as f32,
        gun: weapon,
        source: crate::match_ids::HarmOrigin::Projectile(crate::match_ids::ProjectileId(0)),
        attacker,
        killer_life: Default::default(),
        cam_ent_since: 0,
    };
    diag::info!(Sim, "codmp frag test: client {} frag at {spot:?}, radius {}", attacker.0, blast.reach);
    crate::harm::apply_explosion_blast(&mut f, t, &blast);
}

/// BO2MP_T6_PROPBLAST=<secs>[@x,y,z,r] (test aid, pictures): once, at that
/// match time, a blast of 500 on each Black Ops II prop, as if a grenade
/// went off on it; with a spot, one blast of radius r at x,y,z instead
/// (the glass in it too). With a fifth number, the panes in reach only take
/// that much, as a rifle bullet does (to crack one).
/// The count in reach is logged.
pub(super) fn prop_blast_test(world: &mut World, now_ms: i64) {
    frag_test(world, now_ms);
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
    if now_ms < at || DONE.swap(true, Ordering::Relaxed) {
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
        .filter(|e| !e.unseen && e.destructible.as_deref().is_some_and(is_dyn))
        .map(|e| e.origin)
        .filter(|o| near.is_none_or(|(c, r)| gsc_t6::math::length(gsc_t6::math::sub(*o, c)) < r))
        .collect();
    match near {
        Some((c, r)) => {
            script_blast(world, c, r, |_| 500.0, "MOD_EXPLOSIVE", &ScriptValue::Undefined, &ScriptValue::Undefined, "");
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
                    &ScriptValue::Undefined,
                    &ScriptValue::Undefined,
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
    game: &mut World, spot: [f32; 3],
    reach: f32,
) -> Vec<(ScriptModelId, [f32; 3], f32)> {
    let shown: Vec<(ScriptModelId, [f32; 3])> = {
        let zm = game.resource::<Zm>();
        zm.ents
            .iter()
            .where_value(|en| en.destructible.is_some())
            .filter_map(|(nn, en)| Some((shown(zm, *nn)?, en.origin)))
            .collect()
    };
    if shown.is_empty() {
        return Vec::new();
    }
    let f = frame(game);
    shown
        .into_iter()
        .filter_map(|(cid, at)| {
            let parts: Vec<[f32; 3]> = f.entity_hit_support()
                .iter()
                .find(|row| row.owner.scripted_model() == Some(cid))
                .and_then(|row| row.dobj.as_ref())
                .and_then(|d| d.live_hitbox.as_ref())
                .map(|c| c.bones.iter().map(|rhs| rhs.center).collect())
                .unwrap_or_default();
            let (mid, d) = parts
                .into_iter()
                .chain(std::iter::once(at))
                .map(|p| (p, gsc_t6::math::length(gsc_t6::math::sub(p, spot))))
                .min_by(|lhs, rhs| lhs.1.total_cmp(&rhs.1))?;
            (d < reach).then_some((cid, mid, d))
        })
        .collect()
}

/// A script's `dodamage` on entity `n` from `point` (with BO2's damage
/// `flags`): a breakable takes it the way BO2's ScrCmd_DoDamage gives it (a
/// burning car's fire running out). False for anything else.
#[allow(clippy::too_many_arguments)]
pub(super) fn script_damage(
    world: &mut World, n: u32,
    place: [f32; 3],
    qty: i32,
    means: &str,
    flags: i32,
    hitter: Option<ClientId>, gun: &str,
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
    let gun = crate::client_script::weapon_named(&f, gun).unwrap_or(0);
    crate::staged_props::apply_script_damage(
        &mut f,
        t,
        ServerModelSource::ScriptModel(id),
        place,
        qty as f32,
        crate::glass_and_props::t6_means_from_name(means),
        kind(means),
        flags,
        hitter,
        gun,
    )
}

/// Each breakable's notices since they were last taken. A loose prop (ours)
/// ends with its base piece (its model swaps here); BO2's own hear their
/// G_Damage instead.
fn collect(world: &mut World) -> Vec<(gsc_t6::ObjRef, Notice, String)> {
    let rows: Vec<(u32, gsc_t6::ObjRef, ScriptModelId)> = {
        let zm = world.resource::<Zm>();
        if zm.destructibles.is_empty() {
            return Vec::new();
        }
        zm.ents
            .iter()
            .where_value(|en| en.destructible.is_some())
            .filter_map(|(nn, en)| Some((*nn, en.obj?, shown(zm, *nn)?)))
            .collect()
    };
    let mut heard = Vec::new();
    {
        let mut f = frame(world);
        for (nn, obj, id) in rows {
            let Some(dobj) = f.collision_owner_mut(id).and_then(|r| r.dobj.as_mut()) else {
                continue;
            };
            let notices = crate::staged_props::take_notices(dobj);
            if !notices.is_empty() {
                heard.push((nn, obj, notices));
            }
        }
    }
    if heard.is_empty() {
        return Vec::new();
    }
    {
        let mut zm = world.resource_mut::<Zm>();
        for (nn, _, notices) in &mut heard {
            let gone = notices.iter().any(|x| matches!(x, Notice::BaseDestroyed { .. }));
            let Some(en) = zm.ents.get_mut(nn) else {
                continue;
            };
            let dyn_prop = en.destructible.as_deref().is_some_and(is_dyn);
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
                    en.repeating_audio = Some(alias.to_owned());
                } else if notify.contains("explo") {
                    en.repeating_audio = None;
                }
            }
            if gone {
                // Blown up: the fire loop stops with the blast.
                if en.repeating_audio.as_deref().is_some_and(|a| a == "dst_car_fire" || a == "exp_barrel_fire") {
                    en.repeating_audio = None;
                }
                // codmp: a Black Ops II prop (t6_m2 `dyn_ent_props`): a
                // destruct one turns into its broken model, done breaking;
                // a clutter one flew off as debris, so its place is empty.
                if dyn_prop {
                    match en.dyn_destroyed.take() {
                        Some(m) => {
                            en.mesh = m;
                            en.destructible = None;
                        }
                        None => en.unseen = true,
                    }
                }
            }
        }
    }
    let mut named = Vec::new();
    for (nn, obj, notices) in heard {
        for notice in notices {
            // The base piece's stage loop is the entity's own (BO2 fades it in
            // over a second; ours starts at once).
            if let Notice::Loop(alias) = notice {
                if let Some(en) = world.resource_mut::<Zm>().ents.get_mut(&nn) {
                    en.repeating_audio = alias;
                }
                continue;
            }
            let weapon = match &notice {
                Notice::Broken { gun: weapon, .. } | Notice::Damage { gun: weapon, .. } => {
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
fn dispatch(vm: &mut Vm<World>, game: &mut World, named: Vec<(gsc_t6::ObjRef, Notice, String)>) {
    let cb = super::mp::destructible_callbacks(game);
    for (obj, notice, weapon) in named {
        match notice {
            Notice::Broken {
                notify, attacker, ..
            } => {
                diag::info!(Sim, "bo2mp destructible broken: {notify} ({weapon})");
                let args = vec![
                    vm.string("broken"),
                    vm.string(&notify),
                    player(game, attacker),
                    vm.string(&weapon),
                ];
                vm.spawn_named(
                    game,
                    cb,
                    "codecallback_destructibleevent",
                    ScriptValue::Object(obj),
                    args,
                );
            }
            Notice::BreakAfter {
                part: piece,
                time,
                harm: damage,
            } => {
                let args = vec![
                    vm.string("breakafter"),
                    ScriptValue::Int(i32::try_from(piece).unwrap_or(0)),
                    ScriptValue::Float(time),
                    ScriptValue::Int(damage),
                ];
                vm.spawn_named(
                    game,
                    cb,
                    "codecallback_destructibleevent",
                    ScriptValue::Object(obj),
                    args,
                );
            }
            Notice::BaseDestroyed { attacker } => {
                let who = player(game, attacker);
                vm.notify_str(game, obj, "death", &[who]);
            }
            Notice::Damage {
                amount,
                attacker,
                means,
                at: point,
                way: dir,
                flags,
                ..
            } => {
                // BO2's G_Damage: the health loses it, the entity hears
                // "damage", then (the health read again: a script may have
                // topped it up) "death" once it is below 1.
                let f = vm.intern("health");
                let health = int_of(&vm.raw_field(obj, f)).saturating_sub(amount);
                vm.set_raw_field(obj, f, ScriptValue::Int(health));
                let means = crate::glass_and_props::t6_means_name(means);
                if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
                    diag::info!(
                        Sim,
                        "bo2mp destructible damage {amount} {means} flags {flags}: health {health}"
                    );
                }
                let who = player(game, attacker);
                let args = vec![
                    ScriptValue::Int(amount),
                    who.clone(),
                    dir.map_or(ScriptValue::Undefined, ScriptValue::Vec3),
                    ScriptValue::Vec3(point),
                    vm.string(means),
                    vm.string(""),
                    vm.string(""),
                    vm.string(""),
                    vm.string(&weapon),
                    ScriptValue::Int(flags),
                ];
                vm.notify_str(game, obj, "damage", &args);
                let now = int_of(&vm.raw_field(obj, f));
                if now < 1 {
                    vm.set_raw_field(obj, f, ScriptValue::Int(now.max(-999)));
                    vm.notify_str(game, obj, "death", &[who]);
                }
            }
            // Taken in `collect` (the entity's loop row).
            Notice::Loop(_) => {}
        }
    }
}

fn int_of(v: &ScriptValue) -> i32 {
    match v {
        ScriptValue::Int(i) => *i,
        ScriptValue::Float(x) => *x as i32,
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

fn player(world: &World, who: Option<ClientId>) -> ScriptValue {
    who.and_then(|a| {
        world
            .resource::<Zm>()
            .players
            .get(&a.0)
            .map(|p| ScriptValue::Object(p.obj))
    })
    .unwrap_or(ScriptValue::Undefined)
}

#[cfg(test)]
mod tests {
    use super::kind;
    use crate::staged_props::DamageKind;

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
