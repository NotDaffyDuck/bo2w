//! The crosshair: four side pieces and a centre piece, sized and spread
//! from the gun in hand and the player's movement, and drawn as UI images.

use assets::StagedWeapons;
use bevy::prelude::*;
use bevy::ui::Display;
use bevy::ui::PositionType;
use bevy::ui::UiTransform;
use frame::SpawnLifeBegun;
use frame::EyeSubject;
use hud_t6::reticle_plan::{
    STOCK_RETICLE_OPACITY, STOCK_RETICLE_OPACITY_FLOOR, CrosshairFrame, ReticlePic,
    ViewCrosshairGate, WeaponAdsCrosshair, WeaponCrosshairGate, WeaponReticle,
    allowed_to_draw_crosshair, calc_reticle_alpha, calc_reticle_spread, centered_square,
    plan_crosshair, reticle_center_size, reticle_sides_alpha_in, reticle_sides_layout,
    should_draw_crosshair,
};
use hud_t6::flashbang::{ShockViewType, cg_flashbanged};
use net::{FrameClock, HostViewClient, ShownWorldFrame, ViewweaponAim};
use playerstate_t6::game::{
    secondary_state_bits, weapon_state_bits,
};
use playerstate_t6::game::NO_ENTITY_SLOT;
use playerstate_t6::game::DeathReplayStyle;
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::AvatarState;
use playerstate_t6::game::ThirdPersonViewInputs;
use playerstate_t6::game::is_third_person_view;

use crate::gaps::GapReason;
use crate::gaps::HudShortage;
use crate::gaps::HudGapBook;
use crate::gaps::LookupFailure;
use crate::gaps::ReticlePart;
use crate::images::{CHROME_NAMESPACE, HudPictures};
use crate::screen_fit::WindowScale;
use crate::screen_fit::UnitClass;
use crate::surface::FlatOverlayCanvas;
use crate::ui_emit::take_over_monitor;

/// Weapon-position changes smaller than this are jitter and give no direction.
const POS_FRAC_JITTER: f32 = 1e-4;
/// The script dvar that blanks the HUD for the game-over shot.
const HUD_HIDDEN_DVAR: &str = "bo2zm_hud_hidden";

