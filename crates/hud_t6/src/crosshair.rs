//! BO2 hip crosshair: the view and weapon draw gates, the ADS shrink and shift, the reticle side
//! and centre layout, the crosshair and reticle colours, and the vehicle centre reticle.
//! Facts: scratch/hud-facts/crosshair.md.

/// Virtual screen half sizes the reticle maths uses (640x480).
pub const RETICLE_VIRTUAL_HALF_WIDTH: f32 = 320.0;
pub const RETICLE_VIRTUAL_HALF_HEIGHT: f32 = 240.0;

/// aimSpreadScale runs 0..255.
pub const AIM_SPREAD_SCALE_MAX: f32 = 255.0;

pub const CG_CROSSHAIR_ALPHA_DEFAULT: f32 = 1.0;
pub const CG_CROSSHAIR_ALPHA_MIN_DEFAULT: f32 = 0.5;

/// Below this alpha the crosshair, the sides and the vehicle reticle are skipped.
pub const CROSSHAIR_ALPHA_DRAW_MIN: f32 = 0.01;

/// ADS shrink: the reticle loses up to half its size on the way in.
pub const ADS_CROSSHAIR_SHRINK: f32 = 0.5;

/// Side pics sit an eighth of their size further out than the spread.
pub const RETICLE_SIDE_GAP_FRAC: f32 = 0.125;

/// Cooked grenade centre grows by (time left % 1000) * 0.01.
pub const COOK_CENTER_GROWTH_PER_MS: f32 = 0.01;

/// ADS aim indicator size factor is (1.5 - posFrac).
pub const ADS_AIM_INDICATOR_BASE: f32 = 1.5;

/// BO2 weaponstate_t numbers (facts/weaponstate-t6.md:3). Each is a copy of
/// its weapon_t6::state twin, so this crate stays libm-only; sim's
/// crosshair_weaponstate_twins test holds them equal, and a fix goes here.
pub const WEAPONSTATE_RAISING: i32 = 1; // weapon_t6::state::RAISING
pub const WEAPONSTATE_DROPPING_ALTSWITCH: i32 = 5; // weapon_t6::state::DROPPING_ALTSWITCH
pub const WEAPONSTATE_RELOADING: i32 = 11; // weapon_t6::state::RELOADING
pub const WEAPONSTATE_RELOAD_START: i32 = 13; // weapon_t6::state::RELOAD_START
pub const WEAPONSTATE_MELEE_INIT: i32 = 19; // weapon_t6::state::MELEE_INIT
pub const WEAPONSTATE_MELEE_FIRE: i32 = 20; // weapon_t6::state::MELEE_FIRE
pub const WEAPONSTATE_MELEE_END: i32 = 21; // weapon_t6::state::MELEE_END
pub const WEAPONSTATE_LOWREADY_RAISE: i32 = 56; // weapon_t6::state::LOWREADY_RAISE
pub const WEAPONSTATE_LOWREADY_DROP: i32 = 58; // weapon_t6::state::LOWREADY_DROP

/// playerState linkFlags bit: view forced to the weapon's ADS zoom (skips the view gate).
pub const LINK_FLAG_FORCE_ADS_ZOOM: u32 = 4;
/// playerState eFlags bits: on a turret (no hip crosshair).
pub const EFLAGS_TURRET: u32 = 0x300;
/// playerState eFlags bit: in a vehicle seat that owns the view.
pub const EFLAGS_VEHICLE_VIEW: u32 = 0x4000;
/// playerState eFlags2 bit returned for pm_type > 9.
pub const EFLAGS2_CROSSHAIR_IN_SPECIAL_PM: u32 = 0x1000_0000;
/// playerState eFlags2 bit: view locked to an entity whose weapon has an overlay.
pub const EFLAGS2_VIEWLOCKED_OVERLAY: u32 = 0x4_0000;
/// pm_type values above this read the eFlags2 bit instead.
pub const PM_TYPE_SPECIAL_ABOVE: i32 = 9;
/// Vehicle type that keeps the crosshair with eFlags 0x4000 above pm_type 9.
pub const VEHICLE_TYPE_KEEPS_CROSSHAIR: i32 = 6;
/// Water level from which the weapon reticle hides unless the weapon allows swimming.
pub const WATERLEVEL_UNDER: i32 = 4;

