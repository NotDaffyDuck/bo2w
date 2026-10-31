//! Origins' elemental staffs, the client half: what each client sees and
//! hears from the fire, lightning and ice staffs' client fields
//! (clientscripts/mp/zombies/_zm_weap_staff_{fire,lightning,water}.csc).
//! The server scripts do the damage; the wind staff's throw lives in
//! clientfields.rs. Not drawn: the fire char and the ice frost on the
//! zombie's skin (the csc's `setshaderconstant` ramps).

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::Vm;

use super::Zm;
use crate::EventRecipients;

const FIRE_TORSO: &str = "env/fire/fx_fire_zombie_torso";
const FIRE_HEAD: &str = "env/fire/fx_fire_zombie_md";
const ELEC_HIT: &str = "weapon/zmb_staff/fx_zmb_staff_elec_ug_impact_hit_torso";
const ELEC_BALL: &str = "weapon/zmb_staff/fx_zmb_staff_elec_ug_impact_miss";
const ELEC_ARC: &str = "weapon/zmb_staff/fx_zmb_staff_elec_trail_bolt_cheap";
const ICE_STORM: &str = "weapon/zmb_staff/fx_zmb_staff_ice_ug_impact_hit";
const ICE_SHARD: &str = "weapon/zmb_staff/fx_zmb_staff_ice_trail_bolt";
/// The bone the ice shard sticks in.
const SHARD_BONE: &str = "j_spine4";

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

fn set_loop(game: &mut World, nn: u32, snd_alias: Option<&str>) {
    if let Some(e) = game.resource_mut::<Zm>().into_inner().ents.get_mut(&nn) {
        e.repeating_audio = snd_alias.map(str::to_owned);
    }
}

fn spine(world: &mut World, n: u32) -> Option<[f32; 3]> {
    super::tags::tag_world(world, n, "J_SpineUpper")
        .map(|(o, _)| o)
        .or_else(|| origin(world, n).map(|o| [o[0], o[1], o[2] + 48.0]))
}

/// An entity's staff field changed. True when it was one of these.
pub(super) fn entity_field(vm: &mut Vm<World>, game: &mut World, nn: u32, ident: &str, val: i64) -> bool {
    let on = val != 0;
    match ident {
        // zombie_fire_fx: the torso and head burn, with the fire loop.
        "fire_char_fx" => {
            let burns = [(FIRE_TORSO, "j_spinelower"), (FIRE_HEAD, "j_head")];
            for (fx, bone) in burns {
                super::natives_fx::bolt_effect(game, nn, fx, bone, true);
            }
            if val == 1 {
                for (fx, bone) in burns {
                    super::clientfields::bolt_queued(game, nn, fx, bone, false);
                }
            }
            set_loop(game, nn, (val == 1).then_some("zmb_fire_loop"));
        }
        // lightning_impact_play_fx: the shock on the zombie's chest.
        "lightning_impact_fx" if on => {
            super::clientfields::bolt_queued(game, nn, ELEC_HIT, "J_SpineUpper", false);
            if let Some(at) = origin(game, nn) {
                super::natives_fx::sound(game, EventRecipients::All, "wpn_imp_lightningstaff", at);
            }
        }
        // lightning_miss_play_fx: the ball and its hum; it is the ball the
        // arcs start from.
        "lightning_miss_fx" if on => {
            super::clientfields::bolt_queued(game, nn, ELEC_BALL, "tag_origin", false);
            set_loop(game, nn, Some("wpn_lightningstaff_ball"));
            let at = origin(game, nn).unwrap_or([0.0; 3]);
            game.get_resource_or_insert_with(StaffFx::default).ball = Some((nn, at));
        }
        "lightning_arc_fx" => {
            stop_arc(vm, game, nn);
            if on {
                start_arc(vm, game, nn);
            }
        }
        // staff_blizzard_fx: the storm, its burst and its wind loop.
        "staff_blizzard_fx" => {
            if on {
                super::clientfields::bolt_queued(game, nn, ICE_STORM, "tag_origin", false);
                if let Some(at) = origin(game, nn) {
                    super::natives_fx::sound(game, EventRecipients::All, "wpn_waterstaff_storm_imp", at);
                }
            } else {
                super::natives_fx::bolt_effect(game, nn, ICE_STORM, "tag_origin", true);
            }
            set_loop(game, nn, on.then_some("wpn_waterstaff_storm"));
        }
        // attach_model: the ice shard in the frozen zombie's back.
        "attach_bullet_model" => {
            super::natives_fx::bolt_effect(game, nn, ICE_SHARD, SHARD_BONE, true);
            if on {
                super::clientfields::bolt_queued(game, nn, ICE_SHARD, SHARD_BONE, false);
                if let Some(at) = origin(game, nn) {
                    super::natives_fx::sound(game, EventRecipients::All, "wpn_waterstaff_freeze_zombie", at);
                }
            }
        }
        _ => return false,
    }
    true
}

