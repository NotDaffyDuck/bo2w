//! BO2 ADS overlay and scope reticle (scratch/hud-facts/iris.md): reticle zoom, overlay alpha and
//! the hip side-piece alpha, view-weapon visibility, overlay material choice (normal, low-res, the
//! global EMP scope material) and the four-quad or one-quad overlay layout.

/// Reticle zoom counts as active only above this.
pub const SCOPE_APERTURE_ENGAGE_FLOOR: f32 = 0.01;
/// Real viewport heights at or below this use the low-res overlay material (not for a shoutcaster).
pub const LOW_RES_VIEWPORT_MAX_HEIGHT: u32 = 480;
/// Overlays no bigger than this are drawn as four mirrored quarter quads.
pub const ADS_OVERLAY_FOUR_QUAD_MAX_WIDTH: f32 = 320.0;
pub const ADS_OVERLAY_FOUR_QUAD_MAX_HEIGHT: f32 = 240.0;
/// One-quad overlays at or above this alpha restrict the 3D viewport to the overlay rect.
pub const ADS_OVERLAY_SCISSOR_ALPHA_MIN: f32 = 0.99;
/// ps.otherFlags bit for the infrared overlay.
pub const OTHER_FLAGS_INFRARED_OVERLAY: u32 = 0x1000;

/// weapOverlayReticle_t.
pub const WEAP_OVERLAY_RETICLE_NONE: u32 = 0;
pub const WEAP_OVERLAY_RETICLE_CROSSHAIR: u32 = 1;
/// WeapOverlayInteface_t.
pub const WEAP_OVERLAY_INTERFACE_NONE: u32 = 0;
pub const WEAP_OVERLAY_INTERFACE_JAVELIN: u32 = 1;
pub const WEAP_OVERLAY_INTERFACE_TURRETSCOPE: u32 = 2;

/// CG_GetWeapReticleZoom: Some(zoom, capped at 1) when above 0.01. `window` is the ADS zoom-in
/// fraction (zoom-out while leaving ADS); BO2 does not guard a zero window.
#[must_use]
pub fn get_weap_reticle_zoom(gun_raise_share: f32, window: f32) -> Option<f32> {
    let zoom = (gun_raise_share - (1.0 - window)) / window;
    (zoom > SCOPE_APERTURE_ENGAGE_FLOOR).then(|| zoom.min(1.0))
}

/// The weapon fields the overlay path reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponOverlayDef {
    /// Material handles; 0 is none.
    pub hud_layer_surface: u32,
    pub vignette_surface_coarse: u32,
    pub hud_crosshair_layer: u32,
    pub overlay_interface: u32,
    pub overlay_width: f32,
    pub overlay_height: f32,
    pub overlay_alpha_scale: f32,
    pub infrared: bool,
    pub tv_guided: bool,
}

/// An overlay is configured when a material and a reticle are set, or the weapon is infrared or
/// TV guided.
#[must_use]
pub fn iris_overlay_configured(w: &WeaponOverlayDef) -> bool {
    let material = w.hud_layer_surface != 0 || w.vignette_surface_coarse != 0;
    (material && w.hud_crosshair_layer != WEAP_OVERLAY_RETICLE_NONE) || w.infrared || w.tv_guided
}

/// CG_DrawWeapReticle: overlay colour alpha.
#[must_use]
pub fn ads_overlay_alpha(zoom: f32, overlay_alpha_scale: f32) -> f32 {
    zoom / overlay_alpha_scale.max(1.0)
}

/// CG_DrawReticleSides: hip side pieces fade out as the overlay fades in.
#[must_use]
pub fn reticle_sides_alpha(zoom: f32, overlay_alpha_scale: f32) -> f32 {
    1.0 - ads_overlay_alpha(zoom, overlay_alpha_scale)
}

/// CG_AddViewWeapon: the gun is hidden while the reticle zoom is active, with cg_drawGun off, or in a
/// cubemap shot.
#[must_use]
pub fn view_weapon_draw_gun(reticle_zoom_active: bool, show_held_gun: bool, cubemap_shot: bool) -> bool {
    !reticle_zoom_active && show_held_gun && !cubemap_shot
}