/// playerState weapFlags bits the crosshair colour reads.
pub const WEAP_FLAGS_ON_FRIENDLY: u64 = 8;
pub const WEAP_FLAGS_ON_ENEMY: u64 = 0x10;
pub const WEAP_FLAGS_ZM_CYAN: u64 = 0x4000_0000;

const DEG2RAD: f32 = core::f32::consts::PI / 180.0;

/// The view-level crosshair gate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewCrosshairGate {
    pub missile_cam: bool,
    pub link_flags: u32,
    pub demo_free_camera: bool,
    pub weaponstate: i32,
    pub third_person_for_model: bool,
    pub killcam_entity: bool,
    pub pm_type: i32,
    pub e_flags: u32,
    pub e_flags2: u32,
    pub vehicle_type: i32,
}

#[must_use]
pub fn should_draw_crosshair(g: &ViewCrosshairGate) -> bool {
    if g.missile_cam {
        return true;
    }
    if g.link_flags & LINK_FLAG_FORCE_ADS_ZOOM != 0 {
        return true;
    }
    if g.demo_free_camera {
        return false;
    }
    if (WEAPONSTATE_LOWREADY_RAISE..=WEAPONSTATE_LOWREADY_DROP).contains(&g.weaponstate) {
        return false;
    }
    let vehicle_view = g.e_flags & EFLAGS_VEHICLE_VIEW != 0;
    if g.third_person_for_model && (!vehicle_view || g.killcam_entity) {
        return false;
    }
    if g.pm_type > PM_TYPE_SPECIAL_ABOVE && (!vehicle_view || g.vehicle_type != VEHICLE_TYPE_KEEPS_CROSSHAIR) {
        return g.e_flags2 & EFLAGS2_CROSSHAIR_IN_SPECIAL_PM != 0;
    }
    true
}

/// The weapon-level crosshair gate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponCrosshairGate {
    pub splitscreen: bool,
    pub paused: bool,
    pub draw_paused: bool,
    pub hardcore: bool,
    pub cg_draw_crosshair: bool,
    pub ui_allows: bool,
    pub demo_hud_hidden: bool,
    /// The playerState HUD-block nibble; any bit set hides the crosshair.
    pub hud_block_nibble: u8,
    pub weaponstate: i32,
    /// The left hand's weaponstate when dual wielding.
    pub weaponstate_left: Option<i32>,
    pub used_on_vehicle: bool,
}

impl Default for WeaponCrosshairGate {
    fn default() -> Self {
        Self {
            splitscreen: false,
            paused: false,
            draw_paused: false,
            hardcore: false,
            cg_draw_crosshair: true,
            ui_allows: true,
            demo_hud_hidden: false,
            hud_block_nibble: 0,
            weaponstate: 0,
            weaponstate_left: None,
            used_on_vehicle: false,
        }
    }
}

#[must_use]
pub fn allowed_to_draw_crosshair(g: &WeaponCrosshairGate) -> bool {
    if !g.splitscreen && g.paused && g.draw_paused {
        return false;
    }
    if g.hardcore || !g.cg_draw_crosshair || !g.ui_allows || g.demo_hud_hidden {
        return false;
    }
    if g.hud_block_nibble & 0xf != 0 {
        return false;
    }
    if matches!(
        g.weaponstate,
        WEAPONSTATE_MELEE_INIT | WEAPONSTATE_MELEE_FIRE | WEAPONSTATE_MELEE_END
    ) {
        return false;
    }
    match g.weaponstate_left {
        Some(left) => {
            if g.weaponstate == WEAPONSTATE_RELOADING && left == WEAPONSTATE_RELOADING {
                return false;
            }
        }
        None => {
            if g.weaponstate == WEAPONSTATE_RELOADING {
                return false;
            }
            if g.used_on_vehicle && g.weaponstate == WEAPONSTATE_RELOAD_START {
                return false;
            }
        }
    }
    !(WEAPONSTATE_RAISING..=WEAPONSTATE_DROPPING_ALTSWITCH).contains(&g.weaponstate)
}

