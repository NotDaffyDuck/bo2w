//! bo2mp: Black Ops II's weapon pickup hint. Looking at a gun on the
//! ground the game prints "Press F to swap for" and the gun's name under the
//! crosshair (`CG_DrawCursorhint`, owner-draw 72, with the CoD1-style weapon
//! hints on: the use line then " %s", the two centred together on the
//! item's rect). The line is `CG_GetWeaponUseString`'s: a primary says
//! `PLATFORM_PICKUPNEWWEAPON` while he holds fewer than two primaries
//! (`BG_PlayerWeaponCountPrimaryTypes`) and `PLATFORM_SWAPWEAPONS` once
//! both are full, the key bound to `+activate`; a frag-class grenade says
//! `PLATFORM_THROWBACKGRENADE` with the `+frag` key and no name; anything
//! else says `PLATFORM_PICKUPNEWWEAPON`. Never in hardcore. The sim sets
//! `hint_kind` to the gun plus 4 (`HINT_WEAPON`'s offset) while one is
//! offered to the use button.

use bevy::prelude::*;
use frame::{ClientStep, OverlayControlSnapshot};
use net::{HostViewClient, ShownWorldFrame};

use crate::{MainWindow, covering_node, pinned_node};
use crate::tiers::{OverlayTier, OverlayStackShown};

/// `hint_kind` past this is a weapon: the gun is `hint_kind - HINT_WEAPON_BASE`.
const HINT_WEAPON_BASE: i32 = 4;
/// The text's centre height in the 720-high root. The `cursorhints` menu's
/// `chRect` (owner-draw 72) is 70 below the 480-high screen's middle (310
/// of 480); `CG_DrawCursorhint` puts the line's baseline half a text
/// height under the rect's top, so the letters sit about on 312 (468).
const LINE_Y: f32 = 468.0;
/// The font's height in the 720-high root: the item's text scale 0.315 of
/// the 48-unit normalised font, 15.1 of 480.
const FONT_PX: f32 = 22.7;
/// `WEAPINVENTORY_PRIMARY` and `OFFHAND_CLASS_FRAG_GRENADE`.
const INVENTORY_PRIMARY: i32 = 0;
const OFFHAND_FRAG: i32 = 1;

#[derive(Component)]
struct PickupHintRoot {
    shows: String,
}

/// What the hint says for a gun: (localization key, command whose key fills
/// `&&1`, whether the gun's name follows).
pub(crate) fn use_line(carry_slot_kind: i32, offhand_class: i32, primaries_held: usize) -> (&'static str, &'static str, bool) {
    if carry_slot_kind == INVENTORY_PRIMARY {
        let key = if primaries_held < 2 { "PLATFORM_PICKUPNEWWEAPON" } else { "PLATFORM_SWAPWEAPONS" };
        (key, "+activate", true)
    } else if offhand_class == OFFHAND_FRAG {
        ("PLATFORM_THROWBACKGRENADE", "+frag", false)
    } else {
        ("PLATFORM_PICKUPNEWWEAPON", "+activate", true)
    }
}

/// The gun offered to the use button, if the hint is a weapon's.
pub(crate) fn offered_gun(hint_kind: i32) -> Option<u32> {
    (hint_kind > HINT_WEAPON_BASE).then(|| (hint_kind - HINT_WEAPON_BASE) as u32)
}

