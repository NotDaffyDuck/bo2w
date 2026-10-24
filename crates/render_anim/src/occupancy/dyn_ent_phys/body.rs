use super::*;

pub(super) fn advance_in_world(entity: Entity, body: &mut PhysBody, clip: &ClipCollision, dt: f32) {
    body.vel.z += GRAVITY * dt;
    if !body.noted_fast && body.vel.length() > FAST_NOTE && hit_log_on() {
        body.noted_fast = true;
        diag::info!(
            World,
            "dyn ent piece {entity:?} fast {:?} at {:?}",
            body.vel,
            body.origin
        );
    }
    // How far to raise a piece to clear the surface under it.
    let rise = if body.mins[2] < 0.0 {
        -body.mins[2]
    } else {
        1.0
    };
    let (mins, maxs) = (body.mins, body.maxs);
    let sweep = |from: Vec3, to: Vec3| {
        clip.sweep_box(from.to_array(), to.to_array(), mins, maxs, PIECE_CLIP_MASK)
    };

    let mut from = body.origin;
    let mut to = from + body.vel * dt;
    let mut hit = sweep(from, to);
    if hit.startsolid {
        // The piece begins inside geometry (shelves can be authored that
        // way). Lift it first, then look outward in growing rings for the
        // nearest open spot, as a contact solver would push it out.
        let open = |at: Vec3| !sweep(at, at).startsolid;
        let mut shift = open(from + Vec3::Z * rise).then_some(Vec3::Z * rise);
        let axes = [
            Vec3::Z,
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::NEG_Z,
        ];
        let mut reach = ESCAPE_STEP;
        while shift.is_none() && reach <= ESCAPE_REACH {
            shift = axes
                .iter()
                .map(|axis| *axis * reach)
                .find(|s| open(from + *s));
            reach += ESCAPE_STEP;
        }
        if let Some(shift) = shift {
            if shift.length() > 8.0 && hit_log_on() {
                diag::info!(
                    World,
                    "dyn ent piece {entity:?} pushed out {shift:?} from {from:?}"
                );
            }
            from += shift;
            to += shift;
            body.origin = from;
            hit = sweep(from, to);
        }
    }
    if hit.allsolid || hit.startsolid {
        // No way out: stop, and keep it from settling inside the wall.
        if !body.noted_stuck && hit_log_on() {
            body.noted_stuck = true;
            diag::info!(
                World,
                "dyn ent piece {entity:?} stuck inside clip at {:?}",
                body.origin
            );
        }
        body.vel = Vec3::ZERO;
        body.grounded = false;
        body.origin.z += rise;
        return;
    }
    if hit.fraction >= 1.0 {
        body.origin = to;
        body.grounded = false;
        return;
    }
    let normal = Vec3::from_array(hit.normal);
    body.origin = Vec3::from_array(hit.endpos) + normal * SKIN;
    body.vel = contact_response(body.vel, normal, body.bounce, body.friction, dt);
    body.grounded = normal.z > GROUND_NORMAL_Z;
}
