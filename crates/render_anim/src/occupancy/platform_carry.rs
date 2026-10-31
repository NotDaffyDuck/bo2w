//! bo2zm: riding a moving platform (the Tranzit bus) looks smooth.
//!
//! The server carries a rider with the bus each tick, so his place and his
//! turn arrive 20 times a second while the bus itself is drawn gliding on
//! its path. While he stands in the bus, his view keeps his place in the
//! bus as the server last sent it, and moves it with the bus as drawn.

use std::sync::OnceLock;

use bevy::prelude::*;
use net::{FrameClock, HostViewClient, ShownWorldFrame};
use render_scene::LevelScriptModelInstance;
use diag::info;

/// Where the bus the local player rides is: as the latest snapshot has it
/// (`snap_*`) and as it is drawn this frame (`drawn_*`). None when he
/// rides nothing.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PlatformCarry(pub Option<PlatformCarryPose>);

#[derive(Clone, Copy, Debug)]
pub struct PlatformCarryPose {
    pub snap_origin: [f32; 3],
    pub snap_angles: [f32; 3],
    pub drawn_origin: [f32; 3],
    pub drawn_angles: [f32; 3],
}

fn rotation(angles: [f32; 3]) -> Quat {
    // Stored as [tilt, turn, bank] in degrees; applied turn first, then tilt, then bank.
    let [tilt, turn, bank] = angles.map(f32::to_radians);
    Quat::from_euler(EulerRot::ZYX, turn, tilt, bank)
}

impl PlatformCarryPose {
    /// A world place he holds in the snapshot's bus, moved to the drawn bus.
    pub fn carry_point(&self, point_in: [f32; 3]) -> [f32; 3] {
        let local = rotation(self.snap_angles).inverse()
            * (Vec3::from_array(point_in) - Vec3::from_array(self.snap_origin));
        (Vec3::from_array(self.drawn_origin) + rotation(self.drawn_angles) * local).to_array()
    }

    /// How far the drawn bus has turned past the snapshot's.
    pub fn yaw_lead(&self) -> f32 {
        game_math::degree_gap(self.drawn_angles[1], self.snap_angles[1])
    }
}

pub fn track_platform_carry(
    clk: Res<FrameClock>,
    shown: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    slots: Option<Res<net::EntityTableCells>>,
    pose_states: Query<&net::ClientEntityPoseState>,
    owners_in: Query<&LevelScriptModelInstance>,
    mut carry: ResMut<PlatformCarry>,
) {
    carry.0 = None;
    static OFF: OnceLock<bool> = OnceLock::new();
    if *OFF.get_or_init(|| std::env::var_os("BO2MP_NO_CARRY").is_some()) {
        return;
    }
    let (Some(slots), Some(snap), Some(pstate)) =
        (slots, shown.snapshot(), shown.player(local.0))
    else {
        return;
    };
    let snap_time = sim::level_time_ms(snap.tick);
    let drawn_time = shown.trajectory_time_ms(if clk.started() {
        clk.time()
    } else {
        snap_time
    });
    for owner in &owners_in {
        let model = &owner.active_mesh.0;
        let ride_box = sim::platform_ride_box(model);
        // A Die Rise car carries whoever stands on its floor or roof: his
        // ground entity is the car.
        let grounded_now = sim::platform_rides_on_ground(model);
        if ride_box.is_none() && !grounded_now {
            continue;
        }
        let Some(number) = owner.game_actor_number else {
            continue;
        };
        let Some(mover) = snap
            .meta
            .script_movers
            .iter()
            .find(|m| m.state.slot_index == i32::from(number))
        else {
            continue;
        };
        let Some(runtime) = slots
            .object_at_index(number)
            .and_then(|entity| pose_states.get(entity).ok())
        else {
            continue;
        };
        // A bus eased from last tick's place stands, at the server, where
        // its easing ends: that is the bus his place was carried with.
        let pos = sim::move_path_of(&mover.state);
        let apos = sim::turn_path_of(&mover.state);
        let end = |tr: &entity_t6::Trajectory| {
            if tr.tr_type == entity_t6::path_kind::STRAIGHT_THEN_STOP {
                tr.began_at.wrapping_add(tr.span_ms)
            } else {
                snap_time
            }
        };
        let pose = PlatformCarryPose {
            snap_origin: entity_t6::evaluate_trajectory(&pos, end(&pos)),
            snap_angles: entity_t6::evaluate_trajectory(&apos, end(&apos)),
            drawn_origin: entity_t6::evaluate_trajectory(&runtime.current.path, drawn_time),
            drawn_angles: entity_t6::evaluate_trajectory(&runtime.current.spin, drawn_time),
        };
        let inside = match ride_box {
            Some((low, high)) => {
                let local_place = rotation(pose.snap_angles).inverse()
                    * (Vec3::from_array(pstate.origin) - Vec3::from_array(pose.snap_origin));
                (0..3).all(|i| local_place[i] >= low[i] && local_place[i] <= high[i])
            }
            None => pstate.standing_on == i32::from(number),
        };
        if inside {
            carry.0 = Some(pose);
            carry_log(&pose, pstate.origin, clk.time());
            carry_trace(&pose, pstate.origin, pstate.look_angles[1], clk.time(), snap.tick.0);
            return;
        }
    }
}

