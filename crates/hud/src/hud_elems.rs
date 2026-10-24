//! Script-driven HUD elements (pictures and waypoints), drawn in two passes:
//! the ones behind the rest of the HUD, then the ones in front.

use asset_core::{AssetKey, AssetKind, AssetNamespace};
use bevy::prelude::*;
use hud_t6::font::hudelem_em_px;
use hud_t6::hudelem::{
    HE_TYPE_WAYPOINT, HUDELEM_NO_TARGET_ENT, HudElem, HudElemBody, HudElemPlacement, HudElemViewer,
    hud_elem_body, hud_elem_draw_color, hud_elem_lerp_font_scale, hud_elem_passes_filter,
    hud_elem_placement, sorted_hud_elems,
};
use net::{CEntity, CEntityRuntime, FrameClock, LocalPresentClient, PresentedSnapshot};
use sim::ClientLifecycle;

use crate::draw2d::{Draw2dCmd, Draw2dList, Draw2dOp, Draw2dProvenance, tessellate};
use crate::font_overlay::spawn_overlay;
use crate::gaps::{GapCause, HudPresentationGaps};
use crate::gpu_list::{HudTessPass, TessJob};
use crate::images::{HUD_CHROME_NAMESPACE, HudImages};
use crate::surface::Hud2dSurface;

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
    spawn_overlay(root, HudElemsRaster);
}

pub(crate) fn spawn_hud_elems_back(root: &mut ChildSpawnerCommands) {
    spawn_overlay(root, HudElemsBackRaster);
}

fn hide_both(pass: &mut HudTessPass) {
    pass.hud_elems = TessJob::Hide;
    pass.hud_elems_back = TessJob::Hide;
}

/// Where a waypoint goes and how big it is, plus whether it is the
/// off-screen arrow. Only that arrow (unless the element opts out) is a 2D
/// draw in the game; the on-screen marker is a 3D surface on the render
/// side, so the on-screen case here is a stand-in.
fn place_waypoint(
    elem: &HudElem,
    (camera, transform): (&Camera, &GlobalTransform),
    entities: &Query<(&CEntity, &CEntityRuntime)>,
    surface: &Hud2dSurface,
) -> Option<(Vec2, f32, bool)> {
    let offset = Vec3::new(elem.x, elem.y, elem.z);
    let world = if elem.target_ent_num == HUDELEM_NO_TARGET_ENT {
        offset
    } else {
        let (_, runtime) = entities.iter().find(|(entity, runtime)| {
            i32::from(entity.number()) == elem.target_ent_num && runtime.in_next_snap()
        })?;
        Vec3::from_array(runtime.origin) + offset
    };
    let eye = transform.translation();
    let forward = *transform.forward();
    let depth = (world - eye).dot(forward);
    // The waypoint's value chooses between two draw-flag sets; what the second
    // one means is not pinned down, so a constant size stands in for it.
    let constant_size = elem.value > 0.0;
    let scale = surface.placement().scale_virtual_to_real[1];
    let (width, height) = (surface.width(), surface.height());
    // Behind the eye, mirror the point through it so the arrow points the
    // right way when it is clamped to the window edge.
    let projected = if depth <= 0.0 {
        world - forward * (2.0 * depth - 1.0)
    } else {
        world
    };
    let pixel = camera.world_to_viewport(transform, projected).ok()?;
    let on_screen =
        depth > 0.0 && (0.0..=width).contains(&pixel.x) && (0.0..=height).contains(&pixel.y);

    if !constant_size {
        if !on_screen {
            return None;
        }
        let side = camera
            .world_to_viewport(transform, world + *transform.right() * elem.width as f32)
            .ok()?;
        return Some((pixel, (side - pixel).length(), false));
    }
    let size = WAYPOINT_ICON_SIZE * scale;
    if on_screen {
        return Some((pixel, size, false));
    }
    if hud_elem_body(elem.elem_type, elem.flags) != HudElemBody::OffscreenWaypoint {
        return None;
    }
    // Slide the arrow out from the window centre along the direction of the
    // point until it meets the margin box.
    let centre = Vec2::new(width, height) * 0.5;
    let reach_box = (centre - Vec2::splat(WAYPOINT_EDGE_MARGIN * scale)).max(Vec2::ONE);
    let direction = pixel - centre;
    let travel = (reach_box.x / direction.x.abs().max(f32::EPSILON))
        .min(reach_box.y / direction.y.abs().max(f32::EPSILON));
    Some((centre + direction * travel, size, true))
}

fn unit_color(rgba: [u8; 4]) -> [f32; 4] {
    rgba.map(|channel| f32::from(channel) / 255.0)
}

