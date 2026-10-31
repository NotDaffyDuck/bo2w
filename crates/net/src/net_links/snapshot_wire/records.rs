//! The small per-tick records a snapshot carries next to the match events: entity events,
//! shotgun pellet effects, broken-off debris, blockers and the random-number debug counters.

use entity_t6::EventCode;
use sim::{
    DebrisRecord, EntityEventPayload, EntityEventRecord, OrientedBlocker, PelletFxRecord,
    RngDebugMeta,
};

use super::codec::{Wire, put_text8, refuse, take_text8};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

impl Wire for EventCode {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.0.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let kind = Self(i32::take(rd)?);
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
    wall_pass_bits,
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
    body_hit_bits,
});

wire_fields!(OrientedBlocker { ent_no, origin, frame, lo, hi, face_bits });

wire_fields!(RngDebugMeta { root_seed, scheme, spawn_draws, combat_draws, bot_draws });

impl Wire for DebrisRecord {
    fn put(&self, wr: &mut ByteSinkWriter) {
        put_text8(wr, &self.mesh);
        self.origin.put(wr);
        self.rotation.put(wr);
        self.velocity.put(wr);
        self.mass.put(wr);
        self.bounce.put(wr);
        self.friction.put(wr);
        self.bullet_force_scale.put(wr);
        self.explosive_force_scale.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        Ok(Self {
            mesh: take_text8(rd, "debris model is not utf-8")?,
            origin: Wire::take(rd)?,
            rotation: Wire::take(rd)?,
            velocity: Wire::take(rd)?,
            mass: Wire::take(rd)?,
            bounce: Wire::take(rd)?,
            friction: Wire::take(rd)?,
            bullet_force_scale: Wire::take(rd)?,
            explosive_force_scale: Wire::take(rd)?,
        })
    }
}
