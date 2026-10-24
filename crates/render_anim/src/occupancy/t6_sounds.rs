//! bo2mp lane F: Black Ops II's own sounds for bullets striking and for
//! other players' bodies, where its engine names them by the surface:
//!
//! - a bullet's impact `prj_bullet_impact_<small|large|ap|xtreme>[_exit]_<surface>`
//!   (shotguns `prj_bulletspray_*`, bolts `prj_bolt_impact`, blades
//!   `prj_blade_impact`); on another player's body the `flesh` one, on his
//!   head also `prj_bullet_impact_headshot`; on the local player the 2D
//!   `_player` one (his own hurt);
//! - a crossbow bolt or a thrown blade striking (`EV_BOLT_IMPACT`):
//!   `prj_bolt_impact_<surface>` or `prj_blade_impact_<surface>` where it
//!   went in, a player's body `flesh` (the same for the local player);
//! - a body hitting the ground: its death animation's `bodyfall large` /
//!   `bodyfall small` notetracks, `fly_bodyfall_<large|small>_<surface>`;
//! - a dive to prone: `fly_dtp_launch_<npc|plr>` as he leaves the ground,
//!   `fly_dtp_land_<npc|plr>_<class>` as he lands (`plr` for his own, 2D),
//!   the class from Black Ops II's dive table (`Dtp_CalcSurfaceType`): its
//!   32 surfaces fold into the nine its dive sounds exist for.
//!
//! The surfaces are BO2's own list (its index order, with `player`,
//! `tallgrass` and `riotshield` after `paintedmetal`).

use bevy::prelude::*;
use net::LocalPresentClient;

use asset_core::AssetNamespace;

/// Black Ops II's surface types, by index (t6mp's own list).
pub(crate) const T6_SURFACE_NAMES: [&str; 32] = [
    "default",
    "bark",
    "brick",
    "carpet",
    "cloth",
    "concrete",
    "dirt",
    "flesh",
    "foliage",
    "glass",
    "grass",
    "gravel",
    "ice",
    "metal",
    "mud",
    "paper",
    "plaster",
    "rock",
    "sand",
    "snow",
    "water",
    "wood",
    "asphalt",
    "ceramic",
    "plastic",
    "rubber",
    "cushion",
    "fruit",
    "paintedmetal",
    "player",
    "tallgrass",
    "riotshield",
];

const SURF_FLESH: usize = 7;

/// A surface's name from surface flags (bits 20..25, masked 0x3f as BO2 does; 32 and up are "default").
pub(crate) fn surface_name(surface_flags: u32) -> &'static str {
    T6_SURFACE_NAMES
        .get(((surface_flags >> 20) & 0x3f) as usize)
        .copied()
        .unwrap_or("default")
}

/// The nine surface classes Black Ops II's dive sounds exist for.
const DTP_SURFACE_NAMES: [&str; 9] = [
    "default", "metal", "concrete", "wood", "snow", "dirt", "gravel", "grass", "mud",
];

/// Black Ops II's dive table (`Dtp_CalcSurfaceType`): each of its 32
/// surfaces (by index) to the class its dive sounds use.
const DTP_SURFACE_REMAP: [u8; 32] = [
    0, 3, 2, 4, 4, 2, 5, 2, 7, 5, 7, 6, 4, 1, 8, 2, 2, 2, 5, 4, 8, 3, 2, 2, 2, 2, 5, 5, 1, 5, 7, 0,
];

/// The dive sounds' class for the surface under him (surface flags, bits 20..25).
pub(crate) fn dive_surface_name(surface_flags: u32) -> &'static str {
    let surf = ((surface_flags >> 20) & 0x3f) as usize;
    DTP_SURFACE_REMAP
        .get(surf)
        .and_then(|&c| DTP_SURFACE_NAMES.get(c as usize))
        .copied()
        .unwrap_or("default")
}

/// A world sound by its BO2 name (with a fallback for a surface it lacks).
pub(crate) fn play_t6(
    out: &mut MessageWriter<audio::AliasCommand>,
    alias: String,
    fallback: Option<String>,
    origin: Option<[f32; 3]>,
    snd_ent: Option<u32>,
) {
    out.write(audio::AliasCommand::Play(audio::PlayAlias {
        namespace: AssetNamespace::T6,
        alias,
        fallback,
        origin_inches: origin,
        snd_ent,
    }));
}

/// The impact sound's base name for a weapon's impact type (Black Ops'
/// list), and the base of its exit sound.
fn impact_bases(impact_type: i32) -> Option<(&'static str, &'static str)> {
    Some(match impact_type {
        1 => ("prj_bullet_impact_small", "prj_bullet_impact_small_exit"),
        2 => ("prj_bullet_impact_large", "prj_bullet_impact_large_exit"),
        3 => ("prj_bullet_impact_ap", "prj_bullet_impact_ap_exit"),
        4 => ("prj_bullet_impact_xtreme", "prj_bullet_impact_xtreme_exit"),
        5 => ("prj_bulletspray_impact_small", "prj_bulletspray_small_exit"),
        14 => ("prj_bolt_impact", "prj_bolt_impact_exit"),
        15 => ("prj_blade_impact", "prj_blade_impact"),
        _ => return None,
    })
}