/// The namespace and name an element's material string refers to; a string
/// that is not a material key is a chrome image by that name.
fn split_material(text: &str) -> (AssetNamespace, String) {
    match AssetKey::parse(text) {
        Ok(key) if key.kind == AssetKind::Material => (key.namespace, key.name),
        _ => (HUD_CHROME_NAMESPACE, text.to_owned()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_hud_elems(
    surface: Res<Hud2dSurface>,
    clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudPresentationGaps>,
    mut pass: ResMut<HudTessPass>,
    view: Option<Res<frame::ViewSubject>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    entities: Query<(&CEntity, &CEntityRuntime)>,
) {
    if !surface.is_ready() {
        return hide_both(&mut pass);
    }
    let Some(snapshot) = presented.snapshot() else {
        return hide_both(&mut pass);
    };
    let meta = &snapshot.meta;
    let Some(client) = meta.for_client(local.0) else {
        return hide_both(&mut pass);
    };

    // The viewer as the game's HUD-element pass sees it. It does not hide
    // anything in a menu. Its "dead but still shown" test is the player-move
    // type being below dead, or remote control; the meta carries neither the
    // move type nor the scope state, so alive and remote missile stand in.
    let remote_controlling = client.remote_missile.is_some();
    let viewer = HudElemViewer {
        dead_visible: client.lifecycle == ClientLifecycle::Alive || remote_controlling,
        in_sniper_scope: false,
        killcam: view.is_some_and(|subject| subject.in_killcam()),
        remote_controlling,
    };
    let elems = sorted_hud_elems(&client.hud_current, &client.hud_archival);
    if elems.is_empty() {
        return hide_both(&mut pass);
    }

    let placement = surface.placement();
    let now = clock.time();
    // Index 0 is the pass behind the rest, index 1 the one in front.
    let mut layers: [Vec<Draw2dCmd>; 2] = Default::default();
    for (cmds, foreground) in layers.iter_mut().zip([false, true]) {
        for (index, elem) in elems.iter().enumerate() {
            if !hud_elem_passes_filter(elem, foreground, &viewer) {
                continue;
            }
            let Some(color) = hud_elem_draw_color(elem, now, None) else {
                continue;
            };
            // Clock and perk elements are not drawn yet. Text elements are not
            // drawn here either: the scripts' text goes to the zombies HUD in
            // `ui`, which leaves pictures and waypoints for this pass.
            let is_waypoint = elem.elem_type == HE_TYPE_WAYPOINT;
            if hud_elem_body(elem.elem_type, elem.flags) != HudElemBody::Material && !is_waypoint {
                continue;
            }
            let (rect, material_index) = if is_waypoint {
                let Some(camera) = cameras.iter().find(|(camera, _)| camera.is_active) else {
                    continue;
                };
                let Some((centre, size, off_screen)) =
                    place_waypoint(elem, camera, &entities, &surface)
                else {
                    continue;
                };
                // Which material the off-screen arrow uses was never read, so
                // it takes the element's own off-screen one when it has one.
                let material_index = if off_screen && elem.offscreen_material_index != 0 {
                    elem.offscreen_material_index
                } else {
                    elem.material_index
                };
                let rect = HudElemPlacement {
                    x: centre.x - size * 0.5,
                    y: centre.y - size * 0.5,
                    w: size,
                    h: size,
                };
                (rect, material_index)
            } else {
                let font_height =
                    hudelem_em_px(elem.font, hud_elem_lerp_font_scale(elem, now), placement) as f32;
                (
                    hud_elem_placement(placement, elem, now, 0.0, font_height),
                    elem.material_index,
                )
            };
            let Some(text) = u8::try_from(material_index)
                .ok()
                .and_then(|slot| sim::name_in_occupied(&meta.hud_materials, slot))
            else {
                gaps.raise(GapCause::HudElemMaterialUnbound { material_index });
                continue;
            };
            let (material_namespace, material) = split_material(text);
            if hud_images
                .get(material_namespace, &material, &mut images)
                .is_none()
            {
                gaps.raise(GapCause::HudElemImageMissing {
                    name: material,
                    miss: hud_images.miss_reason(),
                });
                continue;
            }
            cmds.push(Draw2dCmd {
                material_namespace,
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                s0: 0.0,
                t0: 0.0,
                s1: 1.0,
                t1: 1.0,
                color: unit_color(color),
                material,
                op: Draw2dOp::StretchPic,
                provenance: Draw2dProvenance::HudElem {
                    index: index as i32,
                },
                layer: 1,
            });
        }
    }

    if layers.iter().all(Vec::is_empty) {
        return hide_both(&mut pass);
    }
    let [back, front] = layers.map(|cmds| {
        let (quads, _) = tessellate(&Draw2dList { cmds });
        if quads.is_empty() {
            TessJob::Hide
        } else {
            TessJob::Quads(quads)
        }
    });
    pass.hud_elems_back = back;
    pass.hud_elems = front;
}