/// A weapon's ADS crosshair numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponAdsCrosshair {
    pub ads_aim_pitch: f32,
    pub ads_crosshair_in_frac: f32,
    pub ads_crosshair_out_frac: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdsTransition {
    pub scale: f32,
    pub shift: f32,
}

impl Default for AdsTransition {
    fn default() -> Self {
        Self { scale: 1.0, shift: 0.0 }
    }
}

/// Shrink and drop the hip reticle over the last in/out frac of the ADS raise.
#[must_use]
pub fn transition_to_ads(
    pos_frac: f32,
    to_ads: bool,
    weap: &WeaponAdsCrosshair,
    tan_half_fov_y: f32,
) -> AdsTransition {
    let window = if to_ads {
        weap.ads_crosshair_in_frac
    } else {
        weap.ads_crosshair_out_frac
    };
    let fa = pos_frac - (1.0 - window);
    if fa <= 0.0 || window == 0.0 || tan_half_fov_y == 0.0 {
        return AdsTransition::default();
    }
    let fa = fa / window;
    AdsTransition {
        scale: 1.0 - fa * ADS_CROSSHAIR_SHRINK,
        shift: libm::tanf(weap.ads_aim_pitch * DEG2RAD) * (fa * RETICLE_VIRTUAL_HALF_HEIGHT / tan_half_fov_y),
    }
}

fn forward_from_angles(pitch_deg: f32, yaw_deg: f32) -> [f32; 3] {
    let (sp, cp) = libm::sincosf(pitch_deg * DEG2RAD);
    let (sy, cy) = libm::sincosf(yaw_deg * DEG2RAD);
    [cp * cy, cp * sy, -sp]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Where the gun points on the virtual screen, relative to the centre. Roll does not move the
/// forward vector, so only the gun pitch and yaw matter.
#[must_use]
pub fn calc_crosshair_position(
    gun_pitch: f32,
    gun_yaw: f32,
    view_axis: [[f32; 3]; 3],
    tan_half_fov_x: f32,
    tan_half_fov_y: f32,
) -> [f32; 2] {
    let dir = forward_from_angles(gun_pitch, gun_yaw);
    let d = dot(view_axis[0], dir);
    if d > 0.0 && tan_half_fov_x > 0.0 {
        [
            dot(view_axis[1], dir) / (tan_half_fov_x * d) * -RETICLE_VIRTUAL_HALF_WIDTH,
            dot(view_axis[2], dir) / (tan_half_fov_y * d) * -RETICLE_VIRTUAL_HALF_HEIGHT,
        ]
    } else {
        [0.0, 0.0]
    }
}

/// Team colours the crosshair colour can pick (already unpacked from their dvars).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosshairTeamColors {
    pub my_team: [f32; 3],
    pub my_team_alt: [f32; 3],
    pub enemy_team: [f32; 3],
    pub enemy_team_alt: [f32; 3],
    pub party: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosshairColorInputs {
    pub alpha_in: f32,
    pub cg_crosshair_alpha: f32,
    pub weapon_color_change: bool,
    pub e_flags: u32,
    pub weap_flags: u64,
    /// Milliseconds since the crosshair last touched a player.
    pub since_hit_ms: i32,
    pub friendly_fade_out_ms: i32,
    pub enemy_fade_out_ms: i32,
    pub zombies: bool,
    pub target_in_party: bool,
    pub alt_colors: bool,
    pub server_red_crosshair: bool,
    pub cg_crosshair_enemy_color: bool,
}

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

#[must_use]
pub fn crosshair_color(i: &CrosshairColorInputs, c: &CrosshairTeamColors) -> [f32; 4] {
    let rgb = if !i.weapon_color_change && i.e_flags & EFLAGS_VEHICLE_VIEW == 0 {
        WHITE
    } else if i.weap_flags & WEAP_FLAGS_ON_FRIENDLY != 0 {
        if i.since_hit_ms < i.friendly_fade_out_ms {
            if i.zombies && i.weap_flags & WEAP_FLAGS_ZM_CYAN != 0 {
                [0.0, 1.0, 1.0]
            } else if i.target_in_party {
                c.party
            } else if i.alt_colors {
                c.my_team_alt
            } else {
                c.my_team
            }
        } else {
            WHITE
        }
    } else if i.weap_flags & WEAP_FLAGS_ON_ENEMY != 0
        && i.server_red_crosshair
        && i.cg_crosshair_enemy_color
        && i.since_hit_ms < i.enemy_fade_out_ms
    {
        if i.alt_colors { c.enemy_team_alt } else { c.enemy_team }
    } else {
        WHITE
    };
    [rgb[0], rgb[1], rgb[2], i.cg_crosshair_alpha * i.alpha_in]
}

/// Reticle alpha: fades with aim spread, never under cg_crosshairAlphaMin.
#[must_use]
pub fn calc_reticle_alpha(alpha_in: f32, cg_crosshair_alpha: f32, cg_crosshair_alpha_min: f32, aim_spread_scale: f32) -> f32 {
    let a = (1.0 - aim_spread_scale / AIM_SPREAD_SCALE_MAX) * cg_crosshair_alpha * alpha_in;
    if a < cg_crosshair_alpha_min { cg_crosshair_alpha_min } else { a }
}

/// Incoming alpha for the reticle sides: faded out while the weapon reticle zooms in.
#[must_use]
pub fn reticle_sides_alpha_in(reticle_zoom: Option<f32>, overlay_alpha_scale: f32) -> f32 {
    match reticle_zoom {
        Some(zoom) => 1.0 - zoom / overlay_alpha_scale.max(1.0),
        None => 1.0,
    }
}

/// A weapon's hip reticle numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponReticle {
    pub reticle_min_ofs: i32,
    pub hip_reticle_side_pos: f32,
    pub reticle_side_size: i32,
    pub reticle_center_size: i32,
}

