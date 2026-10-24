use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use render_gpu::drawsurf::{Ui3dFrame, Ui3dQuad};

/// Where each player's head is, from the pose the animation side settled on,
/// for the overhead names.
pub(super) fn publish_overhead_posed_players(
    posed: Res<render_anim::PosedPlayerFrame>,
    target: Option<ResMut<hud::OverheadPosedPlayerFrame>>,
) {
    let Some(mut target) = target else {
        return;
    };
    target.replace(posed.iter().map(|player| {
        let head = match player.head {
            render_anim::PosedPlayerHead::Exact(world) => {
                hud::OverheadPosedHead::ExactWorld(world.to_array())
            }
            render_anim::PosedPlayerHead::NoDObjOrHead => hud::OverheadPosedHead::NoDObjOrHead,
        };
        (player.entnum, head)
    }));
}

/// The Target Finder's sight screen, into the UI3D texture: the HUD's enemy
/// spots placed through the held gun's `tag_hud`, framed by the rangefinder
/// and drawn later by the GPU side. With the screen off there are no quads and
/// the texture stays cleared.
#[allow(clippy::too_many_arguments)]
pub(super) fn publish_ui3d_frame(
    spots: Option<Res<hud::RangefinderSpots>>,
    bolts: Res<render_anim::FpvBoltTargets>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    icons: Option<Res<assets::T6HudIcons>>,
    mut images: ResMut<Assets<Image>>,
    mut atlas: Local<Option<Handle<Image>>>,
    mut published: ResMut<Ui3dFrame>,
) {
    use hud_t6::rangefinder as sight;

    let quads: Vec<Ui3dQuad> = match spots.as_deref().filter(|spots| spots.draws_screen) {
        None => Vec::new(),
        Some(spots) => {
            let eye = cameras
                .iter()
                .find(|(camera, _)| camera.is_active)
                .map(|(_, at)| at.translation().to_array());
            let diamonds: Vec<[f32; 4]> = match (bolts.hud, eye) {
                (Some(hud), Some(eye)) if sight::DIAMOND_INDICATOR => {
                    let tag = sight::TagHud {
                        origin: hud.orientation.origin,
                        axis: hud.orientation.axis,
                    };
                    spots
                        .spots
                        .iter()
                        .filter_map(|&(origin, stance)| {
                            sight::diamond_rect(&tag, eye, origin, stance, sight::DIAMOND_SIZE)
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            sight::frame_quads(spots.targeting, &diamonds)
                .into_iter()
                .map(|quad| Ui3dQuad {
                    xy: quad.corners.map(|corner| corner.xy),
                    uv: quad.corners.map(|corner| corner.uv),
                    textured: quad.textured,
                })
                .collect()
        }
    };

    // The backdrop is copied out of the HUD icons once, with usage reset so
    // the render world can take it.
    if atlas.is_none() {
        let icon = icons.as_deref().and_then(|icons| {
            icons
                .0
                .iter()
                .find(|(name, _)| name == sight::BACKGROUND_MATERIAL)
        });
        if let Some((_, picture)) = icon {
            let mut picture = (**picture).clone();
            picture.asset_usage = RenderAssetUsages::default();
            *atlas = Some(images.add(picture));
        }
    }

    let next = Ui3dFrame {
        quads,
        atlas: atlas.clone(),
    };
    if *published != next {
        *published = next;
    }
}