fn translated<'a>(strings: &'a assets::T6Ui, key: &str) -> Option<&'a str> {
    strings
        .strings
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    roots: Query<(Entity, &PickupHintRoot)>,
    window: MainWindow,
    presented: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    weapons: Option<Res<assets::StagedWeapons>>,
    strings: Option<Res<assets::T6Ui>>,
    keys: Option<Res<OverlayControlSnapshot>>,
    fonts: Option<Res<crate::bo2_font::Bo2Fonts>>,
) {
    let wanted = (|| {
        let (p, l, w, s) = (
            presented.as_deref()?,
            local.as_deref()?,
            weapons.as_deref()?,
            strings.as_deref()?,
        );
        let dvars = p.snapshot()?.meta.script_dvars(l.0);
        if dvars.string("bo2mp_gametype").is_none()
            || dvars.int("g_hardcore").or_else(|| dvars.int("scr_hardcore")).unwrap_or(0) != 0
        {
            return None;
        }
        let ps = p.living_participant(l.0)?;
        let gun = offered_gun(ps.hint_kind)?;
        let facts = w.0.summary_for(gun)?;
        let primaries = ps
            .weapons
            .iter()
            .filter(|&&h| h > 0)
            .filter(|&&h| w.0.summary_for(h as u32).is_some_and(|f| f.carry_slot_kind == INVENTORY_PRIMARY))
            .count();
        let (key, command, named) = use_line(facts.carry_slot_kind, facts.secondary_hand_category, primaries);
        let keys = keys.as_deref().cloned().unwrap_or_default();
        let bind = keys.binding_keys.get(command).cloned().unwrap_or_else(|| "UNBOUND".to_owned());
        let line = translated(s, key)?.replace("&&1", &bind.to_uppercase());
        let name = named
            .then(|| w.0.title_token_for(gun).and_then(|t| translated(s, t)).map(str::to_owned))
            .flatten();
        Some((line, name, keys))
    })();
    let (Some((line, name, keys)), Some(win), Some(fonts)) = (wanted, window.shown(), fonts.as_deref()) else {
        for (e, _) in &roots {
            commands.entity(e).try_despawn();
        }
        return;
    };
    let scale = win.height() / 720.0;
    let shows = format!("{line}|{}|{scale:.2}|{}", name.as_deref().unwrap_or(""), win.width());
    if roots.iter().any(|(_, r)| r.shows == shows) {
        return;
    }
    for (e, _) in &roots {
        commands.entity(e).try_despawn();
    }
    let px = FONT_PX * scale;
    let mut runs = crate::zm_hud::hint_runs(&line, &keys);
    if let Some(name) = &name {
        runs.push((format!(" {name}"), Color::WHITE));
    }
    let plain: String = runs.iter().map(|(t, _)| t.as_str()).collect();
    let width = fonts.line_width("Default", &plain, px);
    commands
        .spawn((
            PickupHintRoot { shows },
            OverlayTier::Hud,
            OverlayStackShown,
            covering_node(),
        ))
        .with_children(|outer| {
            outer
                .spawn(pinned_node(win.width() * 0.5 - width * 0.5, LINE_Y * scale - px * 0.6))
                .with_children(|c| {
                    fonts.spawn_line(c, "Default", &runs, px, 1.0 * scale);
                });
        });
}

pub(crate) fn register(host: &mut App) {
    host.add_systems(Update, update.after(crate::bo2_font::load_bo2_fonts).in_set(ClientStep::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_primary_picks_up_until_both_slots_are_full_then_swaps() {
        assert_eq!(use_line(0, 0, 0), ("PLATFORM_PICKUPNEWWEAPON", "+activate", true));
        assert_eq!(use_line(0, 0, 1), ("PLATFORM_PICKUPNEWWEAPON", "+activate", true));
        assert_eq!(use_line(0, 0, 2), ("PLATFORM_SWAPWEAPONS", "+activate", true));
    }

    #[test]
    fn a_frag_is_thrown_back_without_a_name_and_others_are_picked_up() {
        assert_eq!(use_line(1, 1, 2), ("PLATFORM_THROWBACKGRENADE", "+frag", false));
        assert_eq!(use_line(1, 2, 2), ("PLATFORM_PICKUPNEWWEAPON", "+activate", true));
    }

    #[test]
    fn only_a_weapon_hint_offers_a_gun() {
        assert_eq!(offered_gun(0), None);
        assert_eq!(offered_gun(4), None);
        assert_eq!(offered_gun(9), Some(5));
    }
}