/// Side offsets (x, y) from the spread cone.
#[must_use]
pub fn calc_reticle_spread(
    spread_min: f32,
    spread_max: f32,
    aim_spread_scale: f32,
    scale: f32,
    tan_half_fov_y: f32,
    weap: &WeaponReticle,
    draw_size: [f32; 2],
) -> [f32; 2] {
    let spread = (spread_min + (spread_max - spread_min) * (aim_spread_scale / AIM_SPREAD_SCALE_MAX)) * scale;
    let mut px = libm::tanf(spread * DEG2RAD) * RETICLE_VIRTUAL_HALF_HEIGHT / tan_half_fov_y;
    let min_ofs = weap.reticle_min_ofs as f32;
    if px < min_ofs {
        px = min_ofs;
    }
    [
        px - draw_size[0] * weap.hip_reticle_side_pos,
        px - draw_size[1] * weap.hip_reticle_side_pos,
    ]
}

/// One rotated reticle pic, centre-aligned virtual coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReticlePic {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub angle: f32,
}

/// The four side pics in draw order (top, right, bottom, left; split screen draws top, bottom,
/// right, left). `spread` is from calc_reticle_spread with draw size (size, size).
#[must_use]
pub fn reticle_sides_layout(center: [f32; 2], size: f32, spread: [f32; 2], dual_play: bool) -> [ReticlePic; 4] {
    let [cx, cy] = center;
    let [sx, sy] = spread;
    let s = size;
    if dual_play {
        let long = s * 2.0;
        let short = s * 0.5;
        let gap = long * RETICLE_SIDE_GAP_FRAC;
        let gap_short = short * RETICLE_SIDE_GAP_FRAC;
        [
            ReticlePic { x: cx - s, y: (-s - sy) * 0.5 + cy + 0.5 - gap_short, w: long, h: short, angle: 0.0 },
            ReticlePic { x: cx - s - gap, y: sy * 0.5 + cy + 0.5 + gap_short, w: long, h: short, angle: 180.0 },
            ReticlePic { x: sx + cx + gap, y: cy - s + gap_short + 0.5, w: short, h: long, angle: 90.0 },
            ReticlePic { x: cx - s - sx - gap, y: cy - s + 0.5, w: short, h: long, angle: 270.0 },
        ]
    } else {
        let gap = s * RETICLE_SIDE_GAP_FRAC;
        let half = s * 0.5;
        [
            ReticlePic { x: cx - half, y: cy - s - sy - gap, w: s, h: s, angle: 0.0 },
            ReticlePic { x: sx + cx + gap, y: cy - half + gap, w: s, h: s, angle: 90.0 },
            ReticlePic { x: cx - half - gap, y: sy + cy + gap, w: s, h: s, angle: 180.0 },
            ReticlePic { x: cx - s - sx - gap, y: cy - half, w: s, h: s, angle: 270.0 },
        ]
    }
}

