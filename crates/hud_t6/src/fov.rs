//! BO2 view field of view: cg_fov dvars, the missile / forced-ADS / shoutcaster / default
//! shortcuts, the weapon and turret ADS zoom blends, and the half-angle tangents.
//! Facts: scratch/hud-facts/fov.md.

pub const STOCK_VIEW_CONE_DEGREES: f32 = 65.0;
pub const CG_FOV_RANGE: (f32, f32) = (1.0, 160.0);

pub const STOCK_VIEW_CONE_RATIO: f32 = 1.0;
pub const CG_FOV_SCALE_RANGE: (f32, f32) = (0.2, 2.0);

pub const STOCK_VIEW_CONE_FLOOR: f32 = 10.0;

pub const CG_FOV_EXTRA_CAM_DEFAULT: f32 = 30.0;

pub const CG_MISSILE_FOV_DEFAULT: f32 = 15.0;
pub const CG_MISSILE_FOV_RANGE: (f32, f32) = (0.0, 65.0);

/// Turret view fov before any ADS zoom.
pub const TURRET_VIEW_FOV: f32 = 55.0;

/// Forced-ADS view when the weapon is not raised or has no zoom fov.
pub const FORCED_ADS_FALLBACK_FOV: f32 = 45.0;

/// Landscape screens: tan(half fov Y) = tan(half fov) * 0.75.
pub const TAN_HALF_FOV_Y_SCALE: f32 = 0.75;

/// Alt-scope ADS transition frame step cap, seconds.
pub const ALT_SCOPE_DT_CAP_S: f32 = 0.1;

/// playerState linkFlags bit: view locked to the weapon's first ADS zoom fov.
pub const LINK_FLAG_FORCE_ADS_ZOOM: u32 = 4;
/// playerState eFlags bits: on a turret.
pub const EFLAGS_TURRET: u32 = 0x300;
/// playerState eFlags bit the shoutcaster model view checks.
pub const EFLAGS_SHOUTCASTER_DEFAULT: u32 = 0x4000;
/// playerState eFlags bits that keep the weapon-driven fov in third person.
pub const EFLAGS_NO_DEFAULT_FOV: u32 = 0x4300;
/// playerState eFlags2 bit that skips the turret and forced-ADS fovs.
pub const EFLAGS2_NO_TURRET_FOV: u32 = 0x1000_0000;
/// playerState otherFlags bit read by the default-fov test.
pub const OTHER_FLAGS_DEFAULT_FOV: u32 = 2;
/// Vehicle entity flag that makes its cameraFOV apply.
pub const VEHICLE_FLAG_CAMERA_FOV: u32 = 2;

const RADIANS_PER_ARC_DEGREE: f32 = core::f32::consts::PI / 180.0;
const DEGREES_PER_RADIAN: f32 = 180.0 / core::f32::consts::PI;

/// A weapon's three ADS zoom fovs and its zoom-in / zoom-out fractions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdsZoom {
    pub fov: [f32; 3],
    pub zoom_in_frac: f32,
    pub zoom_out_frac: f32,
}

/// How far the zoom has gone (0..1) at weapon position frac `pos_frac`. The zoom only runs over
/// the last `zoom_frac` of the raise: in-frac going into ADS, out-frac coming out.
#[must_use]
pub fn ads_zoom_progress(pos_frac: f32, to_ads: bool, zoom: &AdsZoom) -> f32 {
    if pos_frac >= 1.0 {
        return 1.0;
    }
    let zoom_frac = if to_ads {
        zoom.zoom_in_frac
    } else {
        zoom.zoom_out_frac
    };
    let into = pos_frac - (1.0 - zoom_frac);
    if into <= 0.0 || zoom_frac <= 0.0 {
        return 0.0;
    }
    let t = into / zoom_frac;
    if t > 0.0 { t } else { 0.0 }
}

/// cg_adsZoomToggleStyle: 0 cycles four zoom steps, anything else two.
#[must_use]
pub fn wrap_zoom_select(raw: u32, toggle_style: i32) -> u32 {
    raw % if toggle_style == 0 { 4 } else { 2 }
}

/// Zoom fov for a zoom select: 0 = third fov, 1 = second, 2 = first, others = the toggle default
/// (second fov for toggle style 0, else third).
#[must_use]
pub fn zoom_select_fov(zoom: &AdsZoom, select: u32, toggle_style: i32) -> f32 {
    match select {
        0 => zoom.fov[2],
        1 => zoom.fov[1],
        2 => zoom.fov[0],
        _ if toggle_style == 0 => zoom.fov[1],
        _ => zoom.fov[2],
    }
}