/// The overlay counts as infrared with the otherFlags bit, cg_forceInfrared, or at full zoom on an
/// infrared weapon.
#[must_use]
pub fn overlay_is_infrared(secondary_state_bits: u32, cg_force_infrared: bool, full_zoom: bool, weapon_infrared: bool) -> bool {
    secondary_state_bits & OTHER_FLAGS_INFRARED_OVERLAY != 0 || cg_force_infrared || (full_zoom && weapon_infrared)
}

/// eFlags bits that turn the infrared view off.
pub const E_FLAGS_INFRARED_BLOCK: u32 = 0x300;
/// Full ADS: reticle zoom at or above this.
pub const INFRARED_FULL_ZOOM_MIN: f32 = 0.99;
/// cg_infraredBlur default and range.
pub const CG_INFRARED_BLUR_DEFAULT: f32 = 1.0;
pub const CG_INFRARED_BLUR_MIN: f32 = 1.0;
pub const CG_INFRARED_BLUR_MAX: f32 = 10.0;
/// cg_infraredBlurTime and cg_InfraredFadeoutTime defaults (ms).
pub const CG_INFRARED_BLUR_TIME_DEFAULT: i32 = 50;
pub const CG_INFRARED_FADEOUT_TIME_DEFAULT: i32 = 2000;
/// Entity draw flags for a hot body in the infrared view; the second is dropped with
/// cg_infraredUseDefaultMaterial.
pub const ENTITY_INFRARED_HEAT: u32 = 0x80;
pub const ENTITY_INFRARED_MATERIAL: u32 = 0x100;

/// What CG_IsInfrared reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InfraredState {
    pub demo_free_camera: bool,
    pub emp_jammed: bool,
    pub killcam_entity: bool,
    pub secondary_state_bits: u32,
    pub cg_force_infrared: bool,
    pub ent_bits: u32,
    pub third_person: bool,
    /// get_weap_reticle_zoom.
    pub reticle_zoom: Option<f32>,
    /// BG_IsInfraRed on the view weapon (variant or any attachment).
    pub weapon_infrared: bool,
}

/// CG_IsInfrared: never in the demo free camera, while EMP jammed or in a killcam; on with the
/// otherFlags bit or cg_forceInfrared; else at full ADS in first person on an infrared weapon.
#[must_use]
pub fn is_infrared(s: &InfraredState) -> bool {
    if s.demo_free_camera || s.emp_jammed || s.killcam_entity {
        return false;
    }
    if s.secondary_state_bits & OTHER_FLAGS_INFRARED_OVERLAY != 0 || s.cg_force_infrared {
        return true;
    }
    s.ent_bits & E_FLAGS_INFRARED_BLOCK == 0
        && !s.third_person
        && s.reticle_zoom.is_some_and(|z| z >= INFRARED_FULL_ZOOM_MIN)
        && s.weapon_infrared
}

/// The refdef fields the infrared view sets for the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InfraredRefdef {
    pub sonar_active: bool,
    pub sonar_blur: f32,
    pub draw_infrared: bool,
    pub sonar_override_blend: bool,
}

/// CG infrared refdef: all off unless infrared; cg_infraredBlur is clamped to its range.
#[must_use]
pub fn infrared_refdef(infrared: bool, cg_infrared_blur: f32) -> InfraredRefdef {
    if !infrared {
        return InfraredRefdef::default();
    }
    InfraredRefdef {
        sonar_active: true,
        sonar_blur: cg_infrared_blur.clamp(CG_INFRARED_BLUR_MIN, CG_INFRARED_BLUR_MAX),
        draw_infrared: true,
        sonar_override_blend: true,
    }
}

/// CG_EntityInfrared: draw flags for one entity (0 = normal). Friendlies glow only with
/// cg_infraredFriendlies >= 2.
#[must_use]
pub fn entity_infrared_flags(infrared: bool, friendly: bool, cg_infrared_friendlies: i32, use_default_material: bool) -> u32 {
    if !infrared || (friendly && cg_infrared_friendlies < 2) {
        return 0;
    }
    if use_default_material { ENTITY_INFRARED_HEAT } else { ENTITY_INFRARED_HEAT | ENTITY_INFRARED_MATERIAL }
}

