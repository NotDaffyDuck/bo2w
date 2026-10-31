//! Weapons' damage to the scorestreaks' vehicles (the helicopters, the
//! drones, the RC-XD, the AGR), as Black Ops II's engine hands it to the
//! scripts.
//!
//! A hit on a vehicle (BO2's `G_DamageVehicle`) goes to the gametype's
//! `codecallback_vehicledamage`, a blast's (`G_RadiusDamage`) to
//! `codecallback_vehicleradiusdamage`. BO2's `_globallogic_vehicle` asks
//! whether the vehicle type takes that kind of damage at all
//! (`isvehicleimmunetodamage`: the type's bullet, grenade, projectile...
//! flags), scales it by its own tables and calls back
//! `finishvehicledamage` / `finishvehicleradiusdamage`. Those take the
//! vehicle's health and tell it `damage`, and `death` once its health is
//! gone. A blast's share needs the blast to see the vehicle (BO2's
//! `CanDamage`: its middle or a corner of its box as the blast faces it).

use bevy_ecs::prelude::World;
use gsc_t6::math::{length, normalize, sub};
use gsc_t6::{ObjRef, ScriptValue, Vm};

use super::super::{T6VehicleDrive, Zm, arg, entnum, frame, text};
use super::{Streaks, ride, streaks};

/// BO2's means of death, by number (`meansOfDeath_t`).
const MEANS: [&str; 21] = [
    "MOD_UNKNOWN",
    "MOD_PISTOL_BULLET",
    "MOD_RIFLE_BULLET",
    "MOD_GRENADE",
    "MOD_GRENADE_SPLASH",
    "MOD_PROJECTILE",
    "MOD_PROJECTILE_SPLASH",
    "MOD_MELEE",
    "MOD_BAYONET",
    "MOD_HEAD_SHOT",
    "MOD_CRUSH",
    "MOD_TELEFRAG",
    "MOD_FALLING",
    "MOD_SUICIDE",
    "MOD_TRIGGER_HURT",
    "MOD_EXPLOSIVE",
    "MOD_IMPACT",
    "MOD_BURNED",
    "MOD_HIT_BY_OBJECT",
    "MOD_DROWN",
    "MOD_GAS",
];

/// The damage flag for a weapon that pierces armour.
const IDFLAGS_NO_ARMOR: i32 = 2;
/// A blast's damage flags as `G_RadiusDamage` passes them to a vehicle.
const IDFLAGS_BLAST: i32 = 5;
/// A weapon's projectile explosion type: heavy explosive.
const EXPLOSION_HEAVY: i32 = 6;
/// What a blast's sight of a vehicle stops at (BO2's `CanDamage` mask).
const CAN_DAMAGE_MASK: u32 = super::can_damage::DAMAGE_MASK;

/// A means of death's number; an unknown name is `MOD_UNKNOWN`.
pub(super) fn means_index(means: &str) -> i32 {
    MEANS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(means))
        .map_or(0, |i| i as i32)
}

/// BO2's `VEH_IsImmune`: the vehicle type `d` takes no damage of this kind
/// (`heavy`: the weapon's projectile is a heavy explosive).
pub(super) fn immune(d: &T6VehicleDrive, dflags: i32, damage_type: i32, heavy: bool) -> bool {
    let pierces = dflags & IDFLAGS_NO_ARMOR != 0 && d.armor_piercing_damage != 0;
    match damage_type {
        // Unknown, explosive and gas always hurt.
        0 | 15 | 20 => false,
        // Bullets.
        1 | 2 => d.bullet_damage == 0 && !pierces,
        // Grenades.
        3 | 4 => {
            if heavy {
                d.heavy_explosive_damage == 0
            } else {
                !pierces && d.grenade_damage == 0
            }
        }
        // Projectiles and their impact.
        5 | 16 => !pierces && d.projectile_damage == 0,
        6 => d.projectile_splash_damage == 0,
        // Crushed: only a vehicle of camera mode 4.
        10 => d.camera_mode != 4,
        // Fire burns only a remote-controlled vehicle.
        17 => d.remote_operated == 0,
        _ => true,
    }
}

