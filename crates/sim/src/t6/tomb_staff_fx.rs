//! Origins' elemental staffs, the client half: what each client sees and
//! hears from the fire, lightning and ice staffs' client fields
//! (clientscripts/mp/zombies/_zm_weap_staff_{fire,lightning,water}.csc).
//! The server scripts do the damage; the wind staff's throw lives in
//! clientfields.rs. Not drawn: the fire char and the ice frost on the
//! zombie's skin (the csc's `setshaderconstant` ramps).

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::Vm;

use super::Zm;
use crate::EventAudience;

const FIRE_TORSO: &str = "env/fire/fx_fire_zombie_torso";
const FIRE_HEAD: &str = "env/fire/fx_fire_zombie_md";
const ELEC_HIT: &str = "weapon/zmb_staff/fx_zmb_staff_elec_ug_impact_hit_torso";
const ELEC_BALL: &str = "weapon/zmb_staff/fx_zmb_staff_elec_ug_impact_miss";
const ELEC_ARC: &str = "weapon/zmb_staff/fx_zmb_staff_elec_trail_bolt_cheap";
const ICE_STORM: &str = "weapon/zmb_staff/fx_zmb_staff_ice_ug_impact_hit";
const ICE_SHARD: &str = "weapon/zmb_staff/fx_zmb_staff_ice_trail_bolt";

/// One arc from the lightning ball to a zombie it shocks.
struct Arc {
    actor: u32,
    model: u32,
    /// The arc's next move: to the zombie's spine or back to the ball.
    next: i64,
    to_spine: bool,
}

#[derive(Resource, Default)]
struct StaffFx {
    /// `level.lightning_ball_fx`: the ball standing, and where it last was.
    ball: Option<(u32, [f32; 3])>,
    arcs: Vec<Arc>,
}

fn origin(world: &World, n: u32) -> Option<[f32; 3]> {
    world.resource::<Zm>().ents.get(&n).map(|e| e.origin)
}

fn set_loop(world: &mut World, n: u32, alias: Option<&str>) {
    if let Some(e) = world.resource_mut::<Zm>().into_inner().ents.get_mut(&n) {
        e.loop_sound = alias.map(str::to_owned);
    }
}

fn spine(world: &mut World, n: u32) -> Option<[f32; 3]> {
    super::tags::tag_world(world, n, "J_SpineUpper")
        .map(|(o, _)| o)
        .or_else(|| origin(world, n).map(|o| [o[0], o[1], o[2] + 48.0]))
}

/// An entity's staff field changed. True when it was one of these.
pub(super) fn entity_field(vm: &mut Vm<World>, world: &mut World, n: u32, name: &str, value: i64) -> bool {
    let on = value != 0;
    match name {
        // zombie_fire_fx: the torso and head burn, with the fire loop.
        "fire_char_fx" => {
            super::natives_fx::bolt_effect(world, n, FIRE_TORSO, "j_spinelower", true);
            super::natives_fx::bolt_effect(world, n, FIRE_HEAD, "j_head", true);
            if value == 1 {
                super::clientfields::bolt_queued(world, n, FIRE_TORSO, "j_spinelower", false);
                super::clientfields::bolt_queued(world, n, FIRE_HEAD, "j_head", false);
            }
            set_loop(world, n, (value == 1).then_some("zmb_fire_loop"));
        }
        // lightning_impact_play_fx: the shock on the zombie's chest.
        "lightning_impact_fx" if on => {
            super::clientfields::bolt_queued(world, n, ELEC_HIT, "J_SpineUpper", false);
            if let Some(at) = origin(world, n) {
                super::natives_fx::sound(world, EventAudience::All, "wpn_imp_lightningstaff", at);
            }
        }
        // lightning_miss_play_fx: the ball and its hum; it is the ball the
        // arcs start from.
        "lightning_miss_fx" if on => {
            super::clientfields::bolt_queued(world, n, ELEC_BALL, "tag_origin", false);
            set_loop(world, n, Some("wpn_lightningstaff_ball"));
            let at = origin(world, n).unwrap_or([0.0; 3]);
            world.get_resource_or_insert_with(StaffFx::default).ball = Some((n, at));
        }
        "lightning_arc_fx" => {
            stop_arc(vm, world, n);
            if on {
                start_arc(vm, world, n);
            }
        }
        // staff_blizzard_fx: the storm, its burst and its wind loop.
        "staff_blizzard_fx" => {
            if on {
                super::clientfields::bolt_queued(world, n, ICE_STORM, "tag_origin", false);
                if let Some(at) = origin(world, n) {
                    super::natives_fx::sound(world, EventAudience::All, "wpn_waterstaff_storm_imp", at);
                }
            } else {
                super::natives_fx::bolt_effect(world, n, ICE_STORM, "tag_origin", true);
            }
            set_loop(world, n, on.then_some("wpn_waterstaff_storm"));
        }
        // attach_model: the ice shard in the frozen zombie's back.
        "attach_bullet_model" => {
            super::natives_fx::bolt_effect(world, n, ICE_SHARD, "j_spine4", true);
            if on {
                super::clientfields::bolt_queued(world, n, ICE_SHARD, "j_spine4", false);
                if let Some(at) = origin(world, n) {
                    super::natives_fx::sound(world, EventAudience::All, "wpn_waterstaff_freeze_zombie", at);
                }
            }
        }
        _ => return false,
    }
    true
}