/// Centre reticle size; a held cook grows it with the fuse.
#[must_use]
pub fn reticle_center_size(center_size: i32, cook_hold_grenade: bool, grenade_time_left_ms: i32, scale: f32) -> f32 {
    let mut size = center_size as f32;
    if cook_hold_grenade && grenade_time_left_ms != 0 {
        size += (grenade_time_left_ms % 1000) as f32 * COOK_CENTER_GROWTH_PER_MS;
    }
    size * scale
}

/// Top-left corner and size of a square pic of `size` centred on `pos`.
#[must_use]
pub fn centered_square(pos: [f32; 2], size: f32) -> ReticlePic {
    ReticlePic { x: pos[0] - size * 0.5, y: pos[1] - size * 0.5, w: size, h: size, angle: 0.0 }
}

/// ADS aim indicator size, or None when it is not drawn.
#[must_use]
pub fn ads_aim_indicator_size(cg_draw_gun: bool, pos_frac: f32, indicator_size: Option<i32>) -> Option<f32> {
    match indicator_size {
        Some(size) if !cg_draw_gun && pos_frac < 1.0 => Some(size as f32 * (ADS_AIM_INDICATOR_BASE - pos_frac)),
        _ => None,
    }
}

/// Whether the vehicle seat's centre reticle shows.
#[must_use]
pub fn should_show_crosshair_on_vehicle(
    keep_when_ads: bool,
    pos_frac: f32,
    vehicle_seat: i32,
    has_reticle_center: bool,
    cg_crosshair_alpha: f32,
) -> bool {
    (keep_when_ads || pos_frac == 0.0 || !(1..=4).contains(&vehicle_seat))
        && has_reticle_center
        && cg_crosshair_alpha >= CROSSHAIR_ALPHA_DRAW_MIN
}

/// Everything the crosshair draw needs to decide what to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosshairFrame {
    pub emp_jammed: bool,
    pub view_gate: bool,
    pub flashbanged: bool,
    pub flared: bool,
    pub hud_shown: bool,
    pub color_alpha: f32,
    pub pos_frac: f32,
    pub to_ads: bool,
    pub cg_draw_gun: bool,
    pub keep_when_ads: bool,
    pub weapon_gate: bool,
    pub dynamic: bool,
    pub hardcore: bool,
    /// Gun direction on screen from calc_crosshair_position.
    pub position: [f32; 2],
}

/// What to draw for the hip crosshair this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrosshairPlan {
    pub center: [f32; 2],
    pub transition: AdsTransition,
    pub ads_aim_indicator: bool,
    pub sides_and_center: bool,
}