/// Blend between zoom selects over cg_cameraVzoomToggleTime seconds.
#[must_use]
pub fn vzoom_toggle_blend(prev_fov: f32, next_fov: f32, elapsed_ms: f32, toggle_time_s: f32) -> f32 {
    let span = toggle_time_s * 1000.0;
    let t = if span > 0.0 {
        (elapsed_ms / span).clamp(0.0, 1.0)
    } else {
        1.0
    };
    prev_fov + (next_fov - prev_fov) * t
}

/// Weapon view fov: `base` blended toward min(base, the selected zoom fov) by the ADS progress.
#[must_use]
pub fn weapon_view_fov(
    base: f32,
    zoom: &AdsZoom,
    select: u32,
    toggle_style: i32,
    pos_frac: f32,
    to_ads: bool,
) -> f32 {
    let target = base.min(zoom_select_fov(zoom, select, toggle_style));
    base - (base - target) * ads_zoom_progress(pos_frac, to_ads, zoom)
}

/// Turret view fov: 55, zooming toward min(55, first ADS fov) when the turret weapon has ADS.
#[must_use]
pub fn turret_view_fov(turret_ads: bool, zoom: &AdsZoom, pos_frac: f32, to_ads: bool) -> f32 {
    if !turret_ads {
        return TURRET_VIEW_FOV;
    }
    let target = TURRET_VIEW_FOV.min(zoom.fov[0]);
    TURRET_VIEW_FOV - (TURRET_VIEW_FOV - target) * ads_zoom_progress(pos_frac, to_ads, zoom)
}

/// Default-fov test: demo cameras always; otherwise only a third-person-for-model view with
/// otherFlags 2 set and none of the eFlags 0x4300 bits.
#[must_use]
pub fn should_default_view_fov(
    demo_camera: bool,
    entity_bits: u32,
    secondary_state_bits: u32,
    third_person_for_model: bool,
) -> bool {
    demo_camera
        || ((entity_bits & EFLAGS_NO_DEFAULT_FOV) == 0
            && (secondary_state_bits & OTHER_FLAGS_DEFAULT_FOV) != 0
            && third_person_for_model)
}

/// Vehicle camera inputs for the view fov.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleFov {
    pub camera_fov: f32,
    /// The player is using the vehicle's weapon (not "none").
    pub using_vehicle_weapon: bool,
}

/// Everything the view fov reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFovInputs {
    pub view_fov: f32,
    pub cg_fov_scale: f32,
    pub cg_fov_min: f32,
    pub cg_fov_default: f32,
    pub cg_missile_fov: f32,
    pub toggle_style: i32,
    pub missile_cam: bool,
    pub shoutcaster_model_view: bool,
    pub default_view: bool,
    pub attach_bits: u32,
    pub ent_bits: u32,
    pub e_flags2: u32,
    pub has_weapon: bool,
    pub zoom: AdsZoom,
    pub zoom_select: u32,
    pub pos_frac: f32,
    pub to_ads: bool,
    /// Some(turret ADS) while on a turret.
    pub turret: Option<(bool, AdsZoom)>,
    /// Some when the vehicle entity's camera fov applies.
    pub vehicle: Option<VehicleFov>,
}

impl Default for ViewFovInputs {
    fn default() -> Self {
        Self {
            view_fov: STOCK_VIEW_CONE_DEGREES,
            cg_fov_scale: STOCK_VIEW_CONE_RATIO,
            cg_fov_min: STOCK_VIEW_CONE_FLOOR,
            cg_fov_default: STOCK_VIEW_CONE_DEGREES,
            cg_missile_fov: CG_MISSILE_FOV_DEFAULT,
            toggle_style: 0,
            missile_cam: false,
            shoutcaster_model_view: false,
            default_view: false,
            attach_bits: 0,
            ent_bits: 0,
            e_flags2: 0,
            has_weapon: false,
            zoom: AdsZoom::default(),
            zoom_select: 0,
            pos_frac: 0.0,
            to_ads: false,
            turret: None,
            vehicle: None,
        }
    }
}

