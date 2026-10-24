//! The crosshair: four side pieces and a centre piece, sized and spread
//! from the gun in hand and the player's movement, and drawn as UI images.

use assets::PreparedWeapons;
use bevy::prelude::*;
use bevy::ui::Display;
use frame::{LifeStarted, ViewSubject};
use hud_t6::crosshair::{
    CG_CROSSHAIR_ALPHA_DEFAULT, CG_CROSSHAIR_ALPHA_MIN_DEFAULT, CrosshairFrame, ViewCrosshairGate,
    WeaponAdsCrosshair, WeaponCrosshairGate, WeaponReticle, allowed_to_draw_crosshair,
    calc_reticle_alpha, calc_reticle_spread, centered_square, plan_crosshair, reticle_center_size,
    reticle_sides_alpha_in, reticle_sides_layout, should_draw_crosshair,
};
use hud_t6::flashbang::{ShockViewType, cg_flashbanged};
use net::{FrameClock, LocalPresentClient, PresentedSnapshot, ViewweaponAim};
use playerstate_t6::game::{
    ENTITYNUM_NONE, KillCamMode, PM_TYPE_DEAD, PlayerState, ThirdPersonViewInputs, is_third_person_view,
    other_flags, weap_flags,
};

use crate::gaps::{GapCause, HudGap, HudPresentationGaps, ImageMiss, ReticleSlot};
use crate::images::{HUD_CHROME_NAMESPACE, HudImages};
use crate::presentation_scale::{PresentationScale, ScaleClass};
use crate::surface::Hud2dSurface;
use crate::ui_write::adopt_display;

/// A change in weapon position smaller than this is noise, not a direction.
const POS_FRAC_EPSILON: f32 = 1e-4;
/// Script dvar that hides the HUD for the game-over shot.
const HUD_HIDDEN_DVAR: &str = "bo2zm_hud_hidden";

/// The weapon shown in the player's hands. The offhand bit of the state is
/// the game's using-offhand flag, which the sim mirrors onto it.
///
/// The state has no melee weapon yet, so a swing keeps the held gun.
pub(crate) fn viewmodel_weapon_index(ps: &PlayerState) -> u32 {
    let inputs = weapon_t6::pm::ViewmodelWeaponInputs {
        using_offhand: ps.weap_flags & weap_flags::OFFHAND_VIEW != 0,
        weaponstate: ps.weapon_state_t6,
        weapon: ps.weapon,
        off_hand_weapon: u32::try_from(ps.off_hand_index).unwrap_or(0),
        melee_weapon: ps.weapon,
        weapon_is_melee: false,
    };
    weapon_t6::pm::viewmodel_weapon_of(&inputs)
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReticleQuad {
    /// One of the four side pieces: top, right, bottom, left.
    Side(u8),
    Center,
}

/// Which way the weapon is currently moving between hip and ADS.
#[derive(Resource, Default)]
pub(crate) struct ReticleAdsLatch {
    last_frac: f32,
    pub(crate) position_to_ads: bool,
}

impl ReticleAdsLatch {
    fn follow(&mut self, frac: f32) {
        if frac > self.last_frac + POS_FRAC_EPSILON {
            self.position_to_ads = true;
        } else if frac + POS_FRAC_EPSILON < self.last_frac {
            self.position_to_ads = false;
        }
        self.last_frac = frac;
    }
}

type QuadQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static ReticleQuad,
        &'static mut Node,
        &'static mut ImageNode,
        &'static mut UiTransform,
    ),
>;

pub(crate) fn spawn_reticle(root: &mut ChildSpawnerCommands) {
    let pieces = (0..4u8)
        .map(ReticleQuad::Side)
        .chain(std::iter::once(ReticleQuad::Center));
    for piece in pieces {
        let node = Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            ..default()
        };
        root.spawn((piece, node, UiTransform::IDENTITY, ImageNode::default()));
    }
}

fn hide_all(quads: &mut QuadQuery) {
    for (_, mut node, _, _) in quads.iter_mut() {
        adopt_display(&mut node, Display::None);
    }
}

/// What a reticle slot ended up with.
enum SlotImage {
    /// The weapon names no image for this slot.
    Unnamed,
    Drawn(Handle<Image>),
    Missing { name: String, miss: ImageMiss },
}

struct SlotResolution {
    slot: ReticleSlot,
    /// Whether the weapon authored this slot at all; an unauthored slot with
    /// no image is not a gap.
    authored: bool,
    image: SlotImage,
}

impl SlotResolution {
    fn handle(&self) -> Option<Handle<Image>> {
        match &self.image {
            SlotImage::Drawn(handle) => Some(handle.clone()),
            SlotImage::Unnamed | SlotImage::Missing { .. } => None,
        }
    }