/// The scope heartbeat effect plays with cg_infraredFriendlies >= 1.
#[must_use]
pub fn infrared_heartbeat_fx(infrared: bool, cg_infrared_friendlies: i32) -> bool {
    infrared && cg_infrared_friendlies >= 1
}

/// CG_GetWeaponOverlay: the global EMP scope material when jammed on an infrared overlay; else
/// low-res at small real viewports (not shoutcasting) when the weapon has one; else the normal one.
#[must_use]
pub fn ads_overlay_material(
    w: &WeaponOverlayDef,
    viewport_height: u32,
    shoutcaster: bool,
    emp_jammed: bool,
    infrared: bool,
    emp_scope_material: u32,
) -> u32 {
    if emp_jammed && infrared {
        emp_scope_material
    } else if viewport_height <= LOW_RES_VIEWPORT_MAX_HEIGHT && !shoutcaster && w.vignette_surface_coarse != 0 {
        w.vignette_surface_coarse
    } else {
        w.hud_layer_surface
    }
}

/// One overlay quad in virtual-screen units; s/t flip by swapping 0 and 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeVignetteRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub s0: f32,
    pub t0: f32,
    pub s1: f32,
    pub t1: f32,
}

/// CG_DrawAdsOverlay's layout around a centre point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdsOverlayLayout {
    /// Each quarter mirrored on x and/or y around the centre.
    Four([ScopeVignetteRect; 4]),
    /// One centred quad; also feeds the generic-overlay post-process rect and, at alpha > 0.99, the
    /// scissor viewport.
    One { quad: ScopeVignetteRect, scissor: bool },
}

impl AdsOverlayLayout {
    /// The rect the overlay covers; CG_DrawFrameOverlay fills black around it.
    #[must_use]
    pub fn rect(&self) -> (f32, f32, f32, f32) {
        match self {
            Self::Four(q) => (q[0].x, q[0].y, q[0].w * 2.0, q[0].h * 2.0),
            Self::One { quad, .. } => (quad.x, quad.y, quad.w, quad.h),
        }
    }
}

#[must_use]
pub fn ads_overlay_uses_four_quads(w: f32, h: f32) -> bool {
    w <= ADS_OVERLAY_FOUR_QUAD_MAX_WIDTH && h <= ADS_OVERLAY_FOUR_QUAD_MAX_HEIGHT
}

