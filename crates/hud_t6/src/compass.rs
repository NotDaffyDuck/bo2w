//! BO2 compass and minimap, C side: the minimap config string, the PARTIAL (minimap) and FULL
//! (fullscreen map) dimensions and world-to-compass math, the rotated map picture, the tickertape
//! strip, the coordinate text, the radar sweep margin and ping rule, and the fade.
//! BO2 draws no player, friendly, vehicle or objective icons in C (the UI layer does); it keeps
//! the actor data only, so no icon names live here.
//! Facts: scratch/hud-facts/compass.md.

/// `compassMaxRange` (inches covered by the minimap's height).
pub const COMPASS_MAX_RANGE_DEFAULT: f32 = 2500.0;
pub const COMPASS_MAX_RANGE_MIN: f32 = 0.0001;
pub const COMPASS_MIN_RANGE_DEFAULT: f32 = 0.0001;
pub const COMPASS_MIN_RADIUS_DEFAULT: f32 = 0.0001;
pub const COMPASS_CLAMP_ICONS_DEFAULT: bool = true;
pub const COMPASS_SHOW_ENEMIES_DEFAULT: bool = true;
pub const COMPASS_FORCE_PLAYER_ICON_DEFAULT: bool = false;

pub const COMPASS_SOUND_PING_FADE_TIME_DEFAULT: f32 = 2.0;
pub const COMPASS_SOUND_PING_FADE_TIME_RANGE: (f32, f32) = (0.0, 10.0);
pub const COMPASS_RADAR_PING_FADE_TIME_DEFAULT: f32 = 4.0;
pub const COMPASS_RADAR_PING_FADE_TIME_RANGE: (f32, f32) = (0.01, 60.0);
pub const COMPASS_RADAR_UPDATE_TIME_DEFAULT: f32 = 4.0;
pub const COMPASS_RADAR_UPDATE_FAST_TIME_DEFAULT: f32 = 2.0;
/// `compassRadarLineThickness`: radar line margin on the minimap, a fraction of the max range.
pub const COMPASS_RADAR_LINE_THICKNESS_DEFAULT: f32 = 0.4;
pub const COMPASS_RADAR_LINE_THICKNESS_RANGE: (f32, f32) = (0.01, 10.0);
/// `cg_hudMapRadarLineThickness`: the same margin on the fullscreen map.
pub const CG_HUD_MAP_RADAR_LINE_THICKNESS_DEFAULT: f32 = 0.15;

pub const COMPASS_LOCAL_RADAR_UPDATE_TIME_DEFAULT: f32 = 2.25;
pub const COMPASS_LOCAL_RADAR_RADIUS_DEFAULT: f32 = 700.0;

pub const COMPASS_SATELLITE_STATIC_IMAGE_FADE_TIME_DEFAULT: f32 = 0.0;
pub const COMPASS_SATELLITE_PING_FADE_TIME_DEFAULT: f32 = 8.0;
pub const COMPASS_STATIC_IMAGE_UPDATE_TIME_MS_DEFAULT: i32 = 4000;
pub const COMPASS_SATELLITE_SCAN_TIME_MS_DEFAULT: i32 = 1000;

pub const COMPASS_OBJECTIVE_WIDTH_DEFAULT: f32 = 32.0;
pub const COMPASS_OBJECTIVE_HEIGHT_DEFAULT: f32 = 32.0;
/// Metres.
pub const COMPASS_OBJECTIVE_NEARBY_DIST_DEFAULT: f32 = 4.0;
pub const COMPASS_OBJECTIVE_MIN_DIST_RANGE_DEFAULT: f32 = 1.0;
/// Metres.
pub const COMPASS_OBJECTIVE_DETAIL_DIST_DEFAULT: f32 = 10.0;
pub const COMPASS_OBJECTIVE_MIN_HEIGHT_DEFAULT: f32 = -70.0;
pub const COMPASS_OBJECTIVE_MAX_HEIGHT_DEFAULT: f32 = 70.0;

pub const COMPASS_ENEMY_FOOTSTEP_MAX_RANGE_DEFAULT: f32 = 500.0;
pub const COMPASS_ENEMY_FOOTSTEP_MAX_Z_DEFAULT: f32 = 100.0;
pub const COMPASS_ENEMY_FOOTSTEP_MIN_SPEED_DEFAULT: f32 = 140.0;
pub const COMPASS_ENEMY_FOOTSTEP_ENABLED_DEFAULT: bool = false;