/// The hip crosshair part of the draw, after the weapon reticle / overlays. None draws nothing.
#[must_use]
pub fn plan_crosshair(f: &CrosshairFrame, weap: &WeaponAdsCrosshair, tan_half_fov_y: f32) -> Option<CrosshairPlan> {
    if f.emp_jammed || !f.view_gate || f.flashbanged || f.flared || !f.hud_shown {
        return None;
    }
    if f.color_alpha < CROSSHAIR_ALPHA_DRAW_MIN {
        return None;
    }
    if !(f.pos_frac != 1.0 || !f.cg_draw_gun || f.keep_when_ads) || !f.weapon_gate {
        return None;
    }
    let mut plan = CrosshairPlan { center: f.position, ..CrosshairPlan::default() };
    if f.pos_frac != 0.0 && !f.keep_when_ads {
        plan.transition = transition_to_ads(f.pos_frac, f.to_ads, weap, tan_half_fov_y);
        plan.ads_aim_indicator = true;
    }
    if f.pos_frac != 1.0 || f.cg_draw_gun {
        if !f.dynamic {
            plan.center = [0.0, plan.transition.shift];
        }
        plan.sides_and_center = !f.hardcore;
    }
    Some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn view_gate_low_ready_and_special_pm() {
        let mut g = ViewCrosshairGate::default();
        assert!(should_draw_crosshair(&g));
        g.weaponstate = WEAPONSTATE_LOWREADY_RAISE;
        assert!(!should_draw_crosshair(&g));
        g.link_flags = LINK_FLAG_FORCE_ADS_ZOOM;
        assert!(should_draw_crosshair(&g));
        let mut g = ViewCrosshairGate { pm_type: 10, ..ViewCrosshairGate::default() };
        assert!(!should_draw_crosshair(&g));
        g.e_flags2 = EFLAGS2_CROSSHAIR_IN_SPECIAL_PM;
        assert!(should_draw_crosshair(&g));
        let g = ViewCrosshairGate {
            pm_type: 10,
            e_flags: EFLAGS_VEHICLE_VIEW,
            vehicle_type: VEHICLE_TYPE_KEEPS_CROSSHAIR,
            ..ViewCrosshairGate::default()
        };
        assert!(should_draw_crosshair(&g));
    }

    #[test]
    fn weapon_gate_reload_and_raise() {
        let mut g = WeaponCrosshairGate::default();
        assert!(allowed_to_draw_crosshair(&g));
        g.weaponstate = WEAPONSTATE_RELOADING;
        assert!(!allowed_to_draw_crosshair(&g));
        g.weaponstate_left = Some(0);
        assert!(allowed_to_draw_crosshair(&g));
        g.weaponstate_left = Some(WEAPONSTATE_RELOADING);
        assert!(!allowed_to_draw_crosshair(&g));
        let g = WeaponCrosshairGate { weaponstate: 3, ..WeaponCrosshairGate::default() };
        assert!(!allowed_to_draw_crosshair(&g));
        let g = WeaponCrosshairGate { hardcore: true, ..WeaponCrosshairGate::default() };
        assert!(!allowed_to_draw_crosshair(&g));
        let g = WeaponCrosshairGate { used_on_vehicle: true, weaponstate: WEAPONSTATE_RELOAD_START, ..WeaponCrosshairGate::default() };
        assert!(!allowed_to_draw_crosshair(&g));
    }

    #[test]
    fn ads_transition_runs_over_the_last_frac() {
        let w = WeaponAdsCrosshair { ads_aim_pitch: 0.0, ads_crosshair_in_frac: 0.5, ads_crosshair_out_frac: 0.25 };
        assert_eq!(transition_to_ads(0.4, true, &w, 0.6), AdsTransition::default());
        let t = transition_to_ads(1.0, true, &w, 0.6);
        assert!(close(t.scale, 0.5));
        assert!(close(t.shift, 0.0));
        let t = transition_to_ads(0.875, false, &w, 0.6);
        assert!(close(t.scale, 0.75));
    }

    #[test]
    fn position_centre_when_gun_matches_view() {
        let axis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let p = calc_crosshair_position(0.0, 0.0, axis, 0.8, 0.6);
        assert!(close(p[0], 0.0) && close(p[1], 0.0));
        let p = calc_crosshair_position(0.0, 180.0, axis, 0.8, 0.6);
        assert_eq!(p, [0.0, 0.0]);
    }

    #[test]
    fn colour_white_unless_flagged() {
        let teams = CrosshairTeamColors {
            my_team: [0.0, 1.0, 0.0],
            my_team_alt: [0.0, 0.0, 1.0],
            enemy_team: [1.0, 0.0, 0.0],
            enemy_team_alt: [1.0, 1.0, 0.0],
            party: [1.0, 0.0, 1.0],
        };
        let mut i = CrosshairColorInputs {
            alpha_in: 0.5,
            cg_crosshair_alpha: 1.0,
            weapon_color_change: true,
            e_flags: 0,
            weap_flags: WEAP_FLAGS_ON_ENEMY,
            since_hit_ms: 10,
            friendly_fade_out_ms: 1500,
            enemy_fade_out_ms: 250,
            zombies: false,
            target_in_party: false,
            alt_colors: false,
            server_red_crosshair: true,
            cg_crosshair_enemy_color: true,
        };
        assert_eq!(crosshair_color(&i, &teams), [1.0, 0.0, 0.0, 0.5]);
        i.since_hit_ms = 300;
        assert_eq!(crosshair_color(&i, &teams), [1.0, 1.0, 1.0, 0.5]);
        i.weap_flags = WEAP_FLAGS_ON_FRIENDLY;
        assert_eq!(crosshair_color(&i, &teams)[..3], [0.0, 1.0, 0.0]);
        i.weapon_color_change = false;
        assert_eq!(crosshair_color(&i, &teams)[..3], WHITE);
    }

    #[test]
    fn reticle_alpha_floor_and_sides_fade() {
        assert!(close(calc_reticle_alpha(1.0, 1.0, 0.5, 0.0), 1.0));
        assert!(close(calc_reticle_alpha(1.0, 1.0, 0.5, 255.0), 0.5));
        assert!(close(reticle_sides_alpha_in(Some(0.5), 0.5), 0.5));
        assert!(close(reticle_sides_alpha_in(Some(0.5), 2.0), 0.75));
        assert_eq!(reticle_sides_alpha_in(None, 2.0), 1.0);
    }

    #[test]
    fn spread_floors_at_min_offset() {
        let w = WeaponReticle { reticle_min_ofs: 10, hip_reticle_side_pos: 0.5, reticle_side_size: 16, reticle_center_size: 4 };
        let s = calc_reticle_spread(0.0, 0.0, 0.0, 1.0, 0.6, &w, [16.0, 16.0]);
        assert_eq!(s, [2.0, 2.0]);
        let s = calc_reticle_spread(3.0, 3.0, 0.0, 1.0, 0.6, &w, [16.0, 16.0]);
        assert!(s[0] > 2.0);
    }

    #[test]
    fn sides_layout_one_screen() {
        let pics = reticle_sides_layout([0.0, 0.0], 16.0, [4.0, 6.0], false);
        assert_eq!(pics[0], ReticlePic { x: -8.0, y: -24.0, w: 16.0, h: 16.0, angle: 0.0 });
        assert_eq!(pics[1], ReticlePic { x: 6.0, y: -6.0, w: 16.0, h: 16.0, angle: 90.0 });
        assert_eq!(pics[2], ReticlePic { x: -10.0, y: 8.0, w: 16.0, h: 16.0, angle: 180.0 });
        assert_eq!(pics[3], ReticlePic { x: -22.0, y: -8.0, w: 16.0, h: 16.0, angle: 270.0 });
    }

    #[test]
    fn center_grows_with_cook() {
        assert_eq!(reticle_center_size(8, true, 2500, 1.0), 13.0);
        assert_eq!(reticle_center_size(8, false, 2500, 0.5), 4.0);
        assert_eq!(centered_square([0.0, 0.0], 4.0).x, -2.0);
    }

    #[test]
    fn plan_full_ads_hides_sides_unless_kept() {
        let w = WeaponAdsCrosshair { ads_aim_pitch: 0.0, ads_crosshair_in_frac: 0.5, ads_crosshair_out_frac: 0.5 };
        let mut f = CrosshairFrame {
            emp_jammed: false,
            view_gate: true,
            flashbanged: false,
            flared: false,
            hud_shown: true,
            color_alpha: 1.0,
            pos_frac: 0.0,
            to_ads: true,
            cg_draw_gun: true,
            keep_when_ads: false,
            weapon_gate: true,
            dynamic: false,
            hardcore: false,
            position: [3.0, 4.0],
        };
        let p = plan_crosshair(&f, &w, 0.6).unwrap();
        assert!(p.sides_and_center && !p.ads_aim_indicator);
        assert_eq!(p.center, [0.0, 0.0]);
        f.pos_frac = 1.0;
        assert!(plan_crosshair(&f, &w, 0.6).is_none());
        f.keep_when_ads = true;
        assert!(plan_crosshair(&f, &w, 0.6).unwrap().sides_and_center);
        f.emp_jammed = true;
        assert!(plan_crosshair(&f, &w, 0.6).is_none());
        assert_eq!(ads_aim_indicator_size(false, 0.5, Some(10)), Some(10.0));
        assert_eq!(ads_aim_indicator_size(true, 0.5, Some(10)), None);
    }
}
