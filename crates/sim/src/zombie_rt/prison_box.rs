//! Mob of the Dead's Mystery Box glow: the client half of
//! `_zm_magicbox_prison` (its .csc). Each box place gets an anchor (the
//! script's `fx_obj`, a tag_origin where the box stands) that its effects
//! hang on: the low glow and hum while the box is away, the bright one where
//! it stands, the locked glow while Brutus holds it, the burst as it opens
//! and as it leaves.

use std::collections::HashMap;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{ObjKind, Vm};

use super::{Ent, Zm};
use crate::EventRecipients;

const OPEN: &str = "maps/zombie_alcatraz/fx_alcatraz_magicbox_open";
const LOCKED: &str = "maps/zombie_alcatraz/fx_alcatraz_magicbox_lock";
const AWAY: &str = "maps/zombie_alcatraz/fx_alcatraz_magicbox_amb";
const HERE: &str = "maps/zombie_alcatraz/fx_alcatraz_magicbox_arrive";
const LEAVING: &str = "maps/zombie_alcatraz/fx_alcatraz_magicbox_leave";
const HUM_LOW: &str = "zmb_hellbox_amb_low";
const HUM_HIGH: &str = "zmb_hellbox_amb_high";

/// Each box place (its zbarrier) -> its anchor and the glow on it now.
#[derive(Resource, Default)]
struct Anchors(HashMap<u32, (u32, Option<&'static str>)>);

/// One of the box's client fields changed on zbarrier `n`.
pub(super) fn field(vm: &mut Vm<World>, game: &mut World, nn: u32, ident: &str, val: i64) {
    let Some((spot, rot)) = game.resource::<Zm>().ents.get(&nn).map(|en| (en.origin, en.rot)) else {
        return;
    };
    let known = game
        .get_resource::<Anchors>()
        .and_then(|a| a.0.get(&nn).copied());
    let (anchor, glow) = match known {
        Some(a) => a,
        None => {
            let a = game.resource_mut::<Zm>().alloc_entnum();
            let obj = vm.alloc_object(ObjKind::Entity(a));
            game.resource_mut::<Zm>().ents.insert(
                a,
                Ent {
                    obj: Some(obj),
                    class_tag: "script_model".into(),
                    mesh: "tag_origin".into(),
                    origin: spot,
                    rot,
                    ..Default::default()
                },
            );
            game.init_resource::<Anchors>();
            game.resource_mut::<Anchors>().0.insert(nn, (a, None));
            (a, None)
        }
    };
    match ident {
        // magicbox_initial_closed_fx: only on a place not set up before.
        "magicbox_initial_fx" if known.is_none() && val == 1 => {
            set_glow(game, nn, anchor, glow, AWAY, HUM_LOW);
        }
        "magicbox_amb_fx" => {
            let (fx, hum, sound) = match val {
                0 => (AWAY, HUM_LOW, Some("zmb_hellbox_leave")),
                1 => (HERE, HUM_HIGH, Some("zmb_hellbox_arrive")),
                2 => (LOCKED, HUM_HIGH, None),
                _ => return,
            };
            set_glow(game, nn, anchor, glow, fx, hum);
            if let Some(snd_alias) = sound {
                super::natives_fx::sound(game, EventRecipients::All, snd_alias, spot);
            }
        }
        "magicbox_open_fx" => super::clientfields::bolt_queued(game, anchor, OPEN, "tag_origin", val == 0),
        "magicbox_leaving_fx" if val == 1 => {
            super::clientfields::bolt_queued(game, anchor, LEAVING, "tag_origin", false);
        }
        _ => {}
    }
}

/// The glow on the anchor swapped for `fx`, and its hum.
fn set_glow(
    world: &mut World,
    n: u32,
    anchor: u32,
    old: Option<&'static str>,
    fx: &'static str,
    hum: &str,
) {
    if let Some(old) = old {
        super::clientfields::bolt_queued(world, anchor, old, "tag_origin", true);
    }
    super::clientfields::bolt_queued(world, anchor, fx, "tag_origin", false);
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&anchor) {
        e.repeating_audio = Some(hum.to_owned());
    }
    world.resource_mut::<Anchors>().0.insert(n, (anchor, Some(fx)));
}
