//! The map's moving props (BO2 drives them from a client script,
//! zm_nuked_fx.csc): every map model named "fxanim" plays its scene's
//! animation (washing on the lines, shutters, curtains, cabinets, roaches,
//! the roof vent) 3 s in plus its own `fxanim_wait`; the wires
//! ("wirespark_*") start 2 s later and spark at their end joint every 4 s;
//! the far dust devils ("fxanim_mp_dustdevil") whirl every 15-30 s with a
//! fire effect on their joint and the tornado sound for 12 s; the porch
//! creaks and falls 5000-10000 s in (a long game's 83-167 minutes; the
//! script shares this with other maps: Nuketown Zombies has no porch prop).

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;

use super::{Zm, with_vm};

/// A scene's animation (the script's `level.scr_anim["fxanim_props"]`).
fn scene_anim(scene: &str) -> Option<&'static str> {
    Some(match scene {
        "pant01_fast" => "fxanim_gp_pant01_fast_anim",
        "shirt01_fast" => "fxanim_gp_shirt01_fast_anim",
        "sheet_med" => "fxanim_gp_cloth_sheet_med_fast_anim",
        "wirespark_long" => "fxanim_gp_wirespark_long_anim",
        "wirespark_med" => "fxanim_gp_wirespark_med_anim",
        "roaches" => "fxanim_gp_roaches_anim",
        "wht_shutters" => "fxanim_zom_nuketown_shutters_anim",
        "wht_shutters02" => "fxanim_zom_nuketown_shutters02_anim",
        "win_curtains" => "fxanim_zom_curtains_anim",
        "cabinets_brwn" => "fxanim_zom_nuketown_cabinets_brwn_anim",
        "cabinets_brwn02" => "fxanim_zom_nuketown_cabinets_brwn02_anim",
        "cabinets_red" => "fxanim_zom_nuketown_cabinets_red_anim",
        "porch" => "fxanim_zom_nuketown_porch_anim",
        "roofvent" => "fxanim_gp_roofvent_small_wobble_anim",
        // Die Rise (zm_highrise_fx.csc): its wires only sway, the dragons
        // wait for the easter egg's end, the rock slide for its notify.
        "wirespark_med_lrg" => "fxanim_gp_wirespark_med_lrg_anim",
        "wirespark_long_lrg" => "fxanim_gp_wirespark_long_lrg_anim",
        "dragon_a" => "fxanim_zom_highrise_dragon_a_anim",
        "dragon_b" => "fxanim_zom_highrise_dragon_b_anim",
        "dragon" => "fxanim_zom_highrise_dragon_idle_anim",
        "rock_slide" => "fxanim_zom_highrise_rock_slide_anim",
        _ => return None,
    })
}

const DEVIL_ANIM: &str = "fxanim_mp_dustdevil_anim";
const WIRE_FX: &str = "electrical/fx_elec_wire_spark_burst_xsm";

#[derive(Clone, Debug)]
enum Kind {
    Scene,
    /// The joint its spark comes from.
    Wire(&'static str),
    Devil,
    Porch,
}

/// A whirl's fire effect and when it is taken off.
#[derive(Clone, Copy, Debug)]
struct FireOff {
    at: i64,
    fx: &'static str,
}

#[derive(Clone, Debug)]
struct Prop {
    n: u32,
    kind: Kind,
    reel: String,
    len: i64,
    looping: bool,
    /// When its animation (re)started.
    start: Option<i64>,
    /// The next spark / whirl.
    next: i64,
    /// A whirl's fire effect, while it burns.
    fire_off: Option<FireOff>,
    /// The level notify a scene waits for first (`fxanim_waittill_1`),
    /// then its `fxanim_wait` (ms).
    waits: Option<(String, i64)>,
}

#[derive(Default)]
pub(crate) struct FxProps {
    seen: bool,
    props: Vec<Prop>,
    seed: u64,
}

impl FxProps {
    fn rand(&mut self) -> u64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.seed >> 33
    }
    /// 15 to 30 s, as `randomfloatrange(15, 30)`.
    fn whirl_wait(&mut self) -> i64 {
        15000 + (self.rand() % 15000) as i64
    }
}