/// A scorestreak vehicle (`spawnhelicopter`, `spawnvehicle`), not a plane.
pub(super) fn is_vehicle(world: &World, n: u32) -> bool {
    world
        .get_resource::<Streaks>()
        .is_some_and(|s| s.craft.contains_key(&n))
}

/// The vehicle type's own data (by its `vehicletype`).
fn drive_of(vm: &mut Vm<World>, game: &mut World, obj: ObjRef) -> Option<T6VehicleDrive> {
    let f = vm.intern("vehicletype");
    let kind = vm.to_text(&vm.raw_field(obj, f)).to_ascii_lowercase();
    streaks(game).vehicle_drive.get(&kind).copied()
}

/// The weapon's projectile is a heavy explosive.
fn heavy(world: &mut World, weapon: &str) -> bool {
    let Ok(w) = super::super::weapon(world, weapon) else {
        return false;
    };
    w != 0
        && frame(world)
            .gadget_stats_lookup(w)
            .is_some_and(|f| f.shell_blast_kind == EXPLOSION_HEAVY)
}

/// A vehicle's health and most health. BO2's scorestreak vehicles have
/// health their scripts never set (the helicopter sets only its
/// `maxhealth`, the RC-XD neither, yet the vehicle callback divides by
/// `maxhealth` and a hatchet destroys the RC-XD); no source for it is
/// found. bo2mp: an unset `health` is the vehicle's `maxhealth`, an unset
/// `maxhealth` its `health`, and with neither set both are 1.
pub(super) fn vehicle_health(vm: &mut Vm<World>, obj: ObjRef) -> (i32, i32) {
    let (hf, mf) = (vm.intern("health"), vm.intern("maxhealth"));
    match (vm.raw_field(obj, hf).as_int(), vm.raw_field(obj, mf).as_int()) {
        (Some(h), Some(m)) => (h, m),
        (Some(h), None) => (h, h),
        (None, Some(m)) => (m, m),
        (None, None) => (1, 1),
    }
}

fn health(vm: &mut Vm<World>, obj: ObjRef) -> i32 {
    vehicle_health(vm, obj).0
}

/// Its health after damage; its most health stays what it read before.
fn set_health(vm: &mut Vm<World>, obj: ObjRef, h: i32) {
    let (_, most) = vehicle_health(vm, obj);
    let (hf, mf) = (vm.intern("health"), vm.intern("maxhealth"));
    if vm.raw_field(obj, mf).as_int().is_none() {
        vm.set_raw_field(obj, mf, ScriptValue::Int(most));
    }
    vm.set_raw_field(obj, hf, ScriptValue::Int(h));
}

/// The vehicle's origin, angles and model bounds (middle, half size, about
/// the origin). A vehicle without its model is a 32-unit cube on its origin.
fn bounds(world: &mut World, n: u32) -> Option<([f32; 3], [f32; 3], [f32; 3], [f32; 3])> {
    let (spot, rot, mdl) = {
        let e = world.resource::<Zm>().ents.get(&n)?;
        (e.origin, e.rot, e.mesh.clone())
    };
    let (centre, half_ext) = frame(world)
        .model_capability(&mdl)
        .flatten()
        .and_then(|c| c.bounds)
        .unwrap_or(([0.0, 0.0, 16.0], [16.0; 3]));
    Some((spot, rot, centre, half_ext))
}

/// BO2's blast from underneath (a grenade's splash): below the vehicle's
/// middle and close in under it, unless the type takes no harder blast from
/// below.
fn from_underneath(game: &mut World, nn: u32, dd: Option<&T6VehicleDrive>, place: [f32; 3]) -> bool {
    if dd.is_some_and(|dd| dd.no_directional_damage != 0) {
        return false;
    }
    let Some((spot, _, centre, half_ext)) = bounds(game, nn) else {
        return false;
    };
    let maxs: [f32; 3] = std::array::from_fn(|i| centre[i] + half_ext[i]);
    place[2] - 10.0 < spot[2] + maxs[2] * 0.5 && length(sub(spot, place)) < maxs[1] * 0.8
}