pub const COMPASS_GRID_ENABLED_DEFAULT: bool = true;
pub const COMPASS_GRID_ROWS_DEFAULT: i32 = 5;
pub const COMPASS_GRID_ROWS_RANGE: (i32, i32) = (1, 9);
pub const COMPASS_GRID_COLS_DEFAULT: i32 = 5;
pub const COMPASS_GRID_ALIGN_DEFAULT: i32 = 0;
pub const COMPASS_GRID_ALIGN_RANGE: (i32, i32) = (0, 8);

pub const CG_HUD_MAP_FRIENDLY_WIDTH_DEFAULT: f32 = 15.0;
pub const CG_HUD_MAP_FRIENDLY_HEIGHT_DEFAULT: f32 = 15.0;
pub const CG_HUD_MAP_PLAYER_WIDTH_DEFAULT: f32 = 20.0;
pub const CG_HUD_MAP_PLAYER_HEIGHT_DEFAULT: f32 = 20.0;
pub const CG_HUD_MAP_BORDER_WIDTH_DEFAULT: f32 = 2.0;

/// Config string carrying "material ulx uly lrx lry".
pub const CS_MINIMAP: usize = 0x423;
/// The jammed map picture is the map material's name plus this.
pub const MINIMAP_SCRAMBLED_SUFFIX: &str = "_scrambled";
/// A zero world size from the config string becomes this.
pub const MINIMAP_ZERO_SIZE_FALLBACK: f32 = 1000.0;

/// Compass actors kept per local client.
pub const COMPASS_ACTOR_COUNT: usize = 18;
/// CompassActor bit-field: the enemy bit.
pub const COMPASS_ACTOR_BIT_ENEMY: u8 = 1 << 3;
/// A viewer on this team (spectator) skips the actor update, as does a dead one.
pub const COMPASS_ACTOR_SKIP_TEAM: i32 = 9;

/// A radar ping only takes an actor seen within this many ms.
pub const RADAR_PING_FRESH_MS: i32 = 1500;
/// The perk whose radar check reads the actor's gpsJammerActive bit.
pub const RADAR_PERK_GPS_JAMMER: u32 = 0x1b;

/// Tickertape distance text: inches to metres.
pub const INCHES_TO_METRES: f32 = 0.0254;
/// Snapshot objectives the tickertape reads; state 1 is drawn.
pub const TICKERTAPE_OBJECTIVES: usize = 32;
pub const TICKERTAPE_OBJECTIVE_STATE_ACTIVE: i32 = 1;
/// An objective whose entNum is this uses its own origin, not an entity's.
pub const OBJECTIVE_NO_ENTITY: i32 = 0x3ff;

/// Coordinate text is drawn turned this many degrees, at this scale.
pub const COORD_TEXT_ROTATION_DEG: f32 = -90.0;
pub const COORD_TEXT_SCALE: f32 = 0.8;

/// Which map is drawn: the minimap or the fullscreen map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum CompassType {
    Partial = 0,
    Full = 1,
}

/// The minimap's place in the world: cg_t's compassMapUpperLeft, compassMapWorldSize and
/// compassNorth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompassMapBounds {
    pub upper_left: [f32; 2],
    pub world_size: [f32; 2],
    pub north: [f32; 2],
}

/// compassNorth: (cos, sin) of the north yaw in degrees.
#[must_use]
pub fn compass_north(north_yaw_deg: f32) -> [f32; 2] {
    let rad = north_yaw_deg.to_radians();
    [libm::cosf(rad), libm::sinf(rad)]
}

/// CG_MiniMapChanged: the bounds from the config string's corners and the north yaw. No corner
/// reordering and no aspect widening; a zero size becomes 1000.
#[must_use]
pub fn mini_map_changed(
    upper_left: [f32; 2],
    lower_right: [f32; 2],
    north_yaw_deg: f32,
) -> CompassMapBounds {
    let north = compass_north(north_yaw_deg);
    let (dx, dy) = (lower_right[0] - upper_left[0], lower_right[1] - upper_left[1]);
    let mut world_size = [dx * north[1] - dy * north[0], -(dx * north[0] + dy * north[1])];
    for size in &mut world_size {
        if *size == 0.0 {
            *size = MINIMAP_ZERO_SIZE_FALLBACK;
        }
    }
    CompassMapBounds { upper_left, world_size, north }
}

