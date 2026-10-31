//! How hard a hit throws a piece it breaks off.

use super::DamageKind;
use glam::Vec3;
use xmodel_runtime::DebrisPreset;

// Black Ops II's push on a loose piece per unit of hit (`dynEnt_bulletForce`, `dynEnt_explodeForce`), a blast's lift
// (`dynEnt_explodeUpbias`) and the weakest push a blast gives (`dynEnt_explodeMinForce`). The push is an impulse, so
// the speed it gives is the push over the piece's mass.
const BULLET_FORCE: f32 = 0.8;
const EXPLODE_FORCE: f32 = 12.0;
const EXPLODE_UPBIAS: f32 = 0.5;
const EXPLODE_MIN_FORCE: f32 = 0.01;

/// The velocity of a piece knocked off by a hit travelling along `dir` (`None`, as from a script's damage, lets it
/// just drop). The push is scaled by the piece's own preset.
pub(super) fn launch_velocity(preset: &DebrisPreset, kind: DamageKind, way: Option<[f32; 3]>) -> [f32; 3] {
    let raw = way.map(Vec3::from_array);
    let Some(heading) = raw.and_then(Vec3::try_normalize) else {
        return [0.0; 3];
    };
    let push = match kind {
        DamageKind::Explosive | DamageKind::Blast | DamageKind::Splash => {
            // A blast's direction is as long as its falloff, so the length weakens the push.
            let falloff = raw.map_or(1.0, |r| r.length().min(1.0));
            let force = EXPLODE_FORCE * preset.blast_push * falloff;
            if force <= EXPLODE_MIN_FORCE {
                return [0.0; 3];
            }
            let lifted = (heading + Vec3::Z * EXPLODE_UPBIAS).try_normalize().unwrap_or(heading);
            lifted * force
        }
        DamageKind::Bullet | DamageKind::Melee | DamageKind::Impact => heading * BULLET_FORCE * preset.bullet_push,
        DamageKind::Burned | DamageKind::Script => Vec3::ZERO,
    };
    (push / preset.mass.max(1e-4)).to_array()
}