/// The horizontal view fov in degrees.
#[must_use]
pub fn view_fov(i: &ViewFovInputs) -> f32 {
    if i.missile_cam {
        return i.cg_missile_fov;
    }
    let skip_turret = (i.e_flags2 & EFLAGS2_NO_TURRET_FOV) != 0;
    if (i.attach_bits & LINK_FLAG_FORCE_ADS_ZOOM) != 0 && i.has_weapon && !skip_turret {
        let fov = i.zoom.fov[0];
        if i.pos_frac <= 0.0 || fov <= 0.0 {
            return FORCED_ADS_FALLBACK_FOV;
        }
        return fov;
    }
    if i.shoutcaster_model_view && (i.ent_bits & EFLAGS_SHOUTCASTER_DEFAULT) != 0 {
        return i.cg_fov_default;
    }
    if i.default_view {
        return i.view_fov;
    }
    let weapon = |base: f32| {
        weapon_view_fov(base, &i.zoom, i.zoom_select, i.toggle_style, i.pos_frac, i.to_ads)
    };
    let mut fov = weapon(i.view_fov);
    if (i.ent_bits & EFLAGS_TURRET) != 0
        && !skip_turret
        && let Some((ads, zoom)) = i.turret
    {
        fov = turret_view_fov(ads, &zoom, i.pos_frac, i.to_ads);
    }
    if let Some(v) = i.vehicle {
        if i.cg_fov_min < v.camera_fov {
            fov = v.camera_fov;
        }
        if v.using_vehicle_weapon {
            fov = weapon(fov);
        }
    }
    fov *= i.cg_fov_scale;
    if fov <= i.cg_fov_min {
        fov = i.cg_fov_min;
    }
    fov
}

/// (tan half fov X, tan half fov Y) for a horizontal-ish fov. Landscape (or unset) aspect scales Y by
/// 0.75 and X by 0.75 * aspect; portrait keeps X and divides Y by the aspect.
#[must_use]
pub fn fov_half_tan(fov_deg: f32, aspect: f32) -> (f32, f32) {
    let t = libm::tanf(fov_deg * RADIANS_PER_ARC_DEGREE * 0.5);
    if aspect >= 1.0 || aspect <= 0.0 {
        let y = t * TAN_HALF_FOV_Y_SCALE;
        (y * aspect, y)
    } else {
        (t, t / aspect)
    }
}

/// Vertical fov in degrees for a landscape view.
#[must_use]
pub fn vertical_fov_deg(fov_deg: f32) -> f32 {
    let vertical_cone_slope = libm::tanf(fov_deg * RADIANS_PER_ARC_DEGREE * 0.5) * TAN_HALF_FOV_Y_SCALE;
    2.0 * libm::atanf(vertical_cone_slope) * DEGREES_PER_RADIAN
}

/// Split screen with two local clients and no forced fullscreen halves tan half fov X.
#[must_use]
pub fn dual_play_tan_x(tan_x: f32, dual_play_enable: bool, local_clients: u32, force_fullscreen: bool) -> (f32, bool) {
    if dual_play_enable && local_clients == 2 && !force_fullscreen {
        (tan_x * 0.5, true)
    } else {
        (tan_x, false)
    }
}

