//! Copying this frame's shader inputs into the frame record the material
//! executor reads.

use render_frame::{MaterialRunFrame, OpenAirProbe};

use super::frame_outputs::{FrameBuildFeed, MaterialFrameFeed};

/// Fill `frame` from the per-frame material inputs. The light tables come
/// from the assembly inputs when those exist for this frame; without them the
/// frame carries no lights and no image-height scale.
pub fn refresh(
    frame: &mut MaterialRunFrame,
    frame_res: &MaterialFrameFeed,
    inputs: Option<&FrameBuildFeed>,
) {
    frame.engine_sources.clone_from(&frame_res.engine_sources);
    frame.eye_origin = frame_res.eye_origin;
    frame.time_float = frame_res.time_float;
    frame.world_to_clip = frame_res.world_to_clip;
    frame.world_to_view = frame_res.world_to_view;
    frame.view_to_clip = frame_res.view_to_clip;
    frame.hand_model_clip_from_level = frame_res.hand_model_clip_from_level;
    frame.hand_near_plane = frame_res.hand_near_plane;
    frame.outdoor = frame_res.outdoor.map(|map| OpenAirProbe {
        image: map.image,
        lookup: map.find_in,
    });

    frame.key_lights.clear();
    frame.attenuation.truncate(0);
    frame.classic_falloff.clear();
    frame.cone_receivers.clear();
    frame.recip_image_height = inputs.and_then(|inputs| inputs.recip_image_height);
    if let Some(in_set) = inputs {
        frame
            .key_lights
            .extend_from_slice(&in_set.key_lights);
        frame.attenuation.extend_from_slice(&in_set.attenuation);
        frame.classic_falloff.extend_from_slice(&in_set.classic_falloff);
        frame.cone_receivers.clone_from(&frame_res.cone_receivers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Mat4, Vec3};

    fn inputs_with_scale(scale: f32) -> FrameBuildFeed {
        FrameBuildFeed {
            recip_image_height: Some(scale),
            ..Default::default()
        }
    }

    #[test]
    fn view_state_is_copied() {
        let res = MaterialFrameFeed {
            eye_origin: Vec3::new(1.0, 2.0, 3.0),
            time_float: 4.5,
            world_to_clip: Some(Mat4::IDENTITY),
            hand_near_plane: Some(0.25),
            ..Default::default()
        };
        let mut frame = MaterialRunFrame::default();
        refresh(&mut frame, &res, None);
        assert_eq!(frame.eye_origin, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(frame.time_float, 4.5);
        assert_eq!(frame.world_to_clip, Some(Mat4::IDENTITY));
        assert_eq!(frame.world_to_view, None);
        assert_eq!(frame.hand_near_plane, Some(0.25));
    }

    #[test]
    fn without_assembly_inputs_the_frame_has_no_lights_or_scale() {
        let mut frame = MaterialRunFrame {
            recip_image_height: Some(1.0),
            cone_receivers: vec![None, None],
            ..Default::default()
        };
        refresh(&mut frame, &MaterialFrameFeed::default(), None);
        assert_eq!(frame.recip_image_height, None);
        assert!(frame.cone_receivers.is_empty());
        assert!(frame.key_lights.is_empty());
    }

    #[test]
    fn assembly_inputs_supply_the_scale() {
        let mut frame = MaterialRunFrame::default();
        refresh(
            &mut frame,
            &MaterialFrameFeed::default(),
            Some(&inputs_with_scale(0.5)),
        );
        assert_eq!(frame.recip_image_height, Some(0.5));
    }

    #[test]
    fn spot_receivers_are_replaced_not_appended() {
        let res = MaterialFrameFeed {
            cone_receivers: vec![None, None, None],
            ..Default::default()
        };
        let mut frame = MaterialRunFrame {
            cone_receivers: vec![None],
            ..Default::default()
        };
        let inputs = FrameBuildFeed::default();
        refresh(&mut frame, &res, Some(&inputs));
        refresh(&mut frame, &res, Some(&inputs));
        assert_eq!(frame.cone_receivers.len(), 3);
    }

    #[test]
    fn outdoor_lookup_is_copied_when_present() {
        let res = MaterialFrameFeed {
            outdoor: Some(super::super::MapOpenair {
                image: None,
                find_in: [7; 16],
            }),
            ..Default::default()
        };
        let mut frame = MaterialRunFrame::default();
        refresh(&mut frame, &res, None);
        assert_eq!(frame.outdoor.map(|o| o.lookup), Some([7; 16]));
    }
}