    fn gap(&self) -> Option<GapCause> {
        if !self.authored {
            return None;
        }
        let slot = self.slot;
        match &self.image {
            SlotImage::Drawn(_) => None,
            SlotImage::Unnamed => Some(GapCause::ReticleSlotNamesNoImage { slot }),
            SlotImage::Missing { name, miss } => Some(GapCause::ReticleImageMissing {
                slot,
                name: name.clone(),
                miss: *miss,
            }),
        }
    }
}

/// Looks a slot's image up in the HUD image cache.
fn resolve_from_cache(
    slot: ReticleSlot,
    authored: bool,
    name: Option<&str>,
    namespace: asset_core::AssetNamespace,
    hud_images: &mut HudImages,
    images: &mut Assets<Image>,
) -> SlotResolution {
    let image = match name {
        None => SlotImage::Unnamed,
        Some(name) => match hud_images.get(namespace, name, images) {
            Some(handle) => SlotImage::Drawn(handle),
            None => SlotImage::Missing {
                name: name.to_owned(),
                miss: hud_images.miss_reason(),
            },
        },
    };
    SlotResolution {
        slot,
        authored,
        image,
    }
}

/// Looks a slot's material up among the Black Ops II HUD icons.
fn resolve_from_icons(
    slot: ReticleSlot,
    authored: bool,
    material: Option<&str>,
    uploaded: &[(String, Handle<Image>)],
) -> SlotResolution {
    let image = match material {
        None => SlotImage::Unnamed,
        Some(material) => match uploaded.iter().find(|(name, _)| name == material) {
            Some((_, handle)) => SlotImage::Drawn(handle.clone()),
            None => SlotImage::Missing {
                name: material.to_owned(),
                miss: ImageMiss::NotDecoded,
            },
        },
    };
    SlotResolution {
        slot,
        authored,
        image,
    }
}

/// The two ADS-dependent crosshair inputs taken from a gun's facts.
fn ads_crosshair_of(facts: &asset_game::WeaponBodyFacts) -> WeaponAdsCrosshair {
    WeaponAdsCrosshair {
        ads_aim_pitch: facts.ads_aim_pitch,
        ads_crosshair_in_frac: facts.ads_crosshair_in_frac,
        ads_crosshair_out_frac: facts.ads_crosshair_out_frac,
    }
}