/// lightning_arc_play_fx_thread: a `tag_origin` model with the arc on it,
/// 0.1-0.5 s later, flicking between the ball and the zombie's spine.
fn start_arc(vm: &mut Vm<World>, game: &mut World, actor: u32) {
    let Some((ball, _)) = game.get_resource_or_insert_with(StaffFx::default).ball else {
        return;
    };
    let Some(at) = origin(game, ball) else { return };
    let delay = 100 + i64::from(vm.rand_u32() % 400);
    let model = game.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(gsc_t6::ObjKind::Entity(model));
    let now = {
        let mut zm = game.resource_mut::<Zm>();
        zm.ents.insert(
            model,
            super::Ent {
                obj: Some(obj),
                class_tag: "script_model".into(),
                origin: at,
                mesh: "tag_origin".into(),
                ..Default::default()
            },
        );
        zm.clock_millis
    };
    super::clientfields::bolt_later(game, model, delay, ELEC_ARC, "tag_origin");
    game.get_resource_or_insert_with(StaffFx::default).arcs.push(Arc {
        actor,
        model,
        next: now + delay,
        to_spine: true,
    });
}

fn remove_model(vm: &mut Vm<World>, game: &mut World, mdl: u32) {
    game.resource_mut::<Zm>().movers.list.remove(&mdl);
    if let Some(o) = game.resource_mut::<Zm>().ents.remove(&mdl).and_then(|e| e.obj) {
        vm.free_object(game, o);
    }
}

fn stop_arc(vm: &mut Vm<World>, game: &mut World, actor: u32) {
    let gone: Vec<u32> = {
        let mut s = game.get_resource_or_insert_with(StaffFx::default);
        let models = s.arcs.iter().filter(|a| a.actor == actor).map(|a| a.model).collect();
        s.arcs.retain(|a| a.actor != actor);
        models
    };
    for m in gone {
        remove_model(vm, game, m);
    }
}

/// Each tick: the ball's end (its burst sound where it last was) and each
/// arc's next flick.
pub(super) fn advance(game: &mut World, now: i64) {
    let Some(mut s) = game.get_resource_mut::<StaffFx>().map(|mut s| std::mem::take(&mut *s)) else {
        return;
    };
    let mut gone = Vec::new();
    if let Some((ball, last)) = s.ball {
        match origin(game, ball) {
            Some(at) => s.ball = Some((ball, at)),
            // watch_ball_fx_killed: the ball's burst; its arcs end with it.
            None => {
                super::natives_fx::sound(game, EventRecipients::All, "wpn_lightningstaff_ball_explo", last);
                s.ball = None;
                gone.extend(s.arcs.drain(..).map(|a| a.model));
            }
        }
    }
    let ball = s.ball.map(|(_, at)| at);
    s.arcs.retain_mut(|a| {
        if origin(game, a.actor).is_none() {
            gone.push(a.model);
            return false;
        }
        if now < a.next {
            return true;
        }
        let from = origin(game, a.model).unwrap_or([0.0; 3]);
        let to = if a.to_spine { spine(game, a.actor) } else { ball };
        if let Some(to) = to {
            game.resource_mut::<Zm>().movers.move_to(a.model, from, to, now, 0.1, 0.0, 0.0);
        }
        a.to_spine = !a.to_spine;
        a.next = now + 500;
        true
    });
    *game.resource_mut::<StaffFx>() = s;
    if !gone.is_empty() {
        super::with_vm(game, |vm, game| {
            for m in gone {
                remove_model(vm, game, m);
            }
        });
    }
}