/// The gun the player is holding up. The sim copies the game's using-offhand
/// flag onto the offhand bit of the state.
///
/// The state carries no melee weapon yet, so a swing keeps the held gun.
pub(crate) fn viewmodel_weapon_index(pstate: &AvatarState) -> u32 {
    let weapon = pstate.weapon;
    let held = weapon_t6::pm::ViewmodelWeaponInputs {
        using_offhand: pstate.weapon_state_bits & weapon_state_bits::OFFHAND_VIEW != 0,
        gun_phase: pstate.weapon_state_t6,
        weapon,
        off_hand_weapon: u32::try_from(pstate.secondary_hand_slot).unwrap_or_default(),
        melee_weapon: weapon,
        weapon_is_melee: false,
    };
    weapon_t6::pm::viewmodel_weapon_of(&held)
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReticlePiece {
    /// One of the four side pieces: top, right, bottom, left.
    Arm(u8),
    Hub,
}

/// Remembers which way the weapon last travelled between hip and ADS.
#[derive(Resource, Default)]
pub(crate) struct CrosshairScopeHold {
    prev_frac: f32,
    pub(crate) place_for_scope: bool,
}

impl CrosshairScopeHold {
    fn follow(&mut self, frac: f32) {
        let delta = frac - self.prev_frac;
        if delta > POS_FRAC_JITTER {
            self.place_for_scope = true;
        } else if delta < -POS_FRAC_JITTER {
            self.place_for_scope = false;
        }
        self.prev_frac = frac;
    }
}

pub(crate) fn spawn_aim_mark(parent: &mut ChildSpawnerCommands) {
    let hidden = || Node {
        position_type: PositionType::Absolute,
        display: Display::None,
        ..default()
    };
    for side in 0..4u8 {
        parent.spawn((ReticlePiece::Arm(side), hidden(), UiTransform::IDENTITY, ImageNode::default()));
    }
    parent.spawn((ReticlePiece::Hub, hidden(), UiTransform::IDENTITY, ImageNode::default()));
}

fn show_when(node: &mut Node, shown: bool) {
    take_over_monitor(node, if shown { Display::Flex } else { Display::None });
}

fn hide_all(quads: &mut Query<(&ReticlePiece, &mut Node, &mut ImageNode, &mut UiTransform)>) {
    quads
        .iter_mut()
        .for_each(|(_, mut node, _, _)| take_over_monitor(&mut node, Display::None));
}

/// What a reticle slot ended up with.
enum SlotArt {
    /// The weapon names no image for this slot.
    NoName,
    Found(Handle<Image>),
    NothingThere { image_key: String, reason: LookupFailure },
}

struct SlotOutcome {
    which: ReticlePart,
    /// Whether the weapon authored this slot at all; an unauthored slot with
    /// no image is not a gap.
    declared: bool,
    art: SlotArt,
}

impl SlotOutcome {
    fn picture(&self) -> Option<Handle<Image>> {
        if let SlotArt::Found(found) = &self.art {
            Some(found.clone())
        } else {
            None
        }
    }

    fn shortfall(&self) -> Option<GapReason> {
        let which = self.which;
        match &self.art {
            _ if !self.declared => None,
            SlotArt::Found(_) => None,
            SlotArt::NoName => Some(GapReason::ReticlePartUnnamed { slot: which }),
            SlotArt::NothingThere { image_key, reason } => Some(GapReason::ReticleImageAbsent {
                slot: which,
                asset_key: image_key.clone(),
                miss: *reason,
            }),
        }
    }
}

/// Finds a slot's image in the HUD image cache.
fn cache_slot(
    which: ReticlePart,
    declared: bool,
    wanted: Option<&str>,
    namespace: asset_core::AssetCategoryScope,
    cache: (&mut HudPictures, &mut Assets<Image>),
) -> SlotOutcome {
    let (hud_images, image_store) = cache;
    let art = wanted.map_or(SlotArt::NoName, |name| {
        match hud_images.get(namespace, name, image_store) {
            Some(found) => SlotArt::Found(found),
            None => SlotArt::NothingThere {
                image_key: name.to_owned(),
                reason: hud_images.lookup_failure(),
            },
        }
    });
    SlotOutcome { which, declared, art }
}

/// Finds a slot's material among the Black Ops II HUD icons.
fn icon_slot(
    which: ReticlePart,
    declared: bool,
    wanted: Option<&str>,
    uploaded: &[(String, Handle<Image>)],
) -> SlotOutcome {
    let art = wanted.map_or(SlotArt::NoName, |material| {
        match uploaded.iter().find(|entry| entry.0 == material) {
            Some(entry) => SlotArt::Found(entry.1.clone()),
            None => SlotArt::NothingThere {
                image_key: String::from(material),
                reason: LookupFailure::Undecoded,
            },
        }
    });
    SlotOutcome { which, declared, art }
}

/// The two ADS-dependent crosshair inputs from a gun's facts.
fn ads_crosshair_of(facts: &asset_game::WeaponChassisSummary) -> WeaponAdsCrosshair {
    let asset_game::WeaponChassisSummary {
        aimed_pitch: ads_aim_pitch,
        scope_reticle_fade_in,
        scope_reticle_fade_out,
        ..
    } = *facts;
    WeaponAdsCrosshair {
        sight_tilt: ads_aim_pitch,
        scope_reticle_fade_in,
        scope_reticle_fade_out,
    }
}

/// Spread table of the catalog's hip numbers, for a gun the Black Ops II
/// table does not know: unscaled, never stacked, no clip.
fn catalog_hip_spread(facts: &asset_game::WeaponChassisSummary) -> weapon_t6::pm::spread::SpreadFacts {
    let asset_game::WeaponChassisSummary {
        hip_cone_up_lo: stand_lo,
        hip_cone_crouch_lo: ducked_lo,
        hip_cone_lie_lo: prone_lo,
        hip_cone_up_hi: stand_hi,
        hip_cone_crouch_hi: ducked_hi,
        hip_cone_lie_hi: prone_hi,
        ..
    } = *facts;
    weapon_t6::pm::spread::SpreadFacts {
        upright_narrowest: stand_lo,
        crouch_narrowest: ducked_lo,
        lying_narrowest: prone_lo,
        upright_widest: stand_hi,
        crouch_widest: ducked_hi,
        lying_widest: prone_hi,
        hip_min_scale: 1.0,
        hip_max_scale: 1.0,
        ..Default::default()
    }
}

/// The gun's spread table, whether it fires stacked, and its clip size.
fn gun_spread(
    t6_facts: Option<&sim::gun_view_facts::T6GunFactsRes>,
    weapon_idx: u32,
    facts: &asset_game::WeaponChassisSummary,
) -> (weapon_t6::pm::spread::SpreadFacts, bool, i32) {
    match t6_facts.map(|res| &res.0).filter(|table| !table.name(weapon_idx).is_empty()) {
        Some(table) => {
            let gun = table.get(weapon_idx);
            let stacked = gun.core.fire_type == weapon_t6::facts::fire_type::STACKED;
            (gun.spread, stacked, gun.core.magazine_capacity)
        }
        None => (catalog_hip_spread(facts), false, 0),
    }
}

/// The game's two crosshair gates, as (view gate, weapon gate). Without a
/// Black Ops II gun brain the weapon state is -1, which neither gate hides on.
fn crosshair_gates(pstate: &AvatarState, replaying_final_death: bool) -> (bool, bool) {
    let attach_bits = pstate.attach_bits;
    let killcam_entity = replaying_final_death && pstate.death_replay_target != NO_ENTITY_SLOT;
    let third_person = killcam_entity
        || is_third_person_view(ThirdPersonViewInputs {
            movement_mode: pstate.movement_mode,
            secondary_state_bits: pstate.secondary_state_bits,
            attach_bits,
            chase_cam_enabled: false,
            replaying_final_death,
            killcam_mode: DeathReplayStyle::Mode0,
        });
    let view = should_draw_crosshair(&ViewCrosshairGate {
        attach_bits,
        gun_state: pstate.weapon_state_t6,
        third_person_for_model: third_person,
        killcam_entity,
        movement_mode: pstate.movement_mode,
        ent_bits: pstate.e_flags,
        ..ViewCrosshairGate::default()
    });
    // `previous_gun_hand` is 1 while a dual-wield gun is up.
    let left_hand_state = (pstate.previous_gun_hand == 1).then_some(pstate.weapon_state_left_t6);
    let weapon = allowed_to_draw_crosshair(&WeaponCrosshairGate {
        gun_state: pstate.weapon_state_t6,
        weaponstate_left: left_hand_state,
        ..WeaponCrosshairGate::default()
    });
    (view, weapon)
}

/// Half the tangent of the first perspective camera's vertical field of view.
fn fov_half_tan(cameras: &Query<&Projection, With<Camera3d>>) -> Option<f32> {
    cameras.iter().find_map(|projection| {
        if let Projection::Perspective(lens) = projection {
            Some((lens.fov * 0.5).tan())
        } else {
            None
        }
    })
}

/// The local client's perks as Black Ops II perk numbers
/// (`weapon_t6::env::perk::by_name`), so steady aim is bit 3.
fn local_perks(presented: &ShownWorldFrame, client: sim::ClientId) -> u64 {
    let shown = presented.snapshot();
    shown
        .and_then(|snapshot| snapshot.meta.for_client(client))
        .map_or(0, |meta| meta.t6_perks)
}

/// The cone the gun's spread gives right now, as (min, max) degrees.
fn spread_cone(
    ps: &AvatarState,
    spread: &weapon_t6::pm::spread::SpreadFacts,
    stacked: bool,
    clip: i32,
    perks: u64,
) -> (f32, f32) {
    let input = weapon_t6::recoil::SpreadInput {
        view_height: ps.eye_height_now,
        override_: weapon_t6::recoil::SpreadOverride {
            value: ps.spread_forced_value,
            state: ps.spread_forced_phase,
        },
        stacked,
        magazine_capacity: clip,
        player_perks: perks,
        zombies: false,
        frac: ps.weapon_raise_fraction.clamp(0.0, 1.0),
    };
    weapon_t6::recoil::spread_for_weapon(spread, &input, &weapon_t6::recoil::RecoilDvars::default())
}

/// Uploads the zone's HUD icons the first time they are wanted.
fn upload_icons_once(
    uploaded: &mut Vec<(String, Handle<Image>)>,
    icons: Option<&assets::T6HudIcons>,
    image_store: &mut Assets<Image>,
) {
    let Some(icons) = icons.filter(|_| uploaded.is_empty()) else {
        return;
    };
    for (name, image) in &icons.0 {
        uploaded.push((name.clone(), image_store.add((**image).clone())));
    }
}

/// Everything the draw pass needs once the slots are settled.
struct ReticleDraw {
    sides: [ReticlePic; 4],
    center: ReticlePic,
    flank_texture: Option<Handle<Image>>,
    middle_texture: Option<Handle<Image>>,
    alpha: f32,
}

/// Puts each piece at its top-left corner, in authored units from the screen
/// centre, or hides it when it has no image or no area.
fn place_pieces(quads: &mut Query<(&ReticlePiece, &mut Node, &mut ImageNode, &mut UiTransform)>, surface: &FlatOverlayCanvas, draw: &ReticleDraw) {
    let tall = surface.height();
    let window = WindowScale::for_window(surface.width(), tall);
    let factor = window.unit_px(UnitClass::Projection);
    let origin = (window.width() * 0.5, window.height() * 0.5);
    for (piece, mut node, mut picture_node, mut spin) in &mut quads.iter_mut() {
        let (picture, pic) = match *piece {
            ReticlePiece::Hub => (&draw.middle_texture, draw.center),
            ReticlePiece::Arm(index) => (&draw.flank_texture, draw.sides[usize::from(index).min(3)]),
        };
        let visible = pic.w > 0.0 && pic.h > 0.0;
        let shown = picture.as_ref().filter(|_| visible);
        show_when(&mut node, shown.is_some());
        let Some(picture) = shown else { continue };
        node.left = Val::Px(origin.0 + pic.x * factor);
        node.top = Val::Px(origin.1 + pic.y * factor);
        node.width = Val::Px(pic.w * factor);
        node.height = Val::Px(pic.h * factor);
        spin.rotation = Rot2::degrees(pic.bend);
        picture_node.image = picture.clone();
        picture_node.color = Color::srgba(1.0_f32, 1.0, 1.0, draw.alpha);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn refresh_reticle(
    canvas: Res<FlatOverlayCanvas>,
    shown_frame: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    staged: Option<Res<StagedWeapons>>,
    lenses: Query<&Projection, With<Camera3d>>,
    mut hud_images: ResMut<HudPictures>,
    mut image_store: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudGapBook>,
    mut latch: ResMut<CrosshairScopeHold>,
    aim: Res<ViewweaponAim>,
    cg_clock: Res<FrameClock>,
    mut quads: Query<(&ReticlePiece, &mut Node, &mut ImageNode, &mut UiTransform)>,
    spawn_view: (
        MessageReader<SpawnLifeBegun>,
        Res<EyeSubject>,
        Option<Res<frame::LuiMenus>>,
    ),
    (t6_icons, mut t6_uploaded): (
        Option<Res<assets::T6HudIcons>>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
    t6_facts: Option<Res<sim::gun_view_facts::T6GunFactsRes>>,
) {
    let (mut births, view, lui_menus) = spawn_view;
    if births.read().any(|event| event.client == local.0.0) {
        *latch = CrosshairScopeHold::default();
    }
    if !canvas.has_size() {
        return;
    }
    let Some(pstate) = shown_frame.player(local.0) else {
        return hide_all(&mut quads);
    };
    // The scripts hide the rest of the HUD at the game-over shot
    // (`hud_visible` 0 on the UI visibility flags); the crosshair goes too.
    let shown = shown_frame.snapshot();
    let script_hidden = shown
        .is_some_and(|snap| snap.meta.script_dvars(local.0).string(HUD_HIDDEN_DVAR) == Some("1"));
    // bo2zm: Black Ops II's menus (pause, options) hide the crosshair.
    let lui_menu_open = lui_menus.is_some_and(|menus| menus.open);
    if pstate.movement_mode >= MOVE_KIND_DEAD || script_hidden || lui_menu_open {
        return hide_all(&mut quads);
    }

    latch.follow(pstate.weapon_raise_fraction);

    let held = viewmodel_weapon_index(pstate);
    let Some(staged) = staged.as_ref() else {
        gaps.log_gap(GapReason::ReticleNoCatalog);
        return hide_all(&mut quads);
    };
    let catalog = &staged.0;
    let Some(facts) = catalog.summary_for(held) else {
        gaps.log_gap(GapReason::ReticleGunMissing {
            viewmodel_index: held,
        });
        return hide_all(&mut quads);
    };
    gaps.settle(HudShortage::ReticleGunDef);

    let (view_gate, weapon_gate) = crosshair_gates(pstate, view.replaying_final_death());
    let Some(tan_half) = fov_half_tan(&lenses) else {
        return hide_all(&mut quads);
    };
    // Whether the crosshair follows the gun is cg_crosshairDynamic's default
    // (an unconfirmed value); it tracks the gun on screen.
    let position = if aim.live { [aim.crosshair_x, aim.crosshair_y] } else { [0.0; 2] };
    let shock_kind = shown_frame
        .shellshock(local.0)
        .map_or(ShockViewType::Blurred, |shock| shock.screen_blend.kind);
    let alpha_default = STOCK_RETICLE_OPACITY;
    let alpha_floor = STOCK_RETICLE_OPACITY_FLOOR;
    let frame = CrosshairFrame {
        emp_jammed: pstate.secondary_state_bits & secondary_state_bits::EMP_JAMMED != 0,
        view_gate,
        blinded: cg_flashbanged(cg_clock.time(), pstate.stun_start, pstate.stun_length, shock_kind),
        flared: false,
        hud_shown: true,
        tint_opacity: alpha_default,
        pos_frac: pstate.weapon_raise_fraction,
        to_ads: latch.place_for_scope,
        show_held_gun: true,
        keep_when_ads: false,
        weapon_gate,
        dynamic: true,
        hardcore: false,
        position,
    };
    let Some(plan) = plan_crosshair(&frame, &ads_crosshair_of(&facts), tan_half).filter(|p| p.sides_and_center)
    else {
        return hide_all(&mut quads);
    };

    let Some(authored) = catalog.crosshair_for(held) else {
        gaps.log_gap(GapReason::ReticleNoArt {
            viewmodel_index: held,
        });
        return hide_all(&mut quads);
    };

    let ads_scale = plan.transition.size;
    let flank_extent = facts.crosshair_arm_length as f32 * ads_scale;
    let (cone, stacked, clip) = gun_spread(t6_facts.as_deref(), held, &facts);
    let perks = local_perks(&shown_frame, local.0);
    let (cone_min, cone_max) = spread_cone(pstate, &cone, stacked, clip, perks);
    let reticle = WeaponReticle {
        reticle_min_ofs: facts.crosshair_inner_gap,
        hipfire_reticle_flank_offset: facts.hipfire_reticle_flank_offset,
        reticle_side_size: facts.crosshair_arm_length,
        reticle_center_size: authored.middle_extent,
    };
    let side_gap = calc_reticle_spread(
        cone_min,
        cone_max,
        pstate.sight_spread_factor,
        ads_scale,
        tan_half,
        &reticle,
        [flank_extent; 2],
    );
    // Not yet fed in: the fuse of a cooked grenade, which grows the centre.
    let middle_extent = reticle_center_size(reticle.reticle_center_size, false, 0, ads_scale);
    // The sides fade out while the scope overlay zooms in.
    let overlay = crate::iris::scope_def(catalog.scope_is_iris(held), &facts);
    let zoom =
        crate::iris::scope_zoom(&overlay, &facts, pstate.weapon_raise_fraction, latch.place_for_scope);
    let alpha = calc_reticle_alpha(
        reticle_sides_alpha_in(zoom, facts.overlay_alpha_scale),
        alpha_default,
        alpha_floor,
        pstate.sight_spread_factor,
    );

    let namespace = catalog.scope_for(held).unwrap_or(CHROME_NAMESPACE);
    let (center, side) = if namespace == asset_core::AssetCategoryScope::T6 {
        // A Black Ops II gun's crosshair pictures are its materials' (the
        // four lines are `reticle_side_small`), taken from the zone's HUD
        // icons.
        upload_icons_once(&mut t6_uploaded, t6_icons.as_deref(), &mut image_store);
        let from_icons = |which, declared, name: &Option<String>| {
            icon_slot(which, declared, name.as_deref(), &t6_uploaded)
        };
        (
            from_icons(ReticlePart::Center, authored.center_authored, &authored.middle_surface),
            from_icons(ReticlePart::Side, authored.side_authored, &authored.flank_surface),
        )
    } else {
        let mut from_cache = |which, declared, name: &Option<String>| {
            cache_slot(which, declared, name.as_deref(), namespace, (&mut hud_images, &mut image_store))
        };
        (
            from_cache(ReticlePart::Center, authored.center_authored, &authored.middle_texture),
            from_cache(ReticlePart::Side, authored.side_authored, &authored.flank_texture),
        )
    };
    if let Some(why) = center.shortfall().or_else(|| side.shortfall()) {
        gaps.log_gap(why);
    } else {
        gaps.settle(HudShortage::ReticleArt);
    }

    // Sides run top, right, bottom, left.
    let draw = ReticleDraw {
        sides: reticle_sides_layout(plan.centre, flank_extent, side_gap, false),
        center: centered_square(plan.centre, middle_extent),
        flank_texture: side.picture(),
        middle_texture: center.picture(),
        alpha,
    };
    place_pieces(&mut quads, &canvas, &draw);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(declared: bool, art: SlotArt) -> SlotOutcome {
        SlotOutcome {
            which: ReticlePart::Side,
            declared,
            art,
        }
    }

    #[test]
    fn latch_follows_only_real_movement() {
        let mut latch = CrosshairScopeHold::default();
        latch.follow(0.5);
        assert!(latch.place_for_scope);
        latch.follow(0.5 + POS_FRAC_JITTER * 0.5);
        assert!(latch.place_for_scope, "jitter keeps the direction");
        latch.follow(0.2);
        assert!(!latch.place_for_scope);
        latch.follow(0.2);
        assert!(!latch.place_for_scope, "standing still keeps the direction");
    }

    #[test]
    fn unauthored_slot_is_never_a_gap() {
        assert!(outcome(false, SlotArt::NoName).shortfall().is_none());
        assert!(outcome(true, SlotArt::Found(Handle::default())).shortfall().is_none());
        assert!(matches!(
            outcome(true, SlotArt::NoName).shortfall(),
            Some(GapReason::ReticlePartUnnamed { .. })
        ));
    }

    #[test]
    fn missing_image_reports_its_name() {
        let missing = outcome(
            true,
            SlotArt::NothingThere {
                image_key: "reticle_dot".into(),
                reason: LookupFailure::Undecoded,
            },
        );
        assert!(missing.picture().is_none());
        match missing.shortfall() {
            Some(GapReason::ReticleImageAbsent { asset_key, .. }) => assert_eq!(asset_key, "reticle_dot"),
            other => panic!("expected a missing-image gap, got {}", other.is_some()),
        }
    }

    #[test]
    fn icon_slot_matches_by_material_name() {
        let uploaded = vec![("reticle_a".to_owned(), Handle::<Image>::default())];
        let hit = icon_slot(ReticlePart::Center, true, Some("reticle_a"), &uploaded);
        assert!(hit.picture().is_some());
        let miss = icon_slot(ReticlePart::Center, true, Some("reticle_b"), &uploaded);
        assert!(miss.picture().is_none() && miss.shortfall().is_some());
        let none = icon_slot(ReticlePart::Center, true, None, &uploaded);
        assert!(matches!(none.art, SlotArt::NoName));
    }

    #[test]
    fn unknown_gun_takes_catalog_hip_spread() {
        let facts = asset_game::WeaponChassisSummary {
            hip_cone_up_lo: 2.0,
            hip_cone_up_hi: 6.0,
            ..Default::default()
        };
        let (cone, stacked, clip) = gun_spread(None, 7, &facts);
        assert_eq!((cone.upright_narrowest, cone.upright_widest), (2.0, 6.0));
        assert_eq!((cone.hip_min_scale, cone.hip_max_scale), (1.0, 1.0));
        assert!(!stacked);
        assert_eq!(clip, 0);
    }

    #[test]
    fn plain_state_draws_both_gates() {
        let ps = AvatarState::ZERO;
        assert_eq!(crosshair_gates(&ps, false), (true, true));
    }
}