/// bo2zm test aid (BO2MP_CARRY_LOG=1): how far the snapshot bus and the drawn
/// bus are apart while he rides, every 120 frames.
fn carry_log(pose: &PlatformCarryPose, org: [f32; 3], time: i32) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static ON: OnceLock<bool> = OnceLock::new();
    static N: AtomicU32 = AtomicU32::new(0);
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_CARRY_LOG").is_some()) {
        return;
    }
    if N.fetch_add(1, Ordering::Relaxed) % 120 != 0 {
        return;
    }
    let gap = Vec3::from_array(pose.drawn_origin) - Vec3::from_array(pose.snap_origin);
    diag::info!(
        World,
        "carry: t={time} gap={:.1} yaw_lead={:.2} at {:?} -> {:?}",
        gap.length(),
        pose.yaw_lead(),
        org.map(|v| v.round()),
        pose.carry_point(org).map(|v| v.round()),
    );
}

/// bo2zm test aid (BO2MP_CARRY_TRACE=1): every riding frame, up to 4,000:
/// time, snapshot tick, his place, the snapshot bus and the drawn bus.
fn carry_trace(pose: &PlatformCarryPose, origin_in: [f32; 3], yaw: f32, time: i32, beat: u32) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static ON: OnceLock<bool> = OnceLock::new();
    static N: AtomicU32 = AtomicU32::new(0);
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_CARRY_TRACE").is_some()) {
        return;
    }
    if N.fetch_add(1, Ordering::Relaxed) >= 4000 {
        return;
    }
    let r = |v: [f32; 3]| v.map(|x| (x * 10.0).round() / 10.0);
    diag::info!(
        World,
        "ctrace t={time} tick={beat} ps={:?} yaw={yaw:.1} snap={:?} {:.1} drawn={:?} {:.1}",
        r(origin_in),
        r(pose.snap_origin),
        pose.snap_angles[1],
        r(pose.drawn_origin),
        pose.drawn_angles[1],
    );
}

/// bo2zm test aid (BO2MP_CAMERA_JITTER=1): how far the camera strays, frame
/// to frame, from where its last frame's speed would put it (a smooth ride
/// strays little; 20 Hz steps stray a lot; speed is per ms): mean and largest every 600
/// frames, and whether he was riding.
pub fn camera_jitter(time: i32, origin_v: [f32; 3], riding: bool) {
    use std::sync::Mutex;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_CAMERA_JITTER").is_some()) {
        return;
    }
    #[derive(Default)]
    struct Tally {
        last: Option<(i32, Vec3, Vec3)>,
        sum: f32,
        max: f32,
        longest_duration: i32,
        n: u32,
        riding: u32,
    }
    static TALLY: Mutex<Option<Tally>> = Mutex::new(None);
    let mut guard = TALLY.lock().unwrap();
    let t = guard.get_or_insert_with(Tally::default);
    let now = Vec3::from_array(origin_v);
    match t.last {
        Some((previous_stamp, last, speed)) if time > previous_stamp => {
            let dt = (time - previous_stamp) as f32;
            let stray = (now - (last + speed * dt)).length();
            t.sum += stray;
            if stray > 30.0 {
                info!(World, "camera jump: t={time} dt={dt} stray {stray:.1} riding {riding}");
            }
            if stray > t.max {
                t.max = stray;
                t.longest_duration = time;
            }
            t.n += 1;
            t.riding += u32::from(riding);
            if t.n >= 600 {
                diag::info!(
                    World,
                    "camera jitter: mean {:.2} max {:.1} (t={}) riding {}/{}",
                    t.sum / t.n as f32,
                    t.max,
                    t.longest_duration,
                    t.riding,
                    t.n
                );
                t.sum = 0.0;
                t.max = 0.0;
                t.n = 0;
                t.riding = 0;
            }
            t.last = Some((time, now, (now - last) / dt));
        }
        Some((previous_stamp, ..)) if time == previous_stamp => {}
        _ => t.last = Some((time, now, Vec3::ZERO)),
    }
}
