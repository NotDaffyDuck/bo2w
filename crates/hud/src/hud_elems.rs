//! Script-driven HUD elements (pictures and waypoints), drawn in two passes:
//! the ones behind the rest of the HUD, then the ones in front.

use asset_core::AssetIdentity;
use asset_core::AssetVariety;
use asset_core::AssetCategoryScope;
use bevy::prelude::*;
use hud_t6::font::hudelem_em_px;
use hud_t6::overlay_piece::{
    HE_TYPE_WAYPOINT, HUDELEM_NO_TARGET_ENT, OverlayPiece, HudElemBody, HudElemPlacement, HudElemViewer,
    hud_elem_body, hud_elem_draw_color, blend_piece_type_size, hud_elem_passes_filter,
    hud_elem_placement, sorted_hud_elems,
};
use net::{ClientEntityRecord, ClientEntityPoseState, FrameClock, HostViewClient, ShownWorldFrame};
use sim::PlayerSessionStage;

use crate::draw2d::FlatPaintEntry;
use crate::draw2d::FlatPaintQueue;
use crate::draw2d::FlatPaintAction;
use crate::draw2d::FlatPaintOrigin;
use crate::glyph_layer::spawn_layer_node;
use crate::gaps::GapReason;
use crate::gaps::HudGapBook;
use crate::draw_batch::LayerRequests;
use crate::draw_batch::LayerJob;
use crate::images::{CHROME_NAMESPACE, HudPictures};
use crate::surface::FlatOverlayCanvas;
use crate::draw2d::expand_paint_list;

/// Edge of a waypoint icon at the authored scale.
const WAYPOINT_ICON_SIZE: f32 = 24.0;
/// How far inside the window an off-screen waypoint arrow stays.
const WAYPOINT_EDGE_MARGIN: f32 = 24.0;

/// Host of the elements drawn in front.
#[derive(Component)]
pub(crate) struct HudElemsRaster;

/// Host of the elements drawn behind.
#[derive(Component)]
pub(crate) struct HudElemsBackRaster;

pub(crate) fn spawn_hud_elems(root: &mut ChildSpawnerCommands) {
    spawn_layer_node(root, HudElemsRaster);
}

pub(crate) fn spawn_hud_elems_back(root: &mut ChildSpawnerCommands) {
    spawn_layer_node(root, HudElemsBackRaster);
}

fn hide_both(pass: &mut LayerRequests) {
    pass.elems_front = LayerJob::Conceal;
    pass.elems_behind = LayerJob::Conceal;
}

/// Where in the world a waypoint sits: its own offset, from the entity it follows when it
/// follows one. `None` when that entity is not in the next snapshot.
fn waypoint_world_spot(
    entities: &Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
    waypoint: &OverlayPiece,
) -> Option<Vec3> {
    let offset = Vec3::new(waypoint.x, waypoint.y, waypoint.z);
    if waypoint.aim_ent == HUDELEM_NO_TARGET_ENT {
        return Some(offset);
    }
    entities
        .iter()
        .find(|(ident, live)| {
            i32::from(ident.number()) == waypoint.aim_ent && live.present_in_newer_frame()
        })
        .map(|(_, live)| Vec3::from_array(live.origin) + offset)
}

/// A world point as the view camera sees it.
struct Sighting {
    world: Vec3,
    pixel: Vec2,
    on_screen: bool,
}

impl Sighting {
    fn of(
        world: Vec3,
        (camera, transform): (&Camera, &GlobalTransform),
        window: Vec2,
    ) -> Option<Sighting> {
        let forward = *transform.forward();
        let depth = (world - transform.translation()).dot(forward);
        // Behind the eye, mirror the point through it so the arrow points the
        // right way when it is clamped to the window edge.
        let projected = match depth <= 0.0 {
            true => world - forward * (2.0 * depth - 1.0),
            false => world,
        };
        let pixel = camera.world_to_viewport(transform, projected).ok()?;
        let within = |at: f32, limit: f32| (0.0..=limit).contains(&at);
        Some(Self {
            world,
            pixel,
            on_screen: depth > 0.0 && within(pixel.x, window.x) && within(pixel.y, window.y),
        })
    }
}