/// The gun's spread table, whether it is a stacked-fire gun, and its clip.
/// A gun outside the Black Ops II table takes the catalog's hip spreads,
/// unscaled, and is never stacked.
fn gun_spread(
    t6_facts: Option<&sim::t6_viewmodel::T6GunFactsRes>,
    viewmodel: u32,
    facts: &asset_game::WeaponBodyFacts,
) -> (weapon_t6::pm::spread::SpreadFacts, bool, i32) {
    let known = t6_facts.filter(|table| !table.0.name(viewmodel).is_empty());
    if let Some(table) = known {
        let gun = table.0.get(viewmodel);
        let stacked = gun.core.fire_type == weapon_t6::facts::fire_type::STACKED;
        return (gun.spread, stacked, gun.core.clip_size);
    }
    let spread = weapon_t6::pm::spread::SpreadFacts {
        stand_min: facts.hip_spread_stand_min,
        ducked_min: facts.hip_spread_ducked_min,
        prone_min: facts.hip_spread_prone_min,
        stand_max: facts.hip_spread_stand_max,
        ducked_max: facts.hip_spread_ducked_max,
        prone_max: facts.hip_spread_prone_max,
        hip_min_scale: 1.0,
        hip_max_scale: 1.0,
        ..Default::default()
    };
    (spread, false, 0)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_reticle(
    surface: Res<Hud2dSurface>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    weapons: Option<Res<PreparedWeapons>>,
    cameras: Query<&Projection, With<Camera3d>>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudPresentationGaps>,
    mut ads_latch: ResMut<ReticleAdsLatch>,
    aim: Res<ViewweaponAim>,
    cg_clock: Res<FrameClock>,
    mut quads: QuadQuery,
    life: (MessageReader<LifeStarted>, Res<ViewSubject>),
    (t6_icons, mut t6_uploaded): (
        Option<Res<assets::T6HudIcons>>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
    t6_facts: Option<Res<sim::t6_viewmodel::T6GunFactsRes>>,
) {
    let (mut started, view) = life;
    let own_life_started = started.read().filter(|event| event.client == local.0.0).count() > 0;
    if own_life_started {
        *ads_latch = ReticleAdsLatch::default();
    }
    if !surface.is_ready() {
        return;
    }
    let Some(ps) = presented.player(local.0) else {
        return hide_all(&mut quads);
    };
    if ps.pm_type >= PM_TYPE_DEAD {
        return hide_all(&mut quads);
    }
    // The scripts hide the rest of the HUD at the game-over shot
    // (`hud_visible` 0 on the UI visibility flags); the crosshair goes too.
    let hud_hidden = presented.snapshot().is_some_and(|snapshot| {
        snapshot.meta.script_dvars(local.0).string(HUD_HIDDEN_DVAR) == Some("1")
    });
    if hud_hidden {
        return hide_all(&mut quads);
    }

    ads_latch.follow(ps.f_weapon_pos_frac);

    let viewmodel = viewmodel_weapon_index(ps);
    let Some(weapons) = weapons.as_ref() else {
        gaps.raise(GapCause::ReticleNoWeaponCatalog);
        return hide_all(&mut quads);
    };
    let catalog = &weapons.0;
    let Some(facts) = catalog.facts_of(viewmodel) else {
        gaps.raise(GapCause::ReticleWeaponNotInCatalog {
            viewmodel_index: viewmodel,
        });
        return hide_all(&mut quads);
    };
    gaps.clear(HudGap::ReticleWeaponDef);

    let in_killcam = view.in_killcam();
    let killcam_entity = in_killcam && ps.kill_cam_entity != ENTITYNUM_NONE;
    let third_person = killcam_entity
        || is_third_person_view(ThirdPersonViewInputs {
            pm_type: ps.pm_type,
            other_flags: ps.other_flags,
            link_flags: ps.link_flags,
            cg_third_person: false,
            in_killcam,
            killcam_mode: KillCamMode::Mode0,
        });
    // The game's two crosshair gates. Without a Black Ops II gun brain the
    // weapon state is -1, which neither gate hides on.
    let view_gate = should_draw_crosshair(&ViewCrosshairGate {
        link_flags: ps.link_flags,
        weaponstate: ps.weapon_state_t6,
        third_person_for_model: third_person,
        killcam_entity,
        pm_type: ps.pm_type,
        e_flags: ps.e_flags,
        ..ViewCrosshairGate::default()
    });
    // `last_weapon_hand` is 1 while a dual-wield gun is up.
    let weapon_gate = allowed_to_draw_crosshair(&WeaponCrosshairGate {
        weaponstate: ps.weapon_state_t6,
        weaponstate_left: (ps.last_weapon_hand == 1).then_some(ps.weapon_state_left_t6),
        ..WeaponCrosshairGate::default()
    });

    let Some(tan_half) = cameras.iter().find_map(|projection| match projection {
        Projection::Perspective(perspective) => Some((perspective.fov * 0.5).tan()),
        _ => None,
    }) else {
        return hide_all(&mut quads);
    };
    // Whether the crosshair should follow the gun is cg_crosshairDynamic's
    // default (an unconfirmed value); it tracks the gun on screen.
    let position = if aim.live {
        [aim.xhair_x, aim.xhair_y]
    } else {
        [0.0, 0.0]
    };
    let shock_kind = presented
        .shellshock(local.0)
        .map_or(ShockViewType::Blurred, |shock| shock.screen_blend.kind);
    let frame = CrosshairFrame {
        emp_jammed: ps.other_flags & other_flags::EMP_JAMMED != 0,
        view_gate,
        flashbanged: cg_flashbanged(
            cg_clock.time(),
            ps.shellshock_time,
            ps.shellshock_duration,
            shock_kind,
        ),
        flared: false,
        hud_shown: true,
        color_alpha: CG_CROSSHAIR_ALPHA_DEFAULT,
        pos_frac: ps.f_weapon_pos_frac,
        to_ads: ads_latch.position_to_ads,
        cg_draw_gun: true,
        keep_when_ads: false,
        weapon_gate,
        dynamic: true,
        hardcore: false,
        position,
    };
    let plan = plan_crosshair(&frame, &ads_crosshair_of(&facts), tan_half);
    let Some(plan) = plan.filter(|plan| plan.sides_and_center) else {
        return hide_all(&mut quads);
    };

    let Some(assets) = catalog.reticle_of(viewmodel) else {
        gaps.raise(GapCause::ReticleNoAuthoredMaterials {
            viewmodel_index: viewmodel,
        });
        return hide_all(&mut quads);
    };

    let ads_scale = plan.transition.scale;
    let side_size = facts.i_reticle_side_size as f32 * ads_scale;
    let (spread, stacked, clip_size) = gun_spread(t6_facts.as_deref(), viewmodel, &facts);
    // Perks as Black Ops II perk numbers (`weapon_t6::env::perk::by_name`),
    // so steady aim is bit 3.
    let perks = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .map_or(0, |meta| meta.t6_perks);
    let spread_input = weapon_t6::recoil::SpreadInput {
        view_height: ps.view_height_current,
        override_: weapon_t6::recoil::SpreadOverride {
            value: ps.spread_override,
            state: ps.spread_override_state,
        },
        stacked,
        clip_size,
        player_perks: perks,
        zombies: false,
        frac: ps.f_weapon_pos_frac.clamp(0.0, 1.0),
    };
    let (cone_min, cone_max) = weapon_t6::recoil::spread_for_weapon(
        &spread,
        &spread_input,
        &weapon_t6::recoil::RecoilDvars::default(),
    );
    let reticle = WeaponReticle {
        reticle_min_ofs: facts.i_reticle_min_ofs,
        hip_reticle_side_pos: facts.hip_reticle_side_pos,
        reticle_side_size: facts.i_reticle_side_size,
        reticle_center_size: assets.center_size,
    };
    let gap = calc_reticle_spread(
        cone_min,
        cone_max,
        ps.aim_spread_scale,
        ads_scale,
        tan_half,
        &reticle,
        [side_size, side_size],
    );
    // Not yet fed in: the fuse of a cooked grenade, which grows the centre.
    let center_size = reticle_center_size(reticle.reticle_center_size, false, 0, ads_scale);
    // The sides fade out while the scope overlay zooms in.
    let overlay = crate::iris::overlay_def(catalog.overlay_is_hud_iris(viewmodel), &facts);
    let zoom = crate::iris::weap_reticle_zoom(
        &overlay,
        &facts,
        ps.f_weapon_pos_frac,
        ads_latch.position_to_ads,
    );
    let alpha = calc_reticle_alpha(
        reticle_sides_alpha_in(zoom, facts.overlay_alpha_scale),
        CG_CROSSHAIR_ALPHA_DEFAULT,
        CG_CROSSHAIR_ALPHA_MIN_DEFAULT,
        ps.aim_spread_scale,
    );

    let namespace = catalog
        .namespace_of(viewmodel)
        .unwrap_or(HUD_CHROME_NAMESPACE);
    let (center, side) = if namespace == asset_core::AssetNamespace::T6 {
        // A Black Ops II gun's crosshair pictures are its materials' (the
        // four lines are `reticle_side_small`), taken from the zone's HUD
        // icons. They are uploaded once, the first time they are wanted.
        if t6_uploaded.is_empty()
            && let Some(icons) = t6_icons.as_deref()
        {
            let uploads = icons
                .0
                .iter()
                .map(|(name, image)| (name.clone(), images.add((**image).clone())));
            t6_uploaded.extend(uploads);
        }
        (
            resolve_from_icons(
                ReticleSlot::Center,
                assets.center_authored,
                assets.center_material.as_deref(),
                &t6_uploaded,
            ),
            resolve_from_icons(
                ReticleSlot::Side,
                assets.side_authored,
                assets.side_material.as_deref(),
                &t6_uploaded,
            ),
        )
    } else {
        (
            resolve_from_cache(
                ReticleSlot::Center,
                assets.center_authored,
                assets.center_image.as_deref(),
                namespace,
                &mut hud_images,
                &mut images,
            ),
            resolve_from_cache(
                ReticleSlot::Side,
                assets.side_authored,
                assets.side_image.as_deref(),
                namespace,
                &mut hud_images,
                &mut images,
            ),
        )
    };
    match center.gap().or_else(|| side.gap()) {
        Some(cause) => gaps.raise(cause),
        None => gaps.clear(HudGap::ReticleMaterial),
    }
    let (center_image, side_image) = (center.handle(), side.handle());

    // Placement: top-left corners in authored units from the screen centre,
    // sides in the order top, right, bottom, left.
    let window = PresentationScale::from_window(surface.width(), surface.height());
    let factor = window.factor(ScaleClass::ProjectionBound);
    let (middle_x, middle_y) = (window.width() * 0.5, window.height() * 0.5);
    let side_pics = reticle_sides_layout(plan.center, side_size, gap, false);
    let center_pic = centered_square(plan.center, center_size);
    for (piece, mut node, mut image_node, mut transform) in quads.iter_mut() {
        let (image, pic) = match *piece {
            ReticleQuad::Center => (center_image.clone(), center_pic),
            ReticleQuad::Side(index) => (side_image.clone(), side_pics[usize::from(index).min(3)]),
        };
        let Some(image) = image.filter(|_| pic.w > 0.0 && pic.h > 0.0) else {
            adopt_display(&mut node, Display::None);
            continue;
        };
        adopt_display(&mut node, Display::Flex);
        node.left = Val::Px(middle_x + pic.x * factor);
        node.top = Val::Px(middle_y + pic.y * factor);
        node.width = Val::Px(pic.w * factor);
        node.height = Val::Px(pic.h * factor);
        transform.rotation = Rot2::degrees(pic.angle);
        image_node.image = image;
        image_node.color = Color::srgba(1.0, 1.0, 1.0, alpha);
    }
}