/// BO2's `CanDamage` for a vehicle: does the blast at `point` see the middle
/// of its box, or one of the four points out from it across the box as the
/// blast faces it? With a cone (`cone_cos` above -1 and a direction), only
/// points inside it count.
fn sees(game: &mut World, nn: u32, place: [f32; 3], cone_cos: f32, cone_dir: Option<[f32; 3]>) -> bool {
    let Some((spot, rot, centre, half_ext)) = bounds(game, nn) else {
        return false;
    };
    // The box in the game: turned with the vehicle's yaw.
    let (s, c) = rot[1].to_radians().sin_cos();
    let centre = [
        spot[0] + c * centre[0] - s * centre[1],
        spot[1] + s * centre[0] + c * centre[1],
        spot[2] + centre[2],
    ];
    let reach = [
        c.abs() * half_ext[0] + s.abs() * half_ext[1],
        s.abs() * half_ext[0] + c.abs() * half_ext[1],
        half_ext[2],
    ];
    let cone = cone_dir
        .filter(|_| cone_cos != -1.0)
        .map(|d| (normalize(d), cone_cos));
    super::can_damage::sees_box(game, centre, reach, place, cone, CAN_DAMAGE_MASK)
}

/// A weapon's hit on vehicle `n` (BO2's `G_DamageVehicle`): the scripts'
/// vehicle callback has it, when the vehicle takes damage and still has
/// health.
#[allow(clippy::too_many_arguments)]
pub(super) fn hit(
    vm: &mut Vm<World>, game: &mut World,
    n: u32,
    inflictor: ScriptValue,
    attacker: ScriptValue,
    qty: i32,
    dflags: i32,
    means: &str,
    gun: &str,
    place: [f32; 3],
    dir: [f32; 3],
) {
    let Some((obj, can)) = game
        .resource::<Zm>()
        .ents
        .get(&n)
        .and_then(|e| Some((e.obj?, e.can_damage)))
    else {
        return;
    };
    if !can || qty <= 0 || !vm.alive(obj) || health(vm, obj) <= 0 {
        return;
    }
    let drive = drive_of(vm, game, obj);
    let under = means_index(means) == 4 && from_underneath(game, n, drive.as_ref(), place);
    let (m, w, hitloc, part) = (vm.string(means), vm.string(gun), vm.string("none"), vm.string(""));
    let args = vec![
        inflictor,
        attacker,
        ScriptValue::Int(qty),
        ScriptValue::Int(dflags),
        m,
        w,
        ScriptValue::Vec3(place),
        ScriptValue::Vec3(normalize(dir)),
        hitloc,
        ScriptValue::Int(0),
        ScriptValue::bool(under),
        ScriptValue::Int(0),
        part,
    ];
    let cb = super::super::mp::callbacks(game);
    if vm
        .spawn_named(game, &cb, "codecallback_vehicledamage", ScriptValue::Object(obj), args.clone())
        .is_none()
    {
        // No gametype callback: the damage lands as it came.
        finish(vm, game, n, obj, &args);
    }
}

/// A blast on vehicle `n` (BO2's `G_RadiusDamage`): its share by distance
/// to the vehicle's origin goes to the scripts' vehicle radius callback.
#[allow(clippy::too_many_arguments)]
pub(super) fn blast(
    vm: &mut Vm<World>, game: &mut World,
    n: u32,
    inflictor: ScriptValue,
    attacker: ScriptValue,
    spot: [f32; 3],
    reach: f32,
    inner: f32,
    outer: f32,
    cone: Option<([f32; 3], f32)>,
    means: &str,
    gun: &str,
) {
    let Some((obj, at)) = game
        .resource::<Zm>()
        .ents
        .get(&n)
        .and_then(|e| Some((e.obj?, e.origin)))
    else {
        return;
    };
    let d = length(sub(at, spot));
    if reach <= 0.0 || d >= reach || !vm.alive(obj) {
        return;
    }
    let share = ((1.0 - d / reach) * (inner - outer) + outer) as i32;
    let (cone_cos, cone_dir) = cone.map_or((-1.0, ScriptValue::Undefined), |(dir, cos)| (cos, ScriptValue::Vec3(dir)));
    let (m, w) = (vm.string(means), vm.string(gun));
    let args = vec![
        inflictor,
        attacker,
        ScriptValue::Int(share),
        ScriptValue::Float(inner),
        ScriptValue::Float(outer),
        ScriptValue::Int(IDFLAGS_BLAST),
        m,
        w,
        ScriptValue::Vec3(spot),
        ScriptValue::Float(reach),
        ScriptValue::Float(cone_cos),
        cone_dir,
        ScriptValue::Int(0),
    ];
    let cb = super::super::mp::callbacks(game);
    if vm
        .spawn_named(game, &cb, "codecallback_vehicleradiusdamage", ScriptValue::Object(obj), args.clone())
        .is_none()
    {
        finish_radius(vm, game, n, &args);
    }
}

