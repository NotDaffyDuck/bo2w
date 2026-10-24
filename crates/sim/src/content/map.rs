//! The map part of the content digest: where players may spawn and what the collision brushes
//! are made of.

use super::digest::Fnv1a;
use crate::spawn::AuthoredSpawnPoint;
use crate::world::SimBrush;

pub(super) fn spawn_points(h: &mut Fnv1a, spawns: &[AuthoredSpawnPoint]) {
    h.put_len(spawns.len());
    for spawn in spawns {
        h.put_text(&spawn.classname);
        h.put_all(spawn.origin);
        h.put_all(spawn.angles);
    }
}

pub(super) fn clip_brushes(h: &mut Fnv1a, brushes: &[SimBrush]) {
    h.put_len(brushes.len());
    for brush in brushes {
        h.put_len(brush.planes.len());
        for plane in &brush.planes {
            h.put_all(*plane);
        }
        h.put(brush.contents);
        h.put_len(brush.plane_surface_flags.len());
        h.put_all(brush.plane_surface_flags.iter().copied());
        h.put(brush.glass_encoded);
    }
}