/// The parts of the minimap config string: material name, upper-left, lower-right.
#[must_use]
pub fn parse_minimap_config(config: &str) -> Option<(&str, [f32; 2], [f32; 2])> {
    let mut tokens = config.split_ascii_whitespace();
    let material = tokens.next()?;
    let mut number = || tokens.next().and_then(|t| t.parse::<f32>().ok());
    let upper_left = [number()?, number()?];
    let lower_right = [number()?, number()?];
    Some((material, upper_left, lower_right))
}

/// CompassCalcDimensions, PARTIAL: the ownerdraw size times compassSize, and on split screen
/// times cg_hudSplitscreenCompassScale. BO2 errors out on a size <= 0 (None here).
#[must_use]
pub fn compass_partial_size(
    rect_size: [f32; 2],
    compass_size: f32,
    splitscreen_scale: Option<f32>,
) -> Option<[f32; 2]> {
    let k = compass_size * splitscreen_scale.unwrap_or(1.0);
    let size = [rect_size[0] * k, rect_size[1] * k];
    (size[0] > 0.0 && size[1] > 0.0).then_some(size)
}

/// CompassCalcDimensions, FULL: the map's aspect fitted centred in `rect` (x, y, w, h), then each
/// side pulled in by min(cg_hudMapBorderWidth, w/4, h/4).
#[must_use]
pub fn compass_full_dimensions(rect: [f32; 4], world_size: [f32; 2], border_width: f32) -> [f32; 4] {
    let [mut x, mut y, mut w, mut h] = rect;
    let aspect = world_size[0] / world_size[1];
    if w > h * aspect {
        let fitted = h * aspect;
        x += (w - fitted) * 0.5;
        w = fitted;
    } else {
        let fitted = w / aspect;
        y += (h - fitted) * 0.5;
        h = fitted;
    }
    let border = border_width.min(w * 0.25).min(h * 0.25);
    [x + border, y + border, w - 2.0 * border, h - 2.0 * border]
}

/// Where a world point lands on the compass, from the rect's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompassPoint {
    pub offset: [f32; 2],
    pub unclamped: [f32; 2],
    /// The point lay outside the rect and `offset` was pulled back onto its edge.
    pub clamped: bool,
}

/// Scale `offset` down along its ray until it fits +-w/2, +-h/2; true when it had to.
#[must_use]
pub fn compass_clamp(offset: [f32; 2], size: [f32; 2]) -> ([f32; 2], bool) {
    let mut scale = 1.0_f32;
    for axis in 0..2 {
        let half = size[axis] * 0.5;
        if offset[axis].abs() > half {
            scale = scale.min(half / offset[axis].abs());
        }
    }
    ([offset[0] * scale, offset[1] * scale], scale < 1.0)
}

/// CG_WorldPosToCompass, PARTIAL: `view_dir` is (cos, sin) of the player's yaw; the map's height
/// covers compassMaxRange.
#[must_use]
pub fn world_pos_to_compass_partial(
    view_dir: [f32; 2],
    view_origin: [f32; 2],
    world: [f32; 2],
    size: [f32; 2],
    compass_max_range: f32,
) -> CompassPoint {
    let scale = size[1] / compass_max_range;
    let (dx, dy) = (world[0] - view_origin[0], world[1] - view_origin[1]);
    let unclamped = [
        (view_dir[1] * dx - view_dir[0] * dy) * scale,
        (-(view_dir[1] * dy) - view_dir[0] * dx) * scale,
    ];
    let (offset, clamped) = compass_clamp(unclamped, size);
    CompassPoint { offset, unclamped, clamped }
}

/// CG_WorldPosToCompass, FULL: the point as a fraction of the map's world size from its upper-left,
/// moved to the rect's centre. The facts name no clamp on this branch.
#[must_use]
pub fn world_pos_to_compass_full(bounds: &CompassMapBounds, world: [f32; 2], size: [f32; 2]) -> [f32; 2] {
    let n = bounds.north;
    let (dx, dy) = (world[0] - bounds.upper_left[0], world[1] - bounds.upper_left[1]);
    [
        size[0] * ((n[1] * dx - n[0] * dy) / bounds.world_size[0] - 0.5),
        size[1] * ((-(n[1] * dy) - n[0] * dx) / bounds.world_size[1] - 0.5),
    ]
}

/// The round minimap is the whole map picture turned by this (degrees); the shader reads the
/// placed rect from GenericParam0. No UV window.
#[must_use]
pub fn minimap_rotation_deg(view_yaw_deg: f32, north_yaw_deg: f32) -> f32 {
    view_yaw_deg - north_yaw_deg
}