/// Where a waypoint goes on the window, how big it is, and whether it is the
/// off-screen arrow. Only that arrow (unless the element opts out) is a 2D
/// draw in the game; the on-screen marker is a 3D surface on the render
/// side, so the on-screen case here is a stand-in.
fn place_waypoint(
    waypoint: &OverlayPiece,
    view: (&Camera, &GlobalTransform),
    entities: &Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
    screen: &FlatOverlayCanvas,
) -> Option<(Vec2, f32, bool)> {
    let window = Vec2::new(screen.width(), screen.height());
    let seen = Sighting::of(waypoint_world_spot(entities, waypoint)?, view, window)?;
    let to_real = screen.placement().layout_to_pixels[1];
    let icon = WAYPOINT_ICON_SIZE * to_real;
    // The waypoint's value chooses between two draw-flag sets; what the second
    // one means is not pinned down, so a constant size stands in for it.
    match (waypoint.val > 0.0, seen.on_screen) {
        (false, false) => None,
        (false, true) => {
            let (camera, transform) = view;
            let beside = seen.world + *transform.right() * waypoint.width as f32;
            let side = camera.world_to_viewport(transform, beside).ok()?;
            Some((seen.pixel, (side - seen.pixel).length(), false))
        }
        (true, true) => Some((seen.pixel, icon, false)),
        (true, false) => {
            let arrow = hud_elem_body(waypoint.piece_kind, waypoint.bits)
                == HudElemBody::OffscreenWaypoint;
            let margin = WAYPOINT_EDGE_MARGIN * to_real;
            arrow.then(|| (clamp_to_margin(seen.pixel, window, margin).0, icon, true))
        }
    }
}

/// Slides a point out from the window centre along its direction until it
/// meets the box `margin` inside the window edge. Gives the spot and how many
/// times the original direction it travelled.
fn clamp_to_margin(pixel: Vec2, window: Vec2, margin: f32) -> (Vec2, f32) {
    let centre = window * 0.5;
    let reach = (centre - Vec2::splat(margin)).max(Vec2::ONE);
    let direction = pixel - centre;
    let travel = (reach.x / direction.x.abs().max(f32::EPSILON))
        .min(reach.y / direction.y.abs().max(f32::EPSILON));
    (centre + direction * travel, travel)
}

fn unit_color(rgba: [u8; 4]) -> [f32; 4] {
    rgba.map(|channel| f32::from(channel) / 255.0)
}

/// The namespace and name an element's material string refers to; a string
/// that is not a material key is a chrome image by that name.
fn split_material(text: &str) -> (AssetCategoryScope, String) {
    match AssetIdentity::parse(text) {
        Ok(key) if key.kind == AssetVariety::Material => (key.namespace, key.name),
        _ => (CHROME_NAMESPACE, text.to_owned()),
    }
}

/// Where an element's picture goes and which server material it names.
/// Waypoints take their place from the world; everything else from its own
/// authored placement. `None` when a waypoint is not drawn this frame.
fn elem_rect(
    hud_elem: &OverlayPiece,
    now: i32,
    screen: &FlatOverlayCanvas,
    view_camera: Option<(&Camera, &GlobalTransform)>,
    entities: &Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
) -> Option<(HudElemPlacement, i32)> {
    if hud_elem.piece_kind != HE_TYPE_WAYPOINT {
        let placement = screen.placement();
        let font_height =
            hudelem_em_px(hud_elem.font, blend_piece_type_size(hud_elem, now), placement) as f32;
        let rect = hud_elem_placement(placement, hud_elem, now, 0.0, font_height);
        return Some((rect, hud_elem.skin_no));
    }
    let (centre, size, off_screen) = place_waypoint(hud_elem, view_camera?, entities, screen)?;
    // Which material the off-screen arrow uses was never read, so it takes the
    // element's own off-screen one when it has one.
    let picked = if off_screen && hud_elem.offscreen_material_index != 0 {
        hud_elem.offscreen_material_index
    } else {
        hud_elem.skin_no
    };
    let rect = HudElemPlacement {
        x: centre.x - size * 0.5,
        y: centre.y - size * 0.5,
        w: size,
        h: size,
    };
    Some((rect, picked))
}