/// A bullet striking: its impact sound, by the weapon's impact type and the
/// surface (another player's body: flesh, and the headshot on his head;
/// the local player: his own 2D hurt).
fn bullet_hit_sound(
    hit: On<net::EntityBulletHit>,
    local: Res<LocalPresentClient>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    bank: Option<Res<audio::SoundBank>>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    let (Some(weapons), Some(bank)) = (weapons, bank) else {
        return;
    };
    // Black Ops II's bank only.
    if bank.0.index_in(AssetNamespace::T6, "prj_bullet_impact_small_default").is_none() {
        return;
    }
    let p = hit.event.payload;
    let Some((base, exit_base)) = weapons.0.facts_of(p.weapon).and_then(|f| impact_bases(f.impact_type)) else {
        return;
    };
    let exit = p.surface_flags & fx_t6::runtime::FX_IMPACT_EXIT_SURFACE_FLAG != 0;
    let base = if exit { exit_base } else { base };
    let surf = usize::from(p.surf_type);
    let on_player = surf == SURF_FLESH && (0..64).contains(&p.other_entity_num);
    let snd_ent = u32::try_from(p.other_entity_num).ok().filter(|&n| n < 1022);
    if on_player && p.other_entity_num == local.0.0 as i32 {
        // His own: the 2D hurt (and a head hit's 2D crack).
        play_t6(&mut out, format!("{base}_player"), Some("prj_bullet_impact_small_player".to_owned()), None, None);
        if p.event_parm & 1 != 0 {
            play_t6(&mut out, "prj_bullet_impact_headshot_2d".to_owned(), None, None, None);
        }
        return;
    }
    let name = if on_player {
        "flesh"
    } else {
        T6_SURFACE_NAMES.get(surf).copied().unwrap_or("default")
    };
    play_t6(
        &mut out,
        format!("{base}_{name}"),
        Some(format!("{base}_default")),
        Some(p.origin),
        snd_ent,
    );
    if on_player && p.event_parm & 1 != 0 {
        play_t6(&mut out, "prj_bullet_impact_headshot".to_owned(), None, Some(p.origin), snd_ent);
    }
}

/// A bolt or blade's impact sound and its fallback, by the weapon's impact
/// type and the surface it struck (BO2's CG_ImpactEffectForWeapon names it
/// `<prefix>_<surface>`).
fn bolt_impact_alias(impact_type: i32, surf: usize) -> Option<(String, String)> {
    let (base, _) = impact_bases(impact_type)?;
    let name = T6_SURFACE_NAMES.get(surf).copied().unwrap_or("default");
    Some((format!("{base}_{name}"), format!("{base}_default")))
}

/// A crossbow bolt or a thrown blade striking: its impact sound where it
/// went in (BO2 plays it at that point, on no entity).
fn bolt_impact_sound(
    hit: On<net::EntityBoltImpact>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    bank: Option<Res<audio::SoundBank>>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    let (Some(weapons), Some(bank)) = (weapons, bank) else {
        return;
    };
    // Black Ops II's bank only.
    if bank.0.index_in(AssetNamespace::T6, "prj_bullet_impact_small_default").is_none() {
        return;
    }
    let p = hit.event.payload;
    let Some((alias, fallback)) = weapons
        .0
        .facts_of(p.weapon)
        .and_then(|f| bolt_impact_alias(f.impact_type, usize::from(p.surf_type)))
    else {
        return;
    };
    play_t6(&mut out, alias, Some(fallback), Some(p.origin), None);
}

pub fn register_t6_sound_systems(app: &mut App) {
    app.add_observer(bullet_hit_sound);
    app.add_observer(bolt_impact_sound);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(name: &str) -> u32 {
        let i = T6_SURFACE_NAMES.iter().position(|n| *n == name).unwrap();
        (i as u32) << 20
    }

    #[test]
    fn bolt_and_blade_sounds_are_named_by_the_surface() {
        let alias = bolt_impact_alias;
        assert_eq!(
            alias(14, 7),
            Some(("prj_bolt_impact_flesh".to_owned(), "prj_bolt_impact_default".to_owned()))
        );
        assert_eq!(alias(15, 31).unwrap().0, "prj_blade_impact_riotshield");
        assert_eq!(alias(14, 5).unwrap().0, "prj_bolt_impact_concrete");
        assert_eq!(alias(15, 40).unwrap().0, "prj_blade_impact_default");
        assert_eq!(alias(0, 5), None);
    }

    #[test]
    fn dive_surfaces_fold_to_their_class() {
        assert_eq!(dive_surface_name(flags("sand")), "dirt");
        assert_eq!(dive_surface_name(flags("water")), "mud");
        assert_eq!(dive_surface_name(flags("paintedmetal")), "metal");
        assert_eq!(dive_surface_name(flags("tallgrass")), "grass");
        assert_eq!(dive_surface_name(flags("riotshield")), "default");
        assert_eq!(dive_surface_name(flags("bark")), "wood");
        assert_eq!(dive_surface_name(flags("ice")), "snow");
        assert_eq!(dive_surface_name(flags("concrete")), "concrete");
        assert_eq!(dive_surface_name(0), "default");
    }
}