/// Tickertape strip texture span: u0..u1 centred on -(view yaw - north yaw)/360,
/// compassTickertapeStretch wide.
#[must_use]
pub fn tickertape_u(view_yaw_deg: f32, north_yaw_deg: f32, stretch: f32) -> (f32, f32) {
    let centre = -(view_yaw_deg - north_yaw_deg) / 360.0;
    (centre - stretch * 0.5, centre + stretch * 0.5)
}

/// Degrees either side of the view the tickertape shows; objective icons clamp to it.
#[must_use]
pub fn tickertape_half_span_deg(stretch: f32) -> f32 {
    stretch * 360.0 * 0.5
}

/// CalcCompassPointerSize: the tickertape objective icon size.
#[must_use]
pub fn compass_pointer_size(compass_size: f32, objective_size: [f32; 2]) -> [f32; 2] {
    [compass_size * objective_size[0], compass_size * objective_size[1]]
}

/// North coordinate text value: the view position along north, over compassCoords.z, plus
/// compassCoords.y.
#[must_use]
pub fn compass_coord_north(north: [f32; 2], view_xy: [f32; 2], coords: [f32; 3]) -> f32 {
    (north[1] * view_xy[1] + north[0] * view_xy[0]) / coords[2] + coords[1]
}

/// East coordinate text value: the same along the perpendicular of north.
/// TODO(fact): the east offset is read as compassCoords.x (the facts name only "the perpendicular").
#[must_use]
pub fn compass_coord_east(north: [f32; 2], view_xy: [f32; 2], coords: [f32; 3]) -> f32 {
    (north[1] * view_xy[0] - north[0] * view_xy[1]) / coords[2] + coords[0]
}

/// GetRadarLineMargin: how near the sweep line a moving enemy counts as crossing it.
#[must_use]
pub fn radar_line_margin(
    compass_type: CompassType,
    compass_max_range: f32,
    radar_line_thickness: f32,
    hud_map_radar_line_thickness: f32,
) -> f32 {
    compass_max_range
        * match compass_type {
            CompassType::Partial => radar_line_thickness,
            CompassType::Full => hud_map_radar_line_thickness,
        }
}

/// RadarPingEnemyPlayer: whether the sweep pings this actor (then lastEnemyPos = lastPos and
/// beginRadarFadeTime = now). Fresh within 1500 ms, and without the counter perk (or the perk is
/// the GPS jammer one and the actor's gpsJammerActive bit is clear).
#[must_use]
pub fn radar_ping_enemy_player(
    actor_last_update_ms: i32,
    time_ms: i32,
    perk: u32,
    actor_has_perk: bool,
    actor_gps_jammer_active: bool,
) -> bool {
    time_ms - actor_last_update_ms <= RADAR_PING_FRESH_MS
        && (!actor_has_perk || (perk == RADAR_PERK_GPS_JAMMER && !actor_gps_jammer_active))
}

/// CG_CompassUpdateActors' enemy bit: a different team, or the viewer is on team 0.
#[must_use]
pub fn compass_actor_is_enemy(viewer_team: i32, actor_team: i32) -> bool {
    viewer_team == 0 || actor_team != viewer_team
}

/// The compass fade goes through the HUD menu fade (hud_fade_compass seconds, as ms) only on the
/// minimap outside Zombies; otherwise the compass is fully shown. No jam multiplier.
#[must_use]
pub fn compass_fades(compass_type: CompassType, zombies: bool) -> bool {
    compass_type == CompassType::Partial && !zombies
}