fn find(world: &mut World) {
    let rows: Vec<(u32, String, String, f32, String)> = with_vm(world, |vm, world| {
        let (tn, scene, wait, until) = (
            vm.intern("targetname"),
            vm.intern("fxanim_scene_1"),
            vm.intern("fxanim_wait"),
            vm.intern("fxanim_waittill_1"),
        );
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .where_value(|en| en.map && en.class_tag == "script_model")
            .filter_map(|(nn, en)| {
                let o = en.obj?;
                let name = vm.to_text(&vm.raw_field(o, tn));
                if name != "fxanim" && name != "fxanim_mp_dustdevil" {
                    return None;
                }
                let s = vm.to_text(&vm.raw_field(o, scene));
                let w = vm.to_text(&vm.raw_field(o, wait)).trim().parse().unwrap_or(0.0);
                let u = vm.to_text(&vm.raw_field(o, until));
                Some((*nn, name, s, w, u))
            })
            .collect()
    })
    .unwrap_or_default();
    let mut zm = world.resource_mut::<Zm>();
    let mut props = Vec::new();
    let mut missing = Vec::new();
    for (nn, name, scene, wait, until) in rows {
        let (kind, clip) = if name == "fxanim_mp_dustdevil" {
            (Kind::Devil, DEVIL_ANIM)
        } else {
            let Some(clip) = scene_anim(&scene) else {
                continue;
            };
            let kind = match scene.as_str() {
                "wirespark_long" => Kind::Wire("long_spark_06_jnt"),
                "wirespark_med" => Kind::Wire("med_spark_06_jnt"),
                "porch" => Kind::Porch,
                _ => Kind::Scene,
            };
            (kind, clip)
        };
        let Some(anim) = zm.anims.get(clip) else {
            missing.push(clip);
            continue;
        };
        let len = ((f32::from(anim.frame_total) / anim.fps.max(1.0)) * 1000.0).round() as i64;
        let wait = (wait * 1000.0) as i64;
        let waits = (!until.is_empty() && matches!(kind, Kind::Scene)).then(|| (until, wait));
        let next = match kind {
            Kind::Scene if waits.is_some() => i64::MAX,
            Kind::Scene => 3000 + wait,
            Kind::Wire(_) => 5000 + wait,
            Kind::Devil => 3000,
            // `wait randomintrange(5, 10) * 1000` (seconds).
            Kind::Porch => 3000 + wait + (5 + i64::from(nn % 5)) * 1_000_000,
        };
        props.push(Prop {
            n: nn,
            looping: anim.looping && !matches!(kind, Kind::Devil),
            kind,
            reel: clip.to_owned(),
            len: len.max(1),
            start: None,
            next,
            fire_off: None,
            waits,
        });
    }
    missing.sort_unstable();
    missing.dedup();
    diag::info!(
        Sim,
        "bo2zm t6 moving props: {} ({} whirls, {} wires, {} porches){}",
        props.len(),
        props.iter().filter(|p| matches!(p.kind, Kind::Devil)).count(),
        props.iter().filter(|p| matches!(p.kind, Kind::Wire(_))).count(),
        props.iter().filter(|p| matches!(p.kind, Kind::Porch)).count(),
        if missing.is_empty() {
            String::new()
        } else {
            format!("; animations not loaded: {missing:?}")
        }
    );
    zm.fxprops.seen = true;
    zm.fxprops.seed = 0x5eed_f00d;
    zm.fxprops.props = props;
}

/// An effect to bolt on a prop once the props are put back: the prop, the
/// effect, the joint and whether it comes off there.
struct Spark {
    n: u32,
    fx: &'static str,
    joint: &'static str,
    off: bool,
}

