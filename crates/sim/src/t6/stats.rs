//! A player's stats as BO2's engine keeps them for the scripts
//! (`getdstat`, `setdstat`, `adddstat`): TranZit's bank (`depositBox`) and
//! weapon locker (`weaponLocker`) live there, under `PlayerStatsByMap`
//! (Die Rise and Buried use TranZit's). Kept for the match; a stat never
//! set reads 0.

use std::collections::HashMap;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Value, Vm};

use super::client;

#[derive(Clone, Debug)]
enum Stat {
    Int(i32),
    Float(f32),
    Text(String),
}

#[derive(Resource, Default)]
pub(crate) struct DStats(HashMap<(u32, Vec<String>), Stat>);

/// The stat's path, e.g. `PlayerStatsByMap`, `zm_transit`, `depositBox`
/// (names are not case sensitive).
fn path(vm: &Vm<World>, keys: &[Value]) -> Vec<String> {
    keys.iter()
        .map(|k| vm.to_text(k).to_ascii_lowercase())
        .collect()
}

fn get(world: &World, key: &(u32, Vec<String>)) -> Option<Stat> {
    world
        .get_resource::<DStats>()
        .and_then(|d| d.0.get(key).cloned())
}

fn put(world: &mut World, key: (u32, Vec<String>), v: Stat) {
    world
        .get_resource_or_insert_with(DStats::default)
        .0
        .insert(key, v);
}

fn stat(vm: &Vm<World>, v: &Value) -> Stat {
    match v {
        Value::Int(i) => Stat::Int(*i),
        Value::Float(f) => Stat::Float(*f),
        Value::Undefined => Stat::Int(0),
        v => Stat::Text(vm.to_text(v)),
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // player getdstat(path...)
    vm.bind("getdstat", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        Ok(match get(world, &(id.0, path(vm, a))) {
            Some(Stat::Int(i)) => Value::Int(i),
            Some(Stat::Float(f)) => Value::Float(f),
            Some(Stat::Text(t)) => vm.string(&t),
            None => Value::Int(0),
        })
    });
    // player setdstat(path..., value)
    vm.bind("setdstat", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        if let Some((v, keys)) = a.split_last() {
            let v = stat(vm, v);
            put(world, (id.0, path(vm, keys)), v);
        }
        Ok(Value::Undefined)
    });
    // player adddstat(path..., amount): added to what is there.
    vm.bind("adddstat", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        if let Some((v, keys)) = a.split_last() {
            let key = (id.0, path(vm, keys));
            let sum = match (get(world, &key), stat(vm, v)) {
                (Some(Stat::Float(x)), Stat::Int(d)) => Stat::Float(x + d as f32),
                (Some(Stat::Float(x)), Stat::Float(d)) => Stat::Float(x + d),
                (Some(Stat::Int(x)), Stat::Float(d)) => Stat::Float(x as f32 + d),
                (Some(Stat::Int(x)), Stat::Int(d)) => Stat::Int(x.saturating_add(d)),
                (_, d) => d,
            };
            put(world, key, sum);
        }
        Ok(Value::Undefined)
    });
}