/// hud_fade_compass seconds as the ms the HUD menu fade takes.
#[must_use]
pub fn compass_fade_ms(hud_fade_compass_s: f32) -> f32 {
    hud_fade_compass_s * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn bounds_keep_corners_and_replace_zero_size() {
        let b = mini_map_changed([-1000.0, 2000.0], [3000.0, -2000.0], 90.0);
        assert!(near(b.north[0], 0.0) && near(b.north[1], 1.0));
        assert!(near(b.world_size[0], 4000.0) && near(b.world_size[1], 4000.0));
        let flat = mini_map_changed([0.0, 0.0], [0.0, 0.0], 90.0);
        assert_eq!(flat.world_size, [MINIMAP_ZERO_SIZE_FALLBACK; 2]);
        // No reorder: swapped corners give negative sizes, as BO2.
        let swapped = mini_map_changed([3000.0, -2000.0], [-1000.0, 2000.0], 90.0);
        assert!(swapped.world_size[0] < 0.0);
    }

    #[test]
    fn config_string_parses() {
        let (m, ul, lr) = parse_minimap_config("compass_map_mp_raid 1 2 3.5 -4").unwrap();
        assert_eq!((m, ul, lr), ("compass_map_mp_raid", [1.0, 2.0], [3.5, -4.0]));
        assert!(parse_minimap_config("compass_map_mp_raid 1 2 3").is_none());
    }

    #[test]
    fn partial_ahead_is_up_and_clamps_far() {
        // Facing +x: a point ahead sits above the centre (negative y).
        let p = world_pos_to_compass_partial([1.0, 0.0], [0.0, 0.0], [1250.0, 0.0], [100.0, 100.0], 2500.0);
        assert!(near(p.offset[0], 0.0) && near(p.offset[1], -50.0) && !p.clamped);
        let far = world_pos_to_compass_partial([1.0, 0.0], [0.0, 0.0], [5000.0, 0.0], [100.0, 100.0], 2500.0);
        assert!(far.clamped && near(far.offset[1], -50.0) && near(far.unclamped[1], -200.0));
        // To the right (-y when facing +x) is +x on the compass.
        let right = world_pos_to_compass_partial([1.0, 0.0], [0.0, 0.0], [0.0, -500.0], [100.0, 100.0], 2500.0);
        assert!(near(right.offset[0], 20.0));
    }

    #[test]
    fn full_map_centre_and_corner() {
        let b = mini_map_changed([-1000.0, 2000.0], [3000.0, -2000.0], 90.0);
        assert_eq!(world_pos_to_compass_full(&b, [-1000.0, 2000.0], [200.0, 200.0]), [-100.0, -100.0]);
        let c = world_pos_to_compass_full(&b, [1000.0, 0.0], [200.0, 200.0]);
        assert!(near(c[0], 0.0) && near(c[1], 0.0));
    }

    #[test]
    fn full_dimensions_fit_aspect_then_border() {
        let r = compass_full_dimensions([0.0, 0.0, 400.0, 200.0], [1000.0, 1000.0], 2.0);
        assert_eq!(r, [102.0, 2.0, 196.0, 196.0]);
        let tall = compass_full_dimensions([0.0, 0.0, 200.0, 400.0], [2000.0, 1000.0], 100.0);
        // Border capped at a quarter of the fitted 100-high map.
        assert_eq!(tall, [25.0, 175.0, 150.0, 50.0]);
    }

    #[test]
    fn partial_size_rejects_zero() {
        assert_eq!(compass_partial_size([100.0, 80.0], 1.5, None), Some([150.0, 120.0]));
        assert_eq!(compass_partial_size([100.0, 80.0], 1.0, Some(0.5)), Some([50.0, 40.0]));
        assert_eq!(compass_partial_size([100.0, 80.0], 0.0, None), None);
    }

    #[test]
    fn tickertape_and_rotation() {
        assert_eq!(minimap_rotation_deg(120.0, 90.0), 30.0);
        let (u0, u1) = tickertape_u(180.0, 90.0, 0.5);
        assert!(near(u0, -0.5) && near(u1, 0.0));
        assert!(near(tickertape_half_span_deg(0.5), 90.0));
    }

    #[test]
    fn radar_margin_and_ping() {
        assert!(near(radar_line_margin(CompassType::Partial, 2500.0, 0.4, 0.15), 1000.0));
        assert!(near(radar_line_margin(CompassType::Full, 2500.0, 0.4, 0.15), 375.0));
        assert!(radar_ping_enemy_player(1000, 2500, 0x10, false, false));
        assert!(!radar_ping_enemy_player(1000, 2501, 0x10, false, false));
        assert!(!radar_ping_enemy_player(1000, 1200, 0x10, true, false));
        assert!(radar_ping_enemy_player(1000, 1200, RADAR_PERK_GPS_JAMMER, true, false));
        assert!(!radar_ping_enemy_player(1000, 1200, RADAR_PERK_GPS_JAMMER, true, true));
    }

    #[test]
    fn enemy_and_fade_rules() {
        assert!(compass_actor_is_enemy(0, 1));
        assert!(!compass_actor_is_enemy(1, 1));
        assert!(compass_fades(CompassType::Partial, false));
        assert!(!compass_fades(CompassType::Partial, true));
        assert!(!compass_fades(CompassType::Full, false));
        assert!(near(compass_coord_north([0.0, 1.0], [10.0, 500.0], [0.0, 3.0, 100.0]), 8.0));
    }
}
