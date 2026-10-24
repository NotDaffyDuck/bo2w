//! Copying this frame's shader inputs into the frame record the material
//! executor reads.

use render_frame::{MaterialExecFrame, OutdoorLookup};

use super::frame_products::{FrameAssemblyInputs, MaterialFrameInputs};

/// Fill `frame` from the per-frame material inputs. The light tables come
/// from the assembly inputs when those exist for this frame; without them the
/// frame carries no lights and no image-height scale.
pub fn refresh(
    frame: &mut MaterialExecFrame,
    res: &MaterialFrameInputs,
    inputs: Option<&FrameAssemblyInputs>,
) {
    frame.code_sources.clone_from(&res.code_sources);
    frame.view_origin = res.view_origin;
    frame.float_time = res.float_time;
    frame.clip_from_world = res.clip_from_world;
    frame.view_from_world = res.view_from_world;
    frame.clip_from_view = res.clip_from_view;
    frame.viewmodel_clip_from_world = res.viewmodel_clip_from_world;
    frame.viewmodel_near = res.viewmodel_near;
    frame.outdoor = res.outdoor.map(|map| OutdoorLookup {
        image: map.image,
        lookup: map.lookup,
    });

    frame.primary_lights.clear();
    frame.attenuation.clear();
    frame.t5_falloff.clear();
    frame.spot_receivers.clear();
    frame.inv_image_height = inputs.and_then(|inputs| inputs.inv_image_height);
    if let Some(inputs) = inputs {
        frame
            .primary_lights
            .extend_from_slice(&inputs.primary_lights);
        frame.attenuation.extend_from_slice(&inputs.attenuation);
        frame.t5_falloff.extend_from_slice(&inputs.t5_falloff);
        frame.spot_receivers.clone_from(&res.spot_receivers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Mat4, Vec3};

    fn inputs_with_scale(scale: f32) -> FrameAssemblyInputs {
        FrameAssemblyInputs {
            inv_image_height: Some(scale),
            ..Default::default()
        }
    }

    #[test]
    fn view_state_is_copied() {
        let res = MaterialFrameInputs {
            view_origin: Vec3::new(1.0, 2.0, 3.0),
            float_time: 4.5,
            clip_from_world: Some(Mat4::IDENTITY),
            viewmodel_near: Some(0.25),
            ..Default::default()
        };
        let mut frame = MaterialExecFrame::default();
        refresh(&mut frame, &res, None);
        assert_eq!(frame.view_origin, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(frame.float_time, 4.5);
        assert_eq!(frame.clip_from_world, Some(Mat4::IDENTITY));
        assert_eq!(frame.view_from_world, None);
        assert_eq!(frame.viewmodel_near, Some(0.25));
    }

    #[test]
    fn without_assembly_inputs_the_frame_has_no_lights_or_scale() {
        let mut frame = MaterialExecFrame {
            inv_image_height: Some(1.0),
            spot_receivers: vec![None, None],
            ..Default::default()
        };
        refresh(&mut frame, &MaterialFrameInputs::default(), None);
        assert_eq!(frame.inv_image_height, None);
        assert!(frame.spot_receivers.is_empty());
        assert!(frame.primary_lights.is_empty());
    }

    #[test]
    fn assembly_inputs_supply_the_scale() {
        let mut frame = MaterialExecFrame::default();
        refresh(
            &mut frame,
            &MaterialFrameInputs::default(),
            Some(&inputs_with_scale(0.5)),
        );
        assert_eq!(frame.inv_image_height, Some(0.5));
    }

    #[test]
    fn spot_receivers_are_replaced_not_appended() {
        let res = MaterialFrameInputs {
            spot_receivers: vec![None, None, None],
            ..Default::default()
        };
        let mut frame = MaterialExecFrame {
            spot_receivers: vec![None],
            ..Default::default()
        };
        let inputs = FrameAssemblyInputs::default();
        refresh(&mut frame, &res, Some(&inputs));
        refresh(&mut frame, &res, Some(&inputs));
        assert_eq!(frame.spot_receivers.len(), 3);
    }

    #[test]
    fn outdoor_lookup_is_copied_when_present() {
        let res = MaterialFrameInputs {
            outdoor: Some(super::super::MapOutdoor {
                image: None,
                lookup: [7; 16],
            }),
            ..Default::default()
        };
        let mut frame = MaterialExecFrame::default();
        refresh(&mut frame, &res, None);
        assert_eq!(frame.outdoor.map(|o| o.lookup), Some([7; 16]));
    }
}
