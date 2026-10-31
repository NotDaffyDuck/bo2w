//! bo2mp probe: print BO2 vehicle defs (VehicleDef) whose name holds
//! VEH_GREP: type, remote control, speeds, boost, button names, steering.
//! cargo run -p asset_t6 --example vehicle_dump -- ZONE...
use asset_transport::T6ZoneMemory;
use asset_transport::mount_zone;
use fastfile_t6::layout::{
    BrushWrapper, Collmap, PhysGeomInfo, PhysGeomList, VehicleDef, VehicleParameter, XModel,
};
use fastfile_t6::{XAssetKind, Loaded, RawRef, WalkSink, StreamRef, ZoneByteReader};

struct Sink {
    grep: String,
    /// MODEL_GREP: also print matching models' bounds and first collmap.
    model_grep: String,
}

fn deref(s: &ZoneByteReader<'_>, p: RawRef, field_at: usize) -> Option<RawRef> {
    match s.address_for(p, field_at).ok()? {
        StreamRef::At(q) => Some(s.follow_entry_link(q)),
        _ => None,
    }
}

fn v3(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> [f32; 3] {
    [0, 4, 8].map(|k| s.float_from_bytes(p, off + k).unwrap_or(f32::NAN))
}

fn dump_model(s: &ZoneByteReader<'_>, x: RawRef, grep: &str) -> fastfile_t6::Result<()> {
    let name = text(s, x, XModel::name);
    if !name.to_ascii_lowercase().contains(grep) {
        return Ok(());
    }
    let n = s.u8_at(x, XModel::numCollmaps).unwrap_or(0);
    println!("model {name}: mins {:?} maxs {:?} radius {} collmaps {n}", v3(s, x, XModel::mins), v3(s, x, XModel::maxs), s.float_from_bytes(x, XModel::radius).unwrap_or(f32::NAN));
    let Some(arr) = deref(s, x, XModel::collmaps) else { return Ok(()) };
    for c in 0..usize::from(n) {
        let Some(list) = deref(s, arr.shifted_by(c * Collmap::SIZE), Collmap::geomList) else { continue };
        let count: usize = s.uint_from_bytes(list, PhysGeomList::count).unwrap_or(0).min(64) as usize;
        println!("  collmap {c}: count {count} contents {:#x}", s.uint_from_bytes(list, PhysGeomList::contents).unwrap_or(0));
        let Some(geoms) = deref(s, list, PhysGeomList::geoms) else { continue };
        for j in 0..count {
            let g = geoms.shifted_by(j * PhysGeomInfo::SIZE);
            let o = PhysGeomInfo::orientation;
            println!("    piece {j}: type {} axes {:?} {:?} {:?} offset {:?} half {:?}", s.int_from_bytes(g, PhysGeomInfo::r#type).unwrap_or(-1),
                v3(s, g, o), v3(s, g, o + 12), v3(s, g, o + 24), v3(s, g, PhysGeomInfo::offset), v3(s, g, PhysGeomInfo::halfLengths));
            if let Some(b) = deref(s, g, PhysGeomInfo::brush) {
                let nv = s.uint_from_bytes(b, BrushWrapper::numverts).unwrap_or(0).min(64) as usize;
                println!("      brush mins {:?} maxs {:?} numsides {} numverts {nv} contents {:#x}", v3(s, b, BrushWrapper::mins), v3(s, b, BrushWrapper::maxs),
                    s.uint_from_bytes(b, BrushWrapper::numsides).unwrap_or(0), s.uint_from_bytes(b, BrushWrapper::contents).unwrap_or(0));
                if let Some(vs) = deref(s, b, BrushWrapper::vertex_points) {
                    for k in 0..nv {
                        println!("        vert {k}: {:?}", v3(s, vs, k * 12));
                    }
                }
            }
        }
    }
    Ok(())
}

fn text(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> String {
    match s.address_for(p, off).ok() {
        Some(StreamRef::At(q)) => s.zero_ended_bytes(s.follow_entry_link(q)).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default(),
        _ => String::new(),
    }
}

impl WalkSink for Sink {
    fn asset_loaded(&mut self, s: &ZoneByteReader<'_>, loaded: Loaded) -> fastfile_t6::Result<()> {
        if loaded.ty == XAssetKind::XModel && !self.model_grep.is_empty() {
            return dump_model(s, loaded.body, &self.model_grep);
        }
        if loaded.ty != XAssetKind::VehicleDef {
            return Ok(());
        }
        let v = loaded.body;
        let name = text(s, v, VehicleDef::name);
        if !name.to_ascii_lowercase().contains(&self.grep) {
            return Ok(());
        }
        let f = |o: usize| s.float_from_bytes(v, o).unwrap_or(f32::NAN);
        let i = |o: usize| s.int_from_bytes(v, o).unwrap_or(-1);
        println!("vehicle {name}: type {} remoteControl {} isNitrous {} cameraMode {}", i(VehicleDef::r#type), i(VehicleDef::remoteControl), i(VehicleDef::isNitrous), i(VehicleDef::cameraMode));
        println!("  maxSpeed {} accel {} rotAccel {} collisionDamage {} collisionSpeed {} skidSpeedMin {}", f(VehicleDef::maxSpeed), f(VehicleDef::speed_up_rate), f(VehicleDef::rotAccel), f(VehicleDef::collisionDamage), f(VehicleDef::collisionSpeed), f(VehicleDef::skidSpeedMin));
        println!("  rotRate {} maxBodyPitch {} maxBodyRoll {} tiltFromAcceleration {} {} tiltSpeed {} {}", f(VehicleDef::rotRate), f(VehicleDef::maxBodyPitch), f(VehicleDef::maxBodyRoll), f(VehicleDef::tiltFromAcceleration), f(VehicleDef::tiltFromAcceleration + 4), f(VehicleDef::tiltSpeed), f(VehicleDef::tiltSpeed + 4));
        println!("  boost accelMult {} duration {} speedIncrease {}", f(VehicleDef::boostAccelMultiplier), f(VehicleDef::boostDuration), f(VehicleDef::boostSpeedIncrease));
        println!("  third person range {} pitch clamp {} {} height {} {} pitch {} {} spring dist {} time {} handbrakeTurnRateInc {} autoRecenterOnAccel {} cameraAlwaysAutoCenter {} lerp {} max {} fov {}",
            f(VehicleDef::thirdPersonCameraRange), f(VehicleDef::thirdPersonCameraMinPitchClamp), f(VehicleDef::thirdPersonCameraMaxPitchClamp),
            f(VehicleDef::thirdPersonCameraHeight), f(VehicleDef::thirdPersonCameraHeight + 4), f(VehicleDef::thirdPersonCameraPitch), f(VehicleDef::thirdPersonCameraPitch + 4),
            f(VehicleDef::thirdPersonCameraSpringDistance), f(VehicleDef::thirdPersonCameraSpringTime), f(VehicleDef::thirdPersonCameraHandbrakeTurnRateInc),
            i(VehicleDef::autoRecenterOnAccel), i(VehicleDef::cameraAlwaysAutoCenter), f(VehicleDef::cameraAutoCenterLerpRate), f(VehicleDef::cameraAutoCenterMaxLerpRate), f(VehicleDef::cameraFOV));
        for (label, off) in [
            ("gas", VehicleDef::gasButtonName), ("reverseBrake", VehicleDef::reverseBrakeButtonName), ("handBrake", VehicleDef::handBrakeButtonName),
            ("attack", VehicleDef::attackButtonName), ("attack2", VehicleDef::attackSecondaryButtonName), ("boost", VehicleDef::boostButtonName),
            ("moveUp", VehicleDef::moveUpButtonName), ("moveDown", VehicleDef::moveDownButtonName), ("switchSeat", VehicleDef::switchSeatButtonName),
            ("steerGraph", VehicleDef::steerGraphName), ("accelGraph", VehicleDef::accelGraphName),
        ] {
            println!("  {label}: {:?} ({})", text(s, v, off), i(off + 4));
        }
        println!("  heli maxSpeedVertical {} accelVertical {} rotRate {} maxBodyPitch {} maxBodyRoll {} tiltFromAccel {} {} tiltFromDecel {} {} tiltFromVel {} {} tiltSpeed {} {}",
            f(VehicleDef::maxSpeedVertical), f(VehicleDef::accelVertical), f(VehicleDef::rotRate), f(VehicleDef::maxBodyPitch), f(VehicleDef::maxBodyRoll),
            f(VehicleDef::tiltFromAcceleration), f(VehicleDef::tiltFromAcceleration + 4), f(VehicleDef::tiltFromDeceleration), f(VehicleDef::tiltFromDeceleration + 4),
            f(VehicleDef::tiltFromVelocity), f(VehicleDef::tiltFromVelocity + 4), f(VehicleDef::tiltSpeed), f(VehicleDef::tiltSpeed + 4));
        println!("  heli useCollmap {} radius {} minHeight {} maxHeight {} boundsRadius {} decel fwd {} side {} up {} tiltFromAxes {} tiltFwdYaw {} velAtMaxTilt {} momentum {} quadRotor {} accelTwardsView {}",
            i(VehicleDef::useCollmap), f(VehicleDef::radius), f(VehicleDef::minHeight), f(VehicleDef::maxHeight), f(VehicleDef::vehHelicopterBoundsRadius),
            f(VehicleDef::vehHelicopterDecelerationFwd), f(VehicleDef::vehHelicopterDecelerationSide), f(VehicleDef::vehHelicopterDecelerationUp),
            f(VehicleDef::vehHelicopterTiltFromControllerAxes), f(VehicleDef::vehHelicopterTiltFromFwdAndYaw), f(VehicleDef::vehHelicopterTiltFromFwdAndYaw_VelAtMaxTilt),
            f(VehicleDef::vehHelicopterTiltMomentum), i(VehicleDef::vehHelicopterQuadRotor), i(VehicleDef::vehHelicopterAccelTwardsView));
        println!("  heli maintainHeight {} maxHeight {} limit {} accel {} minimum {} maximum {} ceilingMinimum {}",
            i(VehicleDef::vehHelicopterMaintainHeight), f(VehicleDef::vehHelicopterMaintainMaxHeight), f(VehicleDef::vehHelicopterMaintainHeightLimit),
            f(VehicleDef::vehHelicopterMaintainHeightAccel), f(VehicleDef::vehHelicopterMaintainHeightMinimum), f(VehicleDef::vehHelicopterMaintainHeightMaximum),
            f(VehicleDef::vehHelicopterMaintainCeilingMinimum));
        let n = VehicleDef::nitrousVehParams;
        println!("  nitrous speed_max {} accel_max {} reverse {} steer_angle_max {} steer_angle_speed_scale {} steer_speed {} wheel_radius {}",
            f(n + VehicleParameter::m_speed_max), f(n + VehicleParameter::m_accel_max), f(n + VehicleParameter::m_reverse_scale), f(n + VehicleParameter::m_steer_angle_max), f(n + VehicleParameter::m_steer_angle_speed_scale), f(n + VehicleParameter::m_steer_speed), f(n + VehicleParameter::m_wheel_radius));
        println!("  tire fwd {} side {} brake {} handbrake {} damp coast {} brake {} hand {} auto_hand_brake_min_speed {} peel_out {} traction {} mass {} name {:?}",
            f(n + VehicleParameter::m_tire_fric_fwd), f(n + VehicleParameter::m_tire_fric_side), f(n + VehicleParameter::m_tire_fric_brake), f(n + VehicleParameter::m_tire_fric_hand_brake),
            f(n + VehicleParameter::m_tire_damp_coast), f(n + VehicleParameter::m_tire_damp_brake), f(n + VehicleParameter::m_tire_damp_hand), f(n + VehicleParameter::m_auto_hand_brake_min_speed),
            f(n + VehicleParameter::m_peel_out_max_speed), s.int_from_bytes(v, n + VehicleParameter::m_traction_type).unwrap_or(-1), f(n + VehicleParameter::m_body_mass), text(s, v, n + VehicleParameter::m_name));
        Ok(())
    }
}

fn main() {
    let grep = std::env::var("VEH_GREP").unwrap_or_default().to_ascii_lowercase();
    for path in std::env::args().skip(1) {
        let Ok(image) = mount_zone(&path) else { continue };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::read_asset_directory(&mut stream) else { continue };
        let mut sink = Sink { grep: grep.clone(), model_grep: std::env::var("MODEL_GREP").unwrap_or_default().to_ascii_lowercase() };
        println!("== {path}");
        if let Err(e) = fastfile_t6::Walker::new(&mut stream, &mut sink).walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
    }
}
