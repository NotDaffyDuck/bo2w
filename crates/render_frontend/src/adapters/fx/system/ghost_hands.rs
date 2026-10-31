//! Mob's Afterlife: the glow on the ghost hands.

use bevy::prelude::*;
use fx::PlaybackResult;
use render_fx::{EffectsSystemHandle, ReadyFxBook, PreparedFxElemTable};

use super::EffectStageAccess;
use crate::adapters::fx::effect_play::EffectStage;

/// bo2zm: Mob's Afterlife ghost hands glow. BO2's client script
/// (`zm_afterlife.csc`, which does not run here) plays
/// `fx_alcatraz_ghost_vm_wrist` on the left wrist and `_r` on the right,
/// then 0.1 seconds on the pixie dust on the gun tag, and again every 0.2
/// seconds while the ghost hands are on.
pub(super) fn present_ghost_hands_fx(
    hands: Option<Res<render_anim::FpvHandsOverride>>,
    bolts: Res<render_anim::AttachTagGoals>,
    fx_book: Option<Res<ReadyFxBook>>,
    mut fx_elem_info: ResMut<PreparedFxElemTable>,
    mut fx_host: ResMut<EffectsSystemHandle>,
    effect_level: EffectStageAccess,
    time: Res<Time>,
    mut next: Local<Option<(f64, bool)>>,
    mut missing: Local<bool>,
) {
    const WRIST: [&str; 2] = [
        "maps/zombie_alcatraz/fx_alcatraz_ghost_vm_wrist",
        "maps/zombie_alcatraz/fx_alcatraz_ghost_vm_wrist_r",
    ];
    const PIXIES: &str = "maps/zombie_alcatraz/fx_alcatraz_afterlife_pixies";
    let ghost = hands
        .as_ref()
        .and_then(|h| h.0.as_deref())
        .is_some_and(|m| m.contains("ghost"));
    let Some(effect_book) = fx_book.filter(|_| ghost) else {
        *next = None;
        return;
    };
    let now = time.elapsed_secs_f64();
    let (at, pixies) = next.unwrap_or((now, false));
    if now < at {
        return;
    }
    let plays: Vec<(&str, Option<fx::EffectBoltTarget>)> = if pixies {
        vec![(PIXIES, bolts.weapon)]
    } else {
        vec![(WRIST[0], bolts.wrist[0]), (WRIST[1], bolts.wrist[1])]
    };
    if plays.iter().any(|(_, target)| target.is_none()) {
        return;
    }
    fx_elem_info.0.restock(&effect_book.0);
    for (name, target) in plays {
        let result = render_fx::effect_play::spawn_named_attached_world_effect(
            &mut fx_host.0,
            &effect_book.0,
            &fx_elem_info.0,
            effect_book.0.map_fx_name(name),
            target.expect("checked above"),
            effect_level.view().as_ref().map(|scene| scene as &dyn EffectStage),
        );
        if result.and_then(PlaybackResult::handle).is_none() && !*missing {
            *missing = true;
            diag::warn!(World, "bo2zm ghost hands fx {name}: did not play ({result:?})");
        }
    }
    *next = Some((at.max(now - 0.1) + 0.1, !pixies));
}
