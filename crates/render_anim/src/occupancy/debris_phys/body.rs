use super::*;

pub(super) fn advance_in_world(ent_in: Entity, body_in: &mut PhysBody, solids: &CollisionClipData, dt: f32) {
    body_in.motion_vec.z += GRAVITY * dt;
    if !body_in.noted_fast && body_in.motion_vec.length() > FAST_NOTE && hit_log_on() {
        body_in.noted_fast = true;
        diag::info!(
            World,
            "dyn ent piece {ent_in:?} fast {:?} at {:?}",
            body_in.motion_vec,
            body_in.pos
        );
    }
    // How far to raise a piece to clear the surface under it.
    let rise = if body_in.mins[2] < 0.0 {
        -body_in.mins[2]
    } else {
        1.0
    };
    let (mins, maxs) = (body_in.mins, body_in.maxs);
    let sweep = |from: Vec3, to: Vec3| {
        solids.sweep_box(from.to_array(), to.to_array(), mins, maxs, PIECE_CLIP_MASK)
    };

    let mut from = body_in.pos;
    let mut to = from + body_in.motion_vec * dt;
    let mut hit_in = sweep(from, to);
    if hit_in.began_inside {
        // The piece begins inside geometry (shelves can be authored that
        // way). Lift it first, then look outward in growing rings for the
        // nearest open spot, as a contact solver would push it out.
        let open = |at: Vec3| !sweep(at, at).began_inside;
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
                    "dyn ent piece {ent_in:?} pushed out {shift:?} from {from:?}"
                );
            }
            from += shift;
            to += shift;
            body_in.pos = from;
            hit_in = sweep(from, to);
        }
    }
    if hit_in.fully_embedded || hit_in.began_inside {
        // No way out: stop, and keep it from settling inside the wall.
        if !body_in.noted_stuck && hit_log_on() {
            body_in.noted_stuck = true;
            diag::info!(
                World,
                "dyn ent piece {ent_in:?} stuck inside clip at {:?}",
                body_in.pos
            );
        }
        body_in.motion_vec = Vec3::ZERO;
        body_in.grounded = false;
        body_in.pos.z += rise;
        return;
    }
    if hit_in.fraction >= 1.0 {
        body_in.pos = to;
        body_in.grounded = false;
        return;
    }
    let normal = Vec3::from_array(hit_in.normal);
    body_in.pos = Vec3::from_array(hit_in.finish_point) + normal * SKIN;
    body_in.motion_vec = contact_response(body_in.motion_vec, normal, body_in.rebound, body_in.drag, dt);
    body_in.grounded = normal.z > GROUND_NORMAL_Z;
}
