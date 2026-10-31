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
use net::HostViewClient;

use asset_core::AssetCategoryScope;

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

/// The surface index packed in surface flags: 6 bits starting at bit 20.
fn surface_slot_number(surface_flags: u32) -> usize {
    (surface_flags >> 20 & 0x3f) as usize
}

/// The name of a surface by index; an index past the list is "default".
fn name_of_surface(index: usize) -> &'static str {
    match T6_SURFACE_NAMES.get(index) {
        Some(name) => name,
        None => "default",
    }
}

/// A surface's name from surface flags (bits 20..25, masked 0x3f as BO2 does; 32 and up are "default").
pub(crate) fn surface_name(surface_flags: u32) -> &'static str {
    name_of_surface(surface_slot_number(surface_flags))
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
    let class = DTP_SURFACE_REMAP.get(surface_slot_number(surface_flags));
    let named = class.and_then(|&class| DTP_SURFACE_NAMES.get(usize::from(class)));
    named.map_or("default", |name| name)
}

/// A world sound by its BO2 name (with a fallback for a surface it lacks).
pub(crate) fn play_t6(
    out: &mut MessageWriter<audio::AliasCommand>,
    other_name: String,
    fallback: Option<String>,
    org: Option<[f32; 3]>,
    source_entity_no: Option<u32>,
) {
    out.write(audio::AliasCommand::Play(audio::TriggerCueMsg {
        cue_ns: AssetCategoryScope::T6,
        alias: other_name,
        fallback,
        position_in_inches: org,
        source_entity_no,
    }));
}

/// The impact sound's base name for a weapon's impact type (Black Ops'
/// list), and the base of its exit sound.

fn impact_bases(impact_kind_id: i32) -> Option<(&'static str, &'static str)> {
    // (impact type, sound base, exit sound base)
    const BASES: [(i32, &str, &str); 7] = [
        (1, "prj_bullet_impact_small", "prj_bullet_impact_small_exit"),
        (2, "prj_bullet_impact_large", "prj_bullet_impact_large_exit"),
        (3, "prj_bullet_impact_ap", "prj_bullet_impact_ap_exit"),
        (
            4,
            "prj_bullet_impact_xtreme",
            "prj_bullet_impact_xtreme_exit",
        ),
        (
            5,
            "prj_bulletspray_impact_small",
            "prj_bulletspray_small_exit",
        ),
        (14, "prj_bolt_impact", "prj_bolt_impact_exit"),
        (15, "prj_blade_impact", "prj_blade_impact"),
    ];
    let &(_, base, exit) = BASES.iter().find(|(kind, ..)| *kind == impact_kind_id)?;
    Some((base, exit))
}

/// True when the loaded sound bank carries Black Ops II's impact sounds.
fn is_t6_bank(bank: &audio::AudioCatalogRes) -> bool {
    bank.0
        .slot_in(AssetCategoryScope::T6, "prj_bullet_impact_small_default")
        .is_some()
}

/// A bullet striking: its impact sound, by the weapon's impact type and the
/// surface (another player's body: flesh, and the headshot on his head;
/// the local player: his own 2D hurt).
fn bullet_hit_sound(
    hit_in: On<net::BulletStrike>,
    local: Res<HostViewClient>,
    weapons: Option<Res<assets::StagedWeapons>>,
    bank: Option<Res<audio::AudioCatalogRes>>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    let p = hit_in.event.payload;
    let surf = usize::from(p.surf_type);
    let victim = p.other_entity_num;
    let on_player = surf == SURF_FLESH && (0..64).contains(&victim);
    let headshot = p.event_parm & 1 != 0;
    // BO2MP_SNDLOG: each head hit's crack beside the hit markers (the hidden hits check counts them),
    // ahead of the sound bank check: a muted run loads no bank.
    if on_player && headshot && std::env::var_os("BO2MP_SNDLOG").is_some() {
        diag::info!(World, "bo2mp sndlog impact prj_bullet_impact_headshot on {victim} heard by {}", local.0.0);
    }
    let (Some(weapons), Some(bank)) = (weapons, bank) else {
        return;
    };
    if !is_t6_bank(&bank) {
        return;
    }
    let Some(facts) = weapons.0.summary_for(p.weapon) else {
        return;
    };
    let Some((first_entry_offset, exit_base)) = impact_bases(facts.impact_kind_id) else {
        return;
    };
    let leaves = p.surface_flags & fx_t6::runtime::HIT_ON_WAY_OUT_BIT != 0;
    let base = [first_entry_offset, exit_base][usize::from(leaves)];
    let source_entity_no = u32::try_from(victim).ok().filter(|&n| n < 1022);
    if on_player && victim == local.0.0 as i32 {
        // His own: the 2D hurt (and a head hit's 2D crack).
        let fallback = Some(String::from("prj_bullet_impact_small_player"));
        play_t6(&mut out, format!("{base}_player"), fallback, None, None);
        if headshot {
            play_t6(
                &mut out,
                String::from("prj_bullet_impact_headshot_2d"),
                None,
                None,
                None,
            );
        }
        return;
    }
    let name = if on_player {
        "flesh"
    } else {
        name_of_surface(surf)
    };
    let at = Some(p.origin);
    play_t6(
        &mut out,
        format!("{base}_{name}"),
        Some(format!("{base}_default")),
        at,
        source_entity_no,
    );
    if on_player && headshot {
        play_t6(
            &mut out,
            String::from("prj_bullet_impact_headshot"),
            None,
            at,
            source_entity_no,
        );
    }
}

