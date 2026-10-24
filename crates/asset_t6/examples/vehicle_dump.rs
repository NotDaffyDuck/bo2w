//! bo2mp probe: print BO2 vehicle defs (VehicleDef) whose name holds
//! VEH_GREP: type, remote control, speeds, boost, button names, steering.
//! cargo run -p asset_t6 --example vehicle_dump -- ZONE...
use asset_transport::{T6ZoneMemory, open_zone};
use fastfile_t6::layout as l;
use fastfile_t6::{AssetType, Loaded, Ptr, WalkSink, ZonePtr, ZoneStream};

struct Sink {
    grep: String,
    /// MODEL_GREP: also print matching models' bounds and first collmap.
    model: String,
}

fn deref(s: &ZoneStream<'_>, p: Ptr, off: usize) -> Option<Ptr> {
    match s.ptr_at(p, off).ok()? {
        ZonePtr::Offset(q) => Some(s.resolve_alias(q)),
        _ => None,
    }
}

fn v3(s: &ZoneStream<'_>, p: Ptr, off: usize) -> [f32; 3] {
    [0, 4, 8].map(|k| s.f32_at(p, off + k).unwrap_or(f32::NAN))
}

fn dump_model(s: &ZoneStream<'_>, x: Ptr, grep: &str) -> fastfile_t6::Result<()> {
    let name = text(s, x, l::XModel::name);
    if !name.to_ascii_lowercase().contains(grep) {
        return Ok(());
    }
    let n = s.u8_at(x, l::XModel::numCollmaps).unwrap_or(0);
    println!("model {name}: mins {:?} maxs {:?} radius {} collmaps {n}", v3(s, x, l::XModel::mins), v3(s, x, l::XModel::maxs), s.f32_at(x, l::XModel::radius).unwrap_or(f32::NAN));
    let Some(arr) = deref(s, x, l::XModel::collmaps) else { return Ok(()) };
    for c in 0..usize::from(n) {
        let Some(list) = deref(s, arr.at(c * l::Collmap::SIZE), l::Collmap::geomList) else { continue };
        let count = s.u32_at(list, l::PhysGeomList::count).unwrap_or(0).min(64) as usize;
        println!("  collmap {c}: count {count} contents {:#x}", s.u32_at(list, l::PhysGeomList::contents).unwrap_or(0));
        let Some(geoms) = deref(s, list, l::PhysGeomList::geoms) else { continue };
        for j in 0..count {
            let g = geoms.at(j * l::PhysGeomInfo::SIZE);
            let o = l::PhysGeomInfo::orientation;
            println!("    piece {j}: type {} axes {:?} {:?} {:?} offset {:?} half {:?}", s.i32_at(g, l::PhysGeomInfo::r#type).unwrap_or(-1),
                v3(s, g, o), v3(s, g, o + 12), v3(s, g, o + 24), v3(s, g, l::PhysGeomInfo::offset), v3(s, g, l::PhysGeomInfo::halfLengths));
            if let Some(b) = deref(s, g, l::PhysGeomInfo::brush) {
                let nv = s.u32_at(b, l::BrushWrapper::numverts).unwrap_or(0).min(64) as usize;
                println!("      brush mins {:?} maxs {:?} numsides {} numverts {nv} contents {:#x}", v3(s, b, l::BrushWrapper::mins), v3(s, b, l::BrushWrapper::maxs),
                    s.u32_at(b, l::BrushWrapper::numsides).unwrap_or(0), s.u32_at(b, l::BrushWrapper::contents).unwrap_or(0));
                if let Some(vs) = deref(s, b, l::BrushWrapper::verts) {
                    for k in 0..nv {
                        println!("        vert {k}: {:?}", v3(s, vs, k * 12));
                    }
                }
            }
        }
    }
    Ok(())
}

fn text(s: &ZoneStream<'_>, p: Ptr, off: usize) -> String {
    match s.ptr_at(p, off).ok() {
        Some(ZonePtr::Offset(q)) => s.cstr_bytes(s.resolve_alias(q)).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default(),
        _ => String::new(),
    }
}

impl WalkSink for Sink {
    fn asset_loaded(&mut self, s: &ZoneStream<'_>, loaded: Loaded) -> fastfile_t6::Result<()> {
        if loaded.ty == AssetType::XModel && !self.model.is_empty() {
            return dump_model(s, loaded.body, &self.model);
        }
        if loaded.ty != AssetType::VehicleDef {
            return Ok(());
        }
        let v = loaded.body;
        let name = text(s, v, l::VehicleDef::name);
        if !name.to_ascii_lowercase().contains(&self.grep) {
            return Ok(());
        }
        let f = |o: usize| s.f32_at(v, o).unwrap_or(f32::NAN);
        let i = |o: usize| s.i32_at(v, o).unwrap_or(-1);
        use l::VehicleDef as d;
        println!("vehicle {name}: type {} remoteControl {} isNitrous {} cameraMode {}", i(d::r#type), i(d::remoteControl), i(d::isNitrous), i(d::cameraMode));
        println!("  maxSpeed {} accel {} rotAccel {} collisionDamage {} collisionSpeed {} skidSpeedMin {}", f(d::maxSpeed), f(d::accel), f(d::rotAccel), f(d::collisionDamage), f(d::collisionSpeed), f(d::skidSpeedMin));
        println!("  rotRate {} maxBodyPitch {} maxBodyRoll {} tiltFromAcceleration {} {} tiltSpeed {} {}", f(d::rotRate), f(d::maxBodyPitch), f(d::maxBodyRoll), f(d::tiltFromAcceleration), f(d::tiltFromAcceleration + 4), f(d::tiltSpeed), f(d::tiltSpeed + 4));
        println!("  boost accelMult {} duration {} speedIncrease {}", f(d::boostAccelMultiplier), f(d::boostDuration), f(d::boostSpeedIncrease));
        println!("  third person range {} pitch clamp {} {} height {} {} pitch {} {} spring dist {} time {} handbrakeTurnRateInc {} autoRecenterOnAccel {} cameraAlwaysAutoCenter {} lerp {} max {} fov {}",
            f(d::thirdPersonCameraRange), f(d::thirdPersonCameraMinPitchClamp), f(d::thirdPersonCameraMaxPitchClamp),
            f(d::thirdPersonCameraHeight), f(d::thirdPersonCameraHeight + 4), f(d::thirdPersonCameraPitch), f(d::thirdPersonCameraPitch + 4),
            f(d::thirdPersonCameraSpringDistance), f(d::thirdPersonCameraSpringTime), f(d::thirdPersonCameraHandbrakeTurnRateInc),
            i(d::autoRecenterOnAccel), i(d::cameraAlwaysAutoCenter), f(d::cameraAutoCenterLerpRate), f(d::cameraAutoCenterMaxLerpRate), f(d::cameraFOV));
        for (label, off) in [
            ("gas", d::gasButtonName), ("reverseBrake", d::reverseBrakeButtonName), ("handBrake", d::handBrakeButtonName),
            ("attack", d::attackButtonName), ("attack2", d::attackSecondaryButtonName), ("boost", d::boostButtonName),
            ("moveUp", d::moveUpButtonName), ("moveDown", d::moveDownButtonName), ("switchSeat", d::switchSeatButtonName),
            ("steerGraph", d::steerGraphName), ("accelGraph", d::accelGraphName),
        ] {
            println!("  {label}: {:?} ({})", text(s, v, off), i(off + 4));
        }
        println!("  heli maxSpeedVertical {} accelVertical {} rotRate {} maxBodyPitch {} maxBodyRoll {} tiltFromAccel {} {} tiltFromDecel {} {} tiltFromVel {} {} tiltSpeed {} {}",
            f(d::maxSpeedVertical), f(d::accelVertical), f(d::rotRate), f(d::maxBodyPitch), f(d::maxBodyRoll),
            f(d::tiltFromAcceleration), f(d::tiltFromAcceleration + 4), f(d::tiltFromDeceleration), f(d::tiltFromDeceleration + 4),
            f(d::tiltFromVelocity), f(d::tiltFromVelocity + 4), f(d::tiltSpeed), f(d::tiltSpeed + 4));
        println!("  heli useCollmap {} radius {} minHeight {} maxHeight {} boundsRadius {} decel fwd {} side {} up {} tiltFromAxes {} tiltFwdYaw {} velAtMaxTilt {} momentum {} quadRotor {} accelTwardsView {}",
            i(d::useCollmap), f(d::radius), f(d::minHeight), f(d::maxHeight), f(d::vehHelicopterBoundsRadius),
            f(d::vehHelicopterDecelerationFwd), f(d::vehHelicopterDecelerationSide), f(d::vehHelicopterDecelerationUp),
            f(d::vehHelicopterTiltFromControllerAxes), f(d::vehHelicopterTiltFromFwdAndYaw), f(d::vehHelicopterTiltFromFwdAndYaw_VelAtMaxTilt),
            f(d::vehHelicopterTiltMomentum), i(d::vehHelicopterQuadRotor), i(d::vehHelicopterAccelTwardsView));
        println!("  heli maintainHeight {} maxHeight {} limit {} accel {} minimum {} maximum {} ceilingMinimum {}",
            i(d::vehHelicopterMaintainHeight), f(d::vehHelicopterMaintainMaxHeight), f(d::vehHelicopterMaintainHeightLimit),
            f(d::vehHelicopterMaintainHeightAccel), f(d::vehHelicopterMaintainHeightMinimum), f(d::vehHelicopterMaintainHeightMaximum),
            f(d::vehHelicopterMaintainCeilingMinimum));
        let n = d::nitrousVehParams;
        use l::VehicleParameter as p;
        println!("  nitrous speed_max {} accel_max {} reverse {} steer_angle_max {} steer_angle_speed_scale {} steer_speed {} wheel_radius {}",
            f(n + p::m_speed_max), f(n + p::m_accel_max), f(n + p::m_reverse_scale), f(n + p::m_steer_angle_max), f(n + p::m_steer_angle_speed_scale), f(n + p::m_steer_speed), f(n + p::m_wheel_radius));
        println!("  tire fwd {} side {} brake {} handbrake {} damp coast {} brake {} hand {} auto_hand_brake_min_speed {} peel_out {} traction {} mass {} name {:?}",
            f(n + p::m_tire_fric_fwd), f(n + p::m_tire_fric_side), f(n + p::m_tire_fric_brake), f(n + p::m_tire_fric_hand_brake),
            f(n + p::m_tire_damp_coast), f(n + p::m_tire_damp_brake), f(n + p::m_tire_damp_hand), f(n + p::m_auto_hand_brake_min_speed),
            f(n + p::m_peel_out_max_speed), s.i32_at(v, n + p::m_traction_type).unwrap_or(-1), f(n + p::m_body_mass), text(s, v, n + p::m_name));
        Ok(())
    }
}

fn main() {
    let grep = std::env::var("VEH_GREP").unwrap_or_default().to_ascii_lowercase();
    for path in std::env::args().skip(1) {
        let Ok(image) = open_zone(&path) else { continue };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::open_asset_table(&mut stream) else { continue };
        let mut sink = Sink { grep: grep.clone(), model: std::env::var("MODEL_GREP").unwrap_or_default().to_ascii_lowercase() };
        println!("== {path}");
        if let Err(e) = fastfile_t6::Walker::new(&mut stream, &mut sink).walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
    }
}