/// `finishvehicledamage` (inflictor, attacker, damage, flags, means, weapon,
/// point, direction, hit location, time offset, from underneath, model
/// index, part, eject): the health goes down and the vehicle hears
/// `damage`; with its health gone (eject: its riders but the attacker get
/// off first) it hears `death`.
fn finish(vm: &mut Vm<World>, game: &mut World, nn: u32, obj: ObjRef, lhs: &[ScriptValue]) {
    let damage = arg(lhs, 2).as_int().unwrap_or(0);
    let left = health(vm, obj);
    if damage <= 0 || left <= 0 {
        return;
    }
    let left = left - damage;
    set_health(vm, obj, left);
    let attacker = arg(lhs, 1).clone();
    if left < 1 && arg(lhs, 13).as_int().unwrap_or(0) != 0 {
        let by = attacker.as_obj().and_then(|o| {
            game
                .resource::<Zm>()
                .players
                .iter()
                .find(|(_, p)| p.obj == o)
                .map(|(c, _)| *c)
        });
        let riders: Vec<u32> = streaks(game)
            .rides
            .iter()
            .filter(|(c, r)| r.vehicle == nn && Some(**c) != by)
            .map(|(c, _)| *c)
            .collect();
        for c in riders {
            ride::end_ride_in(vm, game, c);
        }
    }
    let model_name = game
        .resource::<Zm>()
        .ents
        .get(&nn)
        .map(|e| e.mesh.clone())
        .unwrap_or_default();
    let (model, tag) = (vm.string(&model_name), vm.string(""));
    let damage_args = [
        ScriptValue::Int(damage),
        ScriptValue::clone(&attacker),
        arg(lhs, 7).clone(),
        arg(lhs, 6).clone(),
        arg(lhs, 4).clone(),
        model,
        tag,
        arg(lhs, 12).clone(),
        arg(lhs, 5).clone(),
        arg(lhs, 3).clone(),
    ];
    if vm.alive(obj) {
        vm.notify_str(game, obj, "damage", &damage_args);
    }
    if left < 1 && vm.alive(obj) {
        set_health(vm, obj, left.max(-999));
        let death_args = [
            attacker,
            arg(lhs, 10).clone(),
            arg(lhs, 5).clone(),
            arg(lhs, 6).clone(),
            arg(lhs, 7).clone(),
            arg(lhs, 4).clone(),
        ];
        vm.notify_str(game, obj, "death", &death_args);
    }
}