/// A bolt or blade's impact sound and its fallback, by the weapon's impact
/// type and the surface it struck (BO2's CG_ImpactEffectForWeapon names it
/// `<prefix>_<surface>`).
fn bolt_impact_alias(impact_kind_id: i32, surf: usize) -> Option<(String, String)> {
    let (base, _) = impact_bases(impact_kind_id)?;
    let named = format!("{base}_{}", name_of_surface(surf));
    Some((named, format!("{base}_default")))
}

/// A crossbow bolt or a thrown blade striking: its impact sound where it
/// went in (BO2 plays it at that point, on no entity).
fn bolt_impact_sound(
    hit: On<net::EntityBoltImpact>,
    weapons: Option<Res<assets::StagedWeapons>>,
    bank: Option<Res<audio::AudioCatalogRes>>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    let (Some(weapons), Some(bank)) = (weapons, bank) else {
        return;
    };
    if !is_t6_bank(&bank) {
        return;
    }
    let p = hit.event.payload;
    let Some(facts) = weapons.0.summary_for(p.weapon) else {
        return;
    };
    let Some((sound_alias, fallback)) = bolt_impact_alias(facts.impact_kind_id, usize::from(p.surf_type))
    else {
        return;
    };
    play_t6(&mut out, sound_alias, Some(fallback), Some(p.origin), None);
}

pub fn register_t6_sound_systems(app: &mut App) {
    app.add_observer(bullet_hit_sound);
    app.add_observer(bolt_impact_sound);
}

mod tests {
    #![cfg(test)]
    use super::*;

    fn flags(name: &str) -> u32 {
        let index = T6_SURFACE_NAMES
            .iter()
            .position(|&candidate| candidate == name)
            .expect("a surface in the list");
        u32::try_from(index).unwrap() << 20
    }

    #[test]
    fn bolt_and_blade_sounds_are_named_by_the_surface() {
        let pair = |(first, second): (String, String)| (first, second);
        let flesh = bolt_impact_alias(14, 7).map(pair);
        assert_eq!(
            flesh,
            Some((
                String::from("prj_bolt_impact_flesh"),
                String::from("prj_bolt_impact_default")
            ))
        );
        for (kind, surf, expected) in [
            (15, 31, "prj_blade_impact_riotshield"),
            (14, 5, "prj_bolt_impact_concrete"),
            (15, 40, "prj_blade_impact_default"),
        ] {
            assert_eq!(
                bolt_impact_alias(kind, surf)
                    .map(|names| names.0)
                    .as_deref(),
                Some(expected)
            );
        }
        assert_eq!(bolt_impact_alias(0, 5), None);
    }

    #[test]
    fn dive_surfaces_fold_to_their_class() {
        for (surf_name, class) in [
            ("sand", "dirt"),
            ("water", "mud"),
            ("paintedmetal", "metal"),
            ("tallgrass", "grass"),
            ("riotshield", "default"),
            ("bark", "wood"),
            ("ice", "snow"),
            ("concrete", "concrete"),
        ] {
            assert_eq!(dive_surface_name(flags(surf_name)), class, "{surf_name}");
        }
        assert_eq!(dive_surface_name(0), "default");
    }

    #[test]
    fn surface_names_past_the_list_are_default() {
        assert_eq!(surface_name(flags("flesh")), "flesh");
        assert_eq!(surface_name(0x3f << 20), "default");
        assert_eq!(impact_bases(99), None);
        assert_eq!(
            impact_bases(15),
            Some(("prj_blade_impact", "prj_blade_impact"))
        );
    }
}
