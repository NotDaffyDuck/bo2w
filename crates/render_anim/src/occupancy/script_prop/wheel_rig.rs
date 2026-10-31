//! bo2zm: the wheels of any wheeled model (a bus, a truck), turned from how it
//! moves. The RC-XD's own rig (`VehicleWheels`, in the parent module) takes a
//! model that has its wheel bones; this one takes the rest, from their wheel
//! tags alone.

use bevy::prelude::*;

/// A vehicle's wheel tags, in BO2's names; the front ones steer.
const WHEEL_TAGS: [(&str, bool); 6] = [
    ("tag_wheel_front_left", true),
    ("tag_wheel_front_right", true),
    ("tag_wheel_middle_left", false),
    ("tag_wheel_middle_right", false),
    ("tag_wheel_back_left", false),
    ("tag_wheel_back_right", false),
];

/// A wheel's radius when its tag's height above the model's origin is no
/// size for a wheel (units).
const WHEEL_RADIUS_FALLBACK: f32 = 20.0;

/// Front to back axle when the model has wheels at one end only (units).
const WHEELBASE_FALLBACK: f32 = 200.0;

/// How far the front wheels turn at most (radians, 35 degrees: a stand-in
/// until the vehicle's own number is read).
const WHEEL_MAX_STEER: f32 = 0.61;

/// How much of the way to the wanted steer the wheels go each frame.
const WHEEL_STEER_EASE: f32 = 0.25;

/// A move this long in one frame is a jump, not a roll (units).
const WHEEL_MAX_STEP: f32 = 256.0;

/// The wheels of a drawn model: each wheel tag, the bones under it, and
/// whether it steers.
pub(super) struct WheelRig {
    wheels: Vec<(usize, Vec<usize>, bool)>,
    rad: f32,
    wheelbase: f32,
}

/// Where a wheeled model was last frame and how far its wheels have turned
/// (radians).
#[derive(Default)]
pub(super) struct WheelTrack {
    last: Option<(Vec3, Quat)>,
    spin: f32,
    steer: f32,
}

impl WheelTrack {
    /// BO2 turns a vehicle's wheels on the client
    /// (CG_Vehicle_DoControllersGround): each spins by the ground it rolled
    /// over, the front ones steer into the turn.
    pub(super) fn roll(&mut self, rig: &WheelRig, at: Vec3, rotation: Quat) {
        if let Some((last_at, last_rotation)) = self.last {
            let moved = last_rotation.inverse() * (at - last_at);
            if moved.length() < WHEEL_MAX_STEP {
                let forward = moved.x;
                self.spin = (self.spin + forward / rig.rad).rem_euclid(std::f32::consts::TAU);
                if forward.abs() > 0.5 {
                    let (turn, _, _) = (last_rotation.inverse() * rotation).to_euler(EulerRot::ZYX);
                    let want = (rig.wheelbase * turn / forward)
                        .atan()
                        .clamp(-WHEEL_MAX_STEER, WHEEL_MAX_STEER);
                    self.steer += (want - self.steer) * WHEEL_STEER_EASE;
                }
            }
        }
        self.last = Some((at, rotation));
    }

    /// The wheel angles ride in the pose revision of the cache key, so a
    /// turned wheel re-poses the model in both caches (the pose and the draw
    /// plan). `revision` is the state's own.
    pub(super) fn keyed_revision(&self, revision: u32) -> u32 {
        let spin = (self.spin / std::f32::consts::TAU * 256.0) as u32 & 0xff;
        let steer = ((self.steer.to_degrees() * 2.0).round() as i32 + 128) as u32 & 0xff;
        revision.rotate_left(16) ^ (1u32 << 31 | spin << 8 | steer)
    }
}

/// The wheels of `dobj`, `None` for a model with none.
pub(super) fn wheel_rig(dobj: &xmodel_runtime::PosedSkeleton) -> Option<WheelRig> {
    let wheels: Vec<_> = WHEEL_TAGS
        .iter()
        .filter_map(|&(tag, steers)| {
            let wheel = dobj.find(tag)?;
            let under: Vec<usize> = (0..dobj.bones.len())
                .filter(|&bone| dobj.bones[bone].up == Some(wheel))
                .collect::<_>();
            Some((wheel, under, steers))
        })
        .collect::<_>();
    if wheels.is_empty() {
        return None;
    }
    let at = |bone: usize| {
        dobj.bones[bone]
            .rest_world_matrix
            .w_axis
            .truncate()
    };
    let height = wheels.iter().map(|w| at(w.0).z).sum::<f32>() / wheels.len() as f32;
    let radius = if (8.0..=60.0).contains(&height) {
        height
    } else {
        WHEEL_RADIUS_FALLBACK
    };
    let axle = |front: bool| {
        let xs: Vec<f32> = wheels
            .iter()
            .filter(|w| w.2 == front)
            .map(|w| at(w.0).x)
            .collect::<_>();
        (!xs.is_empty()).then(|| xs.iter().sum::<f32>() / xs.len() as f32)
    };
    let wheelbase = match (axle(true), axle(false)) {
        (Some(front), Some(back)) => (front - back).abs().max(1.0),
        _ => WHEELBASE_FALLBACK,
    };
    Some(WheelRig {
        wheels,
        rad: radius,
        wheelbase,
    })
}

/// BO2's wheel controller: the wheel tag steers, the bones under it spin
/// (the tag itself when nothing hangs under it).
pub(super) fn turn_wheels(
    dobj: &xmodel_runtime::PosedSkeleton,
    locals: &mut [dobj_t6::PartPose],
    rig: &WheelRig,
    track: &WheelTrack,
) {
    for (wheel, under, steers) in &rig.wheels {
        if *steers {
            turn_bone(dobj, locals, *wheel, Vec3::Z, track.steer);
        }
        if under.is_empty() {
            turn_bone(dobj, locals, *wheel, Vec3::Y, track.spin);
        }
        for &bone in under {
            turn_bone(dobj, locals, bone, Vec3::Y, track.spin);
        }
    }
}

/// Turns a bone about one of the model's axes, as it sits at bind.
fn turn_bone(
    dobj: &xmodel_runtime::PosedSkeleton,
    locals: &mut [dobj_t6::PartPose],
    bone_ix: usize,
    model_axis: Vec3,
    angle: f32,
) {
    let Some(bind) = dobj.bones.get(bone_ix) else {
        return;
    };
    let Some(local) = locals.get_mut(bone_ix) else {
        return;
    };
    let (_, rest_orientation, _) = bind.rest_world_matrix.to_scale_rotation_translation();
    let axis_in = (rest_orientation.inverse() * model_axis).normalize_or_zero();
    if axis_in == Vec3::ZERO {
        return;
    }
    let out = (Quat::from_array(local.rotation) * Quat::from_axis_angle(axis_in, angle)).normalize();
    local.rotation = out.to_array();
}