/// Starts, sparks and whirls the props (after the models are shown, so
/// effects bolt to them).
pub(super) fn advance(game: &mut World, now: i64) {
    if !game.resource::<Zm>().fxprops.seen {
        if now < 1000 {
            return;
        }
        find(game);
    }
    let mut sparks: Vec<Spark> = Vec::new();
    let mut sounds: Vec<(u32, &'static str)> = Vec::new();
    {
        let mut zm = game.resource_mut::<Zm>();
        let mut fx = std::mem::take(&mut zm.fxprops);
        for i in 0..fx.props.len() {
            if let Some(FireOff { at: off, fx: name }) = fx.props[i].fire_off
                && now >= off
            {
                sparks.push(Spark { n: fx.props[i].n, fx: name, joint: "dervish_jnt", off: true });
                fx.props[i].fire_off = None;
            }
            if now < fx.props[i].next {
                continue;
            }
            match fx.props[i].kind {
                Kind::Scene => {
                    fx.props[i].start = Some(now);
                    fx.props[i].next = i64::MAX;
                }
                Kind::Porch => {
                    fx.props[i].start = Some(now);
                    fx.props[i].next = i64::MAX;
                    sounds.push((fx.props[i].n, "zmb_porch_collapse"));
                }
                Kind::Wire(joint) => {
                    if fx.props[i].start.is_none() {
                        fx.props[i].start = Some(now);
                    } else {
                        sparks.push(Spark { n: fx.props[i].n, fx: WIRE_FX, joint, off: false });
                    }
                    // `randomintrange(4, 5)`: always 4.
                    fx.props[i].next = now + 4000;
                }
                Kind::Devil => {
                    if fx.props[i].start.is_some() || fx.props[i].next > 3000 {
                        let fire = if fx.rand() % 2 == 0 {
                            "maps/zombie/fx_zmb_fire_devil_sm"
                        } else {
                            "maps/zombie/fx_zmb_fire_devil_lg"
                        };
                        fx.props[i].start = Some(now);
                        fx.props[i].fire_off = Some(FireOff { at: now + 12000, fx: fire });
                        sparks.push(Spark { n: fx.props[i].n, fx: fire, joint: "dervish_jnt", off: false });
                        sounds.push((fx.props[i].n, "amb_fire_tornado"));
                    }
                    let w = fx.whirl_wait();
                    fx.props[i].next = now + w;
                }
            }
        }
        zm.fxprops = fx;
    }
    for spark in sparks {
        super::natives_fx::bolt_effect(game, spark.n, spark.fx, spark.joint, spark.off);
    }
    for (n, snd_alias) in sounds {
        let spot = game.resource::<Zm>().ents.get(&n).map(|e| e.origin);
        if let Some(spot) = spot {
            if snd_alias == "zmb_porch_collapse" {
                diag::info!(Sim, "bo2zm t6 the porch falls at {spot:?}");
            }
            super::natives_fx::sound(game, crate::EventRecipients::All, snd_alias, spot);
        }
    }
}

/// A level notify from the server (`clientnotify`): the scenes waiting on
/// it start after their `fxanim_wait` (BO2 waits 3 s before listening).
pub(super) fn level_notify(world: &mut World, state: &str) {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.clock_millis;
    if now < 3000 {
        return;
    }
    for p in &mut zm.fxprops.props {
        if let Some((until, wait)) = &p.waits
            && until == state
            && p.start.is_none()
            && p.next == i64::MAX
        {
            p.next = now + wait;
            diag::info!(Sim, "bo2zm t6 moving prop {} starts {} on {state}", p.n, p.reel);
        }
    }
}

/// A prop's playing animation: (clip, fraction, rate per second).
pub(super) fn present(world: &World, n: u32, now: i64) -> Option<(String, f32, f32)> {
    let zm = world.resource::<Zm>();
    let p = zm.fxprops.props.iter().find(|p| p.n == n)?;
    let start = p.start?;
    let elapsed = (now - start).max(0);
    if p.looping {
        Some((
            p.reel.clone(),
            (elapsed % p.len) as f32 / p.len as f32,
            1000.0 / p.len as f32,
        ))
    } else if elapsed >= p.len {
        Some((p.reel.clone(), 1.0, 0.0))
    } else {
        Some((
            p.reel.clone(),
            elapsed as f32 / p.len as f32,
            1000.0 / p.len as f32,
        ))
    }
}