/// What one visible element will draw: its slot in the sorted list, colour, box and the
/// server material index it names.
struct Plan {
    sorted_at: usize,
    tint: [u8; 4],
    rect: HudElemPlacement,
    server_material: i32,
}

fn job_for(cmds: Vec<FlatPaintEntry>) -> LayerJob {
    let (quads, _) = expand_paint_list(&FlatPaintQueue { cmds });
    match quads.is_empty() {
        true => LayerJob::Conceal,
        false => LayerJob::Fresh(quads),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_hud_elems(
    surface: Res<FlatOverlayCanvas>,
    clock: Res<FrameClock>,
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    mut pictures: ResMut<HudPictures>,
    mut image_store: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudGapBook>,
    mut pass: ResMut<LayerRequests>,
    subject: Option<Res<frame::EyeSubject>>,
    world_cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    entities: Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
) {
    let scene = surface
        .has_size()
        .then(|| presented.snapshot())
        .flatten()
        .and_then(|snapshot| Some((&snapshot.meta, snapshot.meta.for_client(local.0)?)));
    let Some((meta, client)) = scene else {
        return hide_both(&mut pass);
    };

    // The viewer as the game's HUD-element pass sees it. It does not hide
    // anything in a menu. Its "dead but still shown" test is the player-move
    // type being below dead, or remote control; the meta carries neither the
    // move type nor the scope state, so alive and remote missile stand in.
    let remote_controlling = client.remote_missile.is_some();
    let viewer = HudElemViewer {
        dead_visible: remote_controlling || client.lifecycle == PlayerSessionStage::Alive,
        in_sniper_scope: false,
        killcam: subject.is_some_and(|subject| subject.replaying_final_death()),
        remote_controlling,
    };
    let sorted = sorted_hud_elems(&client.hud_current, &client.hud_archival);
    let now = clock.time();
    let view_camera = world_cameras.iter().find(|(camera, _)| camera.is_active);

    // Clock and perk elements are not drawn yet. Text elements are not drawn here
    // either: the scripts' text goes to the zombies HUD in `ui`, which leaves
    // pictures and waypoints for this pass.
    let plan = |slot: usize, elem: &OverlayPiece| -> Option<Plan> {
        let color = hud_elem_draw_color(elem, now, None)?;
        let picture = hud_elem_body(elem.piece_kind, elem.bits) == HudElemBody::Material;
        (picture || elem.piece_kind == HE_TYPE_WAYPOINT).then_some(())?;
        let (rect, material_index) = elem_rect(elem, now, &surface, view_camera, &entities)?;
        Some(Plan {
            sorted_at: slot,
            tint: color,
            rect,
            server_material: material_index,
        })
    };
    let mut resolve = |plan: Plan| -> Option<FlatPaintEntry> {
        let Plan {
            sorted_at: slot,
            tint,
            rect,
            server_material,
        } = plan;
        let named = u8::try_from(server_material)
            .ok()
            .and_then(|entry| sim::label_in_listed(&meta.hud_materials, entry));
        let Some(name) = named else {
            gaps.log_gap(GapReason::ElemMaterialUnbound {
                material_index: server_material,
            });
            return None;
        };
        let (surface_lineage, material) = split_material(name);
        if pictures
            .get(surface_lineage, &material, &mut image_store)
            .is_none()
        {
            let miss = pictures.lookup_failure();
            gaps.log_gap(GapReason::ElemImageAbsent {
                asset_key: material,
                miss,
            });
            return None;
        }
        Some(FlatPaintEntry {
            surface_lineage,
            paint_material: material,
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            s0: 0.0,
            t0: 0.0,
            s1: 1.0,
            t1: 1.0,
            rgba: unit_color(tint),
            op: FlatPaintAction::Stretched,
            sample_origin_tag: FlatPaintOrigin::HudElem { elem_slot: slot as i32 },
            draw_layer: 1,
        })
    };
    let mut layer = |foreground: bool| -> Vec<FlatPaintEntry> {
        sorted
            .iter()
            .enumerate()
            .filter(|&(_, elem)| hud_elem_passes_filter(elem, foreground, &viewer))
            .filter_map(|(at, item)| plan(at, item))
            .filter_map(&mut resolve)
            .collect()
    };

    // The pass behind the rest first, then the one in front.
    let (behind, in_front) = (layer(false), layer(true));
    if behind.is_empty() && in_front.is_empty() {
        return hide_both(&mut pass);
    }
    pass.elems_behind = job_for(behind);
    pass.elems_front = job_for(in_front);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_bytes_become_unit_floats() {
        assert_eq!(unit_color([0, 255, 0, 255]), [0.0, 1.0, 0.0, 1.0]);
        let half = unit_color([0, 0, 0, 51]);
        assert!((half[3] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn a_plain_name_is_a_chrome_image() {
        let (namespace, name) = split_material("hud_icon");
        assert_eq!(namespace, CHROME_NAMESPACE);
        assert_eq!(name, "hud_icon");
    }

    #[test]
    fn hiding_both_passes_hides_both() {
        let mut pass = LayerRequests::default();
        hide_both(&mut pass);
        assert!(matches!(pass.elems_front, LayerJob::Conceal));
        assert!(matches!(pass.elems_behind, LayerJob::Conceal));
    }

    #[test]
    fn a_point_beyond_the_right_edge_lands_on_the_margin_box() {
        let window = Vec2::new(1000.0, 600.0);
        let (spot, travel) = clamp_to_margin(Vec2::new(1500.0, 300.0), window, 20.0);
        assert!((spot.x - 980.0).abs() < 1e-3, "{spot}");
        assert!((spot.y - 300.0).abs() < 1e-3, "{spot}");
        assert!(travel < 1.0);
    }

    #[test]
    fn a_point_below_the_window_lands_on_the_bottom_margin() {
        let window = Vec2::new(1000.0, 600.0);
        let (spot, _) = clamp_to_margin(Vec2::new(500.0, 2000.0), window, 20.0);
        assert!((spot.y - 580.0).abs() < 1e-3, "{spot}");
        assert!((spot.x - 500.0).abs() < 1e-3, "{spot}");
    }

    #[test]
    fn a_diagonal_point_stops_at_the_nearer_edge() {
        let window = Vec2::new(1000.0, 600.0);
        let (spot, _) = clamp_to_margin(Vec2::new(2500.0, 1500.0), window, 0.0);
        assert!(spot.x <= 1000.0 + 1e-3 && spot.y <= 600.0 + 1e-3, "{spot}");
        assert!((spot.x - 1000.0).abs() < 1e-3 || (spot.y - 600.0).abs() < 1e-3, "{spot}");
    }

    #[test]
    fn a_tess_job_for_nothing_is_hidden() {
        assert!(matches!(job_for(Vec::new()), LayerJob::Conceal));
    }

    #[test]
    fn a_sighting_inside_the_window_is_on_screen() {
        // No camera is available in a unit test, so the window test is pinned through
        // the clamp that the off-screen arrow relies on.
        let window = Vec2::new(640.0, 480.0);
        let (spot, travel) = clamp_to_margin(Vec2::new(320.0, 240.0), window, 10.0);
        assert!(spot.abs_diff_eq(Vec2::new(320.0, 240.0), 1e-3), "{spot}");
        assert!(travel > 1.0);
    }
}