/// lightning_arc_play_fx_thread: a `tag_origin` model with the arc on it,
/// 0.1-0.5 s later, flicking between the ball and the zombie's spine.
fn start_arc(vm: &mut Vm<World>, world: &mut World, actor: u32) {
    let Some((ball, _)) = world.get_resource_or_insert_with(StaffFx::default).ball else {
        return;
    };
    let Some(at) = origin(world, ball) else { return };
    let delay = 100 + i64::from(vm.rand_u32() % 400);
    let model = world.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(gsc_t6::ObjKind::Entity(model));
    let now = {
        let mut zm = world.resource_mut::<Zm>();
        zm.ents.insert(
            model,
            super::Ent {
                obj: Some(obj),
                classname: "script_model".into(),
                origin: at,
                model: "tag_origin".into(),
                ..Default::default()
            },
        );
        zm.now_ms
    };
    super::clientfields::bolt_later(world, model, delay, ELEC_ARC, "tag_origin");
    world.get_resource_or_insert_with(StaffFx::default).arcs.push(Arc {
        actor,
        model,
        next: now + delay,
        to_spine: true,
    });
}

fn remove_model(vm: &mut Vm<World>, world: &mut World, model: u32) {
    world.resource_mut::<Zm>().movers.list.remove(&model);
    if let Some(o) = world.resource_mut::<Zm>().ents.remove(&model).and_then(|e| e.obj) {
        vm.free_object(world, o);
    }
}

fn stop_arc(vm: &mut Vm<World>, world: &mut World, actor: u32) {
    let gone: Vec<u32> = {
        let mut s = world.get_resource_or_insert_with(StaffFx::default);
        let models = s.arcs.iter().filter(|a| a.actor == actor).map(|a| a.model).collect();
        s.arcs.retain(|a| a.actor != actor);
        models
    };
    for m in gone {
        remove_model(vm, world, m);
    }
}

/// Each tick: the ball's end (its burst sound where it last was) and each
/// arc's next flick.
pub(super) fn advance(world: &mut World, now: i64) {
    let Some(mut s) = world.get_resource_mut::<StaffFx>().map(|mut s| std::mem::take(&mut *s)) else {
        return;
    };
    let mut gone = Vec::new();
    if let Some((ball, last)) = s.ball {
        match origin(world, ball) {
            Some(at) => s.ball = Some((ball, at)),
            // watch_ball_fx_killed: the ball's burst; its arcs end with it.
            None => {
                super::natives_fx::sound(world, EventAudience::All, "wpn_lightningstaff_ball_explo", last);
                s.ball = None;
                gone.extend(s.arcs.drain(..).map(|a| a.model));
            }
        }
    }
    let ball = s.ball.map(|(_, at)| at);
    s.arcs.retain_mut(|a| {
        if origin(world, a.actor).is_none() {
            gone.push(a.model);
            return false;
        }
        if now < a.next {
            return true;
        }
        let from = origin(world, a.model).unwrap_or([0.0; 3]);
        let to = if a.to_spine { spine(world, a.actor) } else { ball };
        if let Some(to) = to {
            world.resource_mut::<Zm>().movers.move_to(a.model, from, to, now, 0.1, 0.0, 0.0);
        }
        a.to_spine = !a.to_spine;
        a.next = now + 500;
        true
    });
    *world.resource_mut::<StaffFx>() = s;
    if !gone.is_empty() {
        super::with_vm(world, |vm, world| {
            for m in gone {
                remove_model(vm, world, m);
            }
        });
    }
}
