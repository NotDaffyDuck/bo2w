//! The small per-tick records a snapshot carries next to the match events: entity events,
//! shotgun pellet effects, broken-off debris, blockers and the random-number debug counters.

use entity_t6::EntityEventKind;
use sim::{
    DebrisRecord, EntityEventPayload, EntityEventRecord, OrientedBlocker, PelletFxRecord,
    RngDebugMeta,
};

use super::codec::{Decoded, Wire, put_text8, refuse, take_text8};
use crate::transport::wire::{WireReader, WireWriter};

impl Wire for EntityEventKind {
    fn put(&self, out: &mut WireWriter) {
        self.0.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let kind = Self(i32::take(input)?);
        if kind.on_the_wire() {
            Ok(kind)
        } else {
            refuse("entity event outside the event table")
        }
    }
}

wire_fields!(EntityEventPayload {
    number,
    other_entity_num,
    attacker_entity_num,
    event_parm,
    weapon,
    correlation,
    pellet,
    hand,
    origin,
    origin2,
    direction,
    surf_type,
    surface_flags,
    simulation_flags,
});

wire_fields!(EntityEventRecord { sequence, tick, audience, event, payload });

wire_fields!(PelletFxRecord {
    attacker,
    weapon,
    correlation,
    pellet,
    hand,
    start,
    end,
    normal,
    surf_type,
    surface_flags,
    flesh_flags,
});

wire_fields!(OrientedBlocker { entnum, origin, axis, mins, maxs });

wire_fields!(RngDebugMeta { root_seed, scheme, spawn_draws, combat_draws, bot_draws });

impl Wire for DebrisRecord {
    fn put(&self, out: &mut WireWriter) {
        put_text8(out, &self.model);
        self.origin.put(out);
        self.rotation.put(out);
        self.velocity.put(out);
        self.mass.put(out);
        self.bounce.put(out);
        self.friction.put(out);
        self.bullet_force_scale.put(out);
        self.explosive_force_scale.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        Ok(Self {
            model: take_text8(input, "debris model is not utf-8")?,
            origin: Wire::take(input)?,
            rotation: Wire::take(input)?,
            velocity: Wire::take(input)?,
            mass: Wire::take(input)?,
            bounce: Wire::take(input)?,
            friction: Wire::take(input)?,
            bullet_force_scale: Wire::take(input)?,
            explosive_force_scale: Wire::take(input)?,
        })
    }
}