/// `finishvehicleradiusdamage` (inflictor, attacker, damage, inner, outer,
/// flags, means, weapon, blast origin, radius, cone cosine, cone
/// direction, time offset): when the blast sees the vehicle, its damage
/// lands as a hit from the blast's origin.
fn finish_radius(vm: &mut Vm<World>, game: &mut World, nn: u32, lhs: &[ScriptValue]) {
    let damage = arg(lhs, 2).as_int().unwrap_or(0);
    let Some(point) = arg(lhs, 8).as_vec3() else {
        return;
    };
    if damage <= 0 {
        return;
    }
    let cone_cos = arg(lhs, 10).as_float().unwrap_or(-1.0);
    let cone_dir = arg(lhs, 11).as_vec3();
    if !sees(game, nn, point, cone_cos, cone_dir) {
        return;
    }
    let Some(at) = game.resource::<Zm>().ents.get(&nn).map(|e| e.origin) else {
        return;
    };
    let entity = |v: &ScriptValue| match v {
        ScriptValue::Object(_) => v.clone(),
        _ => ScriptValue::Undefined,
    };
    let (means, weapon) = (vm.to_text(arg(lhs, 6)), vm.to_text(arg(lhs, 7)));
    hit(
        vm,
        game,
        nn,
        entity(arg(lhs, 0)),
        entity(arg(lhs, 1)),
        damage,
        arg(lhs, 5).as_int().unwrap_or(0),
        &means,
        &weapon,
        point,
        sub(at, point),
    );
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    m!("finishvehicledamage", |vm, world, s, a| {
        if let Some(n) = entnum(vm, s)
            && let Some(obj) = s.as_obj()
        {
            finish(vm, world, n, obj, a);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("finishvehicleradiusdamage", |vm, world, s, a| {
        if let Some(n) = entnum(vm, s) {
            finish_radius(vm, world, n, a);
        }
        Ok(ScriptValue::Undefined)
    });
    // `isvehicleimmunetodamage(flags, means, weapon)`.
    m!("isvehicleimmunetodamage", |vm, world, s, a| {
        let Some(obj) = s.as_obj() else {
            return Ok(ScriptValue::bool(false));
        };
        let Some(d) = drive_of(vm, world, obj) else {
            return Ok(ScriptValue::bool(false));
        };
        let dflags = arg(a, 0).as_int().unwrap_or(0);
        let means = means_index(&text(vm, a, 1));
        let weapon = text(vm, a, 2);
        let heavy = matches!(means, 3 | 4) && heavy(world, &weapon);
        Ok(ScriptValue::bool(immune(&d, dflags, means, heavy)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive() -> T6VehicleDrive {
        T6VehicleDrive::default()
    }

    #[test]
    fn means_by_number() {
        assert_eq!(means_index("MOD_UNKNOWN"), 0);
        assert_eq!(means_index("MOD_RIFLE_BULLET"), 2);
        assert_eq!(means_index("mod_grenade_splash"), 4);
        assert_eq!(means_index("MOD_BAYONET"), 8);
        assert_eq!(means_index("MOD_IMPACT"), 16);
        assert_eq!(means_index("MOD_GAS"), 20);
        assert_eq!(means_index("MOD_NOT_A_THING"), 0);
    }

    #[test]
    fn explosive_unknown_and_gas_always_hurt() {
        let d = drive();
        for m in [0, 15, 20] {
            assert!(!immune(&d, 0, m, false));
        }
        // Melee, falling and the rest never do.
        for m in [7, 8, 9, 11, 12, 13, 14, 18, 19] {
            assert!(immune(&d, 0, m, false));
        }
    }

    #[test]
    fn bullets_need_the_flag_or_armour_piercing() {
        let mut d = drive();
        assert!(immune(&d, 0, 2, false));
        d.armor_piercing_damage = 1;
        assert!(immune(&d, 0, 2, false));
        assert!(!immune(&d, IDFLAGS_NO_ARMOR, 1, false));
        d = drive();
        d.bullet_damage = 1;
        assert!(!immune(&d, 0, 1, false));
    }

    #[test]
    fn heavy_grenades_use_the_heavy_flag() {
        let mut d = drive();
        d.grenade_damage = 1;
        assert!(!immune(&d, 0, 3, false));
        assert!(immune(&d, 0, 3, true));
        d.heavy_explosive_damage = 1;
        assert!(!immune(&d, 0, 4, true));
        d = drive();
        d.armor_piercing_damage = 1;
        assert!(!immune(&d, IDFLAGS_NO_ARMOR, 3, false));
    }

    #[test]
    fn projectiles_crush_and_fire() {
        let mut d = drive();
        assert!(immune(&d, 0, 5, false));
        assert!(immune(&d, 0, 16, false));
        assert!(immune(&d, 0, 6, false));
        d.projectile_damage = 1;
        d.projectile_splash_damage = 1;
        assert!(!immune(&d, 0, 5, false));
        assert!(!immune(&d, 0, 16, false));
        assert!(!immune(&d, 0, 6, false));
        assert!(immune(&d, 0, 10, false));
        d.camera_mode = 4;
        assert!(!immune(&d, 0, 10, false));
        assert!(immune(&d, 0, 17, false));
        d.remote_operated = 1;
        assert!(!immune(&d, 0, 17, false));
    }
}
