//! The map part of the content digest: where players may spawn and what the collision brushes
//! are made of.

use super::digest::Fnv1a;
use crate::spawn_pick::MapSpawnSpot;
use crate::sim_parts::SolidBrush;

pub(super) fn spawn_points(h: &mut Fnv1a, spawns: &[MapSpawnSpot]) {
    h.put_len(spawns.len());
    for point in spawns {
        h.put_text(&point.kind_name);
        h.put_all(point.origin);
        h.put_all(point.turn);
    }
}

pub(super) fn collision_solids(h: &mut Fnv1a, brushes: &[SolidBrush]) {
    h.put_len(brushes.len());
    for solid in brushes {
        h.put_len(solid.sides.len());
        solid.sides.iter().for_each(|&plane| h.put_all(plane));
        h.put(solid.solid_bits);
        h.put_len(solid.plane_material_flags.len());
        h.put_all(solid.plane_material_flags.iter().copied());
        h.put(solid.glass_hit_code);
    }
}