/// Mouse zoom sensitivity: tan(fov / 2) over tan(cg_fov / 2).
#[must_use]
pub fn zoom_sensitivity(fov_deg: f32, cg_fov: f32) -> f32 {
    libm::tanf(fov_deg * RADIANS_PER_ARC_DEGREE * 0.5) / libm::tanf(cg_fov * RADIANS_PER_ARC_DEGREE * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoom() -> AdsZoom {
        AdsZoom { fov: [40.0, 30.0, 20.0], zoom_in_frac: 0.5, zoom_out_frac: 0.25 }
    }

    #[test]
    fn zoom_progress_runs_over_the_last_frac() {
        let z = zoom();
        assert_eq!(ads_zoom_progress(1.0, true, &z), 1.0);
        assert_eq!(ads_zoom_progress(0.5, true, &z), 0.0);
        assert!((ads_zoom_progress(0.75, true, &z) - 0.5).abs() < 1e-6);
        assert!((ads_zoom_progress(0.875, false, &z) - 0.5).abs() < 1e-6);
        assert_eq!(ads_zoom_progress(0.7, false, &z), 0.0);
    }

    #[test]
    fn zoom_select_maps_backwards_with_toggle_default() {
        let z = zoom();
        assert_eq!(zoom_select_fov(&z, 0, 0), 20.0);
        assert_eq!(zoom_select_fov(&z, 1, 0), 30.0);
        assert_eq!(zoom_select_fov(&z, 2, 0), 40.0);
        assert_eq!(zoom_select_fov(&z, 3, 0), 30.0);
        assert_eq!(zoom_select_fov(&z, 3, 1), 20.0);
        assert_eq!(wrap_zoom_select(5, 0), 1);
        assert_eq!(wrap_zoom_select(5, 1), 1);
        assert_eq!(wrap_zoom_select(6, 1), 0);
    }

    #[test]
    fn view_fov_shortcuts_skip_scale_and_min() {
        let base = ViewFovInputs { cg_fov_scale: 2.0, ..ViewFovInputs::default() };
        assert_eq!(view_fov(&ViewFovInputs { missile_cam: true, ..base }), 15.0);
        assert_eq!(view_fov(&ViewFovInputs { default_view: true, ..base }), 65.0);
        let forced = ViewFovInputs {
            attach_bits: LINK_FLAG_FORCE_ADS_ZOOM,
            has_weapon: true,
            zoom: zoom(),
            ..base
        };
        assert_eq!(view_fov(&forced), FORCED_ADS_FALLBACK_FOV);
        assert_eq!(view_fov(&ViewFovInputs { pos_frac: 0.5, ..forced }), 40.0);
        assert_eq!(view_fov(&ViewFovInputs { e_flags2: EFLAGS2_NO_TURRET_FOV, ..forced }), 130.0);
    }

    #[test]
    fn view_fov_full_ads_scales_and_clamps() {
        let ads = ViewFovInputs { zoom: zoom(), zoom_select: 2, pos_frac: 1.0, to_ads: true, ..ViewFovInputs::default() };
        assert_eq!(view_fov(&ads), 40.0);
        assert_eq!(view_fov(&ViewFovInputs { cg_fov_scale: 0.2, ..ads }), 10.0);
        let turret = ViewFovInputs { ent_bits: 0x100, turret: Some((false, zoom())), ..ads };
        assert_eq!(view_fov(&turret), 55.0);
        let turret_ads = ViewFovInputs { turret: Some((true, zoom())), ..turret };
        assert_eq!(view_fov(&turret_ads), 40.0);
    }

    #[test]
    fn vehicle_camera_fov_beats_min() {
        let v = ViewFovInputs {
            vehicle: Some(VehicleFov { camera_fov: 80.0, using_vehicle_weapon: false }),
            ..ViewFovInputs::default()
        };
        assert_eq!(view_fov(&v), 80.0);
        let low = ViewFovInputs { vehicle: Some(VehicleFov { camera_fov: 5.0, using_vehicle_weapon: false }), ..v };
        assert_eq!(view_fov(&low), 65.0);
    }

    #[test]
    fn default_test_needs_all_three() {
        assert!(should_default_view_fov(true, 0x4300, 0, false));
        assert!(should_default_view_fov(false, 0, 2, true));
        assert!(!should_default_view_fov(false, 0x100, 2, true));
        assert!(!should_default_view_fov(false, 0, 0, true));
        assert!(!should_default_view_fov(false, 0, 2, false));
    }

    #[test]
    fn tangents_landscape_and_portrait() {
        let (x, y) = fov_half_tan(90.0, 16.0 / 9.0);
        assert!((y - 0.75).abs() < 1e-5);
        assert!((x - 0.75 * 16.0 / 9.0).abs() < 1e-5);
        let (x, y) = fov_half_tan(90.0, 0.5);
        assert!((x - 1.0).abs() < 1e-5);
        assert!((y - 2.0).abs() < 1e-5);
        assert!((zoom_sensitivity(65.0, 65.0) - 1.0).abs() < 1e-6);
        assert_eq!(dual_play_tan_x(1.0, true, 2, false), (0.5, true));
        assert_eq!(dual_play_tan_x(1.0, true, 2, true), (1.0, false));
    }

    #[test]
    fn vzoom_blend_clamps() {
        assert_eq!(vzoom_toggle_blend(40.0, 20.0, 0.0, 0.2), 40.0);
        assert_eq!(vzoom_toggle_blend(40.0, 20.0, 100.0, 0.2), 30.0);
        assert_eq!(vzoom_toggle_blend(40.0, 20.0, 900.0, 0.2), 20.0);
    }
}