/// Layout from the weapon's overlay size (r_dualPlayActive doubles the width).
#[must_use]
pub fn draw_ads_overlay_layout(cx: f32, cy: f32, across: f32, tall: f32, dual_play: bool, alpha: f32) -> AdsOverlayLayout {
    let w = if dual_play { across * 2.0 } else { across };
    let h = tall;
    if ads_overlay_uses_four_quads(w, h) {
        let q = |x: f32, y: f32, s0: f32, s1: f32, t0: f32, t1: f32| ScopeVignetteRect { x, y, w, h, s0, t0, s1, t1 };
        AdsOverlayLayout::Four([
            q(cx - w, cy - h, 0.0, 1.0, 0.0, 1.0),
            q(cx, cy - h, 1.0, 0.0, 0.0, 1.0),
            q(cx - w, cy, 0.0, 1.0, 1.0, 0.0),
            q(cx, cy, 1.0, 0.0, 1.0, 0.0),
        ])
    } else {
        let quad = ScopeVignetteRect { x: cx - w * 0.5, y: cy - h * 0.5, w, h, s0: 0.0, t0: 0.0, s1: 1.0, t1: 1.0 };
        AdsOverlayLayout::One { quad, scissor: alpha > ADS_OVERLAY_SCISSOR_ALPHA_MIN }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_and_alpha() {
        assert_eq!(get_weap_reticle_zoom(1.0, 0.5), Some(1.0));
        assert_eq!(get_weap_reticle_zoom(0.75, 0.5), Some(0.5));
        assert_eq!(get_weap_reticle_zoom(0.5, 0.5), None);
        assert_eq!(ads_overlay_alpha(0.5, 0.0), 0.5);
        assert_eq!(ads_overlay_alpha(1.0, 2.0), 0.5);
        assert_eq!(reticle_sides_alpha(1.0, 4.0), 0.75);
        assert!(!view_weapon_draw_gun(true, true, false));
        assert!(view_weapon_draw_gun(false, true, false));
    }

    #[test]
    fn material_choice() {
        let w = WeaponOverlayDef { hud_layer_surface: 5, vignette_surface_coarse: 6, hud_crosshair_layer: 1, ..WeaponOverlayDef::default() };
        assert!(iris_overlay_configured(&w));
        assert!(!iris_overlay_configured(&WeaponOverlayDef { hud_layer_surface: 5, ..WeaponOverlayDef::default() }));
        assert_eq!(ads_overlay_material(&w, 480, false, false, false, 9), 6);
        assert_eq!(ads_overlay_material(&w, 480, true, false, false, 9), 5);
        assert_eq!(ads_overlay_material(&w, 1080, false, false, false, 9), 5);
        let ir = overlay_is_infrared(0, false, true, true);
        assert_eq!(ads_overlay_material(&w, 1080, false, true, ir, 9), 9);
        assert_eq!(ads_overlay_material(&w, 1080, false, true, false, 9), 5);
    }

    #[test]
    fn infrared_gate() {
        let ads = InfraredState { reticle_zoom: Some(1.0), weapon_infrared: true, ..InfraredState::default() };
        assert!(is_infrared(&ads));
        assert!(!is_infrared(&InfraredState { reticle_zoom: Some(0.9), ..ads }));
        assert!(!is_infrared(&InfraredState { reticle_zoom: None, ..ads }));
        assert!(!is_infrared(&InfraredState { weapon_infrared: false, ..ads }));
        assert!(!is_infrared(&InfraredState { third_person: true, ..ads }));
        assert!(!is_infrared(&InfraredState { ent_bits: 0x100, ..ads }));
        assert!(!is_infrared(&InfraredState { emp_jammed: true, ..ads }));
        assert!(!is_infrared(&InfraredState { killcam_entity: true, cg_force_infrared: true, ..ads }));
        assert!(is_infrared(&InfraredState { secondary_state_bits: OTHER_FLAGS_INFRARED_OVERLAY, ..InfraredState::default() }));
        assert!(is_infrared(&InfraredState { cg_force_infrared: true, third_person: true, ..InfraredState::default() }));

        assert_eq!(infrared_refdef(false, 5.0), InfraredRefdef::default());
        let r = infrared_refdef(true, 0.0);
        assert!(r.sonar_active && r.draw_infrared && r.sonar_override_blend);
        assert_eq!(r.sonar_blur, 1.0);
        assert_eq!(infrared_refdef(true, 20.0).sonar_blur, 10.0);

        assert_eq!(entity_infrared_flags(false, false, 0, false), 0);
        assert_eq!(entity_infrared_flags(true, false, 0, false), 0x180);
        assert_eq!(entity_infrared_flags(true, false, 0, true), 0x80);
        assert_eq!(entity_infrared_flags(true, true, 1, false), 0);
        assert_eq!(entity_infrared_flags(true, true, 2, false), 0x180);
        assert!(infrared_heartbeat_fx(true, 1));
        assert!(!infrared_heartbeat_fx(true, 0));
    }

    #[test]
    fn layout() {
        match draw_ads_overlay_layout(320.0, 240.0, 320.0, 240.0, false, 1.0) {
            l @ AdsOverlayLayout::Four(q) => {
                assert_eq!(l.rect(), (0.0, 0.0, 640.0, 480.0));
                assert_eq!((q[0].x, q[0].y), (0.0, 0.0));
                assert_eq!((q[3].s0, q[3].t0), (1.0, 1.0));
            }
            AdsOverlayLayout::One { .. } => panic!("four expected"),
        }
        match draw_ads_overlay_layout(320.0, 240.0, 200.0, 200.0, true, 1.0) {
            AdsOverlayLayout::One { quad, scissor } => {
                assert_eq!((quad.x, quad.w), (120.0, 400.0));
                assert!(scissor);
            }
            AdsOverlayLayout::Four(_) => panic!("one expected"),
        }
    }
}
